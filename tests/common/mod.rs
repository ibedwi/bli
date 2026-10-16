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
use bli::process::{Output, ProcessRunner};
use bli::tracker::{IssueTracker, Ticket, TicketDetails};
use bli::{Outcome, Providers};
use tempfile::TempDir;

/// An issue tracker whose tickets and frontiers are fixed up front, and which records claims.
#[derive(Default)]
pub struct FakeTracker {
    frontiers: HashMap<String, Vec<Ticket>>,
    tickets: HashMap<String, TicketDetails>,
    claimed: RefCell<Vec<String>>,
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
    checkouts: RefCell<Vec<HostCheckout>>,
    created: RefCell<Vec<(PathBuf, NewLane)>>,
    started: RefCell<Vec<(String, String)>>,
    listed_from: RefCell<Vec<PathBuf>>,
    start_status: StatusHint,
    failing_branches: Vec<String>,
    agents_fail: bool,
    agent_output: HashMap<String, String>,
    checks: Vec<Check>,
}

impl Default for FakeHost {
    fn default() -> Self {
        FakeHost {
            inside: true,
            checkouts: RefCell::new(Vec::new()),
            created: RefCell::new(Vec::new()),
            started: RefCell::new(Vec::new()),
            listed_from: RefCell::new(Vec::new()),
            start_status: StatusHint::Working,
            failing_branches: Vec::new(),
            agents_fail: false,
            agent_output: HashMap::new(),
            checks: Vec::new(),
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

    pub fn starting_agents_as(mut self, status: StatusHint) -> Self {
        self.start_status = status;
        self
    }

    /// Makes creating a lane on `branch` fail.
    pub fn failing_to_create(mut self, branch: &str) -> Self {
        self.failing_branches.push(branch.to_string());
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
}

impl LaneHost for FakeHost {
    fn name(&self) -> &str {
        "fakehost"
    }

    fn is_inside(&self) -> bool {
        self.inside
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
    fn read_agent(&self, ticket_id: &str, lines: u32) -> Result<String, Error> {
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
