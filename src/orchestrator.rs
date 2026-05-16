use crate::cache::{assert_untrusted_cache_write_allowed, prepare_cache_mount};
use crate::concurrency::ConcurrencyLimiter;
use crate::config::Config;
use crate::duration::parse_duration;
use crate::github::{GitHubClient, RepositoryId};
use crate::orchestrator_files::{
    abort_claimed_job, read_secret, runner_token_env_path, write_runner_token_env,
};
use crate::podman::{podman_run, PodmanJobSpec};
use crate::policy::{
    decide, validate_capability_labels, GitHubEventKind, JobContext, PolicyDecision,
    PolicyViolation,
};
use crate::process::{run_with_timeout, RunOutcome};
use crate::state::{JobRecord, JobStatus, Store};
use crate::webhook::JobRequest;
use crate::webhook_server::serve_webhooks;
use crate::workflow_risk::{workflow_risk_decision, WorkflowRiskDecision};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
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
    #[error("cleanup command failed: {0}")]
    CleanupCommandFailed(String),
    #[error("job concurrency limit is currently full")]
    ConcurrencyBusy,
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
        let shared_receiver = Arc::new(Mutex::new(receiver));

        for worker_id in 0..self.config.runtime.max_concurrent_jobs {
            let worker = self.clone();
            let receiver = Arc::clone(&shared_receiver);
            let retry_sender = sender.clone();
            thread::Builder::new()
                .name(format!("runlet-worker-{worker_id}"))
                .spawn(move || worker.worker_loop(receiver, retry_sender))?;
        }

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

    fn worker_loop(&self, receiver: Arc<Mutex<Receiver<JobRequest>>>, sender: Sender<JobRequest>) {
        loop {
            let request = {
                let locked = receiver
                    .lock()
                    .expect("job receiver lock should not be poisoned");
                locked.recv()
            };
            let Ok(request) = request else {
                return;
            };
            match self.run_job(request.clone()) {
                Ok(()) => {}
                Err(OrchestratorError::ConcurrencyBusy) => {
                    if sender.send(request).is_err() {
                        return;
                    }
                    thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(error) => {
                    tracing::warn!(%error, "job failed");
                }
            }
        }
    }

    pub fn run_job(&self, request: JobRequest) -> Result<(), OrchestratorError> {
        let context = JobContext {
            repository: request.repository.clone(),
            event: request.event,
            branch: request.branch.clone(),
        };
        let policy = match decide(&self.config, &context) {
            PolicyDecision::Allow(policy) => policy,
            PolicyDecision::Deny { reason } => {
                return Err(OrchestratorError::PolicyDenied(reason));
            }
        };

        let runlet_labeled = request.labels.iter().any(|label| label == "runlet");
        if !runlet_labeled {
            return Err(OrchestratorError::PolicyDenied(
                "job is missing required runlet label".to_string(),
            ));
        }
        validate_capability_labels(&policy, &request.labels)?;
        let Some(_permit) = self.concurrency.try_acquire(policy.trust_class) else {
            return Err(OrchestratorError::ConcurrencyBusy);
        };
        assert_untrusted_cache_write_allowed(
            &self.config.cache,
            policy.cache_write,
            request.event != GitHubEventKind::PullRequestFromFork,
        )?;

        let repository = RepositoryId::parse(&request.repository)?;
        let store = Store::open(&self.config.state.database_path)?;
        let job_id = format!("{}-{}", request.github_job_id, Uuid::new_v4());
        let runner_name = format!("runlet-{job_id}");
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

        if request.event == GitHubEventKind::PullRequestFromFork {
            if let Some(repository_config) = self.config.repositories.get(&request.repository) {
                let mut changed_files = Vec::new();
                for pull_request_number in &request.pull_request_numbers {
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
                    for pull_request_number in &request.pull_request_numbers {
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

        let registration_token = match self.github.create_registration_token(&repository) {
            Ok(token) => token,
            Err(error) => {
                abort_claimed_job(&store, &job_id, &workspace)?;
                return Err(error.into());
            }
        };
        if let Err(error) = fs::create_dir_all(&workspace)
            .and_then(|()| fs::set_permissions(&workspace, fs::Permissions::from_mode(0o770)))
        {
            abort_claimed_job(&store, &job_id, &workspace)?;
            return Err(error.into());
        }
        let token_env_file = runner_token_env_path(&workspace);
        if let Err(error) = write_runner_token_env(&token_env_file, &registration_token.token) {
            abort_claimed_job(&store, &job_id, &workspace)?;
            return Err(error.into());
        }
        let cache_mount = match prepare_cache_mount(
            &self.config.cache,
            &request.repository,
            policy.cache_write,
        ) {
            Ok(mount) => mount,
            Err(error) => {
                abort_claimed_job(&store, &job_id, &workspace)?;
                return Err(error.into());
            }
        };

        if let Err(error) = store.record_runner_registration(
            &runner_name,
            &request.repository,
            &registration_token.expires_at,
        ) {
            abort_claimed_job(&store, &job_id, &workspace)?;
            return Err(error.into());
        }
        if let Some(mount) = &cache_mount {
            store.upsert_cache_entry(
                &mount.namespace,
                "default",
                &request.repository,
                request.event != GitHubEventKind::PullRequestFromFork,
            )?;
        }
        store.set_job_status(&job_id, JobStatus::Running)?;

        let process = podman_run(
            &self.config.runtime,
            &PodmanJobSpec {
                job_id: job_id.clone(),
                runner_name: runner_name.clone(),
                repo_url: request.repo_url,
                token_env_file: token_env_file.into_os_string(),
                labels: request.labels,
                secrets: policy.secrets.to_string(),
                registry_push: policy.registry_push,
                deploy: policy.deploy,
                trust_class: policy.trust_class,
                network: policy.network.clone(),
                cache_mount: cache_mount
                    .as_ref()
                    .map(|mount| mount.path.as_os_str().to_os_string()),
                cache_writable: policy.cache_write,
            },
        );
        let timeout = parse_duration(&policy.timeout)?;
        let capture_stdout = !self
            .config
            .runtime
            .profile(policy.trust_class)
            .disable_host_log_capture;
        let outcome = match run_with_timeout(process.command(), timeout, capture_stdout) {
            Ok(outcome) => outcome,
            Err(error) => {
                store.set_job_status(&job_id, JobStatus::Failed)?;
                store.append_event(
                    &job_id,
                    "failed",
                    &format!("failed to run runner container: {error}"),
                )?;
                self.cleanup_job(&store, &repository, &job_id, &runner_name, &workspace)?;
                return Err(error.into());
            }
        };
        match outcome {
            RunOutcome::Exited(status) if status.success() => {
                store.set_job_status(&job_id, JobStatus::Succeeded)?;
                store.append_event(&job_id, "succeeded", "runner container exited successfully")?;
            }
            RunOutcome::Exited(status) => {
                store.set_job_status(&job_id, JobStatus::Failed)?;
                store.append_event(
                    &job_id,
                    "failed",
                    &format!("runner container exited with {status}"),
                )?;
            }
            RunOutcome::TimedOut => {
                store.set_job_status(&job_id, JobStatus::Failed)?;
                store.append_event(&job_id, "timeout", "runner container exceeded timeout")?;
            }
        }

        self.cleanup_job(&store, &repository, &job_id, &runner_name, &workspace)
    }
}
