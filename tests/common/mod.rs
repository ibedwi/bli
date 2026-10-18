//! Fakes and fixtures shared by the command-layer tests: an in-memory issue
//! tracker, lane host and code host, and a real git repo with a local bare remote.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use bli::check::Check;
use bli::code_host::{CodeHost, NewPullRequest, PrState, PullRequest};
use bli::config::Config;
use bli::error::Error;
use bli::host::{CreatedLane, HostCheckout, LaneHost, NewLane, StatusHint};
use bli::init::{Cancelled, Choice, Prompter, Validator};
use bli::process::{Output, ProcessRunner};
use bli::tracker::{IssueTracker, OnMerge, Ticket, TicketDetails};
use bli::{Outcome, Providers};
use tempfile::TempDir;

/// An issue tracker whose tickets and frontiers are fixed up front, and which records claims.
#[derive(Default)]
pub struct FakeTracker {
    frontiers: HashMap<String, Vec<Ticket>>,
    tickets: HashMap<String, TicketDetails>,
    claimed: RefCell<Vec<String>>,
    on_merged: RefCell<Vec<(String, OnMerge)>>,
    checks: Vec<Check>,
}

impl FakeTracker {
    pub fn with_frontier(mut self, parent: &str, tickets: &[(&str, &str)]) -> Self {
        let tickets = tickets
            .iter()
            .map(|(id, title)| ticket(id, title))
            .collect();
        self.frontiers.insert(parent.to_string(), tickets);
        self
    }

    pub fn with_ticket(self, id: &str, title: &str) -> Self {
        self.with_assigned_ticket(id, title, &[])
    }

    pub fn with_assigned_ticket(mut self, id: &str, title: &str, assignees: &[&str]) -> Self {
        self.tickets.insert(
            id.to_string(),
            TicketDetails {
                ticket: ticket(id, title),
                open: true,
                claimed_by: assignees.iter().map(|a| a.to_string()).collect(),
            },
        );
        self
    }

    pub fn with_check(mut self, check: Check) -> Self {
        self.checks.push(check);
        self
    }

    /// Every on-merge action applied so far, as (ticket ID, action), in order.
    pub fn on_merged(&self) -> Vec<(String, OnMerge)> {
        self.on_merged.borrow().clone()
    }

    /// The ticket IDs claimed so far, in order.
    pub fn claimed(&self) -> Vec<String> {
        self.claimed.borrow().clone()
    }
}

pub fn ticket(id: &str, title: &str) -> Ticket {
    Ticket {
        id: id.to_string(),
        title: title.to_string(),
        url: format!("https://tracker.test/{id}"),
    }
}

impl IssueTracker for FakeTracker {
    fn frontier(&self, parent: &str) -> Result<Vec<Ticket>, Error> {
        self.frontiers
            .get(parent)
            .cloned()
            .ok_or_else(|| Error::new(format!("no ticket {parent}")))
    }

    fn ticket(&self, id: &str) -> Result<TicketDetails, Error> {
        self.tickets
            .get(id)
            .cloned()
            .ok_or_else(|| Error::new(format!("no ticket {id}")))
    }

    fn claim(&self, id: &str) -> Result<(), Error> {
        self.claimed.borrow_mut().push(id.to_string());
        Ok(())
    }

    /// Marks the body, so tests can see the tracker had its say.
    fn pr_body(&self, id: &str, body: &str) -> Result<String, Error> {
        Ok(format!("{body}[closes {id}]\n"))
    }

    fn on_merge(&self, id: &str, action: &OnMerge) -> Result<(), Error> {
        self.on_merged
            .borrow_mut()
            .push((id.to_string(), action.clone()));
        Ok(())
    }

    fn checks(&self) -> Vec<Check> {
        self.checks.clone()
    }
}

/// A code host holding pull requests in memory, which records pushes and the PRs it opens.
#[derive(Default)]
pub struct FakeCodeHost {
    prs: RefCell<Vec<(String, PullRequest)>>,
    pushed: RefCell<Vec<(PathBuf, String)>>,
    opened: RefCell<Vec<NewPullRequest>>,
}

