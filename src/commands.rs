//! The workflow. Depends only on the provider interfaces and git, never on an adapter.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;

use crate::branch;
use crate::code_host::{CodeHost, NewPullRequest, PullRequest};
use crate::config::Config;
use crate::error::Error;
use crate::git::{Git, REMOTE};
use crate::host::{LaneHost, NewLane, StatusHint};
use crate::tracker::IssueTracker;

pub fn frontier(
    tracker: &dyn IssueTracker,
    host: &dyn LaneHost,
    git: &Git,
    config: &Config,
    parent: Option<&str>,
    json: bool,
) -> Result<String, Error> {
    let parent = parent.or(config.tracker.parent.as_deref()).ok_or_else(|| {
        Error::new("no parent ticket given")
            .with_hint("pass --parent <id>, or set `parent` under [tracker] in bli.toml")
    })?;
    let lanes = lanes(host, git, config)?;
    let tickets: Vec<_> = tracker
        .frontier(parent)?
        .into_iter()
        .filter(|t| !lanes.iter().any(|l| l.ticket == t.id))
        .collect();
    if json {
        return Ok(format!(
            "{}\n",
            json!({ "parent": parent, "tickets": tickets })
        ));
    }
    if tickets.is_empty() {
        return Ok(format!("No tickets on the frontier of {parent}.\n"));
    }
    let mut out = String::new();
    for t in &tickets {
        out.push_str(&format!("{}  {}\n    {}\n", t.id, t.title, t.url));
    }
    Ok(out)
}

/// A command's output, plus the failure to exit with when part of the work failed.
pub struct Report {
    pub stdout: String,
    pub failure: Option<Error>,
}

impl From<String> for Report {
    fn from(stdout: String) -> Self {
        Report {
            stdout,
            failure: None,
        }
    }
}

pub struct StartRequest<'a> {
    pub tickets: &'a [String],
    pub base: Option<&'a str>,
    pub slug: Option<&'a str>,
}

pub fn lane_start(
    tracker: &dyn IssueTracker,
    host: &dyn LaneHost,
    git: &Git,
    config: &Config,
    request: &StartRequest,
    json: bool,
) -> Result<Report, Error> {
    if let Some(slug) = request.slug {
        if request.tickets.len() > 1 {
            return Err(
                Error::new("--slug names one lane, but more than one ticket was given")
                    .with_hint("start the ticket that needs a custom slug on its own"),
            );
        }
        if !branch::is_slug(slug) {
            return Err(Error::new(format!("`{slug}` is not a slug"))
                .with_hint("a slug is lowercase kebab-case, e.g. `fan-out-on-herdr`"));
        }
    }
    if !host.is_inside() {
        let name = host.name();
        return Err(
            Error::new(format!("not inside {name}, the configured lane host")).with_hint(format!(
                "run this from a {name} pane, or change `host` under [lane] in bli.toml"
            )),
        );
    }
    let base = request.base.unwrap_or(&config.lane.base);
    git.fetch()?;
    if !git.has_remote_branch(base)? {
        return Err(
            Error::new(format!("the base branch `{base}` is not on {REMOTE}")).with_hint(format!(
                "push it with `git push {REMOTE} {base}`, or pass --base <branch>"
            )),
        );
    }
    let mut lanes = lanes(host, git, config)?;
    let mut reports = Vec::new();
    // One at a time, so each lane's outcome is its own and one failure doesn't stop the rest.
    for id in request.tickets {
        let mut report = StartReport::new(id);
        if let Err(e) = start_one(
            tracker,
            host,
            git,
            config,
            &lanes,
            base,
            request.slug,
            &mut report,
        ) {
            report.outcome = StartOutcome::Failed;
            report.error = Some(e.to_string());
        }
        // A lane that exists, even with a failed agent, still blocks a second start.
        if let (Some(branch), Some(_)) = (&report.branch, &report.path) {
            lanes.push(Lane {
                ticket: report.ticket.clone(),
                branch: branch.clone(),
                base: base.to_string(),
                path: report.path.clone().unwrap_or_default(),
                workspace: report.workspace.clone(),
                status: report.status.unwrap_or(StatusHint::Unknown),
            });
        }
        reports.push(report);
    }
    let failed = reports
        .iter()
        .filter(|r| r.outcome == StartOutcome::Failed)
        .count();
    let failure = (failed > 0).then(|| {
        Error::new(format!(
            "{failed} of {} lanes failed to start",
            reports.len()
        ))
    });
    if json {
        let stdout = format!("{}\n", json!({ "base": base, "lanes": reports }));
        return Ok(Report { stdout, failure });
    }
    let mut out = String::new();
    for r in &reports {
        let detail = match r.outcome {
            StartOutcome::Started => format!(
                "{}  ({})",
                r.branch.as_deref().unwrap_or_default(),
                r.status.unwrap_or(StatusHint::Unknown).as_str()
            ),
            StartOutcome::Skipped => r.reason.clone().unwrap_or_default(),
            StartOutcome::Failed => format!("error: {}", r.error.as_deref().unwrap_or_default()),
        };
        out.push_str(&format!(
            "{}  {}  {}\n",
            r.ticket,
            r.outcome.as_str(),
            detail
        ));
    }
    Ok(Report {
        stdout: out,
        failure,
    })
}

