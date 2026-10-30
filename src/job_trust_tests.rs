use super::*;
use reqwest::StatusCode;
use std::cell::RefCell;
use std::collections::BTreeMap;

const REPOSITORY_ID: u64 = 100;
const FORK_ID: u64 = 200;
const HEAD_SHA: &str = "1111111111111111111111111111111111111111";

#[derive(Default)]
struct FakeLookup {
    run: Option<WorkflowRun>,
    tags: BTreeMap<String, String>,
    tag_lookup_fails: bool,
    pull_requests: Vec<OpenPullRequest>,
    pull_request_queries: RefCell<Vec<String>>,
}

fn api_error() -> GitHubError {
    GitHubError::Api {
        status: StatusCode::FORBIDDEN,
        body: "Resource not accessible by integration".to_string(),
    }
}

impl TrustLookup for FakeLookup {
    fn workflow_run(
        &self,
        _repository: &RepositoryId,
        _run_id: u64,
    ) -> Result<WorkflowRun, GitHubError> {
        self.run.clone().ok_or_else(api_error)
    }

    fn tag_commit_sha(
        &self,
        _repository: &RepositoryId,
        tag: &str,
    ) -> Result<Option<String>, GitHubError> {
        if self.tag_lookup_fails {
            return Err(api_error());
        }
        Ok(self.tags.get(tag).cloned())
    }

    fn open_pull_requests(
        &self,
        _repository: &RepositoryId,
        head_owner: &str,
        head_branch: &str,
    ) -> Result<Vec<OpenPullRequest>, GitHubError> {
        self.pull_request_queries
            .borrow_mut()
            .push(format!("{head_owner}:{head_branch}"));
        Ok(self.pull_requests.clone())
    }
}

fn repository(id: u64, owner: &str) -> RunRepository {
    RunRepository {
        id,
        owner: RepositoryOwner {
            login: owner.to_string(),
        },
    }
}

fn run(event: &str, head_branch: &str, head_repository: RunRepository) -> WorkflowRun {
    WorkflowRun {
        id: 7001,
        event: event.to_string(),
        head_branch: Some(head_branch.to_string()),
        head_sha: HEAD_SHA.to_string(),
        repository: repository(REPOSITORY_ID, "org"),
        head_repository: Some(head_repository),
    }
}

fn pull_request(number: u64, sha: &str, head_repository_id: u64) -> OpenPullRequest {
    OpenPullRequest {
        number,
        head: PullRequestHead {
            sha: sha.to_string(),
            repo: Some(PullRequestRepository {
                id: head_repository_id,
            }),
        },
    }
}

fn classify(lookup: &FakeLookup) -> JobTrust {
    classify_job(
        lookup,
        &RepositoryId::parse("github:org/project").unwrap(),
        &["main".to_string(), "release/*".to_string()],
        7001,
    )
}

#[test]
fn fork_pull_request_from_a_branch_named_main_is_untrusted() {
    let lookup = FakeLookup {
        run: Some(run("pull_request", "main", repository(FORK_ID, "attacker"))),
        pull_requests: vec![pull_request(7, HEAD_SHA, FORK_ID)],
        ..FakeLookup::default()
    };

    let trust = classify(&lookup);

    assert_eq!(trust.event, GitHubEventKind::Untrusted);
    assert_eq!(
        trust.pull_request_check,
        PullRequestCheck::PullRequests(vec![7])
    );
    assert_eq!(*lookup.pull_request_queries.borrow(), ["attacker:main"]);
}

#[test]
fn same_repository_push_to_a_trusted_branch_is_trusted() {
    let lookup = FakeLookup {
        run: Some(run(
            "push",
            "release/2026-05",
            repository(REPOSITORY_ID, "org"),
        )),
        ..FakeLookup::default()
    };

    assert_eq!(
        classify(&lookup),
        JobTrust {
            event: GitHubEventKind::BranchPush,
            branch: "release/2026-05".to_string(),
            pull_request_check: PullRequestCheck::NotNeeded,
        }
    );
}

#[test]
fn confirmed_tag_push_is_a_release() {
    let lookup = FakeLookup {
        run: Some(run("push", "v1.0.0", repository(REPOSITORY_ID, "org"))),
        tags: BTreeMap::from([("v1.0.0".to_string(), HEAD_SHA.to_string())]),
        ..FakeLookup::default()
    };

    let trust = classify(&lookup);

    assert_eq!(trust.event, GitHubEventKind::Release);
    assert_eq!(trust.pull_request_check, PullRequestCheck::NotNeeded);
}