impl FakeCodeHost {
    /// Adds PR `number` from branch `head`, in `state`.
    pub fn with_pr(self, head: &str, number: u64, state: PrState) -> Self {
        self.prs
            .borrow_mut()
            .push((head.to_string(), pr(number, state)));
        self
    }

    /// Adds merged PR `number` from branch `head`, whose last commit was `sha`.
    pub fn with_merged_pr_at(self, head: &str, number: u64, sha: &str) -> Self {
        let mut merged = pr(number, PrState::Merged);
        merged.head_sha = Some(sha.to_string());
        self.prs.borrow_mut().push((head.to_string(), merged));
        self
    }

    /// Every push, as (repo, branch).
    pub fn pushed(&self) -> Vec<(PathBuf, String)> {
        self.pushed.borrow().clone()
    }

    pub fn opened(&self) -> Vec<NewPullRequest> {
        self.opened.borrow().clone()
    }
}

pub fn pr(number: u64, state: PrState) -> PullRequest {
    PullRequest {
        number,
        url: format!("https://code.test/pull/{number}"),
        state,
        head_sha: None,
    }
}

impl CodeHost for FakeCodeHost {
    fn push(&self, repo: &Path, branch: &str) -> Result<(), Error> {
        self.pushed
            .borrow_mut()
            .push((repo.to_path_buf(), branch.to_string()));
        Ok(())
    }

    fn find_pr(&self, head: &str) -> Result<Option<PullRequest>, Error> {
        Ok(self
            .prs
            .borrow()
            .iter()
            .find(|(h, _)| h == head)
            .map(|(_, pr)| pr.clone()))
    }

    fn open_pr(&self, new: &NewPullRequest) -> Result<PullRequest, Error> {
        self.opened.borrow_mut().push(new.clone());
        let opened = pr(100 + self.prs.borrow().len() as u64, PrState::Open);
        self.prs
            .borrow_mut()
            .push((new.head.clone(), opened.clone()));
        Ok(opened)
    }
}

/// A lane host holding its checkouts in memory. Lanes it creates show up in its list.
pub struct FakeHost {
    inside: bool,
    workspace: Option<String>,
    checkouts: RefCell<Vec<HostCheckout>>,
    created: RefCell<Vec<(PathBuf, NewLane)>>,
    started: RefCell<Vec<(String, String)>>,
    listed_from: RefCell<Vec<PathBuf>>,
    removed: RefCell<Vec<Removal>>,
    start_status: StatusHint,
    failing_branches: Vec<String>,
    agents_fail: bool,
    agent_output: HashMap<String, String>,
    checks: Vec<Check>,
    leaves_records: bool,
}

impl Default for FakeHost {
    fn default() -> Self {
        FakeHost {
            inside: true,
            workspace: None,
            checkouts: RefCell::new(Vec::new()),
            created: RefCell::new(Vec::new()),
            started: RefCell::new(Vec::new()),
            listed_from: RefCell::new(Vec::new()),
            removed: RefCell::new(Vec::new()),
            start_status: StatusHint::Working,
            failing_branches: Vec::new(),
            agents_fail: false,
            agent_output: HashMap::new(),
            checks: Vec::new(),
            leaves_records: false,
        }
    }
}

impl FakeHost {
    pub fn outside() -> Self {
        FakeHost {
            inside: false,
            ..Default::default()
        }
    }

    /// Adds a linked checkout on `branch` with an agent in the given state.
    pub fn with_checkout(self, branch: &str, status: StatusHint) -> Self {
        let n = self.checkouts.borrow().len();
        self.checkouts.borrow_mut().push(HostCheckout {
            path: PathBuf::from(format!("/lanes/{}", branch.replace('/', "-"))),
            branch: Some(branch.to_string()),
            is_main: false,
            workspace: Some(format!("w{}", n + 10)),
            status,
        });
        self
    }

