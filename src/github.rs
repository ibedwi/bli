//! GitHub as the issue tracker and the code host, through the `gh` CLI.

use std::io;
use std::path::Path;

use serde::Deserialize;

use crate::check::Check;
use crate::code_host::{CodeHost, NewPullRequest, PrState, PullRequest};
use crate::error::Error;
use crate::git::REMOTE;
use crate::process::ProcessRunner;
use crate::tracker::{IssueTracker, Ticket, TicketDetails};

pub struct GitHub<R> {
    runner: R,
}

impl<R: ProcessRunner> GitHub<R> {
    pub fn new(runner: R) -> Self {
        GitHub { runner }
    }

    /// Runs `gh` with `args` and returns its stdout, or an error carrying its stderr.
    fn gh(&self, args: &[&str]) -> Result<String, Error> {
        self.run("gh", args).map_err(|e| match e {
            Failure::NotRun(e) if e.kind() == io::ErrorKind::NotFound => Error::new(
                "could not run `gh`: it is not installed",
            )
            .with_hint("install the GitHub CLI (https://cli.github.com) and run `gh auth login`"),
            e => e.into_error("gh", args),
        })
    }

    /// Runs `program` with `args` and returns its stdout.
    fn run(&self, program: &str, args: &[&str]) -> Result<String, Failure> {
        let out = self.runner.run(program, args).map_err(Failure::NotRun)?;
        if !out.success {
            return Err(Failure::Failed(out.stderr));
        }
        Ok(out.stdout)
    }
}

/// Why running a tool failed.
enum Failure {
    NotRun(io::Error),
    Failed(String),
}

impl Failure {
    fn into_error(self, program: &str, args: &[&str]) -> Error {
        match self {
            Failure::NotRun(e) => Error::new(format!("could not run `{program}`: {e}")),
            Failure::Failed(stderr) => Error::new(format!(
                "`{program} {}` failed: {}",
                args.join(" "),
                stderr.trim()
            )),
        }
    }
}

impl<R: ProcessRunner> IssueTracker for GitHub<R> {
    fn frontier(&self, parent: &str) -> Result<Vec<Ticket>, Error> {
        let number = issue_number(parent)?;
        let path = format!("repos/{{owner}}/{{repo}}/issues/{number}/sub_issues");
        let stdout = self.gh(&["api", &path, "--paginate"])?;
        let issues: Vec<Issue> = serde_json::from_str(&stdout)
            .map_err(|e| Error::new(format!("could not read `gh` sub-issue output: {e}")))?;
        Ok(issues
            .into_iter()
            .filter(|i| i.state == "open" && !i.is_blocked() && !i.is_claimed())
            .map(|i| Ticket {
                id: i.number.to_string(),
                title: i.title,
                url: i.html_url,
            })
            .collect())
    }

    fn ticket(&self, id: &str) -> Result<TicketDetails, Error> {
        let number = issue_number(id)?.to_string();
        let stdout = self.gh(&[
            "issue",
            "view",
            &number,
            "--json",
            "number,title,url,state,assignees",
        ])?;
        let view: IssueView = serde_json::from_str(&stdout)
            .map_err(|e| Error::new(format!("could not read `gh issue view` output: {e}")))?;
        Ok(TicketDetails {
            ticket: Ticket {
                id: view.number.to_string(),
                title: view.title,
                url: view.url,
            },
            open: view.state == "OPEN",
            claimed_by: view.assignees.into_iter().map(|a| a.login).collect(),
        })
    }

    /// On GitHub, a ticket is claimed by assigning it.
    fn claim(&self, id: &str) -> Result<(), Error> {
        let number = issue_number(id)?.to_string();
        self.gh(&["issue", "edit", &number, "--add-assignee", "@me"])
            .map(drop)
    }

    /// GitHub closes an issue when a merged PR's body says `Closes #<n>`.
    fn pr_body(&self, id: &str, body: &str) -> Result<String, Error> {
        let number = issue_number(id)?;
        if closes(body, number) {
            return Ok(body.to_string());
        }
        let body = body.trim_end();
        Ok(format!("{body}\n\nCloses #{number}\n"))
    }

    fn checks(&self) -> Vec<Check> {
        let version = self
            .gh(&["--version"])
            .map(|out| out.lines().next().unwrap_or_default().to_string());
        let auth = self
            .gh(&["auth", "status"])
            .map(|out| logged_in_account(&out))
            .map_err(|e| match e.hint() {
                Some(_) => e,
                None => e.with_hint("run `gh auth login`"),
            });
        vec![
            Check::from_result("gh", version),
            Check::from_result("gh auth", auth),
        ]
    }
}

