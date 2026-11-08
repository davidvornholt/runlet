use crate::process::ProcessSpec;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerEntrypointConfig {
    pub runner_dir: PathBuf,
    pub state_dir: Option<PathBuf>,
    pub name: String,
    pub repo_url: String,
    pub token: String,
    pub labels: String,
    pub ephemeral: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RunnerEntrypointError {
    #[error("required environment variable {0} is missing")]
    MissingEnv(&'static str),
}

impl RunnerEntrypointConfig {
    pub fn from_env() -> Result<Self, RunnerEntrypointError> {
        Ok(Self {
            runner_dir: env::var_os("RUNNER_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/actions-runner")),
            state_dir: env::var_os("RUNNER_STATE_DIR")
                .map(PathBuf::from)
                .or_else(|| Some(PathBuf::from("/tmp/actions-runner"))),
            name: required_env("RUNNER_NAME")?,
            repo_url: required_env("RUNNER_REPO_URL")?,
            token: required_env("RUNNER_TOKEN")?,
            labels: env::var("RUNNER_LABELS").unwrap_or_else(|_| "self-hosted,runlet".to_string()),
            ephemeral: env::var("RUNNER_EPHEMERAL")
                .map(|value| value != "false")
                .unwrap_or(true),
        })
    }

    pub fn prepare_writable_runner_dir(&mut self) -> Result<(), std::io::Error> {
        if let Some(home) = env::var_os("HOME") {
            fs::create_dir_all(home)?;
        }
        let Some(state_dir) = &self.state_dir else {
            return Ok(());
        };
        if state_dir == &self.runner_dir {
            return Ok(());
        }
        let source = fs::canonicalize(&self.runner_dir)?;
        fs::create_dir_all(state_dir)?;
        let destination = fs::canonicalize(state_dir)?;
        if source == destination {
            return Ok(());
        }
        if source.starts_with(&destination) || destination.starts_with(&source) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "runner installation and state directories must not contain each other",
            ));
        }
        fs::remove_dir_all(state_dir)?;
        copy_dir_recursive(&self.runner_dir, state_dir)?;
        self.runner_dir = state_dir.clone();
        Ok(())
    }

    pub fn configure_command(&self) -> ProcessSpec {
        let mut args = vec![
            "--unattended".into(),
            "--replace".into(),
            "--url".into(),
            self.repo_url.clone().into(),
            "--token".into(),
            self.token.clone().into(),
            "--name".into(),
            self.name.clone().into(),
            "--labels".into(),
            self.labels.clone().into(),
            "--work".into(),
            "_work".into(),
        ];
        if self.ephemeral {
            args.push("--ephemeral".into());
        }
        ProcessSpec {
            program: self.runner_dir.join("config.sh").into_os_string(),
            args,
        }
    }

    pub fn run_command(&self) -> ProcessSpec {
        ProcessSpec {
            program: self.runner_dir.join("run.sh").into_os_string(),
            args: Vec::<OsString>::new(),
        }
    }
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path)?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(&source_path)?;
            std::os::unix::fs::symlink(target, destination_path)?;
        }
    }
    Ok(())
}

fn required_env(name: &'static str) -> Result<String, RunnerEntrypointError> {
    env::var(name).map_err(|_| RunnerEntrypointError::MissingEnv(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_writable_runner_copy() {
        let directory = tempfile::tempdir().expect("tempdir should be created");
        let source = directory.path().join("source");
        let state = directory.path().join("state");
        fs::create_dir_all(&source).expect("source should be created");
        fs::write(source.join("config.sh"), "#!/bin/sh\n")
            .expect("config script should be written");
        fs::write(source.join("run.sh"), "#!/bin/sh\n").expect("run script should be written");

        let mut config = RunnerEntrypointConfig {
            runner_dir: source,
            state_dir: Some(state.clone()),
            name: "runlet-1".to_string(),
            repo_url: "https://github.com/org/project".to_string(),
            token: "token".to_string(),
            labels: "self-hosted,runlet".to_string(),
            ephemeral: true,
        };

        config
            .prepare_writable_runner_dir()
            .expect("runner copy should be prepared");

        assert_eq!(config.runner_dir, state);
        assert!(config.runner_dir.join("config.sh").exists());
        assert!(config.runner_dir.join("run.sh").exists());

        let alias = directory.path().join("state-alias");
        std::os::unix::fs::symlink(&state, &alias).unwrap();
        config.state_dir = Some(alias.clone());
        config.prepare_writable_runner_dir().unwrap();
        assert_eq!(fs::read_link(alias).unwrap(), state);
        assert!(config.runner_dir.join("config.sh").exists());
    }

    #[test]
    fn rejects_overlapping_runner_state_without_deleting_installation() {
        for nested_state in [false, true] {
            let directory = tempfile::tempdir().expect("tempdir should be created");
            let source = directory.path().join("runner");
            fs::create_dir(&source).unwrap();
            let script = source.join("config.sh");
            fs::write(&script, "runner installation").unwrap();
            let unrelated = directory.path().join("keep.txt");
            fs::write(&unrelated, "unrelated data").unwrap();
            let state = if nested_state {
                source.join("state")
            } else {
                directory.path().to_path_buf()
            };
            let mut config = RunnerEntrypointConfig {
                runner_dir: source.clone(),
                state_dir: Some(state),
                name: "runlet-1".to_string(),
                repo_url: "https://github.com/org/project".to_string(),
                token: "token".to_string(),
                labels: "self-hosted,runlet".to_string(),
                ephemeral: true,
            };

            let result = config.prepare_writable_runner_dir();

            assert!(
                script.exists(),
                "state initialization deleted the runner installation"
            );
            assert_eq!(fs::read_to_string(&unrelated).unwrap(), "unrelated data");
            assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
            assert_eq!(config.runner_dir, source);
        }
    }

    #[test]
    fn config_command_executes_runner_script_directly_without_shell() {
        let config = RunnerEntrypointConfig {
            runner_dir: "/actions-runner".into(),
            state_dir: None,
            name: "runlet-1".to_string(),
            repo_url: "https://github.com/org/project".to_string(),
            token: "token".to_string(),
            labels: "self-hosted,runlet".to_string(),
            ephemeral: true,
        };

        let spec = config.configure_command();
        assert_eq!(spec.program, "/actions-runner/config.sh");
        assert!(spec.args.iter().any(|arg| arg == "--ephemeral"));
        assert!(!spec.args.iter().any(|arg| arg == "sh"));
        assert!(!spec.args.iter().any(|arg| arg == "-c"));
    }
}
