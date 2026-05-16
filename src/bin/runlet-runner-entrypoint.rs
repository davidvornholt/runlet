#[path = "../runner_entrypoint_process.rs"]
mod process;
#[path = "../runner_entrypoint.rs"]
mod runner_entrypoint;

use anyhow::Context;
use process::run;
use runner_entrypoint::RunnerEntrypointConfig;
use std::env;

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
    let status = run(runner_command).context("failed to run GitHub Actions runner")?;
    anyhow::ensure!(status.success(), "runner exited with {status}");

    Ok(())
}
