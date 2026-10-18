//! The workflow. Depends only on the provider interfaces and git, never on an adapter.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;

use crate::branch;
use crate::check::Check;
use crate::code_host::{CodeHost, NewPullRequest, PrState, PullRequest};
use crate::config::Config;
use crate::error::Error;
use crate::git::{Changes, Commit, Git, REMOTE};
use crate::host::{HostCheckout, LaneHost, NewLane, StatusHint};
use crate::tracker::{IssueTracker, OnMerge};

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
    require_inside(host)?;
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

/// Fails unless `bli` runs inside `host`, so host commands have something to act on.
fn require_inside(host: &dyn LaneHost) -> Result<(), Error> {
    if host.is_inside() {
        return Ok(());
    }
    let name = host.name();
    Err(
        Error::new(format!("not inside {name}, the configured lane host")).with_hint(format!(
            "run this from a terminal inside {name}, or change `host` under [lane] in bli.toml"
        )),
    )
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
    let output = host.read_agent(&lane.ticket, lane.workspace.as_deref(), lines)?;
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

pub fn lane_inspect(
    host: &dyn LaneHost,
    code: &dyn CodeHost,
    git: &Git,
    config: &Config,
    cwd: &Path,
    name: &str,
    json: bool,
) -> Result<String, Error> {
    let lane = find_lane(host, git, config, cwd, name)?;
    let inspection = inspect(code, git, lane)?;
    if json {
        return Ok(format!("{}\n", json!(inspection)));
    }
    Ok(inspection.render())
}

/// What removing a lane would lose, and where its work stands.
#[derive(Debug, Serialize)]
struct Inspection {
    #[serde(flatten)]
    lane: Lane,
    #[serde(flatten)]
    changes: Changes,
    unpushed: Vec<Commit>,
    upstream: Option<String>,
    pr: Option<PullRequest>,
}

fn inspect(code: &dyn CodeHost, git: &Git, lane: Lane) -> Result<Inspection, Error> {
    git.fetch()?;
    Ok(Inspection {
        changes: git.changes(&lane.path)?,
        unpushed: git.unpushed(&lane.branch)?,
        upstream: git.upstream(&lane.branch)?,
        pr: code.find_pr(&lane.branch)?,
        lane,
    })
}

impl Inspection {
    fn render(&self) -> String {
        let l = &self.lane;
        let mut out = format!(
            "{}  {}  (base {}, {})\n    {}\n",
            l.ticket,
            l.branch,
            l.base,
            l.status.as_str(),
            l.path.display()
        );
        let list = |files: &[String]| match files {
            [] => "none".to_string(),
            files => files.join(", "),
        };
        out.push_str(&format!("modified:  {}\n", list(&self.changes.modified)));
        out.push_str(&format!("untracked: {}\n", list(&self.changes.untracked)));
        out.push_str(&format!("ignored:   {}\n", list(&self.changes.ignored)));
        if self.unpushed.is_empty() {
            out.push_str("unpushed:  none\n");
        } else {
            let n = self.unpushed.len();
            let plural = if n == 1 { "" } else { "s" };
            out.push_str(&format!("unpushed:  {n} commit{plural}\n"));
            for c in &self.unpushed {
                out.push_str(&format!("    {} {}\n", c.sha, c.subject));
            }
        }
        out.push_str(&format!(
            "upstream:  {}\n",
            self.upstream.as_deref().unwrap_or("none")
        ));
        match &self.pr {
            Some(pr) => out.push_str(&format!(
                "PR:        #{} {}  {}\n",
                pr.number,
                pr.state.as_str(),
                pr.url
            )),
            None => out.push_str("PR:        none\n"),
        }
        out
    }
}

pub struct RmRequest<'a> {
    pub name: &'a str,
    pub force: bool,
    pub delete_branch: bool,
}

