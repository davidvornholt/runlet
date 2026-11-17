use crate::concurrency::ConcurrencyLimiter;
use crate::config::Config;
use crate::duration::parse_duration;
use crate::github::{GitHubClient, RepositoryId};
use crate::job_trust::{classify_job, JobTrust};
use crate::orchestrator_files::{abort_claimed_job, read_secret};
use crate::orchestrator_runner::RunnerLaunch;
use crate::podman_limits::check_limits_enforceable;
use crate::policy::{
    decide, validate_capability_labels, GitHubEventKind, JobContext, PolicyDecision,
    PolicyViolation,
};
use crate::runner_binding::allow_run;
use crate::state::{JobRecord, JobStatus, Store};
use crate::webhook::JobRequest;
use crate::webhook_server::serve_webhooks;
use crate::workflow_risk::{workflow_risk_decision, WorkflowRiskDecision};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum OrchestratorError {
    #[error("state error: {0}")]
    State(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("GitHub error: {0}")]
    GitHub(#[from] crate::github::GitHubError),
    #[error("policy denied job: {0}")]
    PolicyDenied(String),
    #[error("policy denied job: {0}")]
    PolicyViolation(#[from] PolicyViolation),
    #[error("duration error: {0}")]
    Duration(#[from] crate::duration::DurationParseError),
    #[error("cache error: {0}")]
    Cache(#[from] crate::cache::CacheError),
    #[error("refused job: {0}")]
    LimitsUnenforceable(String),
    #[error("cleanup command failed: {0}")]
    CleanupCommandFailed(String),
}

#[derive(Clone)]
pub struct Orchestrator {
    pub(crate) config: Config,
    pub(crate) github: GitHubClient,
    concurrency: Arc<ConcurrencyLimiter>,
}

impl Orchestrator {
    pub fn new(config: Config) -> Self {
        let github = GitHubClient::new(config.github.clone());
        let concurrency = Arc::new(ConcurrencyLimiter::new(&config));
        Self {
            config,
            github,
            concurrency,
        }
    }

    pub fn serve(self) -> Result<(), OrchestratorError> {
        let store = Store::open(&self.config.state.database_path)?;
        store.mark_interrupted_jobs_cleanup_pending()?;
        let webhook_secret = read_secret(&self.config.orchestrator.webhook_secret_file)?;
        let cleanup_interval = parse_duration(&self.config.orchestrator.cleanup_interval)?;
        let (sender, receiver) = mpsc::channel::<JobRequest>();

        // Each delivery gets its own thread, so Runlet admits and records a run
        // while every slot is busy, before a runner already waiting can take
        // the run's job. Jobs then wait for a slot.
        let dispatcher = self.clone();
        thread::Builder::new()
            .name("runlet-dispatcher".to_string())
            .spawn(move || {
                for request in receiver {
                    let worker = dispatcher.clone();
                    let spawned = thread::Builder::new()
                        .name(format!("runlet-job-{}", request.github_job_id))
                        .spawn(move || worker.process_job(request));
                    if let Err(error) = spawned {
                        tracing::warn!(%error, "could not start a thread for a job");
                    }
                }
            })?;

        let cleaner = self.clone();
        thread::Builder::new()
            .name("runlet-cleaner".to_string())
            .spawn(move || loop {
                if let Err(error) = cleaner.cleanup_once() {
                    tracing::warn!(%error, "cleanup pass failed");
                }
                thread::sleep(cleanup_interval);
            })?;

        serve_webhooks(
            &self.config.orchestrator.listen_addr,
            webhook_secret,
            sender,
        )?;
        Ok(())
    }

    fn process_job(&self, request: JobRequest) {
        let result = self
            .verify_trust(&request)
            .and_then(|trust| self.run_job(request, trust));
        if let Err(error) = result {
            tracing::warn!(%error, "job failed");
        }
    }

    fn verify_trust(&self, request: &JobRequest) -> Result<JobTrust, OrchestratorError> {
        if !is_runlet_labeled(&request.labels) {
            return Err(OrchestratorError::PolicyDenied(
                "job is missing required runlet label".to_string(),
            ));
        }
        let repository = RepositoryId::parse(&request.repository)?;
        Ok(match self.config.repositories.get(&request.repository) {
            Some(repository_config) if repository_config.enabled => classify_job(
                &self.github,
                &repository,
                &repository_config.trusted_branches,
                request.run_id,
            ),
            _ => JobTrust::unverified("repository is not enabled in Runlet configuration"),
        })
    }

    fn run_job(&self, request: JobRequest, trust: JobTrust) -> Result<(), OrchestratorError> {
        let JobTrust {
            event,
            branch,
            pull_request_check,
        } = trust;
        let context = JobContext {
            repository: request.repository.clone(),
            event,
            branch,
        };
        let policy = match decide(&self.config, &context) {
            PolicyDecision::Allow(policy) => policy,
            PolicyDecision::Deny { reason } => {
                return Err(OrchestratorError::PolicyDenied(reason));
            }
        };

        let runlet_labeled = is_runlet_labeled(&request.labels);
        validate_capability_labels(&policy, &request.labels)?;
        let pull_request_numbers = pull_request_check
            .pull_request_numbers()
            .map_err(OrchestratorError::PolicyDenied)?;
        let repository = RepositoryId::parse(&request.repository)?;
        let store = Store::open(&self.config.state.database_path)?;
        let job_id = format!("{}-{}", request.github_job_id, Uuid::new_v4());
        let mut runner_name = format!("runlet-{job_id}");
        let container_name = format!("runlet-{job_id}");
        let workspace = self.config.runtime.jobs_dir.join(&job_id);
        if let Err(error) = store.upsert_job(&JobRecord {
            job_id: job_id.clone(),
            github_job_id: request.github_job_id,
            repository: request.repository.clone(),
            runner_name: runner_name.clone(),
            container_name: container_name.clone(),
            workspace: workspace.display().to_string(),
            status: JobStatus::Queued,
        }) {
            if let Some(existing) =
                store.job_by_github_job_id(&request.repository, request.github_job_id)?
            {
                if existing.status == JobStatus::Held {
                    tracing::info!(
                        job_id = %existing.job_id,
                        github_job_id = request.github_job_id,
                        repository = %request.repository,
                        "retrying held workflow_job delivery"
                    );
                    store.delete_job(&existing.job_id)?;
                    store.upsert_job(&JobRecord {
                        job_id: job_id.clone(),
                        github_job_id: request.github_job_id,
                        repository: request.repository.clone(),
                        runner_name: runner_name.clone(),
                        container_name: container_name.clone(),
                        workspace: workspace.display().to_string(),
                        status: JobStatus::Queued,
                    })?;
                } else {
                    tracing::info!(
                        job_id = %existing.job_id,
                        github_job_id = request.github_job_id,
                        repository = %request.repository,
                        "ignoring duplicate workflow_job delivery"
                    );
                    return Ok(());
                }
            } else {
                return Err(error.into());
            }
        }
        store.append_event(&job_id, "queued", "job accepted by runlet")?;

        if event == GitHubEventKind::Untrusted {
            if let Some(repository_config) = self.config.repositories.get(&request.repository) {
                let mut changed_files = Vec::new();
                for pull_request_number in &pull_request_numbers {
                    let files = match self
                        .github
                        .pull_request_changed_files(&repository, *pull_request_number)
                    {
                        Ok(files) => files,
                        Err(error) => {
                            abort_claimed_job(&store, &job_id, &workspace)?;
                            return Err(error.into());
                        }
                    };
                    changed_files.extend(files);
                }
                let mut decision = workflow_risk_decision(
                    &repository_config.workflow_risk,
                    &changed_files,
                    false,
                    runlet_labeled,
                );
                if matches!(decision, WorkflowRiskDecision::RequireApproval { .. }) {
                    let mut approved = false;
                    for pull_request_number in &pull_request_numbers {
                        match self.github.pull_request_has_label(
                            &repository,
                            *pull_request_number,
                            &repository_config.workflow_risk.approval_label,
                        ) {
                            Ok(true) => approved = true,
                            Ok(false) => {}
                            Err(error) => {
                                abort_claimed_job(&store, &job_id, &workspace)?;
                                return Err(error.into());
                            }
                        }
                    }
                    decision = workflow_risk_decision(
                        &repository_config.workflow_risk,
                        &changed_files,
                        approved,
                        runlet_labeled,
                    );
                }
                match decision {
                    WorkflowRiskDecision::Allow => {}
                    WorkflowRiskDecision::Deny { reason } => {
                        store.set_job_status(&job_id, JobStatus::Failed)?;
                        store.append_event(&job_id, "policy-denied", &reason)?;
                        return Err(OrchestratorError::PolicyDenied(reason));
                    }
                    WorkflowRiskDecision::RequireApproval { reason } => {
                        store.set_job_status(&job_id, JobStatus::Held)?;
                        store.append_event(&job_id, "approval-required", &reason)?;
                        return Err(OrchestratorError::PolicyDenied(reason));
                    }
                }
            }
        }

        if let Err(reason) = check_limits_enforceable(&self.config.runtime, policy.trust_class) {
            // Held, so that redelivering the webhook retries once the host is fixed.
            store.set_job_status(&job_id, JobStatus::Held)?;
            store.append_event(&job_id, "limits-unenforceable", &reason)?;
            return Err(OrchestratorError::LimitsUnenforceable(reason));
        }

        if let Err(error) = allow_run(
            &self.config.runtime.jobs_dir,
            policy.trust_class,
            request.run_id,
        ) {
            abort_claimed_job(&store, &job_id, &workspace)?;
            return Err(error.into());
        }

        let _permit = self.concurrency.acquire(policy.trust_class);
        let runner = RunnerLaunch {
            store: &store,
            repository: &repository,
            request: &request,
            policy: &policy,
            event,
            job_id: &job_id,
            workspace: &workspace,
        };
        // A runner that exits cleanly ran a job. While GitHub still queues this
        // one, that was another job, so start a new runner. Each such job is
        // gone once it ran, so this ends.
        let mut replacement = 0;
        loop {
            if !self.run_runner(&runner, &runner_name)? {
                return Ok(());
            }
            let job_is_queued = self
                .github
                .workflow_job_is_queued(&repository, request.github_job_id)
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, %job_id, "could not check whether the job still waits for a runner");
                    false
                });
            if !job_is_queued {
                return Ok(());
            }
            replacement += 1;
            tracing::info!(%job_id, github_job_id = request.github_job_id, replacement, "the job still waits for a runner; starting a replacement");
            store.append_event(
                &job_id,
                "replacing",
                "the job still waits for a runner, so runlet starts a new one",
            )?;
            // GitHub removes an ephemeral runner's registration once the job it
            // took ends, and a runner that reuses the name would go with it.
            runner_name = format!("runlet-{job_id}-{replacement}");
            store.set_job_runner_name(&job_id, &runner_name)?;
        }
    }
}

fn is_runlet_labeled(labels: &[String]) -> bool {
    labels.iter().any(|label| label == "runlet")
}
