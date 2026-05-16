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
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeBackend {
    PodmanRootless,
}

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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RepositoryConfig {
    pub enabled: bool,
    pub public_pull_requests: PublicPullRequestConfig,
    pub trusted_branches: Vec<String>,
    pub trusted_jobs: TrustedJobsConfig,
}

impl Default for RepositoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            public_pull_requests: PublicPullRequestConfig::default(),
            trusted_branches: vec!["main".to_string()],
            trusted_jobs: TrustedJobsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PublicPullRequestConfig {
    pub enabled: bool,
    pub secrets: bool,
    pub network: NetworkPolicy,
    pub cache_write: bool,
    pub timeout: String,
}

impl Default for PublicPullRequestConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            secrets: false,
            network: NetworkPolicy::Restricted,
            cache_write: false,
            timeout: "20m".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkPolicy {
    Restricted,
    Normal,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct TrustedJobsConfig {
    pub allow_registry_push: bool,
    pub allow_deploy: bool,
}

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
mod tests {
    use super::*;

    #[test]
    fn parses_plan_shaped_config() {
        let config: Config = toml::from_str(
            r#"
                [github]
                app_id = 123456
                installation_id = 987654
                private_key_file = "/run/secrets/github-app.pem"
                api_base_url = "https://api.github.com"

                [orchestrator]
                listen_addr = "127.0.0.1:8080"
                webhook_secret_file = "/run/secrets/github-webhook"
                cleanup_interval = "60s"

                [runtime]
                backend = "podman-rootless"
                max_concurrent_jobs = 4
                default_cpu = 2
                default_memory = "4G"
                default_disk = "20G"
                default_timeout = "20m"
                runner_image = "ghcr.io/davidvornholt/runlet-actions-runner:0.1.0"

                [cache]
                enable = true
                backend = "local"
                path = "/var/cache/runlet"
                allow_untrusted_write = false

                [repositories."github:org/project"]
                enabled = true
                trusted_branches = ["main", "release/*"]

                [repositories."github:org/project".public_pull_requests]
                enabled = true
                secrets = false
                network = "restricted"
                cache_write = false
                timeout = "15m"

                [repositories."github:org/project".trusted_jobs]
                allow_registry_push = true
                allow_deploy = false
            "#,
        )
        .expect("config should parse");

        config.validate().expect("config should be valid");
        assert_eq!(config.runtime.backend, RuntimeBackend::PodmanRootless);
        assert!(config.cache.enable);
        assert_eq!(
            config.repositories["github:org/project"].trusted_branches,
            ["main", "release/*"]
        );
    }

    #[test]
    fn rejects_missing_github_credentials() {
        let error = Config::default()
            .validate()
            .expect_err("config should fail");
        assert!(matches!(error, ConfigError::MissingAppId));
    }

    #[test]
    fn rejects_invalid_duration_values() {
        let mut config = Config::default();
        config.github.app_id = 1;
        config.github.installation_id = 1;
        config.github.private_key_file = "/run/secrets/github-app.pem".into();
        config.orchestrator.webhook_secret_file = "/run/secrets/github-webhook".into();
        config.runtime.runner_image =
            "ghcr.io/davidvornholt/runlet-actions-runner:0.1.0".to_string();
        config.orchestrator.cleanup_interval = "soon".to_string();

        assert!(matches!(
            config.validate().unwrap_err(),
            ConfigError::InvalidDuration { name, .. } if name == "orchestrator.cleanup_interval"
        ));

        config.orchestrator.cleanup_interval = "60s".to_string();
        config.runtime.default_timeout = "20".to_string();
        assert!(matches!(
            config.validate().unwrap_err(),
            ConfigError::InvalidDuration { name, .. } if name == "runtime.default_timeout"
        ));

        config.runtime.default_timeout = "20m".to_string();
        config.repositories.insert(
            "github:org/project".to_string(),
            RepositoryConfig {
                public_pull_requests: PublicPullRequestConfig {
                    timeout: "later".to_string(),
                    ..PublicPullRequestConfig::default()
                },
                ..RepositoryConfig::default()
            },
        );
        assert!(matches!(
            config.validate().unwrap_err(),
            ConfigError::InvalidDuration { name, .. }
                if name == "repositories.github:org/project.public_pull_requests.timeout"
        ));
    }

    #[test]
    fn rejects_public_pull_request_secrets() {
        let config: Config = toml::from_str(
            r#"
                [github]
                app_id = 123456
                installation_id = 987654
                private_key_file = "/run/secrets/github-app.pem"

                [orchestrator]
                listen_addr = "127.0.0.1:8080"
                webhook_secret_file = "/run/secrets/github-webhook"

                [runtime]
                runner_image = "ghcr.io/davidvornholt/runlet-actions-runner:0.1.0"

                [repositories."github:org/project"]
                enabled = true

                [repositories."github:org/project".public_pull_requests]
                secrets = true
            "#,
        )
        .unwrap();

        assert!(matches!(
            config.validate().unwrap_err(),
            ConfigError::PublicPullRequestSecrets { .. }
        ));
    }
}