    /// Adds a linked checkout on `branch` at a real `path`, such as one from [`Repo::add_worktree`].
    pub fn with_checkout_at(self, path: &Path, branch: &str, status: StatusHint) -> Self {
        let n = self.checkouts.borrow().len();
        self.checkouts.borrow_mut().push(HostCheckout {
            path: path.to_path_buf(),
            branch: Some(branch.to_string()),
            is_main: false,
            workspace: Some(format!("w{}", n + 10)),
            status,
        });
        self
    }

    /// Adds a linked checkout on `branch` with no open workspace.
    pub fn with_closed_checkout(self, branch: &str) -> Self {
        self.checkouts.borrow_mut().push(HostCheckout {
            path: PathBuf::from(format!("/lanes/{}", branch.replace('/', "-"))),
            branch: Some(branch.to_string()),
            is_main: false,
            workspace: None,
            status: StatusHint::Unknown,
        });
        self
    }

    /// Adds the main checkout at a real `path`.
    pub fn with_main_checkout_at(self, path: &Path, branch: &str) -> Self {
        self.checkouts.borrow_mut().push(HostCheckout {
            path: path.to_path_buf(),
            branch: Some(branch.to_string()),
            is_main: true,
            workspace: Some("w1".into()),
            status: StatusHint::Unknown,
        });
        self
    }

    pub fn with_main_checkout(self, branch: &str) -> Self {
        self.checkouts.borrow_mut().push(HostCheckout {
            path: PathBuf::from("/repo"),
            branch: Some(branch.to_string()),
            is_main: true,
            workspace: Some("w1".into()),
            status: StatusHint::Unknown,
        });
        self
    }

    /// Makes `bli` run in workspace `ws`, as from a pane of that workspace.
    pub fn running_in(mut self, ws: &str) -> Self {
        self.workspace = Some(ws.to_string());
        self
    }

    pub fn starting_agents_as(mut self, status: StatusHint) -> Self {
        self.start_status = status;
        self
    }

    /// Makes creating a lane on `branch` fail.
    pub fn failing_to_create(mut self, branch: &str) -> Self {
        self.failing_branches.push(branch.to_string());
        self
    }

    /// Makes removing a lane delete its directory but leave git's worktree record, as a careless host might.
    pub fn leaving_worktree_records(mut self) -> Self {
        self.leaves_records = true;
        self
    }

    /// Makes every agent fail to start.
    pub fn failing_to_start_agents(mut self) -> Self {
        self.agents_fail = true;
        self
    }

    /// Gives ticket `ticket_id`'s agent `output` to read.
    pub fn with_agent_output(mut self, ticket_id: &str, output: &str) -> Self {
        self.agent_output
            .insert(ticket_id.to_string(), output.to_string());
        self
    }

    pub fn with_check(mut self, check: Check) -> Self {
        self.checks.push(check);
        self
    }

    /// Every lane creation asked for, with the repo it was asked from.
    pub fn created(&self) -> Vec<(PathBuf, NewLane)> {
        self.created.borrow().clone()
    }

    /// Every agent started, as (ticket ID, prompt).
    pub fn started(&self) -> Vec<(String, String)> {
        self.started.borrow().clone()
    }

    pub fn listed_from(&self) -> Vec<PathBuf> {
        self.listed_from.borrow().clone()
    }

    /// Every lane removed, in order.
    pub fn removed(&self) -> Vec<Removal> {
        self.removed.borrow().clone()
    }
}

/// One `remove_lane` call the fake host received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub repo: PathBuf,
    pub path: PathBuf,
    pub workspace: Option<String>,
    pub force: bool,
}

impl LaneHost for FakeHost {
    fn name(&self) -> &str {
        "fakehost"
    }

    fn is_inside(&self) -> bool {
        self.inside
    }

    fn current_workspace(&self) -> Option<&str> {
        self.workspace.as_deref()
    }

    fn create_lane(&self, repo: &Path, lane: &NewLane) -> Result<CreatedLane, Error> {
        if self.failing_branches.contains(&lane.branch) {
            return Err(Error::new(format!("could not create {}", lane.branch)));
        }
        self.created
            .borrow_mut()
            .push((repo.to_path_buf(), lane.clone()));
        let path = PathBuf::from(format!("/lanes/{}", lane.branch.replace('/', "-")));
        let workspace = format!("w{}", self.checkouts.borrow().len() + 10);
        self.checkouts.borrow_mut().push(HostCheckout {
            path: path.clone(),
            branch: Some(lane.branch.clone()),
            is_main: false,
            workspace: Some(workspace.clone()),
            status: StatusHint::Unknown,
        });
        Ok(CreatedLane {
            path,
            workspace,
            handle: format!("pane-{}", lane.branch),
        })
    }