pub fn lane_rm(
    host: &dyn LaneHost,
    git: &Git,
    config: &Config,
    cwd: &Path,
    request: &RmRequest,
    json: bool,
) -> Result<Report, Error> {
    require_inside(host)?;
    let lane = find_lane(host, git, config, cwd, request.name)?;
    refuse_own_lane(host, cwd, &lane, "bli lane rm")?;
    let branch = match (request.delete_branch, request.force) {
        (false, _) => BranchRemoval::Keep,
        (true, false) => BranchRemoval::DeleteIfPushed,
        (true, true) => BranchRemoval::Delete,
    };
    refuse_unsafe_removal(
        git,
        &lane,
        request.force,
        branch,
        "pass --force to lose them",
    )?;
    let failure = remove(host, git, &lane, request.force, branch)?;
    let removed = Removed {
        branch_deleted: request.delete_branch && failure.is_none(),
        ticket: lane.ticket,
        branch: lane.branch,
        path: lane.path,
        workspace: lane.workspace,
        forced: request.force,
    };
    if json {
        let stdout = format!("{}\n", json!(removed));
        return Ok(Report { stdout, failure });
    }
    let stdout = format!(
        "Removed lane {} at {}{}\n",
        removed.ticket,
        removed.path.display(),
        if removed.branch_deleted {
            format!(", and its branch {}", removed.branch)
        } else {
            format!("; its branch {} is kept", removed.branch)
        }
    );
    Ok(Report { stdout, failure })
}

/// What becomes of a lane's local branch when the lane is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BranchRemoval {
    Keep,
    /// Delete it, unless it has commits on no remote.
    DeleteIfPushed,
    Delete,
}

/// Refuses to remove the lane `bli` is running from, by directory or by host workspace.
/// `command` is the `bli` command to run from elsewhere instead.
fn refuse_own_lane(
    host: &dyn LaneHost,
    cwd: &Path,
    lane: &Lane,
    command: &str,
) -> Result<(), Error> {
    let own_workspace =
        lane.workspace.is_some() && lane.workspace.as_deref() == host.current_workspace();
    if own_workspace || normalize(cwd).starts_with(normalize(&lane.path)) {
        return Err(Error::new(format!(
            "lane {} is the one bli is running from",
            lane.ticket
        ))
        .with_hint(format!(
            "run `{command}` from another workspace of the lane host, outside this lane's checkout"
        )));
    }
    Ok(())
}

/// Refuses, unless `force`, to remove a lane whose checkout has uncommitted or untracked
/// files, or to delete a branch with commits on no remote when `branch` asks it be pushed.
/// `lose_them` ends the hint for uncommitted files: how to remove the lane anyway.
fn refuse_unsafe_removal(
    git: &Git,
    lane: &Lane,
    force: bool,
    branch: BranchRemoval,
    lose_them: &str,
) -> Result<(), Error> {
    if force {
        return Ok(());
    }
    let changes = git.changes(&lane.path)?;
    if !changes.is_empty() {
        let mut files = changes.modified;
        files.extend(
            changes
                .untracked
                .into_iter()
                .map(|f| format!("{f} (untracked)")),
        );
        return Err(Error::new(format!(
            "lane {} has uncommitted or untracked files: {}",
            lane.ticket,
            files.join(", ")
        ))
        .with_hint(format!("commit or remove them, or {lose_them}")));
    }
    if branch == BranchRemoval::DeleteIfPushed {
        git.fetch()?;
        let unpushed = git.unpushed(&lane.branch)?;
        if !unpushed.is_empty() {
            let commits: Vec<_> = unpushed
                .iter()
                .map(|c| format!("{} {}", c.sha, c.subject))
                .collect();
            return Err(Error::new(format!(
                "branch {} has commits on no remote: {}",
                lane.branch,
                commits.join("; ")
            ))
            .with_hint(
                "push them, or keep the branch by leaving out --delete-branch, or pass --force to lose them",
            ));
        }
    }
    Ok(())
}