/// Starts the lane for `report`'s ticket, or says why it was skipped, filling in
/// `report` as it goes so a failure part-way still says how far it got.
#[allow(clippy::too_many_arguments)]
fn start_one(
    tracker: &dyn IssueTracker,
    host: &dyn LaneHost,
    git: &Git,
    config: &Config,
    lanes: &[Lane],
    base: &str,
    slug: Option<&str>,
    report: &mut StartReport,
) -> Result<(), Error> {
    let details = tracker.ticket(&report.ticket)?;
    let ticket = details.ticket;
    report.ticket = ticket.id.clone();
    report.title = Some(ticket.title.clone());
    report.url = Some(ticket.url.clone());
    if let Some(lane) = lanes.iter().find(|l| l.ticket == ticket.id) {
        report.branch = Some(lane.branch.clone());
        report.skip(format!("already has a lane on {}", lane.branch));
        return Ok(());
    }
    if !details.claimed_by.is_empty() {
        report.skip(format!(
            "already claimed by {}",
            details.claimed_by.join(", ")
        ));
        return Ok(());
    }
    if !details.open {
        report.skip("is closed".into());
        return Ok(());
    }
    let slug = match slug {
        Some(slug) => slug.to_string(),
        None => branch::slugify(&ticket.title),
    };
    if slug.is_empty() {
        return Err(Error::new("could not make a slug from the title")
            .with_hint("pass --slug <kebab-case-slug>"));
    }
    let branch = config.lane.branch.render(&ticket.id, &slug);
    report.branch = Some(branch.clone());
    report.base = Some(base.to_string());
    let lane = host.create_lane(
        git.main_checkout(),
        &NewLane {
            branch: branch.clone(),
            base_ref: format!("{REMOTE}/{base}"),
            label: format!("#{} {slug}", ticket.id),
        },
    )?;
    report.path = Some(lane.path.clone());
    report.workspace = Some(lane.workspace.clone());
    git.set_base(&branch, base)?;
    // Claim as soon as the lane exists, so a lane whose agent fails to start is still claimed.
    tracker.claim(&ticket.id)?;
    let prompt = config.lane.prompt.replace("{ticket_url}", &ticket.url);
    report.status = Some(host.start_agent(&lane, &ticket.id, &prompt)?);
    report.outcome = StartOutcome::Started;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum StartOutcome {
    Started,
    Skipped,
    Failed,
}

impl StartOutcome {
    fn as_str(self) -> &'static str {
        match self {
            StartOutcome::Started => "started",
            StartOutcome::Skipped => "skipped",
            StartOutcome::Failed => "failed",
        }
    }
}

/// What happened to one ticket during `lane start`.
#[derive(Debug, Serialize)]
struct StartReport {
    ticket: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    outcome: StartOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<StatusHint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl StartReport {
    fn new(ticket: &str) -> Self {
        StartReport {
            ticket: ticket.to_string(),
            title: None,
            url: None,
            outcome: StartOutcome::Failed,
            branch: None,
            base: None,
            path: None,
            workspace: None,
            status: None,
            reason: None,
            error: None,
        }
    }

    fn skip(&mut self, reason: String) {
        self.outcome = StartOutcome::Skipped;
        self.reason = Some(reason);
    }
}

pub fn lane_list(
    host: &dyn LaneHost,
    code: &dyn CodeHost,
    git: &Git,
    config: &Config,
    json: bool,
) -> Result<String, Error> {
    let mut listed = Vec::new();
    for lane in lanes(host, git, config)? {
        let pr = code.find_pr(&lane.branch)?;
        listed.push(ListedLane { lane, pr });
    }
    if json {
        return Ok(format!("{}\n", json!({ "lanes": listed })));
    }
    if listed.is_empty() {
        return Ok("No lanes.\n".into());
    }
    let mut out = String::new();
    for ListedLane { lane: l, pr } in &listed {
        let pr = match pr {
            Some(pr) => format!("PR #{} {}", pr.number, pr.state.as_str()),
            None => "no PR".into(),
        };
        out.push_str(&format!(
            "{}  {}  (base {}, {}, {pr})\n    {}\n",
            l.ticket,
            l.branch,
            l.base,
            l.status.as_str(),
            l.path.display()
        ));
    }
    Ok(out)
}

/// A lane with its pull request, if it has one.
#[derive(Debug, Serialize)]
struct ListedLane {
    #[serde(flatten)]
    lane: Lane,
    pr: Option<PullRequest>,
}

pub fn lane_read(
    host: &dyn LaneHost,
    git: &Git,
    config: &Config,
    ticket: &str,
    lines: u32,
    json: bool,
) -> Result<String, Error> {
    let lane = lane(host, git, config, ticket)?;
    let output = host.read_agent(&lane.ticket, lines)?;
    if json {
        return Ok(format!(
            "{}\n",
            json!({
                "ticket": lane.ticket,
                "branch": lane.branch,
                "status": lane.status,
                "output": output,
            })
        ));
    }
    Ok(output)
}

pub struct ShipRequest<'a> {
    pub ticket: &'a str,
    pub body_file: &'a Path,
    pub title: Option<&'a str>,
}

