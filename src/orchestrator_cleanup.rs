use crate::github::RepositoryId;
use crate::orchestrator::{Orchestrator, OrchestratorError};
use crate::orchestrator_files::runner_token_env_path;
use crate::podman::{podman_remove_container, podman_scoped_prune, PodmanPruneResource};
use crate::process::run;
use crate::state::{JobStatus, Store};
use std::fs;
use std::path::Path;

impl Orchestrator {
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
        if self.config.runtime.cleanup.enable_scoped_prune {
            for resource in self.scoped_prune_resources() {
                for user in self.cleanup_users() {
                    let command = podman_scoped_prune(user, resource);
                    match run(command.command()) {
                        Ok(status) if status.success() => {}
                        Ok(status) => {
                            tracing::warn!(%status, "scoped Podman prune failed");
                            first_error.get_or_insert_with(|| {
                                OrchestratorError::CleanupCommandFailed(format!(
                                    "podman scoped prune exited with {status}"
                                ))
                            });
                        }
                        Err(error) => {
                            tracing::warn!(%error, "scoped Podman prune failed");
                            first_error.get_or_insert_with(|| error.into());
                        }
                    }
                }
            }
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn scoped_prune_resources(&self) -> Vec<PodmanPruneResource> {
        let mut resources = Vec::new();
        if self.config.runtime.cleanup.prune_containers {
            resources.push(PodmanPruneResource::Containers);
        }
        if self.config.runtime.cleanup.prune_images {
            resources.push(PodmanPruneResource::Images);
        }
        if self.config.runtime.cleanup.prune_volumes {
            resources.push(PodmanPruneResource::Volumes);
        }
        resources
    }

    fn cleanup_users(&self) -> Vec<Option<&str>> {
        if !self.config.runtime.users.enabled {
            return vec![None];
        }
        vec![
            Some(self.config.runtime.users.trusted.as_str()),
            Some(self.config.runtime.users.untrusted.as_str()),
        ]
    }

    pub(crate) fn cleanup_job(
        &self,
        store: &Store,
        repository: &RepositoryId,
        job_id: &str,
        runner_name: &str,
        workspace: &Path,
    ) -> Result<(), OrchestratorError> {
        let mut cleanup_error = None;
        for user in self.cleanup_users() {
            let remove_container = podman_remove_container(user, job_id);
            match run(remove_container.command()) {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    tracing::warn!(%status, %job_id, "failed to remove container");
                    cleanup_error.get_or_insert_with(|| {
                        OrchestratorError::CleanupCommandFailed(format!(
                            "podman rm exited with {status}"
                        ))
                    });
                }
                Err(error) => {
                    tracing::warn!(%error, %job_id, "failed to remove container");
                    cleanup_error.get_or_insert_with(|| error.into());
                }
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
