use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use runlet::builder::{build_image, push_image, ImageBuildRequest, ImageBuilder};
use runlet::config::Config;
use runlet::orchestrator::Orchestrator;
use runlet::process::run;
use runlet::state::Store;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[arg(long, env = "RUNLET_CONFIG", default_value = "/etc/runlet/config.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    ValidateConfig,
    InitDb,
    Serve,
    BuildImage {
        #[arg(long, value_enum, default_value_t = CliImageBuilder::Podman)]
        builder: CliImageBuilder,
        #[arg(long)]
        context: PathBuf,
        #[arg(long)]
        tag: String,
        #[arg(long)]
        push: Option<String>,
    },
    PrintDefaultConfig,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliImageBuilder {
    Podman,
    Buildah,
    Buildkit,
    Nix,
}

impl From<CliImageBuilder> for ImageBuilder {
    fn from(builder: CliImageBuilder) -> Self {
        match builder {
            CliImageBuilder::Podman => Self::Podman,
            CliImageBuilder::Buildah => Self::Buildah,
            CliImageBuilder::Buildkit => Self::BuildKit,
            CliImageBuilder::Nix => Self::Nix,
        }
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::ValidateConfig => {
            Config::from_path(&cli.config).context("configuration is invalid")?;
            println!("configuration is valid");
        }
        Command::InitDb => {
            let config = Config::from_path(&cli.config).context("configuration is invalid")?;
            init_db(&config)?;
            println!(
                "state database is ready at {}",
                config.state.database_path.display()
            );
        }
        Command::Serve => {
            let config = Config::from_path(&cli.config).context("configuration is invalid")?;
            Orchestrator::new(config)
                .serve()
                .context("orchestrator failed")?;
        }
        Command::BuildImage {
            builder,
            context,
            tag,
            push,
        } => build_image_command(builder, context, tag, push)?,
        Command::PrintDefaultConfig => print_default_config(),
    }
    Ok(())
}

fn init_db(config: &Config) -> anyhow::Result<()> {
    Store::open(&config.state.database_path).with_context(|| {
        format!(
            "failed to open state database {}",
            config.state.database_path.display()
        )
    })?;
    Ok(())
}

fn build_image_command(
    builder: CliImageBuilder,
    context: PathBuf,
    tag: String,
    push: Option<String>,
) -> anyhow::Result<()> {
    let request = ImageBuildRequest {
        builder: builder.into(),
        context,
        tag,
        push,
    };
    let build = build_image(&request).context("image build request is invalid")?;
    let status = run(build.command()).context("failed to run image builder")?;
    anyhow::ensure!(status.success(), "image builder exited with {status}");

    if let Some(destination) = &request.push {
        if request.builder != ImageBuilder::BuildKit {
            let push =
                push_image(&request.tag, destination).context("image push request is invalid")?;
            let status = run(push.command()).context("failed to run image push")?;
            anyhow::ensure!(status.success(), "image push exited with {status}");
        }
    }
    Ok(())
}

fn print_default_config() {
    println!(
        r#"[github]
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
jobs_dir = "/var/lib/runlet/jobs"

[runtime.users]
enabled = true
orchestrator = "runlet-orchestrator"
trusted = "runlet-trusted"
untrusted = "runlet-untrusted"

[runtime.trusted]
max_concurrent_jobs = 4
pids_limit = 2048
ulimit_nofile = "4096:4096"
ulimit_nproc = "2048:2048"
ipc = "private"

[runtime.untrusted]
max_concurrent_jobs = 1
cpu = 1
memory = "2G"
disk = "10G"
timeout = "15m"
read_only = true
tmpfs = ["/tmp:rw,nosuid,nodev,size=1G", "/run:rw,nosuid,nodev,size=64M"]
pids_limit = 256
ulimit_nofile = "1024:1024"
ulimit_nproc = "512:512"
memory_swap = "memory"
ipc = "private"
log_driver = "k8s-file"
log_size_max = "10m"
disable_host_log_capture = true

[runtime.network]
enable_untrusted_firewall = true
deny_cidrs = ["0.0.0.0/8", "10.0.0.0/8", "100.64.0.0/10", "127.0.0.0/8", "169.254.0.0/16", "172.16.0.0/12", "192.168.0.0/16", "224.0.0.0/4", "::1/128", "fc00::/7", "fe80::/10"]
allow_cidrs = []
allow_tcp_ports = ["80", "443"]

[cache]
enable = false
backend = "local"
path = "/var/cache/runlet"
allow_untrusted_write = false

[state]
database_path = "/var/lib/runlet/runlet.sqlite3"
"#
    );
}
