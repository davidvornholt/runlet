self:
{ config, lib, pkgs, ... }:

let
  cfg = config.services.runlet;
  settingsFormat = pkgs.formats.toml { };
  configFile = settingsFormat.generate "runlet-config.toml" {
    github = {
      app_id = cfg.github.appId;
      installation_id = cfg.github.installationId;
      private_key_file = cfg.github.privateKeyFile;
      api_base_url = cfg.github.apiBaseUrl;
    };
    orchestrator = {
      listen_addr = cfg.orchestrator.listenAddr;
      webhook_secret_file = cfg.orchestrator.webhookSecretFile;
      cleanup_interval = cfg.orchestrator.cleanupInterval;
    };
    runtime = {
      backend = cfg.runtime.backend;
      max_concurrent_jobs = cfg.runtime.maxConcurrentJobs;
      default_cpu = cfg.runtime.defaultCpu;
      default_memory = cfg.runtime.defaultMemory;
      default_disk = cfg.runtime.defaultDisk;
      default_timeout = cfg.runtime.defaultTimeout;
      runner_image = cfg.runtime.runnerImage;
      jobs_dir = cfg.runtime.jobsDir;
    };
    cache = {
      enable = cfg.cache.enable;
      backend = cfg.cache.backend;
      path = cfg.cache.path;
      allow_untrusted_write = cfg.cache.allowUntrustedWrite;
    };
    state = {
      database_path = cfg.state.databasePath;
    };
    repositories = lib.mapAttrs (_: repo: {
      enabled = repo.enabled;
      trusted_branches = repo.trustedBranches;
      public_pull_requests = {
        enabled = repo.publicPullRequests.enabled;
        secrets = repo.publicPullRequests.secrets;
        network = repo.publicPullRequests.network;
        cache_write = repo.publicPullRequests.cacheWrite;
        timeout = repo.publicPullRequests.timeout;
      };
      trusted_jobs = {
        allow_registry_push = repo.trustedJobs.allowRegistryPush;
        allow_deploy = repo.trustedJobs.allowDeploy;
      };
    }) cfg.repositories;
  };
  package = cfg.package;
  databaseDir = builtins.dirOf cfg.state.databasePath;
