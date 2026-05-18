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

## Quick start

Build or enter a development shell with Nix:

```bash
nix build .#runlet
nix develop
```

Without Nix, use the Rust toolchain directly:

```bash
cargo build --release
cargo test --all-features
```

Print a starter TOML configuration:

```bash
runlet print-default-config > config.example.toml
```

For a deployment, continue with [Use Runlet in GitHub Actions](#use-runlet-in-github-actions).

## Use Runlet in GitHub Actions

Follow these steps to move a workflow job from GitHub-hosted runners such as
`ubuntu-latest` to Runlet-managed ephemeral self-hosted runners.

1. Deploy Runlet for the repository.
   - Use the public Runlet runner image or a derived image that contains the
     GitHub Actions runner and `runlet-runner-entrypoint`:

     ```toml
     [runtime]
     runner_image = "ghcr.io/davidvornholt/runlet-actions-runner:0.1.0"
     ```

     For unreleased testing, the workflow also publishes a mutable `main` tag.
     For production, prefer a release tag or digest over `main` or `latest`.
     The first publish may require setting the GHCR package visibility to public
     in GitHub's package settings; the publish workflow verifies unauthenticated
     image access and fails if the package remains private.

   - Create a GitHub App, install it on the repository, subscribe it to the
     `workflow_job` webhook event, and point the webhook at
     `https://<runlet-host>/webhook`. The app needs read/write access to
     repository self-hosted runners for registration token creation and runner
     removal, and read access to pull requests so Runlet can inspect changed
     files and approval labels before running public pull request jobs.
   - Store the app private key and webhook secret outside git and outside the Nix
     store, then reference those files from the Runlet configuration.
   - Add the repository to Runlet configuration with the exact id
     `github:<owner>/<repo>`, for example `github:octo-org/octo-repo`.

2. Start the orchestrator and confirm that the configuration is valid.

   ```bash
   runlet --config /etc/runlet/config.toml validate-config
   runlet --config /etc/runlet/config.toml init-db
   runlet --config /etc/runlet/config.toml serve
   ```

   With the NixOS module, enable `services.runlet` instead and check the
   `runlet-orchestrator` systemd service logs.

3. In the repository, open the workflow file under `.github/workflows/` and find
   each job that currently uses a GitHub-hosted runner:

   ```yaml
   jobs:
     test:
       runs-on: ubuntu-latest
       steps:
         - uses: actions/checkout@v4
         - run: cargo test --all-features
   ```

4. Replace the GitHub-hosted runner label with the Runlet self-hosted labels.
   The `runlet` entry is a GitHub Actions runner label that routes this job to
   Runlet-managed runners. It is configured in the workflow, not added manually
   to each pull request.

   ```yaml
   jobs:
     test:
       runs-on:
         - self-hosted
         - runlet
       steps:
         - uses: actions/checkout@v4
         - run: cargo test --all-features
   ```

   Do not leave `ubuntu-latest`, `macos-latest`, or `windows-latest` in the same
   `runs-on` value when you intend the job to use Runlet. Use only the
   self-hosted labels that Runlet should register for the ephemeral runner.

5. Add optional Runlet capability labels only when the repository policy allows
   them. Runlet denies the job before runner registration if the policy does not
   allow a requested capability.

   ```yaml
   jobs:
     publish:
       runs-on:
         - self-hosted
         - runlet
         - runlet-registry-push
   ```

   Available capability labels are `runlet-secrets`, `runlet-registry-push`,
   `runlet-deploy`, and `runlet-privileged`.

6. Commit the workflow change and trigger the workflow with a push, pull request,
   or `workflow_dispatch` event. In GitHub, the job should queue for a
   self-hosted runner. Runlet receives the `workflow_job` event, creates a fresh
   registration token, starts one runner container for the job, and removes the
   runner when the job finishes.

7. If the job stays queued, check these common causes:
   - The job is missing the `runlet` label.
   - The GitHub App is not installed on the repository that owns the workflow.
   - The webhook is not subscribed to `workflow_job`, cannot reach
     `/webhook`, or uses the wrong secret.
   - The repository id in Runlet configuration does not match
     `github:<owner>/<repo>`.
   - A requested capability label is not allowed by the repository policy.

To roll a job back to the default GitHub-hosted runner, change `runs-on` back to
`ubuntu-latest` or another GitHub-hosted runner label and remove the Runlet
capability labels.

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
      runnerImage = "ghcr.io/davidvornholt/runlet-actions-runner:0.1.0";

      users.enable = true;

      # Keep this at 1 for public pull request jobs on shared production hosts.
      untrusted.maxConcurrentJobs = 1;
      untrusted.readOnly = true;
      untrusted.tmpfs = [
        "/tmp:rw,nosuid,nodev,size=1G"
        "/run:rw,nosuid,nodev,size=64M"
      ];
      untrusted.pidsLimit = 256;
      untrusted.ulimitNofile = "1024:1024";
      untrusted.ulimitNproc = "512:512";
      untrusted.memorySwap = "memory";
      untrusted.logDriver = "k8s-file";
      untrusted.logSizeMax = "10m";

      network.enableUntrustedFirewall = true;
      storage.untrustedStorageRoot = "/var/lib/runlet/podman-untrusted";
      storage.trustedStorageRoot = "/var/lib/runlet/podman-trusted";
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
        network = "strict";
        cacheWrite = false;
        timeout = "15m";
      };

      workflowRisk = {
        denyWorkflowFileChanges = true;
        requireApprovalForWorkflowChanges = false;
        additionalHighRiskPaths = [ "build.rs" "justfile" ];
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

## Migration notes

Existing single-user deployments should review the new NixOS defaults before
switching production traffic. The module now creates `runlet-orchestrator`,
`runlet-trusted`, and `runlet-untrusted`, writes separate Podman storage
configuration for the trusted and untrusted homes, and grants the orchestrator
narrow sudo rules to invoke Podman as the trusted or untrusted job user. Job
staging files are group-readable only to the shared `runlet` group. Move or
prune old rootless Podman state from the former `runlet` user after confirming
no old jobs remain.

If you deploy without the NixOS module, create equivalent users, subuid/subgid
ranges, storage roots, quota-limited mounts, and UID-based egress firewall rules
before enabling `runtime.users.enabled = true`.

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
configured Podman storage limit applies.

Public pull request jobs use the `strict` network policy by default. In the
NixOS module this combines rootless Podman networking without host loopback with
nftables rules for the untrusted Linux user that deny localhost, private,
link-local, multicast, carrier-grade NAT, IPv6 ULA/link-local, and
metadata-like ranges. Operators can add explicit proxy or firewall exceptions,
but should prefer an egress proxy for HTTP(S) if private network access must be
mediated. `runtime.network.allowCidrs` can add explicit CIDR exceptions that are
accepted before the default private-network deny rules.

GitHub should send `workflow_job` webhooks to `/webhook`. Runlet only allocates
runners for workflow jobs whose `runs-on` labels include `runlet`.
Runlet verifies `X-Hub-Signature-256`, creates a fresh registration token for
each queued job, checks pull request changed files for high-risk workflow paths,
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
`config.sh` and `run.sh` with explicit arguments. Runlet publishes a public
multi-architecture runner image at
`ghcr.io/davidvornholt/runlet-actions-runner`. The image is tagged with release
versions, `sha-<commit>`, `main`, `latest`, `runner-<actions-runner-version>`,
and `<release>-runner-<actions-runner-version>`.

GitHub may create new GHCR packages as private. After the first successful image
push, set the package visibility to public in GitHub's package settings. The
image workflow verifies unauthenticated access after publishing and fails if the
package remains private.

A sample runner image recipe is provided at `images/actions-runner/Containerfile`
for operators who need to customize the image. Build it from the repository root
so the recipe can compile `runlet-runner-entrypoint`:

```bash
podman build \
  --file images/actions-runner/Containerfile \
  --build-arg RUNNER_VERSION=2.334.0 \
  --tag ghcr.io/org/runlet-actions-runner:custom \
  .
podman push ghcr.io/org/runlet-actions-runner:custom
```

The configured runner image must contain the `runlet-runner-entrypoint` binary
and the GitHub Actions runner installation.

## Security considerations

- Keep GitHub App private keys, webhook secrets, registration tokens, and local
  deployment TOML files out of git. The repository `.gitignore` covers common
  local filenames, but operators should still use a secret manager for deployed
  systems.
- Do not expose a Docker socket to runner jobs. The NixOS module enables Podman
  and explicitly leaves the Docker socket disabled.
- Public pull request jobs should keep `secrets = false`, `cacheWrite = false`,
  `network = "strict"`, and `runtime.untrusted.maxConcurrentJobs = 1` unless you
  have reviewed the trust boundary.
- `strict` networking depends on host firewall integration. On NixOS the module
  installs nftables rules keyed to the untrusted execution UID; on other systems
  deploy equivalent nftables/iptables rules before accepting public pull
  requests.
- Mount `runtime.storage.trustedStorageRoot`,
  `runtime.storage.untrustedStorageRoot`, and the cache path on separate
  quota-limited filesystems to prevent CI images, layers, logs, caches, or
  cleanup failures from filling production storage.
- Workflow definitions, local composite actions, `action.yml`/`action.yaml`, and
  configured build scripts are high risk. Public pull requests that change those
  paths are denied by default before runner registration tokens are created. If
  approval mode is enabled, apply the configured approval label and then rerun
  the workflow job or redeliver the `workflow_job` webhook so Runlet can observe
  the approval before creating a runner token.
- Optional untrusted seccomp, AppArmor, and SELinux profiles can be configured
  with `runtime.untrusted.seccompProfile`, `runtime.untrusted.apparmorProfile`,
  and `runtime.untrusted.selinuxLabel`. The NixOS module installs
  `/etc/runlet/seccomp-untrusted-example.json` as a conservative starting point
  that denies `keyctl`, `bpf`, and `perf_event_open` families.
- The NixOS service runs as `runlet-orchestrator` and enables compatible
  systemd hardening including `PrivateTmp`, `ProtectSystem=strict`,
  `ProtectHome`, kernel protection options, and
  `LockPersonality`. `NoNewPrivileges` and `RestrictSUIDSGID` are intentionally
  omitted because the orchestrator uses narrow sudo rules to hand Podman
  execution to the trusted and untrusted users.
  `PrivateDevices` is also omitted because rootless Podman networking and
  storage may require host device nodes such as `/dev/net/tun` or `/dev/fuse`.
- These controls reduce blast radius but do not provide VM-equivalent isolation:
  untrusted containers still share the host kernel.
- Treat runner images as part of the trusted computing base. Pin and rebuild the
  GitHub Actions runner version intentionally.

## Development

```bash
just check
```

The check command verifies formatting, clippy, and tests for the Rust code. Use
`just check-fix` to apply Rust formatting before running clippy and tests.
