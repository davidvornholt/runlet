//! GitHub gives a queued job to any idle runner whose labels include the job's
//! labels, so a runner can receive a job Runlet never started it for. Runlet
//! records each workflow run it accepts per trust class, and the runner image's
//! job-started hook fails any job whose run is not recorded for its class.

use crate::config::TrustClass;
use crate::github::{GitHubClient, GitHubError, RepositoryId};
use serde::Deserialize;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Where the runner image's job-started hook looks for accepted runs.
pub const CONTAINER_ALLOWED_RUNS_DIR: &str = "/etc/runlet/allowed-runs";

/// How many extra runners Runlet starts for one job whose runner other jobs took.
pub const MAX_REPLACEMENT_RUNNERS: u32 = 3;

/// A job waits at most a day in GitHub's queue, so older records are no longer needed.
pub const ALLOWED_RUN_MAX_AGE: Duration = Duration::from_secs(2 * 24 * 60 * 60);

pub fn allowed_runs_dir(jobs_dir: &Path, trust_class: TrustClass) -> PathBuf {
    jobs_dir.join("allowed-runs").join(match trust_class {
        TrustClass::Trusted => "trusted",
        TrustClass::Untrusted => "untrusted",
    })
}

/// Records that jobs from `run_id` may run on runners of `trust_class`. The
/// directory is mounted read-only into runner containers, whose user is mapped
/// to an unrelated host ID, so it must be world-readable.
pub fn allow_run(jobs_dir: &Path, trust_class: TrustClass, run_id: u64) -> io::Result<()> {
    let directory = allowed_runs_dir(jobs_dir, trust_class);
    fs::create_dir_all(&directory)?;
    for path in [jobs_dir.join("allowed-runs"), directory.clone()] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    let record = directory.join(run_id.to_string());
    fs::write(&record, b"")?;
    fs::set_permissions(record, fs::Permissions::from_mode(0o644))
}

pub fn prune_allowed_runs(jobs_dir: &Path, max_age: Duration) -> io::Result<()> {
    let now = SystemTime::now();
    for trust_class in [TrustClass::Trusted, TrustClass::Untrusted] {
        let entries = match fs::read_dir(allowed_runs_dir(jobs_dir, trust_class)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let modified = entry.metadata()?.modified()?;
            if now.duration_since(modified).unwrap_or_default() > max_age {
                fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct WorkflowJob {
    status: String,
}

impl GitHubClient {
    pub fn workflow_job_is_queued(
        &self,
        repository: &RepositoryId,
        github_job_id: i64,
    ) -> Result<bool, GitHubError> {
        let token = self.installation_token()?;
        let job_id = github_job_id.to_string();
        let url = self.api_url([
            "repos",
            &repository.owner,
            &repository.name,
            "actions",
            "jobs",
            &job_id,
        ])?;
        let job: WorkflowJob = self.get_json(&token, url)?;
        Ok(job.status == "queued")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn records_runs_per_trust_class_readable_by_runner_containers() {
        let jobs_dir = tempfile::tempdir().unwrap();

        allow_run(jobs_dir.path(), TrustClass::Trusted, 44028721861).unwrap();
        allow_run(jobs_dir.path(), TrustClass::Trusted, 44028721861).unwrap();

        let trusted = allowed_runs_dir(jobs_dir.path(), TrustClass::Trusted);
        let record = trusted.join("44028721861");
        assert!(record.is_file());
        assert_eq!(
            fs::metadata(&trusted).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::metadata(&record).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert!(!allowed_runs_dir(jobs_dir.path(), TrustClass::Untrusted)
            .join("44028721861")
            .exists());
    }

    #[test]
    fn prunes_only_records_older_than_the_maximum_age() {
        let jobs_dir = tempfile::tempdir().unwrap();
        allow_run(jobs_dir.path(), TrustClass::Untrusted, 1).unwrap();
        allow_run(jobs_dir.path(), TrustClass::Untrusted, 2).unwrap();
        let untrusted = allowed_runs_dir(jobs_dir.path(), TrustClass::Untrusted);
        File::options()
            .write(true)
            .open(untrusted.join("1"))
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3 * 24 * 60 * 60))
            .unwrap();

        prune_allowed_runs(jobs_dir.path(), ALLOWED_RUN_MAX_AGE).unwrap();

        assert!(!untrusted.join("1").exists());
        assert!(untrusted.join("2").exists());
    }

    #[test]
    fn pruning_without_records_succeeds() {
        let jobs_dir = tempfile::tempdir().unwrap();

        prune_allowed_runs(jobs_dir.path(), ALLOWED_RUN_MAX_AGE).unwrap();
    }
}
