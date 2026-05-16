use crate::process::ProcessSpec;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerEntrypointConfig {
    pub runner_dir: PathBuf,
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
            name: required_env("RUNNER_NAME")?,
            repo_url: required_env("RUNNER_REPO_URL")?,
            token: required_env("RUNNER_TOKEN")?,
            labels: env::var("RUNNER_LABELS").unwrap_or_else(|_| "self-hosted,runlet".to_string()),
            ephemeral: env::var("RUNNER_EPHEMERAL")
                .map(|value| value != "false")
                .unwrap_or(true),
        })
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

fn required_env(name: &'static str) -> Result<String, RunnerEntrypointError> {
    env::var(name).map_err(|_| RunnerEntrypointError::MissingEnv(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_command_executes_runner_script_directly_without_shell() {
        let config = RunnerEntrypointConfig {
            runner_dir: "/actions-runner".into(),
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
