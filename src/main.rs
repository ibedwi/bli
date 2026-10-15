use std::io::Write;
use std::process::ExitCode;

use bli::config::{Config, TrackerKind};
use bli::error::Error;
use bli::github::GitHub;
use bli::process::SystemRunner;
use bli::tracker::IssueTracker;
use bli::Providers;

/// The providers backed by real tools.
struct System;

impl Providers for System {
    fn issue_tracker(&self, config: &Config) -> Result<Box<dyn IssueTracker + '_>, Error> {
        match config.tracker.kind {
            TrackerKind::Github => Ok(Box::new(GitHub::new(SystemRunner))),
            TrackerKind::Clickup => {
                Err(Error::new("the ClickUp issue tracker is not supported yet")
                    .with_hint("set `kind = \"github\"` under [tracker] in bli.toml"))
            }
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            eprintln!("error: could not read the current directory: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out = bli::run(&args, &cwd, &System);
    let _ = std::io::stdout().write_all(out.stdout.as_bytes());
    let _ = std::io::stderr().write_all(out.stderr.as_bytes());
    ExitCode::from(out.code as u8)
}
