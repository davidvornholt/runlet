use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

fn write_script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}")).expect("script should be written");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .expect("script should be executable");
}

#[test]
fn runner_process_tree_does_not_expose_registration_token() {
    let directory = tempfile::tempdir().expect("tempdir should be created");
    let runner_dir = directory.path().join("runner");
    fs::create_dir_all(&runner_dir).expect("runner directory should be created");
    write_script(&runner_dir.join("config.sh"), "exit 0\n");
    // Job steps run below run.sh as the same user, so they can read the
    // environment of run.sh and of the entrypoint that started it.
    write_script(
        &runner_dir.join("run.sh"),
        r#": > "$(dirname "$0")/run.sh.started"
for pid in $$ $PPID; do
  grep -q RUNNER_TOKEN= "/proc/$pid/environ"
  case $? in
    0) exit 1 ;;
    1) ;;
    *) exit 2 ;;
  esac
done
"#,
    );

    let status = Command::new(env!("CARGO_BIN_EXE_runlet-runner-entrypoint"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", directory.path().join("home"))
        .env("RUNNER_DIR", &runner_dir)
        .env("RUNNER_STATE_DIR", &runner_dir)
        .env("RUNNER_NAME", "runlet-1")
        .env("RUNNER_REPO_URL", "https://github.com/org/project")
        .env("RUNNER_TOKEN", "registration-token")
        .status()
        .expect("entrypoint should start");

    assert!(runner_dir.join("run.sh.started").exists());
    assert!(status.success(), "entrypoint exited with {status}");
}
