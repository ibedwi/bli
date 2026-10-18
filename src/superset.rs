//! Superset as the lane host, through the `superset` CLI. A lane is a Superset
//! worktree workspace on the repo's project, and its agent runs Superset's Claude
//! preset in one of the workspace's terminals.

use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::check::Check;
use crate::error::Error;
use crate::host::{CreatedLane, HostCheckout, LaneHost, NewLane, StatusHint};
use crate::process::{Output, ProcessRunner};

/// How long to wait for a new agent to start working before reporting the state it is in.
const START_TIMEOUT_SECS: &str = "20";

pub struct Superset<R> {
    runner: R,
    workspace: Option<String>,
}

impl<R: ProcessRunner> Superset<R> {
    /// `workspace_env` is the value of `SUPERSET_WORKSPACE_ID`, which Superset sets in its terminals.
    pub fn new(runner: R, workspace_env: Option<&str>) -> Self {
        Superset {
            runner,
            workspace: workspace_env
                .filter(|ws| !ws.is_empty())
                .map(str::to_string),
        }
    }

    /// Runs `superset` with `args` and parses its JSON reply.
    fn superset<T: DeserializeOwned>(&self, args: &[&str]) -> Result<T, Error> {
        parse(args, &self.superset_text(args)?)
    }

    /// Runs `superset` with `args` and returns its stdout, or the error it reported.
    fn superset_text(&self, args: &[&str]) -> Result<String, Error> {
        self.try_superset_text(args)?
            .map_err(|stderr| failed(args, &stderr))
    }

    /// Like [`Superset::superset_text`], but hands back the stderr of a failure, for the caller to inspect.
    fn try_superset_text(&self, args: &[&str]) -> Result<Result<String, String>, Error> {
        let out = self.run("superset", args)?;
        Ok(if out.success {
            Ok(out.stdout)
        } else {
            Err(out.stderr)
        })
    }

    /// Runs `program`, failing only when it could not be run at all.
    fn run(&self, program: &str, args: &[&str]) -> Result<Output, Error> {
        self.runner.run(program, args).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound if program == "superset" => {
                Error::new("could not run `superset`: it is not installed").with_hint(
                    "install Superset's CLI (https://superset.sh), or change `host` under [lane] in bli.toml",
                )
            }
            _ => Error::new(format!("could not run `{program}`: {e}")),
        })
    }

    /// Runs `git` in `dir`.
    fn git(&self, dir: &Path, args: &[&str]) -> Result<String, Error> {
        let dir = dir.to_string_lossy();
        let mut git_args = vec!["-C", &dir];
        git_args.extend_from_slice(args);
        let out = self.run("git", &git_args)?;
        if !out.success {
            return Err(Error::new(format!(
                "`git {}` failed: {}",
                args.join(" "),
                out.stderr.trim()
            )));
        }
        Ok(out.stdout)
    }

    /// The ID of the Superset project whose repo is the main checkout at `repo`.
    fn project(&self, repo: &Path) -> Result<String, Error> {
        let projects: Vec<Project> = self.superset(&["projects", "list", "--local", "--json"])?;
        let repo_path = canonical(repo);
        projects
            .into_iter()
            .find(|p| canonical(&p.path) == repo_path)
            .map(|p| p.id)
            .ok_or_else(|| {
                Error::new(format!("{} is not a Superset project", repo.display())).with_hint(
                    format!(
                        "add it with `superset projects create --local --name <name> --import {}`",
                        repo.display()
                    ),
                )
            })
    }

    /// Whether Superset's host service, which every lane command talks to, is up.
    fn host_running(&self) -> Result<String, Error> {
        let status: HostStatus = self.superset(&["status", "--json"])?;
        if !status.running {
            return Err(Error::new("Superset's host service is not running")
                .with_hint("open the Superset app, or run `superset start --daemon`"));
        }
        if status.healthy == Some(false) {
            return Err(
                Error::new("Superset's host service is running but not responding").with_hint(
                    "restart the Superset app, or run `superset stop` then `superset start --daemon`",
                ),
            );
        }
        Ok(match status.endpoint {
            Some(endpoint) => format!("Superset's host service is running at {endpoint}"),
            None => "Superset's host service is running".into(),
        })
    }

    /// Whether the Claude preset `bli` launches lanes with, the first by order, skips
    /// permission prompts. Superset keeps Claude's flags in the preset, not on `bli`'s command line.
    fn claude_bypasses_permissions(&self) -> Result<String, Error> {
        let presets: Vec<AgentPreset> = self.superset(&["agents", "list", "--local", "--json"])?;
        let add_the_flag = "in Superset, open Settings → Agents and add `--dangerously-skip-permissions` to the Claude agent's arguments";
        let claude = presets
            .iter()
            .filter(|p| p.preset_id == "claude")
            .min_by_key(|p| p.order)
            .ok_or_else(|| Error::new("Superset has no Claude preset").with_hint(add_the_flag))?;
        let argv: Vec<&str> = claude
            .command
            .split_whitespace()
            .chain(claude.args.iter().map(String::as_str))
            .collect();
        let bypasses = argv.iter().enumerate().any(|(i, arg)| {
            *arg == "--dangerously-skip-permissions"
                || *arg == "--permission-mode=bypassPermissions"
                || (*arg == "--permission-mode" && argv.get(i + 1) == Some(&"bypassPermissions"))
        });
        if !bypasses {
            return Err(Error::new(format!(
                "Superset's Claude preset \"{}\" runs `{}`, which stops on permission prompts",
                claude.label,
                argv.join(" ")
            ))
            .with_hint(add_the_flag));
        }
        Ok(format!(
            "Superset's Claude preset \"{}\" runs `{}`",
            claude.label,
            argv.join(" ")
        ))
    }
}

