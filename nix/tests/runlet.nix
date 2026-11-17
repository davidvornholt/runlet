# Boots a host configured like the README's NixOS module example, sends signed
# workflow_job webhooks for a trusted and an untrusted job, and checks that
# each runner container gets its limits, cache access and network policy, and
# that Runlet cleans up after it. A fake GitHub API answers inside the VM, and
# the runner image is a local busybox stand-in that waits for the test instead
# of registering a real GitHub Actions runner.
{ pkgs, self }:
let
  webhookSecret = "vm-test-webhook-secret";
  # The repository's offline RSA test key; it signs requests to the fake API only.
  appKey = builtins.head (builtins.match ".*(-----BEGIN PRIVATE KEY-----[^\"]*-----END PRIVATE KEY-----\n).*"
    (builtins.readFile ../../tests/fixtures/github_app_keys.rs));

  runnerImage = pkgs.dockerTools.buildLayeredImage {
    name = "runlet-test-runner";
    tag = "test";
    contents = [ pkgs.busybox ];
    extraCommands = "mkdir -p tmp && chmod 1777 tmp";
    config = {
      User = "1000:1000";
      Cmd = [ "/bin/sh" "-c" "until [ -e /tmp/release ]; do sleep 1; done" ];
    };
  };
in
pkgs.testers.runNixOSTest {
  name = "runlet";

  nodes.machine = { pkgs, ... }: {
    imports = [ self.nixosModules.runlet ];

    virtualisation = {
      cores = 2;
      memorySize = 2048;
      diskSize = 4096;
    };

    environment.systemPackages = [ pkgs.curl pkgs.sqlite ];
    environment.etc."runlet-test/github-app.pem" = {
      text = appKey;
      mode = "0400";
      user = "runlet-orchestrator";
    };
    environment.etc."runlet-test/github-webhook" = {
      text = webhookSecret;
      mode = "0400";
      user = "runlet-orchestrator";
    };

    systemd.services.fake-github = {
      wantedBy = [ "multi-user.target" ];
      before = [ "runlet-orchestrator.service" ];
      serviceConfig = {
        StateDirectory = "fake-github";
        ExecStart = "${pkgs.python3}/bin/python3 ${./fake-github.py} /var/lib/fake-github/requests.log";
      };
    };

    services.runlet = {
      enable = true;

      github = {
        appId = 123456;
        installationId = 987654;
        privateKeyFile = "/etc/runlet-test/github-app.pem";
        apiBaseUrl = "http://127.0.0.1:9999";
      };

      orchestrator = {
        listenAddr = "127.0.0.1:8080";
        webhookSecretFile = "/etc/runlet-test/github-webhook";
        cleanupInterval = "60s";
      };

      runtime = {
        backend = "podman-rootless";
        maxConcurrentJobs = 4;
        defaultCpu = 2;
        defaultMemory = "4G";
        defaultTimeout = "20m";
        runnerImage = "localhost/runlet-test-runner:test";

        users.enable = true;

        untrusted.maxConcurrentJobs = 1;
        untrusted.pidsLimit = 256;
        untrusted.ulimitNofile = "1024:1024";
        untrusted.ulimitNproc = "512:512";
        untrusted.logDriver = "k8s-file";
        untrusted.logSizeMax = "10m";

        network.enableUntrustedFirewall = true;
      };

      cache.enable = true;

      repositories."github:org/project" = {
        enabled = true;
        publicPullRequests.enabled = true;
        trustedBranches = [ "main" "release/*" ];
      };
    };
  };

  testScript = { nodes, ... }: ''
    import hashlib
    import hmac
    import json
    import shlex

    host_ip = "${nodes.machine.networking.primaryIPAddress}"
    database = "/var/lib/runlet/orchestrator/runlet.sqlite3"
    api_log = "/var/lib/fake-github/requests.log"


    def send_job(job_id):
        body = json.dumps({
            "action": "queued",
            "workflow_job": {"id": job_id, "run_id": job_id, "labels": ["self-hosted", "runlet"]},
            "repository": {"full_name": "org/project", "html_url": "https://github.com/org/project"},
        })
        signature = "sha256=" + hmac.new(b"${webhookSecret}", body.encode(), hashlib.sha256).hexdigest()
        machine.succeed(f"printf %s {shlex.quote(body)} > /tmp/webhook.json")
        machine.succeed(
            "curl --fail --silent --show-error"
            " -H 'X-GitHub-Event: workflow_job'"
            f" -H 'X-Hub-Signature-256: {signature}'"
            " --data-binary @/tmp/webhook.json http://127.0.0.1:8080/webhook"
        )


    def podman(user, args):
        return f"sudo -u runlet-{user} podman {args}"


    def wait_for_container(user, job_id):
        names = podman(user, "ps --format '{{.Names}}'")
        machine.wait_until_succeeds(f"{names} | grep '^runlet-{job_id}-'", timeout=120)
        return machine.succeed(f"{names} | grep '^runlet-{job_id}-'").strip()


    def run_in(user, container, script):
        return machine.succeed(podman(user, f"exec {container} sh -c {shlex.quote(script)}"))


    def sql(query):
        return machine.succeed(f"sqlite3 {database} {shlex.quote(query)}").strip()


    def wait_for_cleanup(user, job_id):
        machine.wait_until_succeeds(
            f"test \"$(sqlite3 {database} 'select status from jobs where github_job_id = {job_id}')\" = cleaned",
            timeout=120,
        )
        assert machine.succeed(podman(user, "ps --all --quiet")).strip() == "", "runner container left behind"
        assert machine.succeed("ls -A /var/lib/runlet/jobs").strip() == "", "job staging files left behind"


    def cgroup_limits(user, container):
        return run_in(
            user,
            container,
            "cat /sys/fs/cgroup/cpu.max /sys/fs/cgroup/memory.max /sys/fs/cgroup/memory.swap.max /sys/fs/cgroup/pids.max",
        ).split()


    start_all()
    machine.wait_for_unit("fake-github.service")
    machine.wait_for_unit("runlet-orchestrator.service")
    machine.wait_for_open_port(8080)
    machine.wait_for_open_port(9999)
    for user in ["trusted", "untrusted"]:
        uid = machine.succeed(f"id -u runlet-{user}").strip()
        machine.wait_for_unit(f"user@{uid}.service")
        machine.succeed(podman(user, "load --input ${runnerImage}"))

    with subtest("Trusted job runs with the trusted limits, a writable cache and private network access"):
        send_job(101)
        container = wait_for_container("trusted", 101)
        # 2 CPUs, 4G memory, swap unchanged, and the trusted pids limit.
        limits = cgroup_limits("trusted", container)
        assert limits[:3] == ["200000", "100000", "4294967296"] and limits[4] == "2048", limits
        run_in("trusted", container, "touch /cache/written-by-trusted-job")
        run_in("trusted", container, f"wget -q -T 5 -O- http://{host_ip}:9999/ping")
        run_in("trusted", container, "touch /tmp/release")
        wait_for_cleanup("trusted", 101)

    with subtest("Untrusted job runs with the untrusted limits, a read-only cache and filesystem, and no private network"):
        send_job(102)
        container = wait_for_container("untrusted", 102)
        # 1 CPU, 2G memory with no swap, and the untrusted pids limit.
        assert cgroup_limits("untrusted", container) == ["100000", "100000", "2147483648", "0", "256"]
        run_in("untrusted", container, "test -e /cache/written-by-trusted-job")
        run_in("untrusted", container, "! touch /cache/written-by-untrusted-job")
        run_in("untrusted", container, "awk '$2 == \"/\" { print $4 }' /proc/mounts | grep -q '^ro'")
        machine.fail(podman("untrusted", f"exec {container} wget -q -T 5 -O- http://{host_ip}:9999/ping"))
        run_in("untrusted", container, "touch /tmp/release")
        wait_for_cleanup("untrusted", 102)

    with subtest("Runlet registered and removed a runner for each job through the GitHub API"):
        requests = machine.succeed(f"cat {api_log}")
        assert requests.count("/actions/runners/registration-token") == 2, requests
        assert "GET /repos/org/project/actions/runners?" in requests, requests

    with subtest("A job is refused before registration when Podman would drop its limits"):
        uid = machine.succeed("id -u runlet-untrusted").strip()
        machine.succeed("loginctl disable-linger runlet-untrusted")
        machine.wait_until_succeeds(f"test ! -e /run/user/{uid}")
        send_job(103)
        refusal = (
            "select message from lifecycle_events join jobs using (job_id)"
            " where github_job_id = 103 and event_type = 'limits-unenforceable'"
        )
        try:
            machine.wait_until_succeeds(f"sqlite3 {database} {shlex.quote(refusal)} | grep Podman", timeout=180)
        except Exception:
            print(machine.execute("journalctl -u runlet-orchestrator --no-pager | tail -n 40")[1])
            raise
        print(sql(refusal))
        # No registration token means no runner and no container for this job.
        assert machine.succeed(f"cat {api_log}").count("/actions/runners/registration-token") == 2
  '';
}
