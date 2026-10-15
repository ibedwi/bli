//! The one way adapters run external tools, so tests can swap it for a fake.

use std::io;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub trait ProcessRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<Output>;
}

/// Runs real processes with no stdin and prompts disabled, so `bli` never waits on input.
pub struct SystemRunner;

impl ProcessRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<Output> {
        let out = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .env("GH_PROMPT_DISABLED", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()?;
        Ok(Output {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

impl<T: ProcessRunner + ?Sized> ProcessRunner for &T {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<Output> {
        (**self).run(program, args)
    }
}
