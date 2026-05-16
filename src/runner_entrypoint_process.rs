use std::ffi::OsString;
use std::process::{Command, ExitStatus};

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
