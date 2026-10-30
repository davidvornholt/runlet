use hmac::{Hmac, KeyInit, Mac};
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

/// A queued job as reported by the `workflow_job` webhook. It carries no trust
/// information: the payload cannot tell a fork pull request from a push, so the
/// orchestrator verifies trust from the workflow run identified by `run_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRequest {
    pub github_job_id: i64,
    pub run_id: u64,
    pub repository: String,
    pub repo_url: String,
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

    Ok(JobRequest {
        github_job_id: payload.workflow_job.id,
        run_id: payload.workflow_job.run_id,
        repository: format!("github:{}", payload.repository.full_name),
        repo_url: payload.repository.html_url,
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
    run_id: u64,
    labels: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Repository {
    full_name: String,
    html_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(workflow_job: &str) -> Vec<u8> {
        format!(
            r#"{{
                "action": "queued",
                "workflow_job": {workflow_job},
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
        let request = parse_event(
            "workflow_job",
            &payload(
                r#"{
                    "id": 42,
                    "run_id": 7001,
                    "head_branch": "main",
                    "labels": ["self-hosted", "runlet"]
                }"#,
            ),
        )
        .unwrap()
        .expect("queued event should produce a job");
        assert_eq!(
            request,
            JobRequest {
                github_job_id: 42,
                run_id: 7001,
                repository: "github:org/project".to_string(),
                repo_url: "https://github.com/org/project".to_string(),
                labels: vec!["self-hosted".to_string(), "runlet".to_string()],
            }
        );
    }

    #[test]
    fn rejects_workflow_jobs_without_run_id() {
        let error = parse_event(
            "workflow_job",
            &payload(r#"{"id": 42, "head_branch": "main", "labels": ["runlet"]}"#),
        )
        .unwrap_err();
        assert!(matches!(error, WebhookError::InvalidPayload(_)));
    }
}
