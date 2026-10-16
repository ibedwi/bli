pub mod branch;
pub mod code_host;
pub mod config;
pub mod error;
pub mod git;
pub mod github;
pub mod herdr;
pub mod host;
pub mod process;
pub mod tracker;

mod commands;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::code_host::CodeHost;
use crate::config::Config;
use crate::error::Error;
use crate::git::Git;
use crate::host::LaneHost;
use crate::tracker::IssueTracker;

/// Builds the providers a repo's config asks for. The real one runs real tools; tests pass fakes.
pub trait Providers {
    fn issue_tracker(&self, config: &Config) -> Result<Box<dyn IssueTracker + '_>, Error>;
    fn lane_host(&self, config: &Config) -> Result<Box<dyn LaneHost + '_>, Error>;
    fn code_host(&self, config: &Config) -> Result<Box<dyn CodeHost + '_>, Error>;
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
    /// Start, list and manage lanes
    #[command(subcommand)]
    Lane(LaneCommand),
}

#[derive(Subcommand)]
enum LaneCommand {
    /// Start one lane per ticket, one at a time, and claim each ticket
    Start {
        /// The tickets to start lanes for
        #[arg(required = true)]
        tickets: Vec<String>,
        /// The branch to cut lanes from; defaults to `[lane] base` in bli.toml
        #[arg(long)]
        base: Option<String>,
        /// The branch slug, instead of one made from the ticket title; one ticket only
        #[arg(long)]
        slug: Option<String>,
    },
    /// List every lane with its ticket, branch, base, PR and the host's status hint
    List,
    /// Print a lane's agent's recent output
    Read {
        /// The lane's ticket
        ticket: String,
        /// How many lines of output to read
        #[arg(long, default_value_t = 80)]
        lines: u32,
    },
    /// Push a finished lane's branch and open its pull request against the lane's base
    Ship {
        /// The lane's ticket
        ticket: String,
        /// A file holding the pull request's body
        #[arg(long)]
        body_file: PathBuf,
        /// The pull request's title; defaults to the ticket's title
        #[arg(long)]
        title: Option<String>,
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
        Ok(commands::Report {
            stdout,
            failure: None,
        }) => Outcome::success(stdout),
        // Part of the work failed: keep its output, so a caller can see what did succeed.
        Ok(commands::Report {
            stdout,
            failure: Some(e),
        }) => Outcome {
            code: 1,
            stdout,
            stderr: render_error(&e),
        },
        Err(e) => Outcome::failure(1, render_error(&e)),
    }
}

fn dispatch(cli: &Cli, cwd: &Path, providers: &dyn Providers) -> Result<commands::Report, Error> {
    let config = Config::load(cwd)?;
    let git = Git::open(cwd)?;
    match &cli.command {
        Command::Frontier { parent } => {
            let tracker = providers.issue_tracker(&config)?;
            let host = providers.lane_host(&config)?;
            commands::frontier(
                tracker.as_ref(),
                host.as_ref(),
                &git,
                &config,
                parent.as_deref(),
                cli.json,
            )
            .map(Into::into)
        }
        Command::Lane(LaneCommand::Start {
            tickets,
            base,
            slug,
        }) => {
            let tracker = providers.issue_tracker(&config)?;
            let host = providers.lane_host(&config)?;
            let request = commands::StartRequest {
                tickets,
                base: base.as_deref(),
                slug: slug.as_deref(),
            };
            commands::lane_start(
                tracker.as_ref(),
                host.as_ref(),
                &git,
                &config,
                &request,
                cli.json,
            )
        }
        Command::Lane(LaneCommand::List) => {
            let host = providers.lane_host(&config)?;
            let code = providers.code_host(&config)?;
            commands::lane_list(host.as_ref(), code.as_ref(), &git, &config, cli.json)
                .map(Into::into)
        }
        Command::Lane(LaneCommand::Read { ticket, lines }) => {
            let host = providers.lane_host(&config)?;
            commands::lane_read(host.as_ref(), &git, &config, ticket, *lines, cli.json)
                .map(Into::into)
        }
        Command::Lane(LaneCommand::Ship {
            ticket,
            body_file,
            title,
        }) => {
            let tracker = providers.issue_tracker(&config)?;
            let host = providers.lane_host(&config)?;
            let code = providers.code_host(&config)?;
            let request = commands::ShipRequest {
                ticket,
                body_file,
                title: title.as_deref(),
            };
            commands::lane_ship(
                tracker.as_ref(),
                host.as_ref(),
                code.as_ref(),
                &git,
                &config,
                &request,
                cli.json,
            )
            .map(Into::into)
        }
    }
}

fn render_error(e: &Error) -> String {
    match e.hint() {
        Some(hint) => format!("error: {e}\nhint: {hint}\n"),
        None => format!("error: {e}\n"),
    }
}