pub fn lane_ship(
    tracker: &dyn IssueTracker,
    host: &dyn LaneHost,
    code: &dyn CodeHost,
    git: &Git,
    config: &Config,
    request: &ShipRequest,
    json: bool,
) -> Result<String, Error> {
    let body = std::fs::read_to_string(request.body_file).map_err(|e| {
        Error::new(format!(
            "could not read the PR body from {}: {e}",
            request.body_file.display()
        ))
    })?;
    let ticket = tracker.ticket(request.ticket)?.ticket;
    let lane = lane(host, git, config, &ticket.id)?;
    // Checked before the commits, so a rerun after the PR merged still reports it.
    let (outcome, pr) = match code.find_pr(&lane.branch)? {
        Some(pr) => (ShipOutcome::Existing, pr),
        None => {
            git.fetch()?;
            if !git.has_remote_branch(&lane.base)? {
                return Err(Error::new(format!(
                    "the lane's base `{}` is not on {REMOTE}",
                    lane.base
                ))
                .with_hint(format!(
                    "push it with `git push {REMOTE} {}`, or record another with `git config branch.{}.bliBase <branch>`",
                    lane.base, lane.branch
                )));
            }
            if git.commits_ahead(&lane.branch, &lane.base)? == 0 {
                return Err(Error::new(format!(
                    "{} has no commits beyond {REMOTE}/{}",
                    lane.branch, lane.base
                ))
                .with_hint(format!(
                    "read the lane's output with `bli lane read {}`: its agent may not have committed its work",
                    lane.ticket
                )));
            }
            code.push(git.main_checkout(), &lane.branch)?;
            let pr = code.open_pr(&NewPullRequest {
                head: lane.branch.clone(),
                base: lane.base.clone(),
                title: request.title.unwrap_or(&ticket.title).to_string(),
                body: tracker.pr_body(&ticket.id, &body)?,
            })?;
            (ShipOutcome::Opened, pr)
        }
    };
    if json {
        return Ok(format!(
            "{}\n",
            json!({
                "ticket": lane.ticket,
                "branch": lane.branch,
                "base": lane.base,
                "outcome": outcome,
                "pr": pr,
            })
        ));
    }
    Ok(format!(
        "{}  {}  {}  ({} into {})\n",
        lane.ticket,
        outcome.as_str(),
        pr.url,
        lane.branch,
        lane.base
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ShipOutcome {
    Opened,
    Existing,
}

impl ShipOutcome {
    fn as_str(self) -> &'static str {
        match self {
            ShipOutcome::Opened => "opened",
            ShipOutcome::Existing => "existing",
        }
    }
}

/// A lane as `bli` sees it: a host checkout whose branch matches the pattern (ADR 0001).
#[derive(Debug, Serialize)]
struct Lane {
    ticket: String,
    branch: String,
    base: String,
    path: PathBuf,
    workspace: Option<String>,
    status: StatusHint,
}

/// The lane for `ticket`, or an error when it has none.
fn lane(host: &dyn LaneHost, git: &Git, config: &Config, ticket: &str) -> Result<Lane, Error> {
    lanes(host, git, config)?
        .into_iter()
        .find(|l| l.ticket == ticket)
        .ok_or_else(|| {
            Error::new(format!("no lane for ticket {ticket}"))
                .with_hint("run `bli lane list` to see every lane and its ticket")
        })
}

/// Rediscovers every lane from the host's live list.
fn lanes(host: &dyn LaneHost, git: &Git, config: &Config) -> Result<Vec<Lane>, Error> {
    let mut lanes = Vec::new();
    for checkout in host.list(git.main_checkout())? {
        if checkout.is_main {
            continue;
        }
        let Some(branch) = checkout.branch else {
            continue;
        };
        let Some(ticket) = config.lane.branch.ticket_id(&branch) else {
            continue;
        };
        let base = git
            .base(&branch)?
            .unwrap_or_else(|| config.lane.base.clone());
        lanes.push(Lane {
            ticket,
            branch,
            base,
            path: checkout.path,
            workspace: checkout.workspace,
            status: checkout.status,
        });
    }
    Ok(lanes)
}
