use std::ffi::OsString;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
}

impl ProcessSpec {
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        command
    }
}

pub fn run(mut command: Command) -> std::io::Result<ExitStatus> {
    command.status()
}

#[derive(Debug)]
pub enum RunOutcome {
    Exited(ExitStatus),
    TimedOut,
}

pub fn run_with_timeout(
    mut command: Command,
    timeout: Duration,
    capture_stdout: bool,
) -> std::io::Result<RunOutcome> {
    if !capture_stdout {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let mut child = command.spawn()?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(RunOutcome::Exited(status));
        }
        if started.elapsed() >= timeout {
            child.kill()?;
            let _ = child.wait();
            return Ok(RunOutcome::TimedOut);
        }
        thread::sleep(Duration::from_millis(100));
    }
}