#[test]
fn tag_like_branch_names_are_not_releases_without_a_matching_tag() {
    for (branch, tags) in [
        ("v1.0.0", BTreeMap::new()),
        ("refs/tags/v1.0.0", BTreeMap::new()),
        (
            "v1.0.0",
            BTreeMap::from([("v1.0.0".to_string(), "2".repeat(40))]),
        ),
    ] {
        let lookup = FakeLookup {
            run: Some(run("push", branch, repository(REPOSITORY_ID, "org"))),
            tags,
            ..FakeLookup::default()
        };

        assert_eq!(
            classify(&lookup).event,
            GitHubEventKind::BranchPush,
            "{branch}"
        );
    }
}

#[test]
fn tag_lookup_failure_does_not_confirm_a_release() {
    let lookup = FakeLookup {
        run: Some(run("push", "v1.0.0", repository(REPOSITORY_ID, "org"))),
        tags: BTreeMap::from([("v1.0.0".to_string(), HEAD_SHA.to_string())]),
        tag_lookup_fails: true,
        ..FakeLookup::default()
    };

    assert_eq!(classify(&lookup).event, GitHubEventKind::BranchPush);
}

#[test]
fn run_lookup_failure_is_untrusted_and_blocks_the_job() {
    let trust = classify(&FakeLookup::default());

    assert_eq!(trust.event, GitHubEventKind::Untrusted);
    let reason = trust
        .pull_request_check
        .pull_request_numbers()
        .expect_err("an unverified job must not skip the workflow-risk check");
    assert!(
        reason.contains("could not verify workflow run 7001"),
        "{reason}"
    );
}

#[test]
fn non_push_events_on_trusted_branches_are_untrusted() {
    for event in [
        "pull_request_target",
        "workflow_dispatch",
        "schedule",
        "merge_group",
    ] {
        let lookup = FakeLookup {
            run: Some(run(event, "main", repository(REPOSITORY_ID, "org"))),
            pull_requests: vec![pull_request(9, HEAD_SHA, REPOSITORY_ID)],
            ..FakeLookup::default()
        };

        assert_eq!(
            classify(&lookup).event,
            GitHubEventKind::Untrusted,
            "{event}"
        );
    }
}

#[test]
fn push_from_another_repository_is_untrusted() {
    let lookup = FakeLookup {
        run: Some(run("push", "main", repository(FORK_ID, "attacker"))),
        ..FakeLookup::default()
    };

    assert_eq!(classify(&lookup).event, GitHubEventKind::Untrusted);
}

#[test]
fn pull_request_runs_without_a_matching_open_pull_request_are_blocked() {
    for pull_requests in [
        Vec::new(),
        vec![pull_request(7, &"2".repeat(40), FORK_ID)],
        vec![pull_request(7, HEAD_SHA, REPOSITORY_ID)],
    ] {
        let lookup = FakeLookup {
            run: Some(run(
                "pull_request",
                "feature",
                repository(FORK_ID, "contributor"),
            )),
            pull_requests,
            ..FakeLookup::default()
        };

        let trust = classify(&lookup);

        assert_eq!(trust.event, GitHubEventKind::Untrusted);
        assert!(matches!(
            trust.pull_request_check,
            PullRequestCheck::Unavailable { .. }
        ));
    }
}

#[test]
fn parses_github_workflow_run_response() {
    let run: WorkflowRun = serde_json::from_str(
        r#"{
            "id": 40971482787,
            "name": "CI",
            "event": "pull_request",
            "status": "queued",
            "head_branch": "main",
            "head_sha": "71a625e8185c74257be08430abf9b42c1f6358ec",
            "pull_requests": [],
            "repository": {"id": 100, "full_name": "org/project", "owner": {"login": "org", "id": 1}},
            "head_repository": {"id": 200, "full_name": "attacker/project", "owner": {"login": "attacker", "id": 2}}
        }"#,
    )
    .unwrap();

    assert_eq!(run.event, "pull_request");
    assert_eq!(run.head_branch.as_deref(), Some("main"));
    assert_eq!(run.repository.id, 100);
    assert_eq!(run.head_repository, Some(repository(200, "attacker")));

    let run: WorkflowRun = serde_json::from_str(
        r#"{
            "id": 1,
            "event": "pull_request",
            "head_branch": null,
            "head_sha": "abc",
            "repository": {"id": 100, "owner": {"login": "org"}},
            "head_repository": null
        }"#,
    )
    .unwrap();
    assert_eq!(run.head_branch, None);
    assert_eq!(run.head_repository, None);
}
