//! herdr as the lane host, through the `herdr` CLI. A lane is a herdr worktree
//! workspace, and its agent is a Claude named `t<ticket id>` in the workspace's root pane.

use std::io;
use std::path::{Path, PathBuf};

use serde::de::{DeserializeOwned, IgnoredAny};
use serde::Deserialize;

use crate::check::Check;
use crate::error::Error;
use crate::host::{CreatedLane, HostCheckout, LaneHost, NewLane, StatusHint};
use crate::process::{Output, ProcessRunner};

/// How long to wait for a new agent to start working before reporting its status as unknown.
const START_TIMEOUT_MS: &str = "20000";

pub struct Herdr<R> {
    runner: R,
    inside: bool,
}

impl<R: ProcessRunner> Herdr<R> {
    /// `herdr_env` is the value of `HERDR_ENV`, which herdr sets to `1` in its panes.
    pub fn new(runner: R, herdr_env: Option<&str>) -> Self {
        Herdr {
            runner,
            inside: herdr_env == Some("1"),
        }
    }

    /// Runs `herdr` with `args` and returns the `result` of its JSON reply.
    fn herdr<T: DeserializeOwned>(&self, args: &[&str]) -> Result<T, Error> {
        self.try_herdr(args)?.map_err(|e| failed(args, e))
    }

    /// Like [`Herdr::herdr`], but hands back an error herdr itself reported, for the caller to inspect.
    fn try_herdr<T: DeserializeOwned>(&self, args: &[&str]) -> Result<Result<T, ErrorBody>, Error> {
        let stdout = match self.try_herdr_text(args)? {
            Ok(stdout) => stdout,
            Err(e) => return Ok(Err(e)),
        };
        serde_json::from_str::<Reply<T>>(&stdout)
            .map(|reply| Ok(reply.result))
            .map_err(|e| Error::new(format!("could not read `{}` output: {e}", command(args))))
    }

    /// Like [`Herdr::try_herdr`], for the commands that reply with plain text rather than JSON.
    fn try_herdr_text(&self, args: &[&str]) -> Result<Result<String, ErrorBody>, Error> {
        let command = command(args);
        let out = self.run(args)?;
        if !out.success {
            return match serde_json::from_str::<ErrorReply>(&out.stderr) {
                Ok(reply) => Ok(Err(reply.error)),
                Err(_) => Err(Error::new(format!(
                    "`{command}` failed: {}",
                    out.stderr.trim()
                ))),
            };
        }
        Ok(Ok(out.stdout))
    }

    /// Runs `herdr` with `args`, failing only when it could not be run at all.
    fn run(&self, args: &[&str]) -> Result<Output, Error> {
        self.runner.run("herdr", args).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::new("could not run `herdr`: it is not installed")
                .with_hint(
                    "install herdr (https://herdr.dev), or change `host` under [lane] in bli.toml",
                ),
            _ => Error::new(format!("could not run `herdr`: {e}")),
        })
    }

    /// Runs a `herdr` command outside the socket API, whose plain stdout is its reply.
    fn run_plain(&self, args: &[&str]) -> Result<String, Error> {
        let out = self.run(args)?;
        if !out.success {
            return Err(Error::new(format!(
                "`{}` failed: {}",
                command(args),
                out.stderr.trim()
            )));
        }
        Ok(out.stdout)
    }

    /// Whether the herdr server is up to take commands. `bli` passes the
    /// permission-mode flag itself in [`LaneHost::start_agent`], so that is all
    /// the permission-mode check needs.
    fn server_running(&self) -> Result<String, Error> {
        let stdout = self.run_plain(&["status", "server", "--json"])?;
        let status: ServerStatus = serde_json::from_str(&stdout)
            .map_err(|e| Error::new(format!("could not read `herdr status` output: {e}")))?;
        if !status.running {
            return Err(Error::new("the herdr server is not running")
                .with_hint("start herdr by running `herdr`"));
        }
        Ok(
            "herdr is running, and bli starts Claude with --permission-mode bypassPermissions"
                .into(),
        )
    }

    /// Like [`Herdr::herdr`], for commands whose reply `bli` doesn't need.
    fn herdr_unit(&self, args: &[&str]) -> Result<(), Error> {
        self.herdr::<IgnoredAny>(args).map(drop)
    }
}

/// An error herdr reported, as a `bli` error.
fn failed(args: &[&str], e: ErrorBody) -> Error {
    Error::new(format!(
        "`{}` failed: {} ({})",
        command(args),
        e.message,
        e.code
    ))
}

impl<R: ProcessRunner> LaneHost for Herdr<R> {
    fn name(&self) -> &str {
        "herdr"
    }

    fn is_inside(&self) -> bool {
        self.inside
    }

    fn create_lane(&self, repo: &Path, lane: &NewLane) -> Result<CreatedLane, Error> {
        // herdr only creates worktrees from the main checkout, never from a linked one.
        let repo = repo.to_string_lossy();
        let created: WorktreeCreated = self.herdr(&[
            "worktree",
            "create",
            "--cwd",
            &repo,
            "--branch",
            &lane.branch,
            "--base",
            &lane.base_ref,
            "--label",
            &lane.label,
            "--no-focus",
            "--trust-repository",
        ])?;
        Ok(CreatedLane {
            path: created.worktree.path,
            workspace: created.workspace.workspace_id,
            handle: created.root_pane.pane_id,
        })
    }