impl<R: ProcessRunner> CodeHost for GitHub<R> {
    fn push(&self, repo: &Path, branch: &str) -> Result<(), Error> {
        let repo = repo.to_string_lossy();
        let args = ["-C", &repo, "push", "--quiet", "-u", REMOTE, branch];
        self.run("git", &args)
            .map(drop)
            .map_err(|e| e.into_error("git", &args))
    }

    fn find_pr(&self, head: &str) -> Result<Option<PullRequest>, Error> {
        let stdout = self.gh(&[
            "pr",
            "list",
            "--head",
            head,
            "--state",
            "all",
            "--json",
            "number,url,state,createdAt",
            "--limit",
            "100",
        ])?;
        let mut prs: Vec<Pr> = serde_json::from_str(&stdout)
            .map_err(|e| Error::new(format!("could not read `gh pr list` output: {e}")))?;
        // An open PR is the one still in play; otherwise the newest, whatever order
        // `gh` lists them in. RFC 3339 UTC timestamps sort as text.
        let pick = match prs.iter().position(|p| p.state == "OPEN") {
            Some(i) => Some(prs.swap_remove(i)),
            None => prs
                .into_iter()
                .max_by(|a, b| a.created_at.cmp(&b.created_at)),
        };
        Ok(pick.map(Pr::into_pull_request))
    }

    fn open_pr(&self, pr: &NewPullRequest) -> Result<PullRequest, Error> {
        let stdout = self.gh(&[
            "pr", "create", "--head", &pr.head, "--base", &pr.base, "--title", &pr.title, "--body",
            &pr.body,
        ])?;
        // `gh pr create` prints the new PR's URL, ending in its number.
        let url = stdout.trim().lines().last().unwrap_or_default().to_string();
        let number = url
            .rsplit('/')
            .next()
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| {
                Error::new(format!(
                    "could not read the new PR's URL from `gh pr create`: {url}"
                ))
            })?;
        Ok(PullRequest {
            number,
            url,
            state: PrState::Open,
        })
    }
}

/// Whether `body` holds a GitHub closing keyword for issue `number`, e.g. `Fixes #12`.
fn closes(body: &str, number: u64) -> bool {
    const KEYWORDS: [&str; 9] = [
        "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
    ];
    let words: Vec<_> = body.split_whitespace().collect();
    words.windows(2).any(|pair| {
        let keyword = pair[0].trim_end_matches(':').to_ascii_lowercase();
        let reference = pair[1].trim_end_matches(|c: char| c.is_ascii_punctuation());
        KEYWORDS.contains(&keyword.as_str()) && reference == format!("#{number}")
    })
}

/// The "Logged in to ..." line of `gh auth status`.
fn logged_in_account(out: &str) -> String {
    out.lines()
        .map(|l| l.trim().trim_start_matches('✓').trim())
        .find(|l| l.starts_with("Logged in"))
        .unwrap_or("logged in")
        .to_string()
}

/// A GitHub ticket ID is an issue number, optionally written `#12`.
fn issue_number(id: &str) -> Result<u64, Error> {
    id.strip_prefix('#')
        .unwrap_or(id)
        .parse()
        .map_err(|_| Error::new(format!("`{id}` is not a GitHub issue number")))
}

#[derive(Deserialize)]
struct Issue {
    number: u64,
    title: String,
    html_url: String,
    state: String,
    assignees: Vec<serde_json::Value>,
    issue_dependencies_summary: DependenciesSummary,
}

impl Issue {
    /// `blocked_by` counts only open blockers.
    fn is_blocked(&self) -> bool {
        self.issue_dependencies_summary.blocked_by > 0
    }

    /// On GitHub, a ticket is claimed by assigning it.
    fn is_claimed(&self) -> bool {
        !self.assignees.is_empty()
    }
}

#[derive(Deserialize)]
struct DependenciesSummary {
    blocked_by: u64,
}

/// `gh issue view --json` output, which spells fields differently from the REST API.
#[derive(Deserialize)]
struct IssueView {
    number: u64,
    title: String,
    url: String,
    state: String,
    assignees: Vec<Assignee>,
}

#[derive(Deserialize)]
struct Assignee {
    login: String,
}

/// `gh pr list --json` output.
#[derive(Deserialize)]
struct Pr {
    number: u64,
    #[serde(rename = "createdAt")]
    created_at: String,
    url: String,
    state: String,
}

impl Pr {
    fn into_pull_request(self) -> PullRequest {
        let state = match self.state.as_str() {
            "MERGED" => PrState::Merged,
            "CLOSED" => PrState::Closed,
            _ => PrState::Open,
        };
        PullRequest {
            number: self.number,
            url: self.url,
            state,
        }
    }
}
