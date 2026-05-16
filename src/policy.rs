use crate::config::{Config, NetworkPolicy};
use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubEventKind {
    PullRequestFromFork,
    BranchPush,
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobContext {
    pub repository: String,
    pub event: GitHubEventKind,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectivePolicy {
    pub secrets: SecretPolicy,
    pub registry_push: bool,
    pub deploy: bool,
    pub network: NetworkPolicy,
    pub cache_write: bool,
    pub privileged: bool,
    pub timeout: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretPolicy {
    False,
    Limited,
    Allowed,
}

impl fmt::Display for SecretPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::False => "false",
            Self::Limited => "limited",
            Self::Allowed => "allowed",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow(EffectivePolicy),
    Deny { reason: String },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyViolation {
    #[error("job requested secrets, but secrets are disabled for this trust level")]
    SecretsDenied,
    #[error("job requested registry push, but registry push is disabled for this repository")]
    RegistryPushDenied,
    #[error("job requested deploy, but deploy is disabled for this repository")]
    DeployDenied,
    #[error("job requested privileged execution, but privileged containers are disabled")]
    PrivilegedDenied,
}

pub fn validate_capability_labels(
    policy: &EffectivePolicy,
    labels: &[String],
) -> Result<(), PolicyViolation> {
    let has_label = |expected: &str| labels.iter().any(|label| label == expected);
    if has_label("runlet-secrets") && policy.secrets == SecretPolicy::False {
        return Err(PolicyViolation::SecretsDenied);
    }
    if has_label("runlet-registry-push") && !policy.registry_push {
        return Err(PolicyViolation::RegistryPushDenied);
    }
    if has_label("runlet-deploy") && !policy.deploy {
        return Err(PolicyViolation::DeployDenied);
    }
    if has_label("runlet-privileged") && !policy.privileged {
        return Err(PolicyViolation::PrivilegedDenied);
    }
    Ok(())
}

pub fn decide(config: &Config, context: &JobContext) -> PolicyDecision {
    let Some(repository) = config.repositories.get(&context.repository) else {
        return PolicyDecision::Deny {
            reason: format!("repository {} is not configured", context.repository),
        };
    };

    if !repository.enabled {
        return PolicyDecision::Deny {
            reason: format!("repository {} is disabled", context.repository),
        };
    }

    match context.event {
        GitHubEventKind::PullRequestFromFork => {
            if !repository.public_pull_requests.enabled {
                return PolicyDecision::Deny {
                    reason: "public pull requests are disabled".to_string(),
                };
            }

            PolicyDecision::Allow(EffectivePolicy {
                secrets: SecretPolicy::False,
                registry_push: false,
                deploy: false,
                network: repository.public_pull_requests.network.clone(),
                cache_write: repository.public_pull_requests.cache_write
                    && config.cache.allow_untrusted_write,
                privileged: false,
                timeout: repository.public_pull_requests.timeout.clone(),
            })
        }
        GitHubEventKind::BranchPush => {
            if !is_trusted_branch(&repository.trusted_branches, &context.branch) {
                return PolicyDecision::Deny {
                    reason: format!("branch {} is not trusted", context.branch),
                };
            }

            PolicyDecision::Allow(EffectivePolicy {
                secrets: SecretPolicy::Limited,
                registry_push: repository.trusted_jobs.allow_registry_push,
                deploy: repository.trusted_jobs.allow_deploy,
                network: NetworkPolicy::Normal,
                cache_write: config.cache.enable,
                privileged: false,
                timeout: config.runtime.default_timeout.clone(),
            })
        }
        GitHubEventKind::Release => PolicyDecision::Allow(EffectivePolicy {
            secrets: SecretPolicy::Allowed,
            registry_push: repository.trusted_jobs.allow_registry_push,
            deploy: repository.trusted_jobs.allow_deploy,
            network: NetworkPolicy::Normal,
            cache_write: config.cache.enable,
            privileged: false,
            timeout: config.runtime.default_timeout.clone(),
        }),
    }
}

fn is_trusted_branch(patterns: &[String], branch: &str) -> bool {
    patterns.iter().any(|pattern| {
        if let Some(prefix) = pattern.strip_suffix("/*") {
            branch.starts_with(prefix) && branch[prefix.len()..].starts_with('/')
        } else {
            pattern == branch
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        CacheConfig, PublicPullRequestConfig, RepositoryConfig, RuntimeConfig, TrustedJobsConfig,
    };
    use std::collections::BTreeMap;

    fn configured() -> Config {
        Config {
            runtime: RuntimeConfig {
                default_timeout: "45m".to_string(),
                ..RuntimeConfig::default()
            },
            cache: CacheConfig {
                enable: true,
                allow_untrusted_write: false,
                ..CacheConfig::default()
            },
            repositories: BTreeMap::from([(
                "github:org/project".to_string(),
                RepositoryConfig {
                    enabled: true,
                    public_pull_requests: PublicPullRequestConfig {
                        enabled: true,
                        timeout: "15m".to_string(),
                        ..PublicPullRequestConfig::default()
                    },
                    trusted_branches: vec!["main".to_string(), "release/*".to_string()],
                    trusted_jobs: TrustedJobsConfig {
                        allow_registry_push: true,
                        allow_deploy: false,
                    },
                },
            )]),
            ..Config::default()
        }
    }

    #[test]
    fn untrusted_public_pull_request_gets_restricted_policy() {
        let config = configured();
        let decision = decide(
            &config,
            &JobContext {
                repository: "github:org/project".to_string(),
                event: GitHubEventKind::PullRequestFromFork,
                branch: "feature".to_string(),
            },
        );

        let PolicyDecision::Allow(policy) = decision else {
            panic!("public pull request should be allowed");
        };
        assert_eq!(policy.secrets, SecretPolicy::False);
        assert!(!policy.cache_write);
        assert!(!policy.privileged);
        assert_eq!(policy.network, NetworkPolicy::Restricted);
        assert_eq!(policy.timeout, "15m");
    }

    #[test]
    fn trusted_branch_push_gets_limited_secrets_and_cache_write() {
        let config = configured();
        let decision = decide(
            &config,
            &JobContext {
                repository: "github:org/project".to_string(),
                event: GitHubEventKind::BranchPush,
                branch: "release/2026-05".to_string(),
            },
        );

        let PolicyDecision::Allow(policy) = decision else {
            panic!("trusted branch should be allowed");
        };
        assert_eq!(policy.secrets, SecretPolicy::Limited);
        assert!(policy.registry_push);
        assert!(policy.cache_write);
        assert!(!policy.privileged);
        assert_eq!(policy.timeout, "45m");
    }

    #[test]
    fn untrusted_branch_is_denied() {
        let config = configured();
        let decision = decide(
            &config,
            &JobContext {
                repository: "github:org/project".to_string(),
                event: GitHubEventKind::BranchPush,
                branch: "feature".to_string(),
            },
        );

        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }

    #[test]
    fn denied_capability_labels_block_jobs() {
        let policy = EffectivePolicy {
            secrets: SecretPolicy::False,
            registry_push: false,
            deploy: false,
            network: NetworkPolicy::Restricted,
            cache_write: false,
            privileged: false,
            timeout: "15m".to_string(),
        };

        assert_eq!(
            validate_capability_labels(&policy, &["runlet-secrets".to_string()]).unwrap_err(),
            PolicyViolation::SecretsDenied
        );
        assert_eq!(
            validate_capability_labels(&policy, &["runlet-registry-push".to_string()]).unwrap_err(),
            PolicyViolation::RegistryPushDenied
        );
        assert_eq!(
            validate_capability_labels(&policy, &["runlet-deploy".to_string()]).unwrap_err(),
            PolicyViolation::DeployDenied
        );
        assert_eq!(
            validate_capability_labels(&policy, &["runlet-privileged".to_string()]).unwrap_err(),
            PolicyViolation::PrivilegedDenied
        );
    }
}
