use super::*;

fn valid_config() -> Config {
    let mut config = Config::default();
    config.github.app_id = 1;
    config.github.installation_id = 1;
    config.github.private_key_file = "/run/secrets/github-app.pem".into();
    config.orchestrator.webhook_secret_file = "/run/secrets/github-webhook".into();
    config.runtime.runner_image = "ghcr.io/davidvornholt/runlet-actions-runner:0.1.0".to_string();
    config
}

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

                [runtime.users]
                enabled = true
                orchestrator = "runlet-orchestrator"
                trusted = "runlet-trusted"
                untrusted = "runlet-untrusted"

                [runtime.untrusted]
                max_concurrent_jobs = 1
                pids_limit = 256
                ulimit_nofile = "1024:1024"
                ulimit_nproc = "512:512"
                log_driver = "k8s-file"
                log_size_max = "10m"

                [cache]
                enable = true
                backend = "local"
                path = "/var/cache/runlet"

                [repositories."github:org/project"]
                enabled = true
                trusted_branches = ["main", "release/*"]

                [repositories."github:org/project".public_pull_requests]
                enabled = true
                timeout = "15m"

                [repositories."github:org/project".trusted_jobs]
                allow_registry_push = true
                allow_deploy = false
            "#,
    )
    .expect("config should parse");

    config.validate().expect("config should be valid");
    assert_eq!(config.runtime.backend, RuntimeBackend::PodmanRootless);
    assert!(config.runtime.users.enabled);
    assert_eq!(config.runtime.untrusted.pids_limit, Some(256));
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
    let mut config = valid_config();
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
fn rejects_removed_untrusted_isolation_options() {
    let error = toml::from_str::<Config>(
        r#"
                [runtime.untrusted]
                read_only = false
            "#,
    )
    .expect_err("removed isolation option should be rejected");

    assert!(error.to_string().contains("unknown field `read_only`"));
}

#[test]
fn rejects_removed_public_pull_request_options() {
    let error = toml::from_str::<Config>(
        r#"
                [repositories."github:org/project".public_pull_requests]
                secrets = true
            "#,
    )
    .expect_err("removed public pull request option should be rejected");

    assert!(error.to_string().contains("unknown field `secrets`"));
}

#[test]
fn partial_untrusted_profile_overrides_keep_strict_defaults() {
    let config: Config = toml::from_str(
        r#"
                [runtime.untrusted]
                max_concurrent_jobs = 2
            "#,
    )
    .expect("partial runtime profile should parse");

    assert_eq!(config.runtime.untrusted.max_concurrent_jobs, 2);
    assert_eq!(config.runtime.untrusted.pids_limit, Some(256));
}

#[test]
fn strict_network_requires_user_split() {
    let mut config = valid_config();
    config.runtime.users.enabled = false;
    config.repositories.insert(
        "github:org/project".to_string(),
        RepositoryConfig {
            enabled: true,
            public_pull_requests: PublicPullRequestConfig {
                enabled: true,
                ..PublicPullRequestConfig::default()
            },
            ..RepositoryConfig::default()
        },
    );

    assert!(matches!(
        config.validate().unwrap_err(),
        ConfigError::StrictNetworkRequiresUserSplit
    ));
}

#[test]
fn rejects_invalid_resource_and_profile_values() {
    let mut config = valid_config();
    config.runtime.untrusted.pids_limit = Some(0);
    assert!(matches!(
        config.validate().unwrap_err(),
        ConfigError::InvalidProfileLimit {
            profile: "untrusted",
            field: "pids_limit"
        }
    ));

    config = valid_config();
    config.runtime.untrusted.seccomp_profile = Some(PathBuf::new());
    assert!(matches!(
        config.validate().unwrap_err(),
        ConfigError::EmptyProfileValue {
            profile: "untrusted",
            field: "seccomp_profile"
        }
    ));
}

#[test]
fn validates_execution_user_split() {
    let mut config = valid_config();
    config.runtime.users.enabled = true;
    config.runtime.users.untrusted.clear();
    assert!(matches!(
        config.validate().unwrap_err(),
        ConfigError::EmptyExecutionUser { field: "untrusted" }
    ));

    let mut config = valid_config();
    config.runtime.users.enabled = true;
    config.runtime.users.untrusted = config.runtime.users.trusted.clone();
    assert!(matches!(
        config.validate().unwrap_err(),
        ConfigError::NonDistinctExecutionUsers
    ));
}
