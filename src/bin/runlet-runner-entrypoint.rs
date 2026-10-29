#[path = "../runner_entrypoint_process.rs"]
mod process;
#[path = "../runner_entrypoint.rs"]
mod runner_entrypoint;

use anyhow::Context;
use process::run;
use runner_entrypoint::RunnerEntrypointConfig;
use std::env;
use std::os::unix::process::CommandExt;

fn main() -> anyhow::Result<()> {
    let mut config = RunnerEntrypointConfig::from_env().context("runner environment is invalid")?;
    config
        .prepare_writable_runner_dir()
        .context("failed to prepare writable GitHub Actions runner directory")?;
    env::remove_var("RUNNER_TOKEN");

    let configure = config.configure_command();
    let status = run(configure.command()).context("failed to configure GitHub Actions runner")?;
    anyhow::ensure!(
        status.success(),
        "runner configuration exited with {status}"
    );

    let runner = config.run_command();
    let mut runner_command = runner.command();
    runner_command.env_remove("RUNNER_TOKEN");
    // Replace this process instead of waiting on a child. As the container's PID 1,
    // a waiting entrypoint keeps its initial environment, including RUNNER_TOKEN,
    // readable through /proc/1/environ for every job step.
    let error = runner_command.exec();
    Err(error).context("failed to run GitHub Actions runner")
}