/// Removes `lane`'s workspace and checkout through the host, then its branch unless
/// `branch` keeps it. Fails only when the host does; a branch that could not be deleted
/// is returned as the failure to report alongside the removal, since the lane is gone
/// and rerunning can no longer find it.
fn remove(
    host: &dyn LaneHost,
    git: &Git,
    lane: &Lane,
    force: bool,
    branch: BranchRemoval,
) -> Result<Option<Error>, Error> {
    host.remove_lane(
        git.main_checkout(),
        &lane.path,
        lane.workspace.as_deref(),
        force,
    )?;
    if branch == BranchRemoval::Keep {
        return Ok(None);
    }
    // Deleting a branch also deletes its config, the recorded base among it.
    Ok(git.delete_branch(&lane.branch).err().map(|e| {
        Error::new(format!(
            "removed lane {}, but could not delete its branch: {e}",
            lane.ticket
        ))
        .with_hint(format!("delete it with `git branch -D {}`", lane.branch))
    }))
}

/// What `lane rm` removed.
#[derive(Debug, Serialize)]
struct Removed {
    ticket: String,
    branch: String,
    path: PathBuf,
    workspace: Option<String>,
    forced: bool,
    branch_deleted: bool,
}

pub fn lane_clean(
    tracker: &dyn IssueTracker,
    host: &dyn LaneHost,
    code: &dyn CodeHost,
    git: &Git,
    config: &Config,
    cwd: &Path,
    json: bool,
) -> Result<Report, Error> {
    require_inside(host)?;
    git.fetch()?;
    let mut reports = Vec::new();
    for lane in lanes(host, git, config)? {
        let Some(pr) = code.find_pr(&lane.branch)? else {
            continue;
        };
        let mut report = CleanReport::new(lane, pr);
        match report.pr.state {
            PrState::Open => continue,
            PrState::Closed => report.outcome = CleanOutcome::Left,
            PrState::Merged => match clean_one(tracker, host, git, config, cwd, &mut report) {
                Ok(None) => report.outcome = CleanOutcome::Cleaned,
                Ok(Some(e)) => {
                    report.outcome = CleanOutcome::Cleaned;
                    report.fail(e);
                }
                Err(e) => report.fail(e),
            },
        }
        reports.push(report);
    }
    let mut failures: Vec<_> = reports
        .iter()
        .filter_map(|r| r.error.as_ref().map(|e| (e.clone(), r.hint.clone())))
        .collect();
    let fast_forwarded = fast_forward_main(git, &reports).unwrap_or_else(|e| {
        failures.push((e.to_string(), e.hint().map(str::to_string)));
        None
    });
    let failure = match failures.as_slice() {
        [] => None,
        [(message, hint)] => Some(match hint {
            Some(hint) => Error::new(message).with_hint(hint),
            None => Error::new(message),
        }),
        all => {
            let mut message = format!("{} problems cleaning up merged lanes:", all.len());
            for (e, hint) in all {
                message.push_str(&format!("\n  {e}"));
                if let Some(hint) = hint {
                    message.push_str(&format!("\n    hint: {hint}"));
                }
            }
            Some(Error::new(message))
        }
    };
    if json {
        let stdout = format!(
            "{}\n",
            json!({ "lanes": reports, "fast_forwarded": fast_forwarded })
        );
        return Ok(Report { stdout, failure });
    }
    let mut out = String::new();
    for r in &reports {
        let detail = match r.outcome {
            CleanOutcome::Left => "closed without merging; left alone".to_string(),
            _ => format!("PR #{} merged", r.pr.number),
        };
        out.push_str(&format!(
            "{}  {}  {}  ({detail})\n",
            r.lane.ticket,
            r.outcome.as_str(),
            r.lane.branch
        ));
    }
    if reports.is_empty() {
        out.push_str("No merged or closed lanes.\n");
    }
    if let Some(branch) = &fast_forwarded {
        out.push_str(&format!(
            "Fast-forwarded the main checkout's {branch} to {REMOTE}/{branch}.\n"
        ));
    }
    Ok(Report {
        stdout: out,
        failure,
    })
}

