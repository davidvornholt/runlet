use crate::policy::GitHubEventKind;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WebhookError {
    #[error("missing X-Hub-Signature-256 header")]
    MissingSignature,
    #[error("webhook signature is invalid")]
    InvalidSignature,
    #[error("unsupported event {0}")]
    UnsupportedEvent(String),
    #[error("failed to parse webhook payload: {0}")]
    InvalidPayload(String),
    #[error("workflow_job action {0} is ignored")]
    IgnoredAction(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRequest {
    pub github_job_id: i64,
    pub repository: String,
    pub repo_url: String,
    pub branch: String,
    pub event: GitHubEventKind,
    pub labels: Vec<String>,
}

pub fn verify_signature(secret: &[u8], body: &[u8], signature: &str) -> Result<(), WebhookError> {
    let Some(hex_signature) = signature.strip_prefix("sha256=") else {
        return Err(WebhookError::InvalidSignature);
    };
    let expected = hex::decode(hex_signature).map_err(|_| WebhookError::InvalidSignature)?;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| WebhookError::InvalidSignature)?;
    mac.update(body);
    mac.verify_slice(&expected)
        .map_err(|_| WebhookError::InvalidSignature)
}

pub fn parse_event(event: &str, body: &[u8]) -> Result<Option<JobRequest>, WebhookError> {
    match event {
        "workflow_job" => parse_workflow_job(body).map(Some),
        "ping" => Ok(None),
        other => Err(WebhookError::UnsupportedEvent(other.to_string())),
    }
}

fn parse_workflow_job(body: &[u8]) -> Result<JobRequest, WebhookError> {
    let payload = serde_json::from_slice::<WorkflowJobPayload>(body)
        .map_err(|error| WebhookError::InvalidPayload(error.to_string()))?;
    if payload.action != "queued" {
        return Err(WebhookError::IgnoredAction(payload.action));
    }

    let repository = format!("github:{}", payload.repository.full_name);
    let head_branch = payload.workflow_job.head_branch.unwrap_or_default();
    let event = if payload.workflow_job.pull_requests.is_empty() {
        if head_branch.starts_with("refs/tags/") {
            GitHubEventKind::Release
        } else {
            GitHubEventKind::BranchPush
        }
    } else {
        GitHubEventKind::PullRequestFromFork
    };

    Ok(JobRequest {
        github_job_id: payload.workflow_job.id,
        repository,
        repo_url: payload.repository.html_url,
        branch: head_branch,
        event,
        labels: payload.workflow_job.labels,
    })
}

#[derive(Debug, Deserialize)]
struct WorkflowJobPayload {
    action: String,
    workflow_job: WorkflowJob,
    repository: Repository,
}

#[derive(Debug, Deserialize)]
struct WorkflowJob {
    id: i64,
    head_branch: Option<String>,
    labels: Vec<String>,
    #[serde(default)]
    pull_requests: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Repository {
    full_name: String,
    html_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(pull_requests: &str) -> Vec<u8> {
        payload_with_branch("main", pull_requests)
    }

    fn payload_with_branch(branch: &str, pull_requests: &str) -> Vec<u8> {
        payload_with_head_branch(&serde_json::to_string(branch).unwrap(), pull_requests)
    }

    fn payload_with_head_branch(head_branch: &str, pull_requests: &str) -> Vec<u8> {
        format!(
            r#"{{
                "action": "queued",
                "workflow_job": {{
                    "id": 42,
                    "head_branch": {head_branch},
                    "labels": ["self-hosted", "runlet"],
                    "pull_requests": {pull_requests}
                }},
                "repository": {{
                    "full_name": "org/project",
                    "html_url": "https://github.com/org/project"
                }}
            }}"#
        )
        .into_bytes()
    }

    #[test]
    fn verifies_github_signature() {
        let secret = b"secret";
        let body = b"hello";
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        verify_signature(secret, body, &signature).unwrap();
        assert_eq!(
            verify_signature(secret, b"tampered", &signature).unwrap_err(),
            WebhookError::InvalidSignature
        );
    }

    #[test]
    fn parses_workflow_job_queue_event() {
        let request = parse_event("workflow_job", &payload("[]"))
            .unwrap()
            .expect("queued event should produce a job");
        assert_eq!(request.github_job_id, 42);
        assert_eq!(request.repository, "github:org/project");
        assert_eq!(request.event, GitHubEventKind::BranchPush);
        assert_eq!(request.labels, ["self-hosted", "runlet"]);
    }

    #[test]
    fn treats_pull_request_jobs_as_untrusted() {
        let request = parse_event("workflow_job", &payload(r#"[{"number": 7}]"#))
            .unwrap()
            .expect("queued event should produce a job");
        assert_eq!(request.event, GitHubEventKind::PullRequestFromFork);
    }

    #[test]
    fn accepts_nullable_head_branch() {
        let request = parse_event("workflow_job", &payload_with_head_branch("null", "[]"))
            .unwrap()
            .expect("queued event should produce a job");
        assert_eq!(request.event, GitHubEventKind::BranchPush);
        assert_eq!(request.branch, "");
    }

    #[test]
    fn treats_release_branch_names_as_branch_pushes() {
        let request = parse_event(
            "workflow_job",
            &payload_with_branch("release/2026-05", "[]"),
        )
        .unwrap()
        .expect("queued event should produce a job");
        assert_eq!(request.event, GitHubEventKind::BranchPush);
        assert_eq!(request.branch, "release/2026-05");
    }

    #[test]
    fn treats_tag_refs_as_release_events() {
        let request = parse_event(
            "workflow_job",
            &payload_with_branch("refs/tags/v1.0.0", "[]"),
        )
        .unwrap()
        .expect("queued event should produce a job");
        assert_eq!(request.event, GitHubEventKind::Release);
    }
}
