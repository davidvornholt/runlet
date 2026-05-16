# Runlet

Runlet is a secure-by-default ephemeral GitHub Actions runner orchestrator for
NixOS hosts. It targets ordinary VPS and root servers with rootless Podman, not
nested virtualization or Kubernetes.

The current implementation provides the Runlet appliance foundation:

- a Rust CLI and library for configuration validation, repository policy,
  rootless Podman command construction, safe image-builder execution, runner
  bootstrap, and SQLite orchestration state
- a Nix flake exposing `packages.runlet`, `nixosModules.runlet`, checks, and a
  development shell
- a NixOS module that renders Runlet configuration, enables Podman without a
  Docker socket, creates service state directories, and installs the
  `runlet-orchestrator` systemd service

## NixOS module

```nix
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
      runnerImage = "ghcr.io/org/runlet-actions-runner:latest";
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
```

## CLI

```bash
runlet --config /etc/runlet/config.toml validate-config
runlet --config /etc/runlet/config.toml init-db
runlet --config /etc/runlet/config.toml serve
runlet build-image --builder podman --context . --tag ghcr.io/org/app:local --push ghcr.io/org/app:latest
runlet print-default-config
```

Runlet stores only orchestration metadata in SQLite. Job host staging data,
runner tokens, and containers are treated as ephemeral data and are expected to
be removed during cleanup. Runner work directories stay container-local so the
configured Podman storage limit applies. Public pull request jobs use the
`restricted` network
policy by default, which keeps the runner on rootless Podman networking while
disabling host loopback access so the runner can still register with GitHub.

GitHub should send `workflow_job` webhooks to `/webhook`. Jobs must include the
`runlet` runner label before Runlet will allocate a runner. Runlet verifies
`X-Hub-Signature-256`, creates a fresh registration token for each queued job,
starts one rootless Podman runner container, enforces the repository policy and
timeout, then removes the container, workspace, and runner registration. On
startup, Runlet marks jobs left in allocated pre-cleanup states (`queued`,
`running`, `succeeded`, or `failed`) as cleanup-pending; active jobs from the
current daemon are not collected by the periodic cleanup pass.

Image builds are executed through rootless-safe builders: Podman, Buildah,
BuildKit, or Nix-built OCI images. Runlet does not mount or expose
`/var/run/docker.sock`.

Jobs can request sensitive capabilities with labels, and Runlet denies the job
before registration if the repository policy does not allow the requested
capability:

- `runlet-secrets`
- `runlet-registry-push`
- `runlet-deploy`
- `runlet-privileged`

The repository includes `src/bin/runlet-runner-entrypoint.rs`, a runner
bootstrap executable that directly invokes the GitHub Actions runner
`config.sh` and `run.sh` with explicit arguments. A sample runner image recipe is
provided at `images/actions-runner/Containerfile`; the configured runner image
must contain the `runlet-runner-entrypoint` binary and the GitHub Actions runner
installation.

## Development

```bash
just check
```

The check command runs formatting, clippy, and tests for the Rust code.