in
{
  options.services.runlet = {
    enable = lib.mkEnableOption "Runlet ephemeral GitHub Actions runner orchestrator";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.runlet;
      defaultText = lib.literalExpression "runlet.packages.${pkgs.stdenv.hostPlatform.system}.runlet";
      description = "Runlet package to run.";
    };

    github = {
      appId = lib.mkOption {
        type = lib.types.ints.positive;
        description = "GitHub App ID used to register ephemeral runners.";
      };

      installationId = lib.mkOption {
        type = lib.types.ints.positive;
        description = "GitHub App installation ID.";
      };

      privateKeyFile = lib.mkOption {
        type = lib.types.str;
        example = "/run/secrets/github-app.pem";
        description = "Runtime path to the GitHub App private key file. Use a string path so secrets are not copied into the Nix store.";
      };

      apiBaseUrl = lib.mkOption {
        type = lib.types.str;
        default = "https://api.github.com";
        description = "Base URL for the GitHub API.";
      };
    };

    orchestrator = {
      listenAddr = lib.mkOption {
        type = lib.types.str;
        default = "127.0.0.1:8080";
        description = "Address where Runlet listens for GitHub webhooks.";
      };

      webhookSecretFile = lib.mkOption {
        type = lib.types.str;
        example = "/run/secrets/github-webhook";
        description = "Runtime path to the GitHub webhook secret file. Use a string path so secrets are not copied into the Nix store.";
      };

      cleanupInterval = lib.mkOption {
        type = lib.types.str;
        default = "60s";
        description = "Interval between cleanup daemon passes.";
      };
    };

    runtime = {
      backend = lib.mkOption {
        type = lib.types.enum [ "podman-rootless" ];
        default = "podman-rootless";
        description = "Container backend for job isolation.";
      };

      maxConcurrentJobs = lib.mkOption {
        type = lib.types.ints.positive;
        default = 4;
        description = "Maximum number of jobs Runlet may execute at once.";
      };

      defaultCpu = lib.mkOption {
        type = lib.types.ints.positive;
        default = 2;
        description = "Default CPU limit for runner containers.";
      };

      defaultMemory = lib.mkOption {
        type = lib.types.str;
        default = "4G";
        description = "Default memory limit for runner containers.";
      };

      defaultDisk = lib.mkOption {
        type = lib.types.str;
        default = "20G";
        description = "Default disk budget for job workspaces.";
      };

      defaultTimeout = lib.mkOption {
        type = lib.types.str;
        default = "20m";
        description = "Default job timeout.";
      };

      runnerImage = lib.mkOption {
        type = lib.types.str;
        example = "ghcr.io/davidvornholt/runlet-actions-runner:0.1.0";
        description = "OCI image containing the runlet-runner-entrypoint binary and GitHub Actions runner installation.";
      };

      jobsDir = lib.mkOption {
        type = lib.types.path;
        default = "/var/lib/runlet/jobs";
        description = "Directory for per-job host staging data. Runner work directories stay container-local so Podman storage limits apply.";
      };
    };

    cache = {
      enable = lib.mkEnableOption "Runlet local build cache";

      backend = lib.mkOption {
        type = lib.types.enum [ "local" ];
        default = "local";
        description = "Cache backend.";
      };

      path = lib.mkOption {
        type = lib.types.path;
        default = "/var/cache/runlet";
        description = "Local cache path.";
      };

      allowUntrustedWrite = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Allow untrusted public pull requests to write to the cache.";
      };
    };

    state.databasePath = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/runlet/runlet.sqlite3";
      description = "SQLite database path for orchestration metadata.";
    };

    repositories = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          enabled = lib.mkEnableOption "this repository";

          publicPullRequests = {
            enabled = lib.mkEnableOption "public pull request jobs";

            secrets = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Expose secrets to public pull request jobs. Runlet rejects true for secure defaults.";
            };

            network = lib.mkOption {
              type = lib.types.enum [ "restricted" "normal" ];
              default = "restricted";
              description = "Network policy for public pull request jobs. Restricted uses rootless Podman networking without host loopback so the runner can still register with GitHub.";
            };

            cacheWrite = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Allow public pull request jobs to write cache entries.";
            };

            timeout = lib.mkOption {
              type = lib.types.str;
              default = "15m";
              description = "Timeout for public pull request jobs.";
            };
          };

          trustedBranches = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ "main" ];
            description = "Branch names or prefix patterns like release/* that may use trusted policy.";
          };

          trustedJobs = {
            allowRegistryPush = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Allow trusted jobs to push registry images.";
            };

            allowDeploy = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Allow trusted jobs to run deployment steps.";
            };
          };
        };
      });
      default = { };
      description = "Per-repository Runlet policy keyed by github:owner/repository.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !(cfg.cache.allowUntrustedWrite && !cfg.cache.enable);
        message = "services.runlet.cache.allowUntrustedWrite requires services.runlet.cache.enable.";
      }
    ];

    virtualisation.podman = {
      enable = true;
      dockerSocket.enable = false;
    };

    users.users.runlet = {
      isSystemUser = true;
      group = "runlet";
      home = "/var/lib/runlet";
      createHome = true;
      autoSubUidGidRange = true;
    };
    users.groups.runlet = { };

    systemd.tmpfiles.rules = [
      "d /var/lib/runlet 0750 runlet runlet -"
      "d ${cfg.runtime.jobsDir} 0750 runlet runlet -"
      "d ${databaseDir} 0750 runlet runlet -"
    ] ++ lib.optionals cfg.cache.enable [
      "d ${cfg.cache.path} 0750 runlet runlet -"
    ];

    systemd.services.runlet-orchestrator = {
      description = "Runlet ephemeral GitHub Actions runner orchestrator";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      path = [ pkgs.podman ];
      serviceConfig = {
        User = "runlet";
        Group = "runlet";
        ExecStartPre = "${package}/bin/runlet --config ${configFile} init-db";
        ExecStart = "${package}/bin/runlet --config ${configFile} serve";
        Restart = "on-failure";
        RestartSec = "5s";
        PrivateTmp = true;
        ProtectSystem = "strict";
        ReadWritePaths = [
          "/var/lib/runlet"
          cfg.runtime.jobsDir
          databaseDir
        ] ++ lib.optionals cfg.cache.enable [
          cfg.cache.path
        ];
      };
    };
  };
}
