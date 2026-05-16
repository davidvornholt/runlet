use serde::{Deserialize, Deserializer};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RuntimeConfig {
    pub backend: RuntimeBackend,
    pub max_concurrent_jobs: u32,
    pub default_cpu: u32,
    pub default_memory: String,
    pub default_disk: String,
    pub default_timeout: String,
    pub runner_image: String,
    pub jobs_dir: PathBuf,
    #[serde(
        default = "default_trusted_profile",
        deserialize_with = "deserialize_trusted_profile"
    )]
    pub trusted: RuntimeProfileConfig,
    #[serde(
        default = "default_untrusted_profile",
        deserialize_with = "deserialize_untrusted_profile"
    )]
    pub untrusted: RuntimeProfileConfig,
    pub users: ExecutionUsersConfig,
    pub network: NetworkControlsConfig,
    pub storage: StorageIsolationConfig,
    pub cleanup: CleanupConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            backend: RuntimeBackend::PodmanRootless,
            max_concurrent_jobs: 4,
            default_cpu: 2,
            default_memory: "4G".to_string(),
            default_disk: "20G".to_string(),
            default_timeout: "20m".to_string(),
            runner_image: String::new(),
            jobs_dir: PathBuf::from("/var/lib/runlet/jobs"),
            trusted: RuntimeProfileConfig::trusted_default(),
            untrusted: RuntimeProfileConfig::untrusted_default(),
            users: ExecutionUsersConfig::default(),
            network: NetworkControlsConfig::default(),
            storage: StorageIsolationConfig::default(),
            cleanup: CleanupConfig::default(),
        }
    }
}

fn default_trusted_profile() -> RuntimeProfileConfig {
    RuntimeProfileConfig::trusted_default()
}

fn default_untrusted_profile() -> RuntimeProfileConfig {
    RuntimeProfileConfig::untrusted_default()
}

fn deserialize_trusted_profile<'de, D>(deserializer: D) -> Result<RuntimeProfileConfig, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_profile_with_base(deserializer, RuntimeProfileConfig::trusted_default())
}

fn deserialize_untrusted_profile<'de, D>(deserializer: D) -> Result<RuntimeProfileConfig, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_profile_with_base(deserializer, RuntimeProfileConfig::untrusted_default())
}

fn deserialize_profile_with_base<'de, D>(
    deserializer: D,
    mut profile: RuntimeProfileConfig,
) -> Result<RuntimeProfileConfig, D::Error>
where
    D: Deserializer<'de>,
{
    let patch = RuntimeProfileConfigPatch::deserialize(deserializer)?;
    patch.apply_to(&mut profile);
    Ok(profile)
}

impl RuntimeConfig {
    pub fn profile(&self, trust: TrustClass) -> &RuntimeProfileConfig {
        match trust {
            TrustClass::Trusted => &self.trusted,
            TrustClass::Untrusted => &self.untrusted,
        }
    }