/// Parses the JSON reply of `superset` run with `args`.
fn parse<T: DeserializeOwned>(args: &[&str], stdout: &str) -> Result<T, Error> {
    serde_json::from_str(stdout)
        .map_err(|e| Error::new(format!("could not read `{}` output: {e}", command(args))))
}

/// An error Superset reported on stderr, as a `bli` error carrying Superset's hint.
fn failed(args: &[&str], stderr: &str) -> Error {
    let mut message = Vec::new();
    let mut hint = None;
    for line in stderr.lines() {
        if let Some(h) = line.strip_prefix("Hint: ") {
            hint = Some(h.to_string());
        } else if !line.trim().is_empty() {
            message.push(line.strip_prefix("Error: ").unwrap_or(line));
        }
    }
    let error = Error::new(format!("`{}` failed: {}", command(args), message.join(" ")));
    match hint {
        Some(hint) => error.with_hint(hint),
        None => error,
    }
}

impl<R: ProcessRunner> LaneHost for Superset<R> {
    fn name(&self) -> &str {
        "superset"
    }

    fn is_inside(&self) -> bool {
        self.workspace.is_some()
    }

    fn current_workspace(&self) -> Option<&str> {
        self.workspace.as_deref()
    }

    fn create_lane(&self, repo: &Path, lane: &NewLane) -> Result<CreatedLane, Error> {
        let project = self.project(repo)?;
        // Superset checks out an existing branch exactly as it is, with no prefix, and
        // only takes a branch name as a base, so `bli` cuts the branch from the base itself.
        self.git(
            repo,
            &["branch", "--no-track", &lane.branch, &lane.base_ref],
        )?;
        let created: Result<WorkspaceCreated, Error> = self.superset(&[
            "workspaces",
            "create",
            "--local",
            "--project",
            &project,
            "--name",
            &lane.label,
            "--branch",
            &lane.branch,
            "--skip-branch-prefix",
            "--json",
        ]);
        let created = match created {
            Ok(created) => created,
            Err(e) => {
                // Leave no branch behind for a lane that was never made.
                let _ = self.git(repo, &["branch", "-D", &lane.branch]);
                return Err(e);
            }
        };
        let id = created.workspace.id;
        let detail: WorkspaceDetail =
            self.superset(&["workspaces", "get", &id, "--local", "--json"])?;
        Ok(CreatedLane {
            path: detail.worktree_path,
            workspace: id.clone(),
            handle: id,
        })
    }

