{
  services.runlet = {
    enable = true;

    github = {
      appId = 123456;
      installationId = 987654;
      privateKeyFile = "/run/secrets/github-app.pem";
    };

    orchestrator = {
      listenAddr = "127.0.0.1:8080";
      webhookSecretFile = "/run/secrets/github-webhook";
      cleanupInterval = "60s";
    };

    runtime = {
      backend = "podman-rootless";
      maxConcurrentJobs = 4;
      defaultCpu = 2;
      defaultMemory = "4G";
      defaultDisk = "20G";
      defaultTimeout = "20m";
      runnerImage = "ghcr.io/davidvornholt/runlet-actions-runner:0.1.0";
    };

    cache = {
      enable = true;
      backend = "local";
      path = "/var/cache/runlet";
      allowUntrustedWrite = false;
    };

    repositories."github:org/project" = {
      enabled = true;

      publicPullRequests = {
        enabled = true;
        secrets = false;
        network = "restricted";
        cacheWrite = false;
        timeout = "15m";
      };

      trustedBranches = [ "main" "release/*" ];

      trustedJobs = {
        allowRegistryPush = true;
        allowDeploy = false;
      };
    };
  };
}
