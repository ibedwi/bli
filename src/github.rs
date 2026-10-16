//! GitHub Issues as the issue tracker, through the `gh` CLI.

use std::io;

use serde::Deserialize;

use crate::error::Error;
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
        let out = self.runner.run("gh", args).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::new("could not run `gh`: it is not installed")
                .with_hint(
                    "install the GitHub CLI (https://cli.github.com) and run `gh auth login`",
                ),
            _ => Error::new(format!("could not run `gh`: {e}")),
        })?;
        if !out.success {
            return Err(Error::new(format!(
                "`gh {}` failed: {}",
                args.join(" "),
                out.stderr.trim()
            )));
        }
        Ok(out.stdout)
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