    pub fn execution_user(&self, trust: TrustClass) -> Option<&str> {
        if !self.users.enabled {
            return None;
        }
        Some(match trust {
            TrustClass::Trusted => self.users.trusted.as_str(),
            TrustClass::Untrusted => self.users.untrusted.as_str(),
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TrustClass {
    Trusted,
    Untrusted,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeBackend {
    PodmanRootless,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RuntimeProfileConfig {
    pub max_concurrent_jobs: u32,
    pub cpu: Option<u32>,
    pub memory: Option<String>,
    pub disk: Option<String>,
    pub timeout: Option<String>,
    pub read_only: bool,
    pub tmpfs: Vec<String>,
    pub pids_limit: Option<u32>,
    pub ulimit_nofile: Option<String>,
    pub ulimit_nproc: Option<String>,
    pub memory_swap: Option<String>,
    pub ipc: IpcMode,
    pub cpuset_cpus: Option<String>,
    pub device_read_bps: Vec<String>,
    pub device_write_bps: Vec<String>,
    pub seccomp_profile: Option<PathBuf>,
    pub apparmor_profile: Option<String>,
    pub selinux_label: Option<String>,
    pub log_driver: Option<String>,
    pub log_size_max: Option<String>,
    pub disable_host_log_capture: bool,
}

impl RuntimeProfileConfig {
    fn trusted_default() -> Self {
        Self {
            max_concurrent_jobs: 4,
            cpu: None,
            memory: None,
            disk: None,
            timeout: None,
            read_only: false,
            tmpfs: Vec::new(),
            pids_limit: Some(2048),
            ulimit_nofile: Some("4096:4096".to_string()),
            ulimit_nproc: Some("2048:2048".to_string()),
            memory_swap: None,
            ipc: IpcMode::Private,
            cpuset_cpus: None,
            device_read_bps: Vec::new(),
            device_write_bps: Vec::new(),
            seccomp_profile: None,
            apparmor_profile: None,
            selinux_label: None,
            log_driver: None,
            log_size_max: None,
            disable_host_log_capture: false,
        }
    }

    fn untrusted_default() -> Self {
        Self {
            max_concurrent_jobs: 1,
            cpu: Some(1),
            memory: Some("2G".to_string()),
            disk: Some("10G".to_string()),
            timeout: Some("15m".to_string()),
            read_only: true,
            tmpfs: vec![
                "/tmp:rw,nosuid,nodev,size=1G".to_string(),
                "/run:rw,nosuid,nodev,size=64M".to_string(),
            ],
            pids_limit: Some(256),
            ulimit_nofile: Some("1024:1024".to_string()),
            ulimit_nproc: Some("512:512".to_string()),
            memory_swap: Some("memory".to_string()),
            ipc: IpcMode::Private,
            cpuset_cpus: None,
            device_read_bps: Vec::new(),
            device_write_bps: Vec::new(),
            seccomp_profile: None,
            apparmor_profile: None,
            selinux_label: None,
            log_driver: Some("k8s-file".to_string()),
            log_size_max: Some("10m".to_string()),
            disable_host_log_capture: true,
        }
    }
}

impl Default for RuntimeProfileConfig {
    fn default() -> Self {
        Self::trusted_default()
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct RuntimeProfileConfigPatch {
    max_concurrent_jobs: Option<u32>,
    cpu: Option<u32>,
    memory: Option<String>,
    disk: Option<String>,
    timeout: Option<String>,
    read_only: Option<bool>,
    tmpfs: Option<Vec<String>>,
    pids_limit: Option<u32>,
    ulimit_nofile: Option<String>,
    ulimit_nproc: Option<String>,
    memory_swap: Option<String>,
    ipc: Option<IpcMode>,
    cpuset_cpus: Option<String>,
    device_read_bps: Option<Vec<String>>,
    device_write_bps: Option<Vec<String>>,
    seccomp_profile: Option<PathBuf>,
    apparmor_profile: Option<String>,
    selinux_label: Option<String>,
    log_driver: Option<String>,
    log_size_max: Option<String>,
    disable_host_log_capture: Option<bool>,
}

impl RuntimeProfileConfigPatch {
    fn apply_to(self, profile: &mut RuntimeProfileConfig) {
        if let Some(value) = self.max_concurrent_jobs {
            profile.max_concurrent_jobs = value;
        }
        if let Some(value) = self.cpu {
            profile.cpu = Some(value);
        }
        if let Some(value) = self.memory {
            profile.memory = Some(value);
        }
        if let Some(value) = self.disk {
            profile.disk = Some(value);
        }
        if let Some(value) = self.timeout {
            profile.timeout = Some(value);
        }
        if let Some(value) = self.read_only {
            profile.read_only = value;
        }
        if let Some(value) = self.tmpfs {
            profile.tmpfs = value;
        }
        if let Some(value) = self.pids_limit {
            profile.pids_limit = Some(value);
        }
        if let Some(value) = self.ulimit_nofile {
            profile.ulimit_nofile = Some(value);
        }
        if let Some(value) = self.ulimit_nproc {
            profile.ulimit_nproc = Some(value);
        }
        if let Some(value) = self.memory_swap {
            profile.memory_swap = Some(value);
        }
        if let Some(value) = self.ipc {
            profile.ipc = value;
        }
        if let Some(value) = self.cpuset_cpus {
            profile.cpuset_cpus = Some(value);
        }
        if let Some(value) = self.device_read_bps {
            profile.device_read_bps = value;
        }
        if let Some(value) = self.device_write_bps {
            profile.device_write_bps = value;
        }
        if let Some(value) = self.seccomp_profile {
            profile.seccomp_profile = Some(value);
        }
        if let Some(value) = self.apparmor_profile {
            profile.apparmor_profile = Some(value);
        }
        if let Some(value) = self.selinux_label {
            profile.selinux_label = Some(value);
        }
        if let Some(value) = self.log_driver {
            profile.log_driver = Some(value);
        }
        if let Some(value) = self.log_size_max {
            profile.log_size_max = Some(value);
        }
        if let Some(value) = self.disable_host_log_capture {
            profile.disable_host_log_capture = value;
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IpcMode {
    Private,
    Host,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExecutionUsersConfig {
    pub enabled: bool,
    pub orchestrator: String,
    pub trusted: String,
    pub untrusted: String,
}

impl Default for ExecutionUsersConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            orchestrator: "runlet-orchestrator".to_string(),
            trusted: "runlet-trusted".to_string(),
            untrusted: "runlet-untrusted".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NetworkControlsConfig {
    pub enable_untrusted_firewall: bool,
    pub deny_cidrs: Vec<String>,
    pub allow_cidrs: Vec<String>,
    pub allow_hosts: Vec<String>,
    pub allow_tcp_ports: Vec<String>,
    pub egress_proxy: Vec<String>,
}

impl Default for NetworkControlsConfig {
    fn default() -> Self {
        Self {
            enable_untrusted_firewall: true,
            deny_cidrs: vec![
                "0.0.0.0/8".to_string(),
                "10.0.0.0/8".to_string(),
                "100.64.0.0/10".to_string(),
                "127.0.0.0/8".to_string(),
                "169.254.0.0/16".to_string(),
                "172.16.0.0/12".to_string(),
                "192.168.0.0/16".to_string(),
                "224.0.0.0/4".to_string(),
                "::1/128".to_string(),
                "fc00::/7".to_string(),
                "fe80::/10".to_string(),
            ],
            allow_cidrs: Vec::new(),
            allow_hosts: Vec::new(),
            allow_tcp_ports: vec!["80".to_string(), "443".to_string()],
            egress_proxy: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct StorageIsolationConfig {
    pub trusted_storage_root: PathBuf,
    pub untrusted_storage_root: PathBuf,
    pub warn_free_space_bytes: u64,
}

impl Default for StorageIsolationConfig {
    fn default() -> Self {
        Self {
            trusted_storage_root: PathBuf::from("/var/lib/runlet/podman-trusted"),
            untrusted_storage_root: PathBuf::from("/var/lib/runlet/podman-untrusted"),
            warn_free_space_bytes: 5 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CleanupConfig {
    pub enable_scoped_prune: bool,
    pub prune_images: bool,
    pub prune_volumes: bool,
    pub prune_containers: bool,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            enable_scoped_prune: true,
            prune_images: false,
            prune_volumes: false,
            prune_containers: true,
        }
    }
}