/// Fast-forwards the main checkout when it is on the base of a lane just cleaned,
/// returning the branch it moved.
fn fast_forward_main(git: &Git, reports: &[CleanReport]) -> Result<Option<String>, Error> {
    let Some(branch) = git.main_branch()? else {
        return Ok(None);
    };
    let cleaned_onto =
        |r: &CleanReport| r.outcome == CleanOutcome::Cleaned && r.lane.base == branch;
    if !reports.iter().any(cleaned_onto) {
        return Ok(None);
    }
    git.fast_forward_main(&branch).map_err(|e| {
        Error::new(format!(
            "could not fast-forward the main checkout's {branch} to {REMOTE}/{branch}: {e}"
        ))
        .with_hint(format!(
            "it has diverged from {REMOTE}/{branch} or its changes conflict; bring it up to date by hand"
        ))
    })?;
    Ok(Some(branch))
}

/// Cleans up `report`'s merged lane: applies the on-merge action to its ticket, then
/// removes it with its branch the way `lane rm --delete-branch` does. The merged PR
/// stands in for pushed commits, since a squash merge leaves the lane's commits on no
/// remote, but commits made after the merge are refused. The refusals come first, so a
/// refused lane's ticket is left as it was.
fn clean_one(
    tracker: &dyn IssueTracker,
    host: &dyn LaneHost,
    git: &Git,
    config: &Config,
    cwd: &Path,
    report: &mut CleanReport,
) -> Result<Option<Error>, Error> {
    let lane = &report.lane;
    let by_hand = format!(
        "remove the lane by hand with `bli lane rm {} --delete-branch --force` to lose them",
        lane.branch
    );
    refuse_own_lane(host, cwd, lane, "bli lane clean")?;
    refuse_unsafe_removal(git, lane, false, BranchRemoval::Delete, &by_hand)?;
    if let Some(head) = &report.pr.head_sha {
        refuse_commits_after_merge(git, lane, report.pr.number, head, &by_hand)?;
    }
    tracker.on_merge(&lane.ticket, &config.tracker.on_merge)?;
    report.on_merge = Some(config.tracker.on_merge.clone());
    remove(host, git, &report.lane, false, BranchRemoval::Delete)
}

/// Refuses to delete a merged lane's branch when it has commits that PR `number`,
/// last at commit `head`, didn't merge.
fn refuse_commits_after_merge(
    git: &Git,
    lane: &Lane,
    number: u64,
    head: &str,
    lose_them: &str,
) -> Result<(), Error> {
    let after = match git.commits_beyond(&lane.branch, head)? {
        Some(after) if after.is_empty() => return Ok(()),
        Some(after) => after,
        None => {
            return Err(Error::new(format!(
                "cannot tell whether branch {} has commits PR #{number} didn't merge: its last commit {head} is not in this repo",
                lane.branch
            ))
            .with_hint(format!("fetch it, or {lose_them}")))
        }
    };
    let commits: Vec<_> = after
        .iter()
        .map(|c| format!("{} {}", c.sha, c.subject))
        .collect();
    Err(Error::new(format!(
        "branch {} has commits made after PR #{number} merged: {}",
        lane.branch,
        commits.join("; ")
    ))
    .with_hint(format!(
        "ship them in a new PR from another branch, or {lose_them}"
    )))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum CleanOutcome {
    /// Merged, and removed.
    Cleaned,
    /// Closed without merging, so left for the developer.
    Left,
    /// Merged, but not removed.
    Failed,
}

impl CleanOutcome {
    fn as_str(self) -> &'static str {
        match self {
            CleanOutcome::Cleaned => "cleaned",
            CleanOutcome::Left => "left",
            CleanOutcome::Failed => "failed",
        }
    }
}

/// What `lane clean` did with one lane whose PR was merged or closed.
#[derive(Debug, Serialize)]
struct CleanReport {
    #[serde(flatten)]
    lane: Lane,
    pr: PullRequest,
    outcome: CleanOutcome,
    /// The on-merge action, once it has been applied to the lane's ticket.
    #[serde(skip_serializing_if = "Option::is_none")]
    on_merge: Option<OnMerge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

impl CleanReport {
    fn new(lane: Lane, pr: PullRequest) -> Self {
        CleanReport {
            lane,
            pr,
            outcome: CleanOutcome::Failed,
            on_merge: None,
            error: None,
            hint: None,
        }
    }

