use crate::duration::{parse_duration, DurationParseError};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("github.app_id must be greater than zero")]
    MissingAppId,
    #[error("github.installation_id must be greater than zero")]
    MissingInstallationId,
    #[error("github.private_key_file must not be empty")]
    MissingPrivateKey,
    #[error("runtime.max_concurrent_jobs must be at least 1")]
    InvalidConcurrency,
    #[error("runtime.default_cpu must be at least 1")]
    InvalidCpu,
    #[error("runtime.default_memory must not be empty")]
    InvalidMemory,
    #[error("runtime.default_disk must not be empty")]
    InvalidDisk,
    #[error("runtime.default_timeout must not be empty")]
    InvalidTimeout,
    #[error("runtime.runner_image must not be empty")]
    MissingRunnerImage,
    #[error("runtime.{profile}.max_concurrent_jobs must be at least 1")]
    InvalidProfileConcurrency { profile: &'static str },
    #[error("runtime.{profile}.{field} must be at least 1")]
    InvalidProfileLimit {
        profile: &'static str,
        field: &'static str,
    },
    #[error("runtime.{profile}.{field} must not be empty")]
    EmptyProfileValue {
        profile: &'static str,
        field: &'static str,
    },
    #[error("runtime.users.{field} must not be empty when runtime.users.enabled is true")]
    EmptyExecutionUser { field: &'static str },
    #[error("runtime.users orchestrator, trusted, and untrusted users must be distinct")]
    NonDistinctExecutionUsers,
    #[error("strict public pull request networking requires runtime.users.enabled = true")]
    StrictNetworkRequiresUserSplit,
    #[error("runtime.network.{field} must not be empty")]
    EmptyNetworkValue { field: &'static str },
    #[error("{name} has an invalid duration: {source}")]
    InvalidDuration {
        name: String,
        source: DurationParseError,
    },
    #[error("orchestrator.listen_addr must not be empty")]
    InvalidListenAddr,
    #[error("orchestrator.webhook_secret_file must not be empty")]
    MissingWebhookSecret,
    #[error("cache.path must not be empty when cache is enabled")]
    InvalidCachePath,
    #[error("repository {name} has an empty trusted branch pattern")]
    EmptyTrustedBranch { name: String },
    #[error("repository {name} cannot expose secrets to public pull requests")]
    PublicPullRequestSecrets { name: String },
    #[error("repository {name} workflow risk path pattern must not be empty")]
    EmptyWorkflowRiskPath { name: String },
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub github: GitHubConfig,
    pub orchestrator: OrchestratorConfig,
    pub runtime: RuntimeConfig,
    pub cache: CacheConfig,
    pub repositories: BTreeMap<String, RepositoryConfig>,
    pub state: StateConfig,
}

impl Config {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str::<Self>(&text)
            .map_err(|source| ConfigError::Parse {
                path: path.to_path_buf(),
                source,
            })
            .and_then(|config| {
                config.validate()?;
                Ok(config)
            })
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.github.app_id == 0 {
            return Err(ConfigError::MissingAppId);
        }
        if self.github.installation_id == 0 {
            return Err(ConfigError::MissingInstallationId);
        }
        if self.github.private_key_file.as_os_str().is_empty() {
            return Err(ConfigError::MissingPrivateKey);
        }
        if self.runtime.max_concurrent_jobs == 0 {
            return Err(ConfigError::InvalidConcurrency);
        }
        if self.runtime.default_cpu == 0 {
            return Err(ConfigError::InvalidCpu);
        }
        if self.runtime.default_memory.is_empty() {
            return Err(ConfigError::InvalidMemory);
        }
        if self.runtime.default_disk.is_empty() {
            return Err(ConfigError::InvalidDisk);
        }
        if self.runtime.default_timeout.is_empty() {
            return Err(ConfigError::InvalidTimeout);
        }
        validate_duration("runtime.default_timeout", &self.runtime.default_timeout)?;
        if self.runtime.runner_image.is_empty() {
            return Err(ConfigError::MissingRunnerImage);
        }
        validate_profile("trusted", &self.runtime.trusted)?;
        validate_profile("untrusted", &self.runtime.untrusted)?;
        validate_users(&self.runtime.users)?;
        validate_network_controls(&self.runtime.network)?;
        if self.orchestrator.listen_addr.is_empty() {
            return Err(ConfigError::InvalidListenAddr);
        }
        if self.orchestrator.webhook_secret_file.as_os_str().is_empty() {
            return Err(ConfigError::MissingWebhookSecret);
        }
        validate_duration(
            "orchestrator.cleanup_interval",
            &self.orchestrator.cleanup_interval,
        )?;
        if self.cache.enable && self.cache.path.as_os_str().is_empty() {
            return Err(ConfigError::InvalidCachePath);
        }
        for (name, repository) in &self.repositories {
            if repository.public_pull_requests.secrets {
                return Err(ConfigError::PublicPullRequestSecrets { name: name.clone() });
            }
            if repository
                .trusted_branches
                .iter()
                .any(|branch| branch.trim().is_empty())
            {
                return Err(ConfigError::EmptyTrustedBranch { name: name.clone() });
            }
            validate_duration(
                &format!("repositories.{name}.public_pull_requests.timeout"),
                &repository.public_pull_requests.timeout,
            )?;
            if repository.public_pull_requests.enabled
                && repository.public_pull_requests.network == NetworkPolicy::Strict
                && !self.runtime.users.enabled
            {
                return Err(ConfigError::StrictNetworkRequiresUserSplit);
            }
            if repository
                .workflow_risk
                .high_risk_paths
                .iter()
                .chain(repository.workflow_risk.additional_high_risk_paths.iter())
                .any(|pattern| pattern.trim().is_empty())
            {
                return Err(ConfigError::EmptyWorkflowRiskPath { name: name.clone() });
            }
        }
        Ok(())
    }
}

fn validate_duration(name: &str, value: &str) -> Result<(), ConfigError> {
    parse_duration(value)
        .map(|_| ())
        .map_err(|source| ConfigError::InvalidDuration {
            name: name.to_string(),
            source,
        })
}

fn validate_profile(
    profile_name: &'static str,
    profile: &RuntimeProfileConfig,
) -> Result<(), ConfigError> {
    if profile.max_concurrent_jobs == 0 {
        return Err(ConfigError::InvalidProfileConcurrency {
            profile: profile_name,
        });
    }
    if let Some(value) = profile.cpu {
        if value == 0 {
            return Err(ConfigError::InvalidProfileLimit {
                profile: profile_name,
                field: "cpu",
            });
        }
    }
    if let Some(value) = profile.pids_limit {
        if value == 0 {
            return Err(ConfigError::InvalidProfileLimit {
                profile: profile_name,
                field: "pids_limit",
            });
        }
    }
    for (field, value) in [
        ("memory", profile.memory.as_deref()),
        ("disk", profile.disk.as_deref()),
        ("timeout", profile.timeout.as_deref()),
        ("ulimit_nofile", profile.ulimit_nofile.as_deref()),
        ("ulimit_nproc", profile.ulimit_nproc.as_deref()),
        ("cpuset_cpus", profile.cpuset_cpus.as_deref()),
        ("memory_swap", profile.memory_swap.as_deref()),
        (
            "seccomp_profile",
            profile
                .seccomp_profile
                .as_ref()
                .map(|path| path.to_string_lossy())
                .as_deref(),
        ),
        ("apparmor_profile", profile.apparmor_profile.as_deref()),
        ("selinux_label", profile.selinux_label.as_deref()),
        ("log_driver", profile.log_driver.as_deref()),
        ("log_size_max", profile.log_size_max.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(ConfigError::EmptyProfileValue {
                profile: profile_name,
                field,
            });
        }
    }
    if let Some(timeout) = &profile.timeout {
        validate_duration(&format!("runtime.{profile_name}.timeout"), timeout)?;
    }
    if profile.tmpfs.iter().any(|value| value.trim().is_empty()) {
        return Err(ConfigError::EmptyProfileValue {
            profile: profile_name,
            field: "tmpfs",
        });
    }
    if profile
        .device_read_bps
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(ConfigError::EmptyProfileValue {
            profile: profile_name,
            field: "device_read_bps",
        });
    }
    if profile
        .device_write_bps
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(ConfigError::EmptyProfileValue {
            profile: profile_name,
            field: "device_write_bps",
        });
    }
    Ok(())
}

