//! The workflow. Depends only on the provider interfaces and git, never on an adapter.

use std::path::PathBuf;

use serde::Serialize;
use serde_json::json;

use crate::branch;
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
) -> Result<String, Error> {
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
            report.outcome = Outcome::Failed;
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
    if json {
        return Ok(format!("{}\n", json!({ "base": base, "lanes": reports })));
    }
    let mut out = String::new();
    for r in &reports {
        let detail = match r.outcome {
            Outcome::Started => format!(
                "{}  ({})",
                r.branch.as_deref().unwrap_or_default(),
                r.status.unwrap_or(StatusHint::Unknown).as_str()
            ),
            Outcome::Skipped => r.reason.clone().unwrap_or_default(),
            Outcome::Failed => format!("error: {}", r.error.as_deref().unwrap_or_default()),
        };
        out.push_str(&format!(
            "{}  {}  {}\n",
            r.ticket,
            r.outcome.as_str(),
            detail
        ));
    }
    Ok(out)
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
    if !details.assignees.is_empty() {
        report.skip(format!(
            "already claimed by {}",
            details.assignees.join(", ")
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
    let prompt = config.lane.prompt.replace("{ticket_url}", &ticket.url);
    report.status = Some(host.start_agent(&lane, &ticket.id, &prompt)?);
    tracker.claim(&ticket.id)?;
    report.outcome = Outcome::Started;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Outcome {
    Started,
    Skipped,
    Failed,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Started => "started",
            Outcome::Skipped => "skipped",
            Outcome::Failed => "failed",
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
    outcome: Outcome,
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
            outcome: Outcome::Failed,
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
        self.outcome = Outcome::Skipped;
        self.reason = Some(reason);
    }
}

pub fn lane_list(
    host: &dyn LaneHost,
    git: &Git,
    config: &Config,
    json: bool,
) -> Result<String, Error> {
    let lanes = lanes(host, git, config)?;
    if json {
        return Ok(format!("{}\n", json!({ "lanes": lanes })));
    }
    if lanes.is_empty() {
        return Ok("No lanes.\n".into());
    }
    let mut out = String::new();
    for l in &lanes {
        out.push_str(&format!(
            "{}  {}  (base {}, {})\n    {}\n",
            l.ticket,
            l.branch,
            l.base,
            l.status.as_str(),
            l.path.display()
        ));
    }
    Ok(out)
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
