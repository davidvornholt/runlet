use crate::cache::{assert_untrusted_cache_write_allowed, prepare_cache_mount};
use crate::config::Config;
use crate::duration::parse_duration;
use crate::github::{GitHubClient, RepositoryId};
use crate::policy::{
    decide, validate_capability_labels, GitHubEventKind, JobContext, PolicyDecision,
    PolicyViolation,
};
use crate::process::{
    podman_remove_container, podman_run, run, run_with_timeout, PodmanJobSpec, RunOutcome,
};
use crate::state::{JobRecord, JobStatus, Store};
use crate::webhook::JobRequest;
use crate::webhook_server::serve_webhooks;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
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
}

#[derive(Clone)]
pub struct Orchestrator {
    config: Config,
    github: GitHubClient,
}

impl Orchestrator {
    pub fn new(config: Config) -> Self {
        let github = GitHubClient::new(config.github.clone());
        Self { config, github }
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
            thread::Builder::new()
                .name(format!("runlet-worker-{worker_id}"))
                .spawn(move || worker.worker_loop(receiver))?;
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

    fn worker_loop(&self, receiver: Arc<Mutex<Receiver<JobRequest>>>) {
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
            if let Err(error) = self.run_job(request) {
                tracing::warn!(%error, "job failed");
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

        if !request.labels.iter().any(|label| label == "runlet") {
            return Err(OrchestratorError::PolicyDenied(
                "job is missing required runlet label".to_string(),
            ));
        }
        validate_capability_labels(&policy, &request.labels)?;
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
                tracing::info!(
                    job_id = %existing.job_id,
                    github_job_id = request.github_job_id,
                    repository = %request.repository,
                    "ignoring duplicate workflow_job delivery"
                );
                return Ok(());
            }
            return Err(error.into());
        }
        store.append_event(&job_id, "queued", "job accepted by runlet")?;

        let registration_token = match self.github.create_registration_token(&repository) {
            Ok(token) => token,
            Err(error) => {
                abort_claimed_job(&store, &job_id, &workspace)?;
                return Err(error.into());
            }
        };
        if let Err(error) = fs::create_dir_all(&workspace) {
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
                network: policy.network.clone(),
                cache_mount: cache_mount
                    .as_ref()
                    .map(|mount| mount.path.as_os_str().to_os_string()),
                cache_writable: policy.cache_write,
            },
        );
        let timeout = parse_duration(&policy.timeout)?;
        let outcome = match run_with_timeout(process.command(), timeout) {
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

    pub fn cleanup_once(&self) -> Result<(), OrchestratorError> {
        let store = Store::open(&self.config.state.database_path)?;
        let mut first_error = None;
        for job in store.cleanup_pending_jobs()? {
            let repository = match RepositoryId::parse(&job.repository) {
                Ok(repository) => repository,
                Err(error) => {
                    tracing::warn!(%error, job_id = %job.job_id, "failed to parse repository for cleanup");
                    first_error.get_or_insert_with(|| error.into());
                    continue;
                }
            };
            if let Err(error) = self.cleanup_job(
                &store,
                &repository,
                &job.job_id,
                &job.runner_name,
                Path::new(&job.workspace),
            ) {
                tracing::warn!(%error, job_id = %job.job_id, "cleanup failed");
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn cleanup_job(
        &self,
        store: &Store,
        repository: &RepositoryId,
        job_id: &str,
        runner_name: &str,
        workspace: &Path,
    ) -> Result<(), OrchestratorError> {
        let mut cleanup_error = None;
        let remove_container = podman_remove_container(job_id);
        match run(remove_container.command()) {
            Ok(status) if status.success() => {}
            Ok(status) => {
                tracing::warn!(%status, %job_id, "failed to remove container");
                cleanup_error = Some(OrchestratorError::CleanupCommandFailed(format!(
                    "podman rm exited with {status}"
                )));
            }
            Err(error) => {
                tracing::warn!(%error, %job_id, "failed to remove container");
                cleanup_error = Some(error.into());
            }
        }
        if workspace.exists() {
            if let Err(error) = fs::remove_dir_all(workspace) {
                tracing::warn!(%error, %job_id, "failed to remove workspace");
                cleanup_error.get_or_insert_with(|| error.into());
            }
        }
        let token_env_file = runner_token_env_path(workspace);
        if token_env_file.exists() {
            if let Err(error) = fs::remove_file(&token_env_file) {
                tracing::warn!(%error, %job_id, "failed to remove runner token file");
                cleanup_error.get_or_insert_with(|| error.into());
            }
        }
        match self.github.remove_runner_by_name(repository, runner_name) {
            Ok(()) => store.mark_runner_revoked(runner_name)?,
            Err(error) => {
                tracing::warn!(%error, %job_id, "failed to remove GitHub runner");
                cleanup_error.get_or_insert_with(|| error.into());
            }
        }
        if let Some(error) = cleanup_error {
            store.set_job_status(job_id, JobStatus::CleanupPending)?;
            return Err(error);
        }
        store.set_job_status(job_id, JobStatus::Cleaned)?;
        store.append_event(
            job_id,
            "cleaned",
            "container, workspace, and runner were removed",
        )?;
        Ok(())
    }
}

fn abort_claimed_job(
    store: &Store,
    job_id: &str,
    workspace: &Path,
) -> Result<(), OrchestratorError> {
    let token_env_file = runner_token_env_path(workspace);
    if token_env_file.exists() {
        fs::remove_file(token_env_file)?;
    }
    if workspace.exists() {
        fs::remove_dir_all(workspace)?;
    }
    store.delete_job(job_id)?;
    Ok(())
}

fn read_secret(path: &Path) -> Result<Vec<u8>, std::io::Error> {
    let secret = trim_ascii_whitespace(fs::read(path)?);
    if secret.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "secret file must not be empty",
        ));
    }
    Ok(secret)
}

fn runner_token_env_path(workspace: &Path) -> std::path::PathBuf {
    let file_name = workspace
        .file_name()
        .map(|name| format!("{}.runner.env", name.to_string_lossy()))
        .unwrap_or_else(|| "runner.env".to_string());
    workspace.with_file_name(file_name)
}

fn write_runner_token_env(path: &Path, token: &str) -> Result<(), std::io::Error> {
    if token.contains(['\r', '\n']) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "runner token must not contain line breaks",
        ));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    writeln!(file, "RUNNER_TOKEN={token}")?;
    Ok(())
}

fn trim_ascii_whitespace(mut value: Vec<u8>) -> Vec<u8> {
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value.pop();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn trims_secret_file_newline() {
        assert_eq!(trim_ascii_whitespace(b"secret\n".to_vec()), b"secret");
    }

    #[test]
    fn rejects_empty_secret_file() {
        let directory = tempfile::tempdir().expect("tempdir should be created");
        let path = directory.path().join("secret");
        fs::write(&path, "\n").expect("secret should be written");

        let error = read_secret(&path).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn derives_runner_token_env_file_outside_workspace() {
        assert_eq!(
            runner_token_env_path(Path::new("/var/lib/runlet/jobs/123")),
            Path::new("/var/lib/runlet/jobs/123.runner.env")
        );
    }

    #[test]
    fn writes_runner_token_env_file_without_line_breaks() {
        let directory = tempfile::tempdir().expect("tempdir should be created");
        let path = directory.path().join("runner.env");

        write_runner_token_env(&path, "token").expect("token env file should be written");

        assert_eq!(fs::read_to_string(&path).unwrap(), "RUNNER_TOKEN=token\n");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            write_runner_token_env(&directory.path().join("bad.env"), "bad\ntoken")
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}