    fn start_agent(
        &self,
        _lane: &CreatedLane,
        ticket_id: &str,
        prompt: &str,
    ) -> Result<StatusHint, Error> {
        if self.agents_fail {
            return Err(Error::new("agent_not_ready"));
        }
        self.started
            .borrow_mut()
            .push((ticket_id.to_string(), prompt.to_string()));
        Ok(self.start_status)
    }

    /// The last `lines` lines of the agent's output, as a host would read them.
    fn read_agent(
        &self,
        ticket_id: &str,
        _workspace: Option<&str>,
        lines: u32,
    ) -> Result<String, Error> {
        let output = self
            .agent_output
            .get(ticket_id)
            .ok_or_else(|| Error::new(format!("no agent t{ticket_id}")))?;
        let all: Vec<_> = output.lines().collect();
        let start = all.len().saturating_sub(lines as usize);
        Ok(all[start..].iter().map(|l| format!("{l}\n")).collect())
    }

    fn list(&self, repo: &Path) -> Result<Vec<HostCheckout>, Error> {
        self.listed_from.borrow_mut().push(repo.to_path_buf());
        Ok(self.checkouts.borrow().clone())
    }

    fn remove_lane(
        &self,
        repo: &Path,
        path: &Path,
        workspace: Option<&str>,
        force: bool,
    ) -> Result<(), Error> {
        self.removed.borrow_mut().push(Removal {
            repo: repo.to_path_buf(),
            path: path.to_path_buf(),
            workspace: workspace.map(str::to_string),
            force,
        });
        self.checkouts.borrow_mut().retain(|c| c.path != path);
        // A real checkout goes the way a real host removes it, refusing changes without force.
        if path.exists() && self.leaves_records {
            std::fs::remove_dir_all(path).unwrap();
        } else if path.exists() {
            let path = path.to_str().unwrap();
            let mut args = vec!["worktree", "remove", path];
            if force {
                args.push("--force");
            }
            let out = Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(&args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            if !out.status.success() {
                return Err(Error::new(String::from_utf8_lossy(&out.stderr).trim()));
            }
        }
        Ok(())
    }

    fn checks(&self) -> Vec<Check> {
        self.checks.clone()
    }
}

/// Answers `git --version` as an installed git would, or as if git were missing.
pub struct FakeGit {
    installed: bool,
}

impl Default for FakeGit {
    fn default() -> Self {
        FakeGit { installed: true }
    }
}

impl FakeGit {
    pub fn missing() -> Self {
        FakeGit { installed: false }
    }
}

impl ProcessRunner for FakeGit {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<Output> {
        assert_eq!((program, args), ("git", &["--version"][..]));
        if !self.installed {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no git"));
        }
        Ok(Output {
            success: true,
            stdout: "git version 2.51.0\n".into(),
            stderr: String::new(),
        })
    }
}

/// The providers a test hands to `bli::run`.
#[derive(Default)]
pub struct Fakes {
    pub tracker: FakeTracker,
    pub host: FakeHost,
    pub code: FakeCodeHost,
    pub git: FakeGit,
    /// Makes building the issue tracker fail, as for a tracker `bli` does not support yet.
    pub tracker_unsupported: bool,
    /// Answers `bli init`'s questions; without one, `bli` runs as if not in a terminal.
    pub prompter: Option<ScriptedPrompter>,
}

impl Providers for Fakes {
    fn issue_tracker(&self, _config: &Config) -> Result<Box<dyn IssueTracker + '_>, Error> {
        if self.tracker_unsupported {
            return Err(Error::new("unsupported issue tracker"));
        }
        Ok(Box::new(&self.tracker))
    }