    fn start_agent(
        &self,
        lane: &CreatedLane,
        ticket_id: &str,
        prompt: &str,
    ) -> Result<StatusHint, Error> {
        let name = format!("t{ticket_id}");
        self.herdr_unit(&[
            "agent",
            "start",
            &name,
            "--kind",
            "claude",
            "--pane",
            &lane.handle,
            "--",
            "--permission-mode",
            "bypassPermissions",
        ])?;
        self.herdr_unit(&["agent", "prompt", &name, prompt])?;
        let waited = self.try_herdr::<AgentReply>(&[
            "agent",
            "wait",
            &name,
            "--until",
            "working",
            "--until",
            "blocked",
            "--timeout",
            START_TIMEOUT_MS,
        ]);
        match waited? {
            Ok(reply) => Ok(reply.agent.status()),
            // An agent slow to pick up its prompt is started all the same; its status just isn't known yet.
            Err(e) if e.code == "timeout" => Ok(StatusHint::Unknown),
            Err(e) => Err(failed(&["agent", "wait"], e)),
        }
    }

    fn read_agent(&self, ticket_id: &str, lines: u32) -> Result<String, Error> {
        let name = format!("t{ticket_id}");
        let lines = lines.to_string();
        let args = [
            "agent",
            "read",
            &name,
            "--source",
            "recent-unwrapped",
            "--lines",
            &lines,
        ];
        self.try_herdr_text(&args)?.map_err(|e| failed(&args, e))
    }

    fn list(&self, repo: &Path) -> Result<Vec<HostCheckout>, Error> {
        let repo = repo.to_string_lossy();
        let worktrees: WorktreeList = self.herdr(&["worktree", "list", "--cwd", &repo])?;
        let agents: AgentList = self.herdr(&["agent", "list"])?;
        Ok(worktrees
            .worktrees
            .into_iter()
            .map(|w| {
                let status = w
                    .open_workspace_id
                    .as_deref()
                    .map_or(StatusHint::Unknown, |ws| agents.status_of(ws));
                HostCheckout {
                    path: w.path,
                    branch: w.branch,
                    is_main: !w.is_linked_worktree,
                    workspace: w.open_workspace_id,
                    status,
                }
            })
            .collect())
    }
    fn checks(&self) -> Vec<Check> {
        let version = self
            .run_plain(&["--version"])
            .map(|out| out.trim().to_string());
        vec![
            Check::from_result("herdr", version),
            Check::from_result("permission mode", self.server_running()),
        ]
    }

    fn remove_lane(
        &self,
        repo: &Path,
        path: &Path,
        workspace: Option<&str>,
        force: bool,
    ) -> Result<(), Error> {
        // `herdr worktree remove` only takes a workspace, so a closed one is reopened first.
        let opened;
        let workspace = match workspace {
            Some(ws) => ws,
            None => {
                let repo = repo.to_string_lossy();
                let path = path.to_string_lossy();
                let reply: WorktreeOpened = self.herdr(&[
                    "worktree",
                    "open",
                    "--cwd",
                    &repo,
                    "--path",
                    &path,
                    "--no-focus",
                    "--trust-repository",
                ])?;
                opened = reply.workspace.workspace_id;
                &opened
            }
        };
        let mut args = vec!["worktree", "remove", "--workspace", workspace];
        if force {
            args.push("--force");
        }
        self.herdr_unit(&args)
    }
}

/// `herdr <group> <verb>`, for messages.
fn command(args: &[&str]) -> String {
    let words: Vec<_> = args.iter().take(2).copied().collect();
    format!("herdr {}", words.join(" "))
}

/// `herdr status server --json` output, which is not wrapped in a `result` like socket API replies.
#[derive(Deserialize)]
struct ServerStatus {
    running: bool,
}

#[derive(Deserialize)]
struct Reply<T> {
    result: T,
}

#[derive(Deserialize)]
struct ErrorReply {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

#[derive(Deserialize)]
struct WorktreeCreated {
    worktree: Worktree,
    workspace: Workspace,
    root_pane: Pane,
}

#[derive(Deserialize)]
struct WorktreeOpened {
    workspace: Workspace,
}

#[derive(Deserialize)]
struct WorktreeList {
    worktrees: Vec<Worktree>,
}

#[derive(Deserialize)]
struct Worktree {
    path: PathBuf,
    branch: Option<String>,
    is_linked_worktree: bool,
    open_workspace_id: Option<String>,
}

#[derive(Deserialize)]
struct Workspace {
    workspace_id: String,
}

#[derive(Deserialize)]
struct Pane {
    pane_id: String,
}

#[derive(Deserialize)]
struct AgentReply {
    agent: Agent,
}

#[derive(Deserialize)]
struct AgentList {
    agents: Vec<Agent>,
}

impl AgentList {
    /// The status of the agents in workspace `ws`, picking the one most in need of attention.
    fn status_of(&self, ws: &str) -> StatusHint {
        let statuses: Vec<_> = self
            .agents
            .iter()
            .filter(|a| a.workspace_id == ws)
            .map(Agent::status)
            .collect();
        [StatusHint::Blocked, StatusHint::Working, StatusHint::Idle]
            .into_iter()
            .find(|s| statuses.contains(s))
            .unwrap_or(StatusHint::Unknown)
    }
}

#[derive(Deserialize)]
struct Agent {
    agent_status: String,
    workspace_id: String,
}

impl Agent {
    fn status(&self) -> StatusHint {
        match self.agent_status.as_str() {
            "working" => StatusHint::Working,
            "blocked" => StatusHint::Blocked,
            "idle" | "done" => StatusHint::Idle,
            _ => StatusHint::Unknown,
        }
    }
}
