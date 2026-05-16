use crate::config::{NetworkPolicy, RuntimeConfig};
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
}

impl ProcessSpec {
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        command
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodmanJobSpec {
    pub job_id: String,
    pub runner_name: String,
    pub repo_url: String,
    pub token_env_file: OsString,
    pub labels: Vec<String>,
    pub network: NetworkPolicy,
    pub cache_mount: Option<OsString>,
    pub cache_writable: bool,
    pub secrets: String,
    pub registry_push: bool,
    pub deploy: bool,
}

pub fn podman_run(runtime: &RuntimeConfig, job: &PodmanJobSpec) -> ProcessSpec {
    let cpus = runtime.default_cpu.to_string();
    let memory = runtime.default_memory.clone();
    let disk = format!("size={}", runtime.default_disk);
    let network = match job.network {
        NetworkPolicy::Restricted => "slirp4netns:allow_host_loopback=false",
        NetworkPolicy::Normal => "slirp4netns",
    };

    let mut spec = ProcessSpec {
        program: OsString::from("podman"),
        args: vec![
            "run".into(),
            "--rm".into(),
            "--name".into(),
            format!("runlet-{}", job.job_id).into(),
            "--cpus".into(),
            cpus.into(),
            "--memory".into(),
            memory.into(),
            "--storage-opt".into(),
            disk.into(),
            "--network".into(),
            network.into(),
            "--security-opt".into(),
            "no-new-privileges".into(),
            "--cap-drop".into(),
            "ALL".into(),
            "--env-file".into(),
            job.token_env_file.clone(),
        ],
    };

    if let Some(cache_mount) = &job.cache_mount {
        let suffix = if job.cache_writable { "Z" } else { "ro,Z" };
        spec.args.extend([
            "--volume".into(),
            format!("{}:/cache:{suffix}", Path::new(cache_mount).display()).into(),
        ]);
    }

    spec.args.extend([
        "--env".into(),
        format!("RUNNER_NAME={}", job.runner_name).into(),
        "--env".into(),
        format!("RUNNER_REPO_URL={}", job.repo_url).into(),
        "--env".into(),
        format!("RUNNER_LABELS={}", job.labels.join(",")).into(),
        "--env".into(),
        "RUNNER_EPHEMERAL=true".into(),
        "--env".into(),
        format!("RUNLET_SECRETS={}", job.secrets).into(),
        "--env".into(),
        format!("RUNLET_REGISTRY_PUSH={}", job.registry_push).into(),
        "--env".into(),
        format!("RUNLET_DEPLOY={}", job.deploy).into(),
        runtime.runner_image.clone().into(),
    ]);

    spec
}

pub fn podman_remove_container(job_id: impl AsRef<OsStr>) -> ProcessSpec {
    ProcessSpec {
        program: OsString::from("podman"),
        args: vec![
            "rm".into(),
            "--force".into(),
            "--ignore".into(),
            format!("runlet-{}", job_id.as_ref().to_string_lossy()).into(),
        ],
    }
}

pub fn run(mut command: Command) -> std::io::Result<ExitStatus> {
    command.status()
}

#[derive(Debug)]
pub enum RunOutcome {
    Exited(ExitStatus),
    TimedOut,
}

pub fn run_with_timeout(mut command: Command, timeout: Duration) -> std::io::Result<RunOutcome> {
    let mut child = command.spawn()?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(RunOutcome::Exited(status));
        }
        if started.elapsed() >= timeout {
            child.kill()?;
            let _ = child.wait();
            return Ok(RunOutcome::TimedOut);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuntimeConfig;

    #[test]
    fn builds_podman_command_without_shell() {
        let spec = podman_run(
            &RuntimeConfig::default(),
            &PodmanJobSpec {
                job_id: "123".to_string(),
                runner_name: "runner-123".to_string(),
                repo_url: "https://github.com/org/project".to_string(),
                token_env_file: "/var/lib/runlet/jobs/123.runner.env".into(),
                labels: vec!["self-hosted".to_string(), "runlet".to_string()],
                network: NetworkPolicy::Restricted,
                cache_mount: None,
                cache_writable: false,
                secrets: "false".to_string(),
                registry_push: false,
                deploy: false,
            },
        );

        assert_eq!(spec.program, "podman");
        assert_eq!(spec.args[0], "run");
        assert!(spec.args.iter().any(|arg| arg == "--rm"));
        assert!(spec
            .args
            .iter()
            .any(|arg| arg == "slirp4netns:allow_host_loopback=false"));
        assert!(spec.args.iter().any(|arg| arg == "size=20G"));
        assert!(!spec
            .args
            .iter()
            .any(|arg| arg == "/var/lib/runlet/jobs/123:/workspace:Z"));
        assert!(spec
            .args
            .iter()
            .any(|arg| arg == "RUNNER_LABELS=self-hosted,runlet"));
        assert!(!spec.args.iter().any(|arg| arg == "sh"));
        assert!(!spec.args.iter().any(|arg| arg == "-c"));
        assert!(spec.args.iter().any(|arg| arg == "--env-file"));
        assert!(spec
            .args
            .iter()
            .any(|arg| arg == "/var/lib/runlet/jobs/123.runner.env"));
        assert!(!spec.args.iter().any(|arg| arg == "RUNNER_TOKEN=token"));
        assert!(spec.args.iter().any(|arg| arg == "RUNNER_EPHEMERAL=true"));
        assert!(spec.args.iter().any(|arg| arg == "RUNLET_SECRETS=false"));
    }

    #[test]
    fn removes_containers_idempotently() {
        let spec = podman_remove_container("123");

        assert_eq!(spec.program, "podman");
        assert!(spec.args.iter().any(|arg| arg == "--force"));
        assert!(spec.args.iter().any(|arg| arg == "--ignore"));
        assert!(spec.args.iter().any(|arg| arg == "runlet-123"));
    }

    #[test]
    fn mounts_cache_read_only_when_writes_are_denied() {
        let spec = podman_run(
            &RuntimeConfig::default(),
            &PodmanJobSpec {
                job_id: "123".to_string(),
                runner_name: "runner-123".to_string(),
                repo_url: "https://github.com/org/project".to_string(),
                token_env_file: "/var/lib/runlet/jobs/123.runner.env".into(),
                labels: vec!["self-hosted".to_string()],
                network: NetworkPolicy::Normal,
                cache_mount: Some("/var/cache/runlet/github_org_project".into()),
                cache_writable: false,
                secrets: "limited".to_string(),
                registry_push: true,
                deploy: false,
            },
        );

        assert!(spec
            .args
            .iter()
            .any(|arg| arg == "/var/cache/runlet/github_org_project:/cache:ro,Z"));
    }
}
