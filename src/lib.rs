pub mod config;
pub mod error;
pub mod github;
pub mod process;
pub mod tracker;

mod commands;

use std::path::Path;

use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::error::Error;
use crate::tracker::IssueTracker;

/// Builds the providers a repo's config asks for. The real one runs real tools; tests pass fakes.
pub trait Providers {
    fn issue_tracker(&self, config: &Config) -> Result<Box<dyn IssueTracker + '_>, Error>;
}

/// What a command run produced, for `main` to print and exit with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Outcome {
    fn success(stdout: String) -> Self {
        Outcome {
            code: 0,
            stdout,
            stderr: String::new(),
        }
    }

    fn failure(code: i32, stderr: String) -> Self {
        Outcome {
            code,
            stdout: String::new(),
            stderr,
        }
    }
}

#[derive(Parser)]
#[command(
    name = "bli",
    version,
    about = "Run tickets as parallel lanes of agent work"
)]
struct Cli {
    /// Print machine-readable JSON instead of text
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the tickets under a parent that can be started now
    Frontier {
        /// The parent ticket; defaults to `[tracker] parent` in bli.toml
        #[arg(long)]
        parent: Option<String>,
    },
}

/// Runs `bli` with `args` (without the program name) from `cwd`.
pub fn run(args: &[&str], cwd: &Path, providers: &dyn Providers) -> Outcome {
    let cli = match Cli::try_parse_from(std::iter::once("bli").chain(args.iter().copied())) {
        Ok(cli) => cli,
        Err(e) => {
            let text = e.render().to_string();
            return if e.use_stderr() {
                Outcome::failure(2, text)
            } else {
                Outcome::success(text)
            };
        }
    };
    match dispatch(&cli, cwd, providers) {
        Ok(stdout) => Outcome::success(stdout),
        Err(e) => Outcome::failure(1, render_error(&e)),
    }
}

fn dispatch(cli: &Cli, cwd: &Path, providers: &dyn Providers) -> Result<String, Error> {
    let config = Config::load(cwd)?;
    match &cli.command {
        Command::Frontier { parent } => {
            let tracker = providers.issue_tracker(&config)?;
            commands::frontier(
                tracker.as_ref(),
                parent.as_deref(),
                config.tracker.parent.as_deref(),
                cli.json,
            )
        }
    }
}

fn render_error(e: &Error) -> String {
    match e.hint() {
        Some(hint) => format!("error: {e}\nhint: {hint}\n"),
        None => format!("error: {e}\n"),
    }
}