    fn start_agent(
        &self,
        lane: &CreatedLane,
        _ticket_id: &str,
        prompt: &str,
    ) -> Result<StatusHint, Error> {
        // Superset's Claude preset carries the permission mode; `bli doctor` checks it.
        let agent: AgentRun = self.superset(&[
            "agents",
            "create",
            "--local",
            "--workspace",
            &lane.workspace,
            "--agent",
            "claude",
            "--prompt",
            prompt,
            "--json",
        ])?;
        let args = [
            "agents",
            "wait",
            "--local",
            "--workspace",
            &lane.workspace,
            "--terminal",
            &agent.session_id,
            "--until",
            "working",
            "--timeout",
            START_TIMEOUT_SECS,
            "--json",
        ];
        let stdout = match self.try_superset_text(&args)? {
            Ok(stdout) => stdout,
            // An agent slow to start working is started all the same; report the state it is in.
            Err(stderr) => {
                return timed_out_state(&stderr)
                    .map(status_of)
                    .ok_or_else(|| failed(&args, &stderr))
            }
        };
        let waited: AgentWaited = parse(&args, &stdout)?;
        Ok(status_of(&waited.state))
    }

    fn read_agent(
        &self,
        ticket_id: &str,
        workspace: Option<&str>,
        lines: u32,
    ) -> Result<String, Error> {
        let workspace = workspace.ok_or_else(|| {
            Error::new(format!(
                "lane {ticket_id} has no Superset workspace to read its agent from"
            ))
        })?;
        let snapshot: Snapshot =
            self.superset(&["snapshot", "--workspace", workspace, "--json"])?;
        let terminal = snapshot
            .workspaces
            .iter()
            .filter(|w| w.id == workspace)
            .flat_map(|w| &w.terminals)
            .find(|t| t.agent.is_some())
            .ok_or_else(|| {
                Error::new(format!(
                    "lane {ticket_id} has no agent running in Superset workspace {workspace}"
                ))
            })?;
        let lines = lines.to_string();
        let screen: Screen = self.superset(&[
            "terminals",
            "read",
            "--local",
            "--workspace",
            workspace,
            "--terminal",
            &terminal.id,
            "--max-lines",
            &lines,
            "--json",
        ])?;
        let mut text = screen.text;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }

    fn list(&self, repo: &Path) -> Result<Vec<HostCheckout>, Error> {
        let project = self.project(repo)?;
        let workspaces: Vec<ListedWorkspace> = self.superset(&[
            "workspaces",
            "list",
            "--local",
            "--project",
            &project,
            "--json",
        ])?;
        let snapshot: Snapshot = self.superset(&["snapshot", "--json"])?;
        Ok(workspaces
            .into_iter()
            .map(|w| HostCheckout {
                path: w.worktree_path,
                branch: Some(w.branch).filter(|b| !b.is_empty()),
                // A `local` workspace shares the project's own checkout.
                is_main: w.kind == "local",
                status: snapshot.status_of(&w.id),
                workspace: Some(w.id),
            })
            .collect())
    }

    fn checks(&self) -> Vec<Check> {
        let version = self
            .superset_text(&["--version"])
            .map(|out| format!("superset {}", out.trim()));
        vec![
            Check::from_result("superset", version),
            Check::from_result("superset host", self.host_running()),
            Check::from_result("permission mode", self.claude_bypasses_permissions()),
        ]
    }

