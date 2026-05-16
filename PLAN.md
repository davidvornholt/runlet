# Runlet: secure ephemeral CI on NixOS with rootless Podman

## Goal

Build Runlet, a self-hosted, secure-by-default ephemeral CI runner appliance for GitHub Actions.

Runlet should:

- run on ordinary VPS/root servers, including Netcup RS2000
- require no nested virtualization
- use NixOS as the host platform
- use rootless Podman for job isolation
- create disposable runners per job
- support public and private repositories
- be reusable as a NixOS module in other Nix projects

## Core idea

Do not build a full CI platform.

Build Runlet as a secure ephemeral runner orchestrator for GitHub Actions.

```text
GitHub Actions
    ↓
Runlet NixOS appliance
    ↓
ephemeral rootless Podman runner container
    ↓
job finishes
    ↓
container, token, workspace destroyed
```

## Non-goals

- No custom CI syntax
- No custom Git hosting
- No Kubernetes requirement
- No Docker socket exposure
- No persistent general-purpose runners
- No privileged containers by default
- No shell-based control plane or generated shell scripts for lifecycle logic

## Implementation language

Runlet should use:

```text
Rust    orchestrator daemon, host lifecycle logic, GitHub API integration, policy enforcement
Nix     flake, NixOS module, packaging, systemd units, host configuration
YAML    GitHub Actions examples and compatibility tests
```

Use Rust for the orchestrator because Runlet is a long-running, security-sensitive infrastructure service that handles untrusted CI jobs, short-lived tokens, filesystem cleanup, container lifecycle, concurrency, timeouts, and policy enforcement.

Avoid shell for implementation logic. The Rust daemon should spawn external tools with explicit argument vectors and no shell interpolation.

Shell snippets may appear in documentation only when showing commands a human runs, such as:

```bash
nixos-rebuild switch
```

If an external tool only exposes a shell-script entrypoint, such as the GitHub Actions runner `config.sh`, Runlet may execute that file directly as a process. Runlet should not generate shell scripts or run lifecycle commands through `sh -c`.

## Target architecture

```text
NixOS host
├── runlet-orchestrator
│   ├── GitHub App authentication
│   ├── runner registration
│   ├── job lifecycle management
│   └── policy enforcement
│
├── rootless-podman-runtime
│   ├── ephemeral runner containers
│   ├── isolated workspaces
│   ├── CPU/RAM/disk limits
│   └── restricted networking
│
├── cache-service
│   ├── optional read/write cache
│   ├── per-repo namespace
│   └── poisoning protection
│
└── garbage-collector
    ├── removes stale containers
    ├── deletes workspaces
    ├── revokes runner tokens
    └── enforces timeouts
```

## Persistence

Runlet should maintain small durable local state for orchestration and cleanup.

Use SQLite as the default persistence layer.

Persistent state may include:

- known runner registrations
- in-flight job records
- cleanup/revocation status
- cache metadata
- recent job lifecycle events for debugging

Job workspaces, runner tokens, and containers remain ephemeral. Only orchestration metadata should persist.

## Security model

Every CI job gets:

```text
fresh runner token
fresh container
fresh workspace
rootless user namespace
no host Docker socket
no sudo
no host secrets
limited CPU
limited memory
limited disk
limited runtime
restricted network
automatic deletion
```

## Trust levels

### Untrusted public pull request

```yaml
secrets: false
registry_push: false
deploy: false
network: restricted
cache_write: false
privileged: false
max_runtime: 20m
```

### Trusted branch push

```yaml
secrets: limited
registry_push: true
deploy: optional
network: normal
cache_write: true
privileged: false
max_runtime: 45m
```

### Release / deployment job

```yaml
secrets: allowed
registry_push: true
deploy: allowed
network: normal
manual_approval: recommended
privileged: false
```

## Rootless Podman runtime

Use rootless Podman as the default execution backend.

Each job should run as a dedicated system user or dynamically assigned user namespace.

```text
ci-runner-<job-id>
├── ~/.local/share/containers
├── /var/lib/runlet/jobs/<job-id>
└── rootless Podman container
```

The container runs the GitHub Actions runner process.

After completion:

```text
Rust daemon removes runner-<job-id> with Podman
Rust daemon deletes /var/lib/runlet/jobs/<job-id>
Rust daemon removes temporary credentials
Rust daemon removes temporary system user if created
```

## Image building

Never expose:

```text
/var/run/docker.sock
```

Supported safe image builders:

```text
rootless Podman build
rootless Buildah
rootless BuildKit
Nix-built OCI images
```

Preferred product abstraction:

```yaml
image:
  build: .
  push: ghcr.io/org/app
```

Internally, the appliance chooses a safe rootless builder.

## NixOS module interface

Final user experience should look like this:

```nix
{
  services.runlet = {
    enable = true;

    github = {
      appId = 123456;
      installationId = 987654;
      privateKeyFile = "/run/secrets/github-app.pem";
    };

    runtime = {
      backend = "podman-rootless";
      maxConcurrentJobs = 4;
      defaultCpu = 2;
      defaultMemory = "4G";
      defaultDisk = "20G";
      defaultTimeout = "20m";
    };

    cache = {
      enable = true;
      backend = "local";
      path = "/var/cache/runlet";
      allowUntrustedWrite = false;
    };
  };
}
```

## Repository-level policy

Allow per-repository configuration:

```nix
services.runlet.repositories."github:org/project" = {
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
```

## Deployment model

CI should not directly mutate production by default.

## Execution phases

### Phase 1 — local ephemeral runner

- Start one GitHub runner in a rootless Podman container
- Register runner dynamically
- Execute one job
- Remove container after completion

### Phase 2 — orchestrator

- GitHub App authentication
- Dynamic runner token creation
- Job queue
- Max concurrency
- Timeout handling
- Cleanup daemon

### Phase 3 — security defaults

- No Docker socket
- No privileged mode
- Resource limits
- Per-job workspace
- Restricted secrets
- Untrusted PR mode

### Phase 4 — NixOS module

- Declarative service config
- systemd units
- Podman setup
- secret integration
- repository policy config

### Phase 5 — reusable flake

Expose:

```text
nixosModules.runlet
packages.runlet
checks
devShells
```

Example usage:

```nix
{
  inputs.runlet.url = "github:your-org/runlet";

  outputs = { self, nixpkgs, runlet, ... }: {
    nixosConfigurations.ci-server = nixpkgs.lib.nixosSystem {
      modules = [
        runlet.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
```

## Success criteria

The project succeeds when a user can provision a secure CI runner with:

```bash
nixos-rebuild switch
```

and get:

```text
ephemeral GitHub Actions runners
rootless Podman isolation
secure defaults
public PR protection
no Kubernetes
no Docker socket
no AWS
no managed CI vendor
```

## Product positioning

> Runlet provides secure self-hosted ephemeral GitHub Actions runners for ordinary VPS servers.
