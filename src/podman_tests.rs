use super::*;
use crate::config::RuntimeConfig;

fn runtime() -> RuntimeConfig {
    RuntimeConfig {
        runner_image: "runner:latest".to_string(),
        ..RuntimeConfig::default()
    }
}

fn job(trust_class: TrustClass, network: NetworkPolicy) -> PodmanJobSpec {
    PodmanJobSpec {
        job_id: "123".to_string(),
        runner_name: "runner-123".to_string(),
        repo_url: "https://github.com/org/project".to_string(),
        token_env_file: "/var/lib/runlet/jobs/123.runner.env".into(),
        labels: vec!["self-hosted".to_string(), "runlet".to_string()],
        trust_class,
        network,
        cache_mount: None,
        cache_writable: false,
        secrets: "false".to_string(),
        registry_push: false,
        deploy: false,
    }
}

#[test]
fn builds_podman_command_without_shell() {
    let mut runtime = runtime();
    runtime.users.enabled = false;
    let spec = podman_run(
        &runtime,
        &job(TrustClass::Trusted, NetworkPolicy::Restricted),
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
fn public_pull_request_uses_strict_untrusted_profile() {
    let runtime = runtime();
    let spec = podman_run(&runtime, &job(TrustClass::Untrusted, NetworkPolicy::Strict));

    assert!(spec.args.iter().any(|arg| arg == "--read-only"));
    assert!(spec.args.iter().any(|arg| arg == "HOME=/tmp/runlet-home"));
    assert!(spec
        .args
        .iter()
        .any(|arg| arg == "/tmp:rw,nosuid,nodev,size=1G"));
    assert!(spec.args.iter().any(|arg| arg == "--pids-limit"));
    assert!(spec.args.iter().any(|arg| arg == "256"));
    assert!(spec.args.iter().any(|arg| arg == "nofile=1024:1024"));
    assert!(spec.args.iter().any(|arg| arg == "nproc=512:512"));
    assert!(spec.args.iter().any(|arg| arg == "--memory-swap"));
    assert!(spec.args.iter().any(|arg| arg == "2G"));
    assert!(spec
        .args
        .iter()
        .any(|arg| arg == "RUNLET_NETWORK_POLICY=strict"));
    assert!(spec.args.iter().any(|arg| arg == "--log-driver"));
    assert!(spec.args.iter().any(|arg| arg == "max-size=10m"));
}

#[test]
fn trusted_profile_is_not_read_only() {
    let runtime = runtime();
    let spec = podman_run(&runtime, &job(TrustClass::Trusted, NetworkPolicy::Normal));

    assert!(!spec.args.iter().any(|arg| arg == "--read-only"));
    assert!(spec.args.iter().any(|arg| arg == "slirp4netns"));
    assert!(spec.args.iter().any(|arg| arg == "nofile=4096:4096"));
}

#[test]
fn wraps_podman_with_execution_user_when_enabled() {
    let mut runtime = runtime();
    runtime.users.enabled = true;
    let spec = podman_run(&runtime, &job(TrustClass::Untrusted, NetworkPolicy::Strict));

    assert_eq!(spec.program, "sudo");
    assert_eq!(spec.args[0], "-n");
    assert!(spec.args.iter().any(|arg| arg == "runlet-untrusted"));
    assert!(spec.args.iter().any(|arg| arg == "podman"));
}

#[test]
fn passes_seccomp_and_lsm_options() {
    let mut runtime = runtime();
    runtime.untrusted.seccomp_profile = Some("/etc/runlet/seccomp-untrusted.json".into());
    runtime.untrusted.apparmor_profile = Some("runlet-untrusted".to_string());
    runtime.untrusted.selinux_label = Some("type:runlet_untrusted_t".to_string());
    let spec = podman_run(&runtime, &job(TrustClass::Untrusted, NetworkPolicy::Strict));

    assert!(spec
        .args
        .iter()
        .any(|arg| arg == "seccomp=/etc/runlet/seccomp-untrusted.json"));
    assert!(spec
        .args
        .iter()
        .any(|arg| arg == "apparmor=runlet-untrusted"));
    assert!(spec
        .args
        .iter()
        .any(|arg| arg == "label=type:runlet_untrusted_t"));
}

#[test]
fn scoped_prune_only_targets_runlet_labeled_resources() {
    let spec = podman_scoped_prune(Some("runlet-untrusted"), PodmanPruneResource::Containers);

    assert_eq!(spec.program, "sudo");
    assert!(spec.args.iter().any(|arg| arg == "runlet-untrusted"));
    assert!(spec.args.iter().any(|arg| arg == "container"));
    assert!(spec.args.iter().any(|arg| arg == "prune"));
    assert!(spec
        .args
        .iter()
        .any(|arg| arg == "label=runlet.managed=true"));
}

#[test]
fn removes_containers_idempotently() {
    let spec = podman_remove_container(None, "123");

    assert_eq!(spec.program, "podman");
    assert!(spec.args.iter().any(|arg| arg == "--force"));
    assert!(spec.args.iter().any(|arg| arg == "--ignore"));
    assert!(spec.args.iter().any(|arg| arg == "runlet-123"));

    let user_spec = podman_remove_container(Some("runlet-untrusted"), "123");
    assert_eq!(user_spec.program, "sudo");
    assert!(user_spec.args.iter().any(|arg| arg == "runlet-untrusted"));
    assert!(user_spec.args.iter().any(|arg| arg == "rm"));
}

#[test]
fn mounts_cache_read_only_when_writes_are_denied() {
    let runtime = runtime();
    let mut spec_job = job(TrustClass::Trusted, NetworkPolicy::Normal);
    spec_job.cache_mount = Some("/var/cache/runlet/github_org_project".into());
    spec_job.cache_writable = false;
    spec_job.secrets = "limited".to_string();
    spec_job.registry_push = true;
    let spec = podman_run(&runtime, &spec_job);

    assert!(spec
        .args
        .iter()
        .any(|arg| arg == "/var/cache/runlet/github_org_project:/cache:ro,Z"));
}
