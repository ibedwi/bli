use std::io::Write;
use std::process::ExitCode;

use bli::code_host::CodeHost;
use bli::config::{Config, HostKind, TrackerKind};
use bli::error::Error;
use bli::github::GitHub;
use bli::herdr::Herdr;
use bli::host::LaneHost;
use bli::process::{ProcessRunner, SystemRunner};
use bli::superset::Superset;
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

    fn lane_host(&self, config: &Config) -> Result<Box<dyn LaneHost + '_>, Error> {
        match config.lane.host {
            HostKind::Herdr => {
                let herdr_env = std::env::var("HERDR_ENV").ok();
                let workspace = std::env::var("HERDR_WORKSPACE_ID").ok();
                Ok(Box::new(
                    Herdr::new(SystemRunner, herdr_env.as_deref())
                        .in_workspace(workspace.as_deref()),
                ))
            }
            HostKind::Superset => {
                let workspace = std::env::var("SUPERSET_WORKSPACE_ID").ok();
                Ok(Box::new(Superset::new(SystemRunner, workspace.as_deref())))
            }
        }
    }

    /// GitHub is the only code host.
    fn code_host(&self, _config: &Config) -> Result<Box<dyn CodeHost + '_>, Error> {
        Ok(Box::new(GitHub::new(SystemRunner)))
    }

    fn git_runner(&self) -> &dyn ProcessRunner {
        &SystemRunner
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
