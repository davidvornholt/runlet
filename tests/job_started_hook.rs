use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

const HOOK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/images/actions-runner/job-started.sh"
);

fn bash() -> PathBuf {
    env::split_paths(&env::var_os("PATH").expect("PATH should be set"))
        .map(|directory| directory.join("bash"))
        .find(|path| path.is_file())
        .expect("bash should be on PATH")
}

/// Runs the hook the way the runner does, below a process named like the
/// runner's worker, and reports how that process ended.
fn run_job(allowed_runs: &Path, run_id: &str) -> (ExitStatus, bool) {
    let directory = tempfile::tempdir().expect("tempdir should be created");
    let worker = directory.path().join("Runner.Worker");
    let survived = directory.path().join("survived");
    fs::write(
        &worker,
        format!(
            "#!{bash}\n{bash} --noprofile --norc -e -o pipefail {HOOK} {allowed} 0\n: > {survived}\n",
            bash = bash().display(),
            allowed = allowed_runs.display(),
            survived = survived.display(),
        ),
    )
    .expect("worker script should be written");
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o755))
        .expect("worker script should be executable");

    let status = Command::new(&worker)
        .env("GITHUB_RUN_ID", run_id)
        .status()
        .expect("worker script should run");
    (status, survived.exists())
}

#[test]
fn jobs_from_accepted_runs_continue() {
    let allowed_runs = tempfile::tempdir().expect("tempdir should be created");
    fs::write(allowed_runs.path().join("44028721861"), b"").expect("record should be written");

    let (status, survived) = run_job(allowed_runs.path(), "44028721861");

    assert!(status.success());
    assert!(survived);
}

#[test]
fn jobs_from_other_runs_stop_the_worker() {
    let allowed_runs = tempfile::tempdir().expect("tempdir should be created");
    fs::write(allowed_runs.path().join("44028721861"), b"").expect("record should be written");

    for run_id in ["44028721862", "", "../44028721861", "1 2"] {
        let (status, survived) = run_job(allowed_runs.path(), run_id);

        assert_eq!(status.signal(), Some(9), "run ID {run_id:?}");
        assert!(!survived, "run ID {run_id:?}");
    }
}