    fn lane_host(&self, _config: &Config) -> Result<Box<dyn LaneHost + '_>, Error> {
        Ok(Box::new(&self.host))
    }

    fn code_host(&self, _config: &Config) -> Result<Box<dyn CodeHost + '_>, Error> {
        Ok(Box::new(&self.code))
    }

    fn git_runner(&self) -> &dyn ProcessRunner {
        &self.git
    }

    fn prompter(&self) -> Result<Box<dyn Prompter + '_>, Error> {
        match &self.prompter {
            Some(prompter) => Ok(Box::new(prompter)),
            None => Err(bli::init::not_a_terminal()),
        }
    }
}

/// One scripted reply to a `bli init` question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Presses enter: takes the default.
    Enter,
    /// Picks the choice with this value.
    Pick(&'static str),
    /// Types this text and presses enter.
    Type(&'static str),
    Yes,
    No,
    /// Presses Esc or Ctrl-C.
    Cancel,
}

/// A question and the choices it offered, as (value, hint).
type Offered = (String, Vec<(String, String)>);

/// Plays back answers in order, and like the real prompts, re-asks a question whose
/// answer the validator rejects, recording the rejection.
pub struct ScriptedPrompter {
    answers: RefCell<std::collections::VecDeque<Answer>>,
    asked: RefCell<Vec<String>>,
    rejections: RefCell<Vec<String>>,
    printed: RefCell<Vec<String>>,
    offered: RefCell<Vec<Offered>>,
}

impl ScriptedPrompter {
    pub fn new(answers: &[Answer]) -> Self {
        ScriptedPrompter {
            answers: RefCell::new(answers.iter().cloned().collect()),
            asked: RefCell::new(Vec::new()),
            rejections: RefCell::new(Vec::new()),
            printed: RefCell::new(Vec::new()),
            offered: RefCell::new(Vec::new()),
        }
    }

    /// Every question asked, in order, re-asks included.
    pub fn asked(&self) -> Vec<String> {
        self.asked.borrow().clone()
    }

    /// Every validation error shown, in order.
    pub fn rejections(&self) -> Vec<String> {
        self.rejections.borrow().clone()
    }

    /// Everything shown besides questions: the intro, notes and the outro.
    pub fn printed(&self) -> Vec<String> {
        self.printed.borrow().clone()
    }

    /// The choices a question offered, as (value, hint), the last time it was asked.
    pub fn offered(&self, prompt: &str) -> Vec<(String, String)> {
        let offered = self.offered.borrow();
        let found = offered.iter().rev().find(|(p, _)| p == prompt);
        found
            .unwrap_or_else(|| panic!("{prompt:?} was not asked"))
            .1
            .clone()
    }

    /// The answers left unused.
    pub fn unused(&self) -> Vec<Answer> {
        self.answers.borrow().iter().cloned().collect()
    }

    fn next(&self, prompt: &str) -> Answer {
        self.asked.borrow_mut().push(prompt.to_string());
        self.answers
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("no answer left for {prompt:?}"))
    }
}

impl Prompter for ScriptedPrompter {
    fn intro(&self, title: &str) {
        self.printed.borrow_mut().push(title.to_string());
    }

