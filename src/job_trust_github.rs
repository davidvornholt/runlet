use crate::github::{GitHubClient, GitHubError, RepositoryId};
use crate::job_trust::{OpenPullRequest, TrustLookup, WorkflowRun};
use serde::Deserialize;

const MAX_TAG_PEEL_DEPTH: usize = 8;

#[derive(Debug, Deserialize)]
struct GitReference {
    object: GitObject,
}

#[derive(Debug, Deserialize)]
struct AnnotatedTag {
    object: GitObject,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct GitObject {
    #[serde(rename = "type")]
    kind: String,
    sha: String,
}

impl TrustLookup for GitHubClient {
    fn workflow_run(
        &self,
        repository: &RepositoryId,
        run_id: u64,
    ) -> Result<WorkflowRun, GitHubError> {
        let token = self.installation_token()?;
        let run_id = run_id.to_string();
        let url = self.api_url([
            "repos",
            &repository.owner,
            &repository.name,
            "actions",
            "runs",
            &run_id,
        ])?;
        self.get_json(&token, url)
    }

    fn tag_commit_sha(
        &self,
        repository: &RepositoryId,
        tag: &str,
    ) -> Result<Option<String>, GitHubError> {
        let Some(tag_segments) = ref_name_segments(tag) else {
            return Ok(None);
        };
        let token = self.installation_token()?;
        let url = self.api_url(
            [
                "repos",
                &repository.owner,
                &repository.name,
                "git",
                "ref",
                "tags",
            ]
            .into_iter()
            .chain(tag_segments),
        )?;
        let Some(reference) = self.get_json_if_found::<GitReference>(&token, url)? else {
            return Ok(None);
        };
        peel_to_commit(reference.object, |tag_sha| {
            let url = self.api_url([
                "repos",
                &repository.owner,
                &repository.name,
                "git",
                "tags",
                tag_sha,
            ])?;
            Ok(self
                .get_json_if_found::<AnnotatedTag>(&token, url)?
                .map(|tag| tag.object))
        })
    }

    fn open_pull_requests(
        &self,
        repository: &RepositoryId,
        head_owner: &str,
        head_branch: &str,
    ) -> Result<Vec<OpenPullRequest>, GitHubError> {
        let token = self.installation_token()?;
        let mut url = self.api_url(["repos", &repository.owner, &repository.name, "pulls"])?;
        url.query_pairs_mut()
            .append_pair("state", "open")
            .append_pair("head", &format!("{head_owner}:{head_branch}"))
            .append_pair("per_page", "100");
        self.get_json(&token, url)
    }
}

/// Splits a ref name into URL path segments, refusing names Git would reject
/// so that `.` or `..` can never change the requested path.
fn ref_name_segments(name: &str) -> Option<impl Iterator<Item = &str>> {
    let valid = name
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    valid.then(|| name.split('/'))
}

fn peel_to_commit(
    mut object: GitObject,
    mut annotated_tag_target: impl FnMut(&str) -> Result<Option<GitObject>, GitHubError>,
) -> Result<Option<String>, GitHubError> {
    for _ in 0..MAX_TAG_PEEL_DEPTH {
        match object.kind.as_str() {
            "commit" => return Ok(Some(object.sha)),
            "tag" => match annotated_tag_target(&object.sha)? {
                Some(target) => object = target,
                None => return Ok(None),
            },
            _ => return Ok(None),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(kind: &str, sha: &str) -> GitObject {
        GitObject {
            kind: kind.to_string(),
            sha: sha.to_string(),
        }
    }

    #[test]
    fn peels_lightweight_and_annotated_tags_to_commits() {
        let no_annotated_tags = |_: &str| -> Result<Option<GitObject>, GitHubError> {
            panic!("lightweight tags need no further lookup")
        };
        assert_eq!(
            peel_to_commit(object("commit", "c1"), no_annotated_tags).unwrap(),
            Some("c1".to_string())
        );

        let annotated = |sha: &str| {
            Ok(Some(match sha {
                "outer" => object("tag", "inner"),
                "inner" => object("commit", "c2"),
                other => panic!("unexpected tag {other}"),
            }))
        };
        assert_eq!(
            peel_to_commit(object("tag", "outer"), annotated).unwrap(),
            Some("c2".to_string())
        );
    }

    #[test]
    fn does_not_confirm_tags_that_never_reach_a_commit() {
        assert_eq!(
            peel_to_commit(object("tree", "t1"), |_| Ok(None)).unwrap(),
            None
        );
        assert_eq!(
            peel_to_commit(object("tag", "gone"), |_| Ok(None)).unwrap(),
            None
        );
        let cycle = |sha: &str| Ok(Some(object("tag", sha)));
        assert_eq!(peel_to_commit(object("tag", "loop"), cycle).unwrap(), None);
    }

    #[test]
    fn splits_ref_names_and_rejects_path_tricks() {
        assert_eq!(
            ref_name_segments("release/v1.0.0")
                .unwrap()
                .collect::<Vec<_>>(),
            ["release", "v1.0.0"]
        );
        for name in ["", "a//b", "../main", "a/./b", "v1/"] {
            assert!(
                ref_name_segments(name).is_none(),
                "{name} should be rejected"
            );
        }
    }

    #[test]
    fn parses_github_git_reference_and_annotated_tag_responses() {
        let reference: GitReference = serde_json::from_str(
            r#"{
                "ref": "refs/tags/v1.0.0",
                "node_id": "REF_x",
                "url": "https://api.github.com/repos/org/project/git/refs/tags/v1.0.0",
                "object": {"type": "tag", "sha": "abc", "url": "https://api.github.com/x"}
            }"#,
        )
        .unwrap();
        assert_eq!(reference.object, object("tag", "abc"));

        let tag: AnnotatedTag = serde_json::from_str(
            r#"{
                "tag": "v1.0.0",
                "sha": "abc",
                "message": "release",
                "object": {"type": "commit", "sha": "def", "url": "https://api.github.com/y"}
            }"#,
        )
        .unwrap();
        assert_eq!(tag.object, object("commit", "def"));
    }
}
