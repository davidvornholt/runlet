//! Checks that Podman will enforce a job's resource limits before Runlet
//! launches it. Rootless Podman without a delegated cgroup only warns and runs
//! the container without the limits, so Runlet refuses the job instead.

use crate::config::{RuntimeConfig, RuntimeProfileConfig, TrustClass};
use crate::podman::as_execution_user;
use crate::process::ProcessSpec;

const CGROUP_INFO_FORMAT: &str =
    "{{.Host.CgroupsVersion}} {{.Host.CgroupManager}} {{range .Host.CgroupControllers}}{{.}} {{end}}";

const UNCHECKED: &str = "can't check that Podman will enforce the job's resource limits";

pub fn podman_cgroup_info(runtime: &RuntimeConfig, trust_class: TrustClass) -> ProcessSpec {
    as_execution_user(
        runtime.execution_user(trust_class),
        vec!["info".into(), "--format".into(), CGROUP_INFO_FORMAT.into()],
    )
}

/// Runs `podman info` as the job's execution user and returns why Podman would
/// drop one of the profile's limits, if it would.
pub fn check_limits_enforceable(
    runtime: &RuntimeConfig,
    trust_class: TrustClass,
) -> Result<(), String> {
    let output = podman_cgroup_info(runtime, trust_class)
        .command()
        .output()
        .map_err(|error| format!("{UNCHECKED}: failed to run podman info: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{UNCHECKED}: podman info exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    limits_enforceable(
        runtime.profile(trust_class),
        &String::from_utf8_lossy(&output.stdout),
    )
}

/// Decides from `podman info` output in [`CGROUP_INFO_FORMAT`] whether Podman
/// can enforce the profile's CPU, memory, process, cpuset, and I/O limits.
pub fn limits_enforceable(profile: &RuntimeProfileConfig, cgroup_info: &str) -> Result<(), String> {
    let mut fields = cgroup_info.split_whitespace();
    let version = fields.next().unwrap_or("unknown");
    let manager = fields.next().unwrap_or("unknown");
    if version != "v2" || manager != "systemd" {
        return Err(format!(
            "Podman would drop the job's resource limits: it reports cgroups {version} with the {manager} cgroup manager, but rootless limits need cgroups v2 and a systemd user session for the execution user, for example through lingering"
        ));
    }
    let controllers: Vec<&str> = fields.collect();
    let mut required = vec!["cpu", "memory"];
    if profile.pids_limit.is_some() {
        required.push("pids");
    }
    if profile.cpuset_cpus.is_some() {
        required.push("cpuset");
    }
    if !profile.device_read_bps.is_empty() || !profile.device_write_bps.is_empty() {
        required.push("io");
    }
    let missing: Vec<&str> = required
        .into_iter()
        .filter(|controller| !controllers.contains(controller))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Podman would drop the job's resource limits: the execution user's cgroup lacks the {} controller, so delegate it to the user's systemd manager",
            missing.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn untrusted() -> RuntimeProfileConfig {
        RuntimeConfig::default().untrusted
    }

    #[test]
    fn accepts_delegated_systemd_cgroup_with_needed_controllers() {
        assert_eq!(
            limits_enforceable(&untrusted(), "v2 systemd cpu io memory pids \n"),
            Ok(())
        );
    }

    #[test]
    fn rejects_cgroupfs_fallback_that_drops_limits() {
        let error = limits_enforceable(&untrusted(), "v2 cgroupfs cpuset cpu io memory pids \n")
            .unwrap_err();
        assert!(error.contains("cgroupfs cgroup manager"));
    }

    #[test]
    fn rejects_cgroups_v1() {
        let error = limits_enforceable(&untrusted(), "v1 systemd cpu memory pids \n").unwrap_err();
        assert!(error.contains("cgroups v1"));
    }

    #[test]
    fn rejects_empty_podman_info_output() {
        let error = limits_enforceable(&untrusted(), "").unwrap_err();
        assert!(error.contains("cgroups unknown"));
    }

    #[test]
    fn requires_controllers_for_configured_limits() {
        let mut profile = untrusted();
        profile.cpuset_cpus = Some("0".to_string());
        profile.device_write_bps = vec!["/dev/vda:1mb".to_string()];
        let error = limits_enforceable(&profile, "v2 systemd cpu memory pids \n").unwrap_err();
        assert!(error.contains("lacks the cpuset, io controller"));

        profile.cpuset_cpus = None;
        profile.device_write_bps.clear();
        profile.pids_limit = None;
        assert_eq!(
            limits_enforceable(&profile, "v2 systemd cpu memory \n"),
            Ok(())
        );
    }

    #[test]
    fn runs_podman_info_as_the_execution_user() {
        let spec = podman_cgroup_info(&RuntimeConfig::default(), TrustClass::Untrusted);
        assert_eq!(spec.program, "sudo");
        assert_eq!(
            &spec.args[..6],
            ["-n", "-u", "runlet-untrusted", "--", "podman", "info"]
        );
    }
}