    fn select(
        &self,
        prompt: &str,
        choices: &[Choice],
        default: &str,
    ) -> Result<&'static str, Cancelled> {
        let offered = choices
            .iter()
            .map(|c| (c.value.to_string(), c.hint.to_string()))
            .collect();
        self.offered
            .borrow_mut()
            .push((prompt.to_string(), offered));
        let wanted = match self.next(prompt) {
            Answer::Enter => default.to_string(),
            Answer::Pick(value) => value.to_string(),
            Answer::Cancel => return Err(Cancelled),
            other => panic!("{prompt:?} is a choice, not {other:?}"),
        };
        let found = choices.iter().find(|c| c.value == wanted);
        Ok(found
            .unwrap_or_else(|| panic!("{prompt:?} has no choice {wanted:?}"))
            .value)
    }

    fn input(&self, prompt: &str, default: &str, validate: Validator) -> Result<String, Cancelled> {
        loop {
            let value = match self.next(prompt) {
                Answer::Enter => default.to_string(),
                Answer::Type(text) => text.to_string(),
                Answer::Cancel => return Err(Cancelled),
                other => panic!("{prompt:?} takes text, not {other:?}"),
            };
            match validate(&value) {
                Ok(()) => return Ok(value),
                Err(why) => self.rejections.borrow_mut().push(why),
            }
        }
    }

    fn confirm(&self, prompt: &str, default: bool) -> Result<bool, Cancelled> {
        match self.next(prompt) {
            Answer::Enter => Ok(default),
            Answer::Yes => Ok(true),
            Answer::No => Ok(false),
            Answer::Cancel => Err(Cancelled),
            other => panic!("{prompt:?} is yes or no, not {other:?}"),
        }
    }

    fn note(&self, title: &str, body: &str) {
        self.printed.borrow_mut().push(format!("{title}\n{body}"));
    }

    fn outro(&self, message: &str) {
        self.printed.borrow_mut().push(message.to_string());
    }

    fn outro_cancel(&self, message: &str) {
        self.printed.borrow_mut().push(message.to_string());
    }
}

pub fn bli(args: &[&str], cwd: &Path, fakes: &Fakes) -> Outcome {
    bli::run(args, cwd, fakes)
}

pub fn json(out: &Outcome) -> serde_json::Value {
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    serde_json::from_str(&out.stdout).unwrap()
}

pub const GITHUB: &str = "[tracker]\nkind = \"github\"\n";

/// Runs git in `dir` and returns its trimmed stdout, panicking on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A real checkout cloned from a local bare `origin`, with one commit on `main` pushed.
pub struct Repo {
    _dir: TempDir,
    pub root: PathBuf,
    pub remote: PathBuf,
}

impl Repo {
    pub fn new(bli_toml: &str) -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let remote = base.join("origin.git");
        let root = base.join("repo");
        git(&base, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
        git(&base, &["clone", "-q", "origin.git", "repo"]);
        configure_identity(&root);
        std::fs::write(root.join("bli.toml"), bli_toml).unwrap();
        git(&root, &["checkout", "-q", "-b", "main"]);
        git(&root, &["add", "bli.toml"]);
        git(&root, &["commit", "-q", "-m", "init"]);
        git(&root, &["push", "-q", "origin", "main"]);
        Repo {
            _dir: dir,
            root,
            remote,
        }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Pushes a new commit to `branch` on origin from a separate clone, leaving this checkout behind.
    pub fn push_elsewhere(&self, branch: &str) -> String {
        let other = self.root.parent().unwrap().join("elsewhere");
        if !other.exists() {
            git(
                self.root.parent().unwrap(),
                &["clone", "-q", "origin.git", "elsewhere"],
            );
            configure_identity(&other);
        }
        git(&other, &["fetch", "-q", "origin"]);
        let start = if git(&other, &["ls-remote", "origin", branch]).is_empty() {
            "origin/main".to_string()
        } else {
            format!("origin/{branch}")
        };
        git(&other, &["checkout", "-q", "-B", branch, &start]);
        git(
            &other,
            &["commit", "-q", "--allow-empty", "-m", "elsewhere"],
        );
        git(&other, &["push", "-q", "origin", branch]);
        git(&other, &["rev-parse", "HEAD"])
    }

    /// Adds a linked checkout of a new `branch` with one commit beyond `main`.
    pub fn add_lane_with_commit(&self, branch: &str) -> PathBuf {
        let path = self.add_worktree(branch);
        git(&path, &["commit", "-q", "--allow-empty", "-m", "work"]);
        path
    }

    /// Writes `text` to a file outside the checkout and returns its path.
    pub fn write_file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.root.parent().unwrap().join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    /// Adds a linked checkout of a new branch, as a lane host would.
    pub fn add_worktree(&self, branch: &str) -> PathBuf {
        let path = self.root.parent().unwrap().join(branch.replace('/', "-"));
        git(
            &self.root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                path.to_str().unwrap(),
            ],
        );
        path
    }
}

fn configure_identity(dir: &Path) {
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}