    fn remove_lane(
        &self,
        _repo: &Path,
        path: &Path,
        workspace: Option<&str>,
        force: bool,
    ) -> Result<(), Error> {
        let workspace = workspace.ok_or_else(|| {
            Error::new(format!(
                "the lane at {} has no Superset workspace to remove",
                path.display()
            ))
        })?;
        // Archiving always discards uncommitted work, so the refusal without `force` is `bli`'s.
        if !force {
            let status = self.git(path, &["status", "--porcelain", "--untracked-files=all"])?;
            if !status.trim().is_empty() {
                return Err(Error::new(format!(
                    "the lane at {} has uncommitted or untracked files",
                    path.display()
                ))
                .with_hint("commit or remove them, or pass --force to lose them"));
            }
        }
        let archived: Archived =
            self.superset(&["workspaces", "archive", workspace, "--local", "--json"])?;
        // Superset archives the workspace even when it leaves the checkout on disk, and
        // says so only in a warning, so `bli` looks for itself.
        if path.exists() {
            let mut message = format!(
                "Superset archived workspace {workspace} but left its checkout at {}",
                path.display()
            );
            if !archived.warnings.is_empty() {
                message = format!("{message}: {}", archived.warnings.join("; "));
            }
            return Err(Error::new(message).with_hint(format!("remove {} by hand", path.display())));
        }
        Ok(())
    }
}

/// The state `superset agents wait` left the agent in when it timed out, from its
/// error `Timed out after <n>s; agent is <state>`, or `None` for any other failure.
fn timed_out_state(stderr: &str) -> Option<&str> {
    stderr
        .lines()
        .find_map(|line| line.strip_prefix("Error: Timed out after "))
        .and_then(|rest| rest.split_once("; agent is "))
        .map(|(_, state)| state.trim())
}

/// Superset's agent state (`agentRunState`) as a status hint. A `starting` agent has
/// not begun its first turn, so what it will do is not known yet.
fn status_of(state: &str) -> StatusHint {
    match state {
        "working" => StatusHint::Working,
        "blocked" => StatusHint::Blocked,
        "idle" => StatusHint::Idle,
        _ => StatusHint::Unknown,
    }
}

/// `path` with symlinks resolved, so two spellings of one repo compare equal.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// `superset <group> <verb>`, for messages.
fn command(args: &[&str]) -> String {
    let words: Vec<_> = args
        .iter()
        .take_while(|a| !a.starts_with('-'))
        .take(2)
        .copied()
        .collect();
    format!("superset {}", words.join(" "))
}

#[derive(Deserialize)]
struct Project {
    id: String,
    path: PathBuf,
}

#[derive(Deserialize)]
struct WorkspaceCreated {
    workspace: Workspace,
}

#[derive(Deserialize)]
struct Workspace {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDetail {
    worktree_path: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentRun {
    /// The terminal the agent runs in.
    session_id: String,
}

#[derive(Deserialize)]
struct AgentWaited {
    state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedWorkspace {
    id: String,
    branch: String,
    #[serde(rename = "type")]
    kind: String,
    worktree_path: PathBuf,
}

#[derive(Deserialize)]
struct Snapshot {
    workspaces: Vec<SnapshotWorkspace>,
}

impl Snapshot {
    /// The status of the agents in workspace `ws`, picking the one most in need of attention.
    fn status_of(&self, ws: &str) -> StatusHint {
        StatusHint::most_urgent(
            self.workspaces
                .iter()
                .filter(|w| w.id == ws)
                .flat_map(|w| &w.terminals)
                .filter_map(|t| t.agent.as_ref())
                .map(|a| status_of(&a.state)),
        )
    }
}

#[derive(Deserialize)]
struct SnapshotWorkspace {
    id: String,
    terminals: Vec<Terminal>,
}

#[derive(Deserialize)]
struct Terminal {
    id: String,
    agent: Option<TerminalAgent>,
}

#[derive(Deserialize)]
struct TerminalAgent {
    state: String,
}

#[derive(Deserialize)]
struct Screen {
    text: String,
}

#[derive(Deserialize)]
struct Archived {
    #[serde(default)]
    warnings: Vec<String>,
}

/// `superset status --json` output; only these fields matter to `bli`.
#[derive(Deserialize)]
struct HostStatus {
    running: bool,
    healthy: Option<bool>,
    endpoint: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentPreset {
    preset_id: String,
    label: String,
    command: String,
    args: Vec<String>,
    order: i64,
}