fn validate_users(users: &ExecutionUsersConfig) -> Result<(), ConfigError> {
    if users.enabled {
        if users.orchestrator.trim().is_empty() {
            return Err(ConfigError::EmptyExecutionUser {
                field: "orchestrator",
            });
        }
        if users.trusted.trim().is_empty() {
            return Err(ConfigError::EmptyExecutionUser { field: "trusted" });
        }
        if users.untrusted.trim().is_empty() {
            return Err(ConfigError::EmptyExecutionUser { field: "untrusted" });
        }
        if users.orchestrator == users.trusted
            || users.orchestrator == users.untrusted
            || users.trusted == users.untrusted
        {
            return Err(ConfigError::NonDistinctExecutionUsers);
        }
    }
    Ok(())
}

fn validate_network_controls(network: &NetworkControlsConfig) -> Result<(), ConfigError> {
    for (field, values) in [
        ("deny_cidrs", &network.deny_cidrs),
        ("allow_cidrs", &network.allow_cidrs),
        ("allow_hosts", &network.allow_hosts),
        ("allow_tcp_ports", &network.allow_tcp_ports),
        ("egress_proxy", &network.egress_proxy),
    ] {
        if values.iter().any(|value| value.trim().is_empty()) {
            return Err(ConfigError::EmptyNetworkValue { field });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GitHubConfig {
    pub app_id: u64,
    pub installation_id: u64,
    pub private_key_file: PathBuf,
    pub api_base_url: String,
}

impl Default for GitHubConfig {
    fn default() -> Self {
        Self {
            app_id: 0,
            installation_id: 0,
            private_key_file: PathBuf::new(),
            api_base_url: "https://api.github.com".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct OrchestratorConfig {
    pub listen_addr: String,
    pub webhook_secret_file: PathBuf,
    pub cleanup_interval: String,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8080".to_string(),
            webhook_secret_file: PathBuf::new(),
            cleanup_interval: "60s".to_string(),
        }
    }
}

pub use crate::runtime_config::{
    CleanupConfig, ExecutionUsersConfig, IpcMode, NetworkControlsConfig, RuntimeBackend,
    RuntimeConfig, RuntimeProfileConfig, StorageIsolationConfig, TrustClass,
};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CacheConfig {
    pub enable: bool,
    pub backend: CacheBackend,
    pub path: PathBuf,
    pub allow_untrusted_write: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enable: false,
            backend: CacheBackend::Local,
            path: PathBuf::from("/var/cache/runlet"),
            allow_untrusted_write: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CacheBackend {
    Local,
}

pub use crate::repository_config::{
    NetworkPolicy, PublicPullRequestConfig, RepositoryConfig, TrustedJobsConfig, WorkflowRiskConfig,
};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct StateConfig {
    pub database_path: PathBuf,
}

impl Default for StateConfig {
    fn default() -> Self {
        Self {
            database_path: PathBuf::from("/var/lib/runlet/runlet.sqlite3"),
        }
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
