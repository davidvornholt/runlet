self:
{ config, lib, pkgs, ... }:

let
  cfg = config.services.runlet;
  settingsFormat = pkgs.formats.toml { };

  profileToToml = profile: lib.filterAttrs (_: value: value != null) {
    max_concurrent_jobs = profile.maxConcurrentJobs;
    cpu = profile.cpu;
    memory = profile.memory;
    timeout = profile.timeout;
    pids_limit = profile.pidsLimit;
    ulimit_nofile = profile.ulimitNofile;
    ulimit_nproc = profile.ulimitNproc;
    cpuset_cpus = profile.cpusetCpus;
    device_read_bps = profile.deviceReadBps;
    device_write_bps = profile.deviceWriteBps;
    seccomp_profile = profile.seccompProfile;
    apparmor_profile = profile.apparmorProfile;
    selinux_label = profile.selinuxLabel;
    log_driver = profile.logDriver;
    log_size_max = profile.logSizeMax;
  };

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
      default_timeout = cfg.runtime.defaultTimeout;
      runner_image = cfg.runtime.runnerImage;
      jobs_dir = cfg.runtime.jobsDir;
      trusted = profileToToml cfg.runtime.trusted;
      untrusted = profileToToml cfg.runtime.untrusted;
      users = {
        enabled = cfg.runtime.users.enable;
        orchestrator = cfg.runtime.users.orchestrator;
        trusted = cfg.runtime.users.trusted;
        untrusted = cfg.runtime.users.untrusted;
      };
      network = {
        enable_untrusted_firewall = cfg.runtime.network.enableUntrustedFirewall;
        deny_cidrs = cfg.runtime.network.denyCidrs;
        allow_cidrs = cfg.runtime.network.allowCidrs;
        egress_proxy = cfg.runtime.network.egressProxy;
      };
      storage = {
        trusted_storage_root = cfg.runtime.storage.trustedStorageRoot;
        untrusted_storage_root = cfg.runtime.storage.untrustedStorageRoot;
        warn_free_space_bytes = cfg.runtime.storage.warnFreeSpaceBytes;
      };
      cleanup = {
        enable_scoped_prune = cfg.runtime.cleanup.enableScopedPrune;
        prune_images = cfg.runtime.cleanup.pruneImages;
        prune_volumes = cfg.runtime.cleanup.pruneVolumes;
        prune_containers = cfg.runtime.cleanup.pruneContainers;
      };
    };
    cache = {
      enable = cfg.cache.enable;
      backend = cfg.cache.backend;
      path = cfg.cache.path;
    };
    state = {
      database_path = cfg.state.databasePath;
    };
    repositories = lib.mapAttrs (_: repo: {
      enabled = repo.enabled;
      trusted_branches = repo.trustedBranches;
      public_pull_requests = {
        enabled = repo.publicPullRequests.enabled;
        timeout = repo.publicPullRequests.timeout;
      };
      trusted_jobs = {
        allow_registry_push = repo.trustedJobs.allowRegistryPush;
        allow_deploy = repo.trustedJobs.allowDeploy;
      };
      workflow_risk = {
        require_approval_for_workflow_changes = repo.workflowRisk.requireApprovalForWorkflowChanges;
        approval_label = repo.workflowRisk.approvalLabel;
        high_risk_paths = repo.workflowRisk.highRiskPaths;
        additional_high_risk_paths = repo.workflowRisk.additionalHighRiskPaths;
      };
    }) cfg.repositories;
  };
  package = cfg.package;
  podmanPackage = config.virtualisation.podman.package;
  databaseDir = builtins.dirOf cfg.state.databasePath;

  runtimeProfileType = lib.types.submodule {
    options = {
      maxConcurrentJobs = lib.mkOption { type = lib.types.nullOr lib.types.ints.positive; default = null; description = "Maximum concurrent jobs for this trust class. Null uses the Runlet trust-class default."; };
      cpu = lib.mkOption { type = lib.types.nullOr lib.types.ints.positive; default = null; description = "CPU limit override for this trust class. Null uses the Runlet trust-class default."; };
      memory = lib.mkOption { type = lib.types.nullOr lib.types.str; default = null; description = "Memory limit override for this trust class. Null uses the Runlet trust-class default."; };
      timeout = lib.mkOption { type = lib.types.nullOr lib.types.str; default = null; description = "Timeout override for this trust class. Null uses the Runlet trust-class default."; };
      pidsLimit = lib.mkOption { type = lib.types.nullOr lib.types.ints.positive; default = null; description = "Podman PID limit. Null uses the Runlet trust-class default."; };
      ulimitNofile = lib.mkOption { type = lib.types.nullOr lib.types.str; default = null; description = "nofile ulimit. Null uses the Runlet trust-class default."; };
      ulimitNproc = lib.mkOption { type = lib.types.nullOr lib.types.str; default = null; description = "nproc ulimit. Null uses the Runlet trust-class default."; };
      cpusetCpus = lib.mkOption { type = lib.types.nullOr lib.types.str; default = null; description = "Optional cpuset CPU pinning. Null uses the Runlet trust-class default."; };
      deviceReadBps = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ ]; description = "Podman --device-read-bps throttles."; };
      deviceWriteBps = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ ]; description = "Podman --device-write-bps throttles."; };
      seccompProfile = lib.mkOption { type = lib.types.nullOr lib.types.path; default = null; description = "Optional seccomp profile path. Null uses the Runlet trust-class default."; };
      apparmorProfile = lib.mkOption { type = lib.types.nullOr lib.types.str; default = null; description = "Optional AppArmor profile name. Null uses the Runlet trust-class default."; };
      selinuxLabel = lib.mkOption { type = lib.types.nullOr lib.types.str; default = null; description = "Optional SELinux label security option. Null uses the Runlet trust-class default."; };
      logDriver = lib.mkOption { type = lib.types.nullOr lib.types.str; default = null; description = "Podman log driver. Null uses the Runlet trust-class default; use a Podman-supported value such as none to disable container log files."; };
      logSizeMax = lib.mkOption { type = lib.types.nullOr lib.types.str; default = null; description = "Podman log max-size option. Null uses the Runlet trust-class default."; };
    };
  };

  untrustedIpv4DenyCidrs = lib.filter (cidr: !(lib.hasInfix ":" cidr)) cfg.runtime.network.denyCidrs;
  untrustedIpv6DenyCidrs = lib.filter (cidr: lib.hasInfix ":" cidr) cfg.runtime.network.denyCidrs;
  untrustedIpv4AllowCidrs = lib.filter (cidr: !(lib.hasInfix ":" cidr)) cfg.runtime.network.allowCidrs;
  untrustedIpv6AllowCidrs = lib.filter (cidr: lib.hasInfix ":" cidr) cfg.runtime.network.allowCidrs;
  cidrSet = values: "{ ${lib.concatStringsSep ", " values} }";
  allowRules = lib.optionalString (untrustedIpv4AllowCidrs != [ ]) ''
        meta skuid "${cfg.runtime.users.untrusted}" ip daddr ${cidrSet untrustedIpv4AllowCidrs} accept
  '' + lib.optionalString (untrustedIpv6AllowCidrs != [ ]) ''
        meta skuid "${cfg.runtime.users.untrusted}" ip6 daddr ${cidrSet untrustedIpv6AllowCidrs} accept
  '';
  denyRules = lib.optionalString (untrustedIpv4DenyCidrs != [ ]) ''
        meta skuid "${cfg.runtime.users.untrusted}" ip daddr ${cidrSet untrustedIpv4DenyCidrs} reject with icmp admin-prohibited
  '' + lib.optionalString (untrustedIpv6DenyCidrs != [ ]) ''
        meta skuid "${cfg.runtime.users.untrusted}" ip6 daddr ${cidrSet untrustedIpv6DenyCidrs} reject with icmpv6 admin-prohibited
  '';
  untrustedFirewallRules = ''
    table inet runlet_untrusted_egress {
      chain output {
        type filter hook output priority filter; policy accept;
${allowRules}${denyRules}      }
    }
  '';