    fn fail(&mut self, e: Error) {
        self.hint = e.hint().map(str::to_string);
        self.error = Some(e.to_string());
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
    as_lanes(host.list(git.main_checkout())?, git, config)
}

/// The one lane `name` refers to, by ticket ID, branch, checkout path or host workspace ID.
fn find_lane(
    host: &dyn LaneHost,
    git: &Git,
    config: &Config,
    cwd: &Path,
    name: &str,
) -> Result<Lane, Error> {
    let checkouts = host.list(git.main_checkout())?;
    let path = normalize(&cwd.join(name));
    let names_main = |c: &HostCheckout| {
        c.is_main
            && (c.branch.as_deref() == Some(name)
                || c.workspace.as_deref() == Some(name)
                || normalize(&c.path) == path)
    };
    if normalize(git.main_checkout()) == path || checkouts.iter().any(names_main) {
        return Err(Error::new(format!(
            "`{name}` is the main checkout, which is never a lane"
        )));
    }
    let lanes = as_lanes(checkouts, git, config)?;
    let ticket = name.strip_prefix('#').unwrap_or(name);
    let (matches, others): (Vec<_>, Vec<_>) = lanes.into_iter().partition(|l| {
        l.ticket == ticket
            || l.branch == name
            || l.workspace.as_deref() == Some(name)
            || normalize(&l.path) == path
    });
    let mut matches = matches.into_iter();
    match (matches.next(), matches.next()) {
        (Some(lane), None) => Ok(lane),
        (None, _) if others.is_empty() => Err(Error::new(format!(
            "no lane matches `{name}`: there are no lanes"
        ))),
        (None, _) => Err(Error::new(format!("no lane matches `{name}`"))
            .with_hint(candidates("the lanes are", &others))),
        (Some(first), Some(second)) => {
            let all: Vec<_> = [first, second].into_iter().chain(matches).collect();
            Err(Error::new(format!("`{name}` matches {} lanes", all.len()))
                .with_hint(candidates("name one by branch or path", &all)))
        }
    }
}

fn candidates(intro: &str, lanes: &[Lane]) -> String {
    let mut out = format!("{intro}:");
    for l in lanes {
        out.push_str(&format!(
            "\n  {}  {}  {}  {}",
            l.ticket,
            l.branch,
            l.path.display(),
            l.workspace.as_deref().unwrap_or("-")
        ));
    }
    out
}

/// `path` resolved through symlinks when it exists, so two spellings of one checkout compare equal.
fn normalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// The lanes among a host's checkouts.
fn as_lanes(checkouts: Vec<HostCheckout>, git: &Git, config: &Config) -> Result<Vec<Lane>, Error> {
    let mut lanes = Vec::new();
    for checkout in checkouts {
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

pub fn doctor(checks: &[Check], json: bool) -> Report {
    let failed = checks.iter().filter(|c| !c.passed()).count();
    let failure =
        (failed > 0).then(|| Error::new(format!("{failed} of {} checks failed", checks.len())));
    if json {
        let stdout = format!("{}\n", json!({ "checks": checks }));
        return Report { stdout, failure };
    }
    let mut out = String::new();
    for c in checks {
        out.push_str(&format!(
            "{}  {}  {}\n",
            c.status.as_str(),
            c.name,
            c.detail
        ));
        if let Some(hint) = &c.hint {
            // Indent every line of a multi-line hint under the first, leaving blank lines blank.
            let mut lines = hint.lines();
            out.push_str(&format!(
                "      hint: {}\n",
                lines.next().unwrap_or_default()
            ));
            for line in lines {
                if !line.is_empty() {
                    out.push_str("      ");
                }
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    Report {
        stdout: out,
        failure,
    }
}
