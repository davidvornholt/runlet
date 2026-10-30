use crate::github::{GitHubError, RepositoryId};
use crate::policy::{is_trusted_branch, GitHubEventKind};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobTrust {
    pub event: GitHubEventKind,
    pub branch: String,
    pub pull_request_check: PullRequestCheck,
}

/// The pull requests whose changed files the workflow-risk check inspects
/// before an untrusted job runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestCheck {
    NotNeeded,
    PullRequests(Vec<u64>),
    /// Runlet cannot tell which pull requests to check, so the job must not run.
    Unavailable {
        reason: String,
    },
}

impl PullRequestCheck {
    pub fn pull_request_numbers(self) -> Result<Vec<u64>, String> {
        match self {
            Self::NotNeeded => Ok(Vec::new()),
            Self::PullRequests(numbers) => Ok(numbers),
            Self::Unavailable { reason } => Err(reason),
        }
    }
}

impl JobTrust {
    pub fn unverified(reason: impl Into<String>) -> Self {
        Self {
            event: GitHubEventKind::Untrusted,
            branch: String::new(),
            pull_request_check: PullRequestCheck::Unavailable {
                reason: reason.into(),
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WorkflowRun {
    pub id: u64,
    pub event: String,
    pub head_branch: Option<String>,
    pub head_sha: String,
    pub repository: RunRepository,
    pub head_repository: Option<RunRepository>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RunRepository {
    pub id: u64,
    pub owner: RepositoryOwner,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RepositoryOwner {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpenPullRequest {
    pub number: u64,
    pub head: PullRequestHead,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PullRequestHead {
    pub sha: String,
    pub repo: Option<PullRequestRepository>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PullRequestRepository {
    pub id: u64,
}

pub trait TrustLookup {
    fn workflow_run(
        &self,
        repository: &RepositoryId,
        run_id: u64,
    ) -> Result<WorkflowRun, GitHubError>;

    /// Returns the commit the tag points at, or `None` when no such tag exists.
    fn tag_commit_sha(
        &self,
        repository: &RepositoryId,
        tag: &str,
    ) -> Result<Option<String>, GitHubError>;

    fn open_pull_requests(
        &self,
        repository: &RepositoryId,
        head_owner: &str,
        head_branch: &str,
    ) -> Result<Vec<OpenPullRequest>, GitHubError>;
}

/// Decides a job's trust class from its workflow run. Only a push from the
/// repository itself can be trusted; everything else, including lookup
/// failures, is untrusted.
pub fn classify_job(
    lookup: &impl TrustLookup,
    repository: &RepositoryId,
    trusted_branches: &[String],
    run_id: u64,
) -> JobTrust {
    let run = match lookup.workflow_run(repository, run_id) {
        Ok(run) => run,
        Err(error) => {
            tracing::warn!(%error, run_id, "could not verify workflow run; treating job as untrusted");
            return JobTrust::unverified(format!(
                "could not verify workflow run {run_id}: {error}"
            ));
        }
    };
    let branch = run.head_branch.clone().unwrap_or_default();
    let from_same_repository = run
        .head_repository
        .as_ref()
        .is_some_and(|head| head.id == run.repository.id);
    match run.event.as_str() {
        "push" if from_same_repository && !branch.is_empty() => JobTrust {
            event: classify_push(lookup, repository, trusted_branches, &run, &branch),
            branch,
            pull_request_check: PullRequestCheck::NotNeeded,
        },
        "pull_request" | "pull_request_target" => JobTrust {
            event: GitHubEventKind::Untrusted,
            pull_request_check: pull_request_check(lookup, repository, &run),
            branch,
        },
        _ => JobTrust {
            event: GitHubEventKind::Untrusted,
            branch,
            pull_request_check: PullRequestCheck::NotNeeded,
        },
    }
}

/// Pushes to branches outside `trusted_branches` stay branch pushes, which
/// policy denies, unless the pushed name is a tag that points at the run's commit.
fn classify_push(
    lookup: &impl TrustLookup,
    repository: &RepositoryId,
    trusted_branches: &[String],
    run: &WorkflowRun,
    branch: &str,
) -> GitHubEventKind {
    if is_trusted_branch(trusted_branches, branch) {
        return GitHubEventKind::BranchPush;
    }
    match lookup.tag_commit_sha(repository, branch) {
        Ok(Some(sha)) if sha == run.head_sha => GitHubEventKind::Release,
        Ok(_) => GitHubEventKind::BranchPush,
        Err(error) => {
            tracing::warn!(%error, run_id = run.id, tag = branch, "could not confirm tag push");
            GitHubEventKind::BranchPush
        }
    }
}

fn pull_request_check(
    lookup: &impl TrustLookup,
    repository: &RepositoryId,
    run: &WorkflowRun,
) -> PullRequestCheck {
    let unavailable = |detail: String| PullRequestCheck::Unavailable {
        reason: format!(
            "cannot check workflow risk for pull request workflow run {}: {detail}",
            run.id
        ),
    };
    let head_branch = run.head_branch.as_deref().unwrap_or_default();
    let Some(head_repository) = run.head_repository.as_ref() else {
        return unavailable("the run has no head repository".to_string());
    };
    if head_branch.is_empty() {
        return unavailable("the run has no head branch".to_string());
    }
    let head_owner = &head_repository.owner.login;
    let pull_requests = match lookup.open_pull_requests(repository, head_owner, head_branch) {
        Ok(pull_requests) => pull_requests,
        Err(error) => return unavailable(format!("pull request lookup failed: {error}")),
    };
    let numbers = pull_requests
        .into_iter()
        .filter(|pull_request| {
            pull_request.head.sha == run.head_sha
                && pull_request
                    .head
                    .repo
                    .as_ref()
                    .is_some_and(|repo| repo.id == head_repository.id)
        })
        .map(|pull_request| pull_request.number)
        .collect::<Vec<_>>();
    if numbers.is_empty() {
        return unavailable(format!(
            "no open pull request has head {head_owner}:{head_branch} at {}",
            run.head_sha
        ));
    }
    PullRequestCheck::PullRequests(numbers)
}

#[cfg(test)]
#[path = "job_trust_tests.rs"]
mod tests;