in
{
  imports = [
    (lib.mkRemovedOptionModule [ "services" "runlet" "runtime" "defaultDisk" ] "Rootless Podman can't limit a container's disk use. Put services.runlet.runtime.storage.trustedStorageRoot and untrustedStorageRoot on size-limited filesystems instead.")
  ];

  options.services.runlet = {
    enable = lib.mkEnableOption "Runlet ephemeral GitHub Actions runner orchestrator";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.runlet;
      defaultText = lib.literalExpression "runlet.packages.${pkgs.stdenv.hostPlatform.system}.runlet";
      description = "Runlet package to run.";
    };

    github = {
      appId = lib.mkOption { type = lib.types.ints.positive; description = "GitHub App ID used to register ephemeral runners."; };
      installationId = lib.mkOption { type = lib.types.ints.positive; description = "GitHub App installation ID."; };
      privateKeyFile = lib.mkOption { type = lib.types.str; example = "/run/secrets/github-app.pem"; description = "Runtime path to the GitHub App private key file. Use a string path so secrets are not copied into the Nix store."; };
      apiBaseUrl = lib.mkOption { type = lib.types.str; default = "https://api.github.com"; description = "Base URL for the GitHub API."; };
    };

    orchestrator = {
      listenAddr = lib.mkOption { type = lib.types.str; default = "127.0.0.1:8080"; description = "Address where Runlet listens for GitHub webhooks."; };
      webhookSecretFile = lib.mkOption { type = lib.types.str; example = "/run/secrets/github-webhook"; description = "Runtime path to the GitHub webhook secret file. Use a string path so secrets are not copied into the Nix store."; };
      cleanupInterval = lib.mkOption { type = lib.types.str; default = "60s"; description = "Interval between cleanup daemon passes."; };
    };

    runtime = {
      backend = lib.mkOption { type = lib.types.enum [ "podman-rootless" ]; default = "podman-rootless"; description = "Container backend for job isolation."; };
      maxConcurrentJobs = lib.mkOption { type = lib.types.ints.positive; default = 4; description = "Global maximum number of jobs Runlet may execute at once. Per-trust-class limits are enforced within this cap."; };
      defaultCpu = lib.mkOption { type = lib.types.ints.positive; default = 2; description = "Default CPU limit for runner containers."; };
      defaultMemory = lib.mkOption { type = lib.types.str; default = "4G"; description = "Default memory limit for runner containers."; };
      defaultTimeout = lib.mkOption { type = lib.types.str; default = "20m"; description = "Default trusted job timeout."; };
      runnerImage = lib.mkOption { type = lib.types.str; example = "ghcr.io/davidvornholt/runlet-actions-runner:0.1.0"; description = "OCI image containing the runlet-runner-entrypoint binary and GitHub Actions runner installation."; };
      jobsDir = lib.mkOption { type = lib.types.path; default = "/var/lib/runlet/jobs"; description = "Directory for per-job host staging data."; };

      trusted = lib.mkOption { type = runtimeProfileType; default = { maxConcurrentJobs = 4; }; description = "Runtime profile for trusted branch and release jobs."; };
      untrusted = lib.mkOption {
        type = runtimeProfileType;
        default = {
          maxConcurrentJobs = 1;
          cpu = 1;
          memory = "2G";
          timeout = "15m";
          pidsLimit = 256;
          ulimitNofile = "1024:1024";
          ulimitNproc = "512:512";
          logDriver = "k8s-file";
          logSizeMax = "10m";
        };
        description = "Hardened runtime profile for untrusted jobs, which are fork pull request jobs. Production hosts should keep maxConcurrentJobs at 1 unless extra headroom is dedicated to untrusted CI.";
      };

      users = {
        enable = lib.mkOption { type = lib.types.bool; default = true; description = "Run Podman as separate Linux users for trusted and untrusted jobs."; };
        orchestrator = lib.mkOption { type = lib.types.str; default = "runlet-orchestrator"; description = "Dedicated orchestrator user that owns coordination state."; };
        trusted = lib.mkOption { type = lib.types.str; default = "runlet-trusted"; description = "Linux user used for trusted job containers."; };
        untrusted = lib.mkOption { type = lib.types.str; default = "runlet-untrusted"; description = "Linux user used for untrusted job containers, which run fork pull request jobs."; };
      };

      network = {
        enableUntrustedFirewall = lib.mkOption { type = lib.types.bool; default = true; description = "Install nftables rules that block untrusted job egress to host-only, private, link-local, multicast, and metadata-like ranges by UID."; };
        denyCidrs = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ "0.0.0.0/8" "10.0.0.0/8" "100.64.0.0/10" "127.0.0.0/8" "169.254.0.0/16" "172.16.0.0/12" "192.168.0.0/16" "224.0.0.0/4" "::1/128" "fc00::/7" "fe80::/10" ]; description = "CIDR ranges denied for the untrusted execution user."; };
        allowCidrs = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ ]; description = "CIDR ranges explicitly allowed for the untrusted execution user before deny rules are applied."; };
        egressProxy = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ ]; description = "Optional egress proxy URLs exposed to strict-network jobs through RUNLET_EGRESS_PROXY."; };
      };

      storage = {
        trustedStorageRoot = lib.mkOption { type = lib.types.path; default = "/var/lib/runlet/podman-trusted"; description = "Dedicated Podman storage root for the trusted job user. Mount this on a size-limited filesystem in production: it bounds the disk use of these jobs."; };
        untrustedStorageRoot = lib.mkOption { type = lib.types.path; default = "/var/lib/runlet/podman-untrusted"; description = "Dedicated Podman storage root for the untrusted job user. Mount this on a size-limited filesystem in production: it bounds the disk use of these jobs."; };
        warnFreeSpaceBytes = lib.mkOption { type = lib.types.ints.unsigned; default = 5368709120; description = "Free-space warning threshold for Runlet-owned storage paths."; };
      };

      cleanup = {
        enableScopedPrune = lib.mkOption { type = lib.types.bool; default = true; description = "Allow Runlet cleanup to target resources carrying Runlet labels."; };
        pruneImages = lib.mkOption { type = lib.types.bool; default = false; description = "Permit scoped image cleanup for Runlet-owned images only."; };
        pruneVolumes = lib.mkOption { type = lib.types.bool; default = false; description = "Permit scoped volume cleanup for Runlet-owned volumes only."; };
        pruneContainers = lib.mkOption { type = lib.types.bool; default = true; description = "Permit cleanup of Runlet-owned containers."; };
      };
    };

    cache = {
      enable = lib.mkEnableOption "Runlet local build cache";
      backend = lib.mkOption { type = lib.types.enum [ "local" ]; default = "local"; description = "Cache backend."; };
      path = lib.mkOption { type = lib.types.path; default = "/var/cache/runlet"; description = "Local cache path. Mount on a size-limited filesystem for production hosts."; };
    };

    state.databasePath = lib.mkOption { type = lib.types.path; default = "/var/lib/runlet/orchestrator/runlet.sqlite3"; description = "SQLite database path for orchestration metadata."; };

    repositories = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          enabled = lib.mkEnableOption "this repository";
          publicPullRequests = {
            enabled = lib.mkEnableOption "fork pull request jobs, which run as untrusted jobs";
            timeout = lib.mkOption { type = lib.types.str; default = "15m"; description = "Timeout for fork pull request jobs."; };
          };
          trustedBranches = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ "main" ]; description = "Branch names or prefix patterns like release/* whose push runs from the repository itself use trusted policy. Other events on these branches, such as workflow_dispatch, are denied."; };
          trustedJobs = {
            allowRegistryPush = lib.mkOption { type = lib.types.bool; default = false; description = "Allow trusted jobs to push registry images."; };
            allowDeploy = lib.mkOption { type = lib.types.bool; default = false; description = "Allow trusted jobs to run deployment steps."; };
          };
          workflowRisk = {
            requireApprovalForWorkflowChanges = lib.mkOption { type = lib.types.bool; default = false; description = "Hold public pull request jobs that modify high-risk workflow paths unless the configured approval label is present. After applying the label, rerun the workflow job or redeliver the workflow_job webhook."; };
            approvalLabel = lib.mkOption { type = lib.types.str; default = "runlet-approved-workflow-change"; description = "Pull request label that approves high-risk workflow changes when approval is required."; };
            highRiskPaths = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ ".github/workflows/**" ".github/actions/**" "**/action.yml" "**/action.yaml" "scripts/**" ]; description = "Path patterns treated as high risk for public pull request jobs."; };
            additionalHighRiskPaths = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ ]; description = "Operator-defined additional high-risk path patterns."; };
          };
        };
      });
      default = { };
      description = "Per-repository Runlet policy keyed by github:owner/repository.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      { assertion = !(cfg.runtime.network.enableUntrustedFirewall && !cfg.runtime.users.enable); message = "services.runlet.runtime.network.enableUntrustedFirewall requires services.runlet.runtime.users.enable."; }
      { assertion = !cfg.runtime.users.enable || (cfg.runtime.users.orchestrator != cfg.runtime.users.trusted && cfg.runtime.users.orchestrator != cfg.runtime.users.untrusted && cfg.runtime.users.trusted != cfg.runtime.users.untrusted); message = "services.runlet.runtime.users orchestrator, trusted, and untrusted users must be distinct when user splitting is enabled."; }
      { assertion = databaseDir != "/var/lib/runlet"; message = "services.runlet.state.databasePath can't be directly in /var/lib/runlet, which the runner users must be able to enter. Stop runlet, move the database to /var/lib/runlet/orchestrator/runlet.sqlite3 keeping its owner, and use that path."; }
    ];

    virtualisation.podman = {
      enable = true;
      dockerSocket.enable = false;
      # Runner containers use slirp4netns networking, which Podman doesn't ship by default.
      extraPackages = [ pkgs.slirp4netns ];
    };

    networking.nftables = lib.mkIf cfg.runtime.network.enableUntrustedFirewall {
      enable = true;
      ruleset = untrustedFirewallRules;
      # The build-time check runs in a sandbox without the untrusted user.
      preCheckRuleset = ''
        sed -i ${lib.escapeShellArg "s/meta skuid \"${cfg.runtime.users.untrusted}\"/meta skuid \"nobody\"/g"} ruleset.conf
      '';
    };

    users.users.${cfg.runtime.users.orchestrator} = {
      isSystemUser = true;
      group = "runlet";
      home = "/var/lib/runlet/orchestrator";
      createHome = true;
    };
    users.users.${cfg.runtime.users.trusted} = lib.mkIf cfg.runtime.users.enable {
      isSystemUser = true;
      group = "runlet-trusted";
      extraGroups = [ "runlet" ];
      home = "/var/lib/runlet/trusted";
      createHome = true;
      autoSubUidGidRange = true;
      # A lingering user manager delegates the cgroup that Podman needs to enforce job limits.
      linger = true;
    };
    users.users.${cfg.runtime.users.untrusted} = lib.mkIf cfg.runtime.users.enable {
      isSystemUser = true;
      group = "runlet-untrusted";
      extraGroups = [ "runlet" ];
      home = "/var/lib/runlet/untrusted";
      createHome = true;
      autoSubUidGidRange = true;
      linger = true;
    };
    users.groups.runlet = { };
    users.groups.runlet-trusted = lib.mkIf cfg.runtime.users.enable { };
    users.groups.runlet-untrusted = lib.mkIf cfg.runtime.users.enable { };

    security.sudo.enable = lib.mkIf cfg.runtime.users.enable true;
    security.sudo.extraConfig = lib.mkIf cfg.runtime.users.enable ''
      ${cfg.runtime.users.orchestrator} ALL=(${cfg.runtime.users.trusted},${cfg.runtime.users.untrusted}) NOPASSWD: ${podmanPackage}/bin/podman *
    '';

    environment.etc."runlet/seccomp-untrusted-example.json".text = builtins.toJSON {
      defaultAction = "SCMP_ACT_ALLOW";
      architectures = [ "SCMP_ARCH_X86_64" "SCMP_ARCH_AARCH64" ];
      syscalls = [
        { names = [ "keyctl" "add_key" "request_key" "bpf" "perf_event_open" ]; action = "SCMP_ACT_ERRNO"; }
      ];
    };

    systemd.tmpfiles.rules = [
      "d /var/lib/runlet 0755 root root -"
      "d /var/lib/runlet/orchestrator 0750 ${cfg.runtime.users.orchestrator} runlet -"
      "d ${cfg.runtime.jobsDir} 0750 ${cfg.runtime.users.orchestrator} runlet -"
      "d ${databaseDir} 0750 ${cfg.runtime.users.orchestrator} runlet -"
    ] ++ lib.optionals cfg.runtime.users.enable [
      "d /var/lib/runlet/trusted 0750 ${cfg.runtime.users.trusted} runlet-trusted -"
      "d /var/lib/runlet/untrusted 0750 ${cfg.runtime.users.untrusted} runlet-untrusted -"
      "d ${cfg.runtime.storage.trustedStorageRoot} 0750 ${cfg.runtime.users.trusted} runlet-trusted -"
      "d ${cfg.runtime.storage.untrustedStorageRoot} 0750 ${cfg.runtime.users.untrusted} runlet-untrusted -"
      "d /var/lib/runlet/trusted/.config 0750 ${cfg.runtime.users.trusted} runlet-trusted -"
      "d /var/lib/runlet/trusted/.config/containers 0750 ${cfg.runtime.users.trusted} runlet-trusted -"
      "L+ /var/lib/runlet/trusted/.config/containers/storage.conf - - - - /etc/runlet/storage-trusted.conf"
      "d /var/lib/runlet/untrusted/.config 0750 ${cfg.runtime.users.untrusted} runlet-untrusted -"
      "d /var/lib/runlet/untrusted/.config/containers 0750 ${cfg.runtime.users.untrusted} runlet-untrusted -"
      "L+ /var/lib/runlet/untrusted/.config/containers/storage.conf - - - - /etc/runlet/storage-untrusted.conf"
    ] ++ lib.optionals cfg.cache.enable [
      "d ${cfg.cache.path} 0770 ${cfg.runtime.users.orchestrator} runlet -"
    ];

    environment.etc."runlet/storage-trusted.conf" = lib.mkIf cfg.runtime.users.enable {
      text = ''
        [storage]
        driver = "overlay"
        graphroot = "${cfg.runtime.storage.trustedStorageRoot}"
      '';
    };
    environment.etc."runlet/storage-untrusted.conf" = lib.mkIf cfg.runtime.users.enable {
      text = ''
        [storage]
        driver = "overlay"
        graphroot = "${cfg.runtime.storage.untrustedStorageRoot}"
      '';
    };

    systemd.services.runlet-orchestrator = {
      description = "Runlet ephemeral GitHub Actions runner orchestrator";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      path = [ podmanPackage ] ++ lib.optionals cfg.runtime.users.enable [ "/run/wrappers" ];
      serviceConfig = {
        User = cfg.runtime.users.orchestrator;
        Group = "runlet";
        ExecStartPre = "${package}/bin/runlet --config ${configFile} init-db";
        ExecStart = "${package}/bin/runlet --config ${configFile} serve";
        Restart = "on-failure";
        RestartSec = "5s";
        # NoNewPrivileges is intentionally omitted: the orchestrator uses narrow
        # sudo rules to hand Podman execution and per-job token ownership to the
        # trusted or untrusted runner user.
        PrivateTmp = true;
        ProtectSystem = "strict";
        # ProtectHome would hide /run/user, where Podman reaches the runner
        # users' systemd managers that enforce job limits, and
        # ProtectControlGroups would stop crun from creating the job's cgroup.
        InaccessiblePaths = [ "-/home" "-/root" ];
        ProtectKernelModules = true;
        LockPersonality = true;
        ReadWritePaths = [
          "/var/lib/runlet"
          cfg.runtime.jobsDir
          databaseDir
          "/run/user"
        ] ++ lib.optionals cfg.runtime.users.enable [
          cfg.runtime.storage.trustedStorageRoot
          cfg.runtime.storage.untrustedStorageRoot
        ] ++ lib.optionals cfg.cache.enable [ cfg.cache.path ];
      };
    };
  };
}
