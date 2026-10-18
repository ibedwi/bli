use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bli::clickup::{self, ClickUp};
use bli::code_host::CodeHost;
use bli::config::{Config, HostKind, TrackerKind};
use bli::error::Error;
use bli::git::Git;
use bli::github::GitHub;
use bli::herdr::Herdr;
use bli::host::LaneHost;
use bli::init::{self, Prompter};
use bli::process::{ProcessRunner, SystemRunner};
use bli::superset::Superset;
use bli::tracker::IssueTracker;
use bli::Providers;

/// The providers backed by real tools, for a `bli` run from `cwd`.
struct System {
    cwd: PathBuf,
}

impl Providers for System {
    fn issue_tracker(&self, config: &Config) -> Result<Box<dyn IssueTracker + '_>, Error> {
        match config.tracker.kind {
            TrackerKind::Github => Ok(Box::new(GitHub::new(SystemRunner))),
            TrackerKind::Clickup => {
                let settings = config.tracker.clickup.as_ref().ok_or_else(|| {
                    Error::new("`kind = \"clickup\"` needs a [tracker.clickup] table")
                })?;
                let main = Git::open(&self.cwd).ok();
                let token = clickup::find_token(
                    std::env::var(clickup::TOKEN_VAR).ok(),
                    checkout_root(&self.cwd),
                    main.as_ref().map(Git::main_checkout),
                );
                Ok(Box::new(ClickUp::new(settings, token)))
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

    /// The prompts read keys from stdin and draw on stderr, so both must be a terminal.
    fn prompter(&self) -> Result<Box<dyn Prompter + '_>, Error> {
        if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
            Ok(Box::new(init::Terminal))
        } else {
            Err(init::not_a_terminal())
        }
    }
}

/// The root of the checkout containing `dir`: the nearest directory holding `.git`.
fn checkout_root(dir: &Path) -> Option<&Path> {
    dir.ancestors().find(|d| d.join(".git").exists())
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
    let out = bli::run(&args, &cwd, &System { cwd: cwd.clone() });
    let _ = std::io::stdout().write_all(out.stdout.as_bytes());
    let _ = std::io::stderr().write_all(out.stderr.as_bytes());
    ExitCode::from(out.code as u8)
}
