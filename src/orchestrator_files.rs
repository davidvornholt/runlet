use crate::orchestrator::OrchestratorError;
use crate::state::Store;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub(crate) fn abort_claimed_job(
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

pub(crate) fn read_secret(path: &Path) -> Result<Vec<u8>, std::io::Error> {
    let secret = trim_ascii_whitespace(fs::read(path)?);
    if secret.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "secret file must not be empty",
        ));
    }
    Ok(secret)
}

pub(crate) fn runner_token_env_path(workspace: &Path) -> std::path::PathBuf {
    let file_name = workspace
        .file_name()
        .map(|name| format!("{}.runner.env", name.to_string_lossy()))
        .unwrap_or_else(|| "runner.env".to_string());
    workspace.with_file_name(file_name)
}

pub(crate) fn write_runner_token_env(path: &Path, token: &str) -> Result<(), std::io::Error> {
    if token.contains(['\r', '\n']) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "runner token must not contain line breaks",
        ));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o640)
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
            0o640
        );
        assert_eq!(
            write_runner_token_env(&directory.path().join("bad.env"), "bad\ntoken")
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}
