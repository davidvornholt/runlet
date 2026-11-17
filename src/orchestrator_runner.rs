use crate::cache::prepare_cache_mount;
use crate::config::TrustClass;
use crate::duration::parse_duration;
use crate::github::RepositoryId;
use crate::orchestrator::{Orchestrator, OrchestratorError};
use crate::orchestrator_files::{abort_claimed_job, runner_token_env_path, write_runner_token_env};
use crate::podman::{podman_run, PodmanJobSpec};
use crate::policy::{EffectivePolicy, GitHubEventKind};
use crate::process::{run_with_timeout, RunOutcome};
use crate::runner_binding::allowed_runs_dir;
use crate::state::{JobStatus, Store};
use crate::webhook::JobRequest;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// One accepted job's runner, which Runlet may start more than once when other
/// jobs take it.
#[derive(Clone, Copy)]
pub(crate) struct RunnerLaunch<'a> {
    pub store: &'a Store,
    pub repository: &'a RepositoryId,
    pub request: &'a JobRequest,
    pub policy: &'a EffectivePolicy,
    pub event: GitHubEventKind,
    pub job_id: &'a str,
    pub workspace: &'a Path,
}

impl Orchestrator {
    /// Registers and runs one runner container named `runner_name`, then cleans
    /// it up. Returns whether the runner exited cleanly, which it does after
    /// the one job it takes.
    pub(crate) fn run_runner(
        &self,
        launch: &RunnerLaunch<'_>,
        runner_name: &str,
    ) -> Result<bool, OrchestratorError> {
        let RunnerLaunch {
            store,
            repository,
            request,
            policy,
            event,
            job_id,
            workspace,
        } = *launch;
        let registration_token = match self.github.create_registration_token(repository) {
            Ok(token) => token,
            Err(error) => {
                abort_claimed_job(store, job_id, workspace)?;
                return Err(error.into());
            }
        };
        if let Err(error) = fs::create_dir_all(workspace)
            .and_then(|()| fs::set_permissions(workspace, fs::Permissions::from_mode(0o770)))
        {
            abort_claimed_job(store, job_id, workspace)?;
            return Err(error.into());
        }
        let token_env_file = runner_token_env_path(workspace);
        if let Err(error) = write_runner_token_env(&token_env_file, &registration_token.token) {
            abort_claimed_job(store, job_id, workspace)?;
            return Err(error.into());
        }
        let cache_mount = match prepare_cache_mount(
            &self.config.cache,
            &request.repository,
            policy.cache_write,
        ) {
            Ok(mount) => mount,
            Err(error) => {
                abort_claimed_job(store, job_id, workspace)?;
                return Err(error.into());
            }
        };

        if let Err(error) = store.record_runner_registration(
            runner_name,
            &request.repository,
            &registration_token.expires_at,
        ) {
            abort_claimed_job(store, job_id, workspace)?;
            return Err(error.into());
        }
        if let Some(mount) = &cache_mount {
            store.upsert_cache_entry(
                &mount.namespace,
                "default",
                &request.repository,
                event != GitHubEventKind::Untrusted,
            )?;
        }
        store.set_job_status(job_id, JobStatus::Running)?;

        let process = podman_run(
            &self.config.runtime,
            &PodmanJobSpec {
                job_id: job_id.to_string(),
                runner_name: runner_name.to_string(),
                repo_url: request.repo_url.clone(),
                token_env_file: token_env_file.into_os_string(),
                labels: request.labels.clone(),
                secrets: policy.secrets.to_string(),
                registry_push: policy.registry_push,
                deploy: policy.deploy,
                trust_class: policy.trust_class,
                network: policy.network.clone(),
                cache_mount: cache_mount
                    .as_ref()
                    .map(|mount| mount.path.as_os_str().to_os_string()),
                cache_writable: policy.cache_write,
                allowed_runs_dir: allowed_runs_dir(
                    &self.config.runtime.jobs_dir,
                    policy.trust_class,
                )
                .into_os_string(),
            },
        );
        let timeout = parse_duration(&policy.timeout)?;
        let capture_stdout = policy.trust_class == TrustClass::Trusted;
        let outcome = match run_with_timeout(process.command(), timeout, capture_stdout) {
            Ok(outcome) => outcome,
            Err(error) => {
                store.set_job_status(job_id, JobStatus::Failed)?;
                store.append_event(
                    job_id,
                    "failed",
                    &format!("failed to run runner container: {error}"),
                )?;
                self.cleanup_job(store, repository, job_id, runner_name, workspace)?;
                return Err(error.into());
            }
        };
        let exited_cleanly = matches!(outcome, RunOutcome::Exited(status) if status.success());
        match outcome {
            RunOutcome::Exited(status) if status.success() => {
                store.set_job_status(job_id, JobStatus::Succeeded)?;
                store.append_event(job_id, "succeeded", "runner container exited successfully")?;
            }
            RunOutcome::Exited(status) => {
                store.set_job_status(job_id, JobStatus::Failed)?;
                store.append_event(
                    job_id,
                    "failed",
                    &format!("runner container exited with {status}"),
                )?;
            }
            RunOutcome::TimedOut => {
                store.set_job_status(job_id, JobStatus::Failed)?;
                store.append_event(job_id, "timeout", "runner container exceeded timeout")?;
            }
        }

        self.cleanup_job(store, repository, job_id, runner_name, workspace)?;
        Ok(exited_cleanly)
    }
}
