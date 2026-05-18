use crate::config::{NetworkPolicy, RuntimeConfig, RuntimeProfileConfig, TrustClass};
use crate::process::ProcessSpec;
use crate::runtime_config::UNTRUSTED_TMPFS;
use std::ffi::{OsStr, OsString};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodmanJobSpec {
    pub job_id: String,
    pub runner_name: String,
    pub repo_url: String,
    pub token_env_file: OsString,
    pub labels: Vec<String>,
    pub trust_class: TrustClass,
    pub network: NetworkPolicy,
    pub cache_mount: Option<OsString>,
    pub cache_writable: bool,
    pub secrets: String,
    pub registry_push: bool,
    pub deploy: bool,
}

pub fn podman_run(runtime: &RuntimeConfig, job: &PodmanJobSpec) -> ProcessSpec {
    let profile = runtime.profile(job.trust_class);
    let cpus = profile.cpu.unwrap_or(runtime.default_cpu).to_string();
    let memory = profile
        .memory
        .as_ref()
        .unwrap_or(&runtime.default_memory)
        .clone();
    let disk = format!(
        "size={}",
        profile.disk.as_ref().unwrap_or(&runtime.default_disk)
    );
    let network = match job.network {
        NetworkPolicy::Strict | NetworkPolicy::Restricted => {
            "slirp4netns:allow_host_loopback=false"
        }
        NetworkPolicy::Normal => "slirp4netns",
        NetworkPolicy::Offline => "none",
    };

    let mut podman_args: Vec<OsString> = vec![
        "run".into(),
        "--rm".into(),
        "--name".into(),
        format!("runlet-{}", job.job_id).into(),
        "--label".into(),
        "runlet.managed=true".into(),
        "--label".into(),
        format!("runlet.job-id={}", job.job_id).into(),
        "--cpus".into(),
        cpus.into(),
        "--memory".into(),
        memory.clone().into(),
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
    ];

    append_profile_args(&mut podman_args, profile);

    if job.trust_class == TrustClass::Untrusted {
        podman_args.push("--read-only".into());
        for tmpfs in UNTRUSTED_TMPFS {
            podman_args.extend(["--tmpfs".into(), (*tmpfs).into()]);
        }
        podman_args.extend(["--memory-swap".into(), memory.clone().into()]);
        podman_args.extend(["--env".into(), "HOME=/tmp/runlet-home".into()]);
    }

    if job.network == NetworkPolicy::Strict {
        podman_args.extend(["--env".into(), "RUNLET_NETWORK_POLICY=strict".into()]);
        for proxy in &runtime.network.egress_proxy {
            podman_args.extend([
                "--env".into(),
                format!("RUNLET_EGRESS_PROXY={proxy}").into(),
            ]);
        }
    }

    if let Some(cache_mount) = &job.cache_mount {
        let suffix = if job.cache_writable { "Z" } else { "ro,Z" };
        podman_args.extend([
            "--volume".into(),
            format!("{}:/cache:{suffix}", Path::new(cache_mount).display()).into(),
        ]);
    }

    podman_args.extend([
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

    if let Some(user) = runtime.execution_user(job.trust_class) {
        let mut args: Vec<OsString> = vec![
            "-n".into(),
            "-u".into(),
            user.into(),
            "--".into(),
            "podman".into(),
        ];
        args.extend(podman_args);
        ProcessSpec {
            program: OsString::from("sudo"),
            args,
        }
    } else {
        ProcessSpec {
            program: OsString::from("podman"),
            args: podman_args,
        }
    }
}

fn append_profile_args(args: &mut Vec<OsString>, profile: &RuntimeProfileConfig) {
    if let Some(limit) = profile.pids_limit {
        args.extend(["--pids-limit".into(), limit.to_string().into()]);
    }
    if let Some(limit) = &profile.ulimit_nofile {
        args.extend(["--ulimit".into(), format!("nofile={limit}").into()]);
    }
    if let Some(limit) = &profile.ulimit_nproc {
        args.extend(["--ulimit".into(), format!("nproc={limit}").into()]);
    }
    args.extend(["--ipc".into(), "private".into()]);
    if let Some(cpus) = &profile.cpuset_cpus {
        args.extend(["--cpuset-cpus".into(), cpus.clone().into()]);
    }
    for throttle in &profile.device_read_bps {
        args.extend(["--device-read-bps".into(), throttle.clone().into()]);
    }
    for throttle in &profile.device_write_bps {
        args.extend(["--device-write-bps".into(), throttle.clone().into()]);
    }
    if let Some(path) = &profile.seccomp_profile {
        args.extend([
            "--security-opt".into(),
            format!("seccomp={}", path.display()).into(),
        ]);
    }
    if let Some(profile_name) = &profile.apparmor_profile {
        args.extend([
            "--security-opt".into(),
            format!("apparmor={profile_name}").into(),
        ]);
    }
    if let Some(label) = &profile.selinux_label {
        args.extend(["--security-opt".into(), format!("label={label}").into()]);
    }
    if let Some(driver) = &profile.log_driver {
        args.extend(["--log-driver".into(), driver.clone().into()]);
    }
    if let Some(size) = &profile.log_size_max {
        args.extend(["--log-opt".into(), format!("max-size={size}").into()]);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PodmanPruneResource {
    Containers,
    Images,
    Volumes,
}

pub fn podman_scoped_prune(user: Option<&str>, resource: PodmanPruneResource) -> ProcessSpec {
    let resource_arg = match resource {
        PodmanPruneResource::Containers => "container",
        PodmanPruneResource::Images => "image",
        PodmanPruneResource::Volumes => "volume",
    };
    let podman_args = vec![
        resource_arg.into(),
        "prune".into(),
        "--force".into(),
        "--filter".into(),
        "label=runlet.managed=true".into(),
    ];
    if let Some(user) = user {
        let mut args: Vec<OsString> = vec![
            "-n".into(),
            "-u".into(),
            user.into(),
            "--".into(),
            "podman".into(),
        ];
        args.extend(podman_args);
        ProcessSpec {
            program: OsString::from("sudo"),
            args,
        }
    } else {
        ProcessSpec {
            program: OsString::from("podman"),
            args: podman_args,
        }
    }
}

pub fn podman_remove_container(user: Option<&str>, job_id: impl AsRef<OsStr>) -> ProcessSpec {
    let podman_args = vec![
        "rm".into(),
        "--force".into(),
        "--ignore".into(),
        format!("runlet-{}", job_id.as_ref().to_string_lossy()).into(),
    ];
    if let Some(user) = user {
        let mut args: Vec<OsString> = vec![
            "-n".into(),
            "-u".into(),
            user.into(),
            "--".into(),
            "podman".into(),
        ];
        args.extend(podman_args);
        ProcessSpec {
            program: OsString::from("sudo"),
            args,
        }
    } else {
        ProcessSpec {
            program: OsString::from("podman"),
            args: podman_args,
        }
    }
}

#[cfg(test)]
#[path = "podman_tests.rs"]
mod tests;
