//! Seam 2: the GitHub adapter. Checks that each operation becomes the right
//! `gh` command line and that saved real `gh` output parses correctly.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::path::Path;

use bli::check::CheckStatus;
use bli::code_host::{CodeHost, NewPullRequest, PrState, PullRequest};
use bli::github::GitHub;
use bli::process::{Output, ProcessRunner};
use bli::tracker::{IssueTracker, Ticket, TicketDetails};

/// Records every command it is asked to run and answers with a canned output,
/// or with each of a sequence of outputs in turn.
struct FakeRunner {
    output: io::Result<Output>,
    queued: RefCell<VecDeque<Output>>,
    calls: RefCell<Vec<Vec<String>>>,
}

impl FakeRunner {
    fn replying(stdout: &str) -> Self {
        Self::with(Ok(Output {
            success: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }))
    }

    fn with(output: io::Result<Output>) -> Self {
        FakeRunner {
            output,
            queued: RefCell::new(VecDeque::new()),
            calls: RefCell::new(Vec::new()),
        }
    }

    /// Answers the next calls with `outputs`, one each, before falling back to the canned output.
    fn then_in_turn(self, outputs: Vec<Output>) -> Self {
        self.queued.borrow_mut().extend(outputs);
        self
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.borrow().clone()
    }
}

impl ProcessRunner for FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<Output> {
        let mut call = vec![program.to_string()];
        call.extend(args.iter().map(|a| a.to_string()));
        self.calls.borrow_mut().push(call);
        if let Some(out) = self.queued.borrow_mut().pop_front() {
            return Ok(out);
        }
        match &self.output {
            Ok(out) => Ok(out.clone()),
            Err(e) => Err(io::Error::new(e.kind(), e.to_string())),
        }
    }
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/gh/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn frontier_lists_the_parents_sub_issues_through_gh_api() {
    let runner = FakeRunner::replying("[]");
    GitHub::new(&runner).frontier("1").unwrap();
    assert_eq!(
        runner.calls(),
        vec![vec![
            "gh",
            "api",
            "repos/{owner}/{repo}/issues/1/sub_issues",
            "--paginate"
        ]]
    );
}

#[test]
fn frontier_accepts_a_hash_prefixed_parent() {
    let runner = FakeRunner::replying("[]");
    GitHub::new(&runner).frontier("#12").unwrap();
    assert_eq!(
        runner.calls()[0][2],
        "repos/{owner}/{repo}/issues/12/sub_issues"
    );
}

#[test]
fn frontier_leaves_out_claimed_and_blocked_tickets() {
    let runner = FakeRunner::replying(&fixture("sub_issues.json"));
    let frontier = GitHub::new(&runner).frontier("1").unwrap();
    assert_eq!(frontier, vec![]);
}

#[test]
fn frontier_keeps_open_unblocked_unclaimed_tickets_and_leaves_out_closed_ones() {
    let runner = FakeRunner::replying(&fixture("sub_issues_mixed.json"));
    let frontier = GitHub::new(&runner).frontier("1").unwrap();
    assert_eq!(
        frontier,
        vec![Ticket {
            id: "3".into(),
            title: "Fan-out on herdr: lane start, lane list, fan-out skill".into(),
            url: "https://github.com/ibedwi/bli/issues/3".into(),
        }]
    );
}

#[test]
fn frontier_rejects_a_parent_that_is_not_an_issue_number_without_calling_gh() {
    let runner = FakeRunner::replying("[]");
    let err = GitHub::new(&runner).frontier("abc").unwrap_err();
    assert!(err.to_string().contains("abc"), "{err}");
    assert_eq!(runner.calls(), Vec::<Vec<String>>::new());
}

#[test]
fn frontier_reports_gh_failure_with_its_stderr() {
    let runner = FakeRunner::with(Ok(Output {
        success: false,
        stdout: String::new(),
        stderr: "gh: Not Found (HTTP 404)".into(),
    }));
    let err = GitHub::new(&runner).frontier("999").unwrap_err();
    assert!(err.to_string().contains("Not Found (HTTP 404)"), "{err}");
}

#[test]
fn frontier_hints_at_installing_gh_when_it_is_missing() {
    let runner = FakeRunner::with(Err(io::Error::new(io::ErrorKind::NotFound, "no gh")));
    let err = GitHub::new(&runner).frontier("1").unwrap_err();
    assert!(err.hint().unwrap().contains("gh"), "{err:?}");
}

#[test]
fn ticket_views_the_issue_through_gh() {
    let runner = FakeRunner::replying(&fixture("issue_unclaimed.json"));
    GitHub::new(&runner).ticket("#4").unwrap();
    assert_eq!(
        runner.calls(),
        vec![vec![
            "gh",
            "issue",
            "view",
            "4",
            "--json",
            "number,title,url,state,assignees"
        ]]
    );
}

#[test]
fn ticket_reads_an_unclaimed_open_issue() {
    let runner = FakeRunner::replying(&fixture("issue_unclaimed.json"));
    let ticket = GitHub::new(&runner).ticket("4").unwrap();
    assert_eq!(
        ticket,
        TicketDetails {
            ticket: Ticket {
                id: "4".into(),
                title: "Remove-lane: lane inspect, lane rm, remove-lane skill".into(),
                url: "https://github.com/ibedwi/bli/issues/4".into(),
            },
            open: true,
            claimed_by: vec![],
        }
    );
}

#[test]
fn ticket_reads_a_claimed_issues_assignee_logins() {
    let runner = FakeRunner::replying(&fixture("issue_claimed.json"));
    let ticket = GitHub::new(&runner).ticket("3").unwrap();
    assert_eq!(ticket.claimed_by, vec!["ibedwi"]);
}

#[test]
fn ticket_reads_a_closed_issue_as_not_open() {
    let closed = fixture("issue_unclaimed.json").replace("\"OPEN\"", "\"CLOSED\"");
    let runner = FakeRunner::replying(&closed);
    assert!(!GitHub::new(&runner).ticket("4").unwrap().open);
}

#[test]
fn ticket_rejects_an_id_that_is_not_an_issue_number_without_calling_gh() {
    let runner = FakeRunner::replying("{}");
    let err = GitHub::new(&runner).ticket("CU-42").unwrap_err();
    assert!(err.to_string().contains("CU-42"), "{err}");
    assert_eq!(runner.calls(), Vec::<Vec<String>>::new());
}

#[test]
fn claim_assigns_the_issue_to_the_current_user() {
    let runner = FakeRunner::replying("https://github.com/ibedwi/bli/issues/4\n");
    GitHub::new(&runner).claim("#4").unwrap();
    assert_eq!(
        runner.calls(),
        vec![vec!["gh", "issue", "edit", "4", "--add-assignee", "@me"]]
    );
}

#[test]
fn claim_reports_gh_failure_with_its_stderr() {
    let runner = FakeRunner::with(Ok(Output {
        success: false,
        stdout: String::new(),
        stderr: "GraphQL: Could not resolve to an issue".into(),
    }));
    let err = GitHub::new(&runner).claim("4").unwrap_err();
    assert!(err.to_string().contains("Could not resolve"), "{err}");
}

#[test]
fn pr_body_adds_a_closing_reference_so_merging_closes_the_ticket() {
    let runner = FakeRunner::replying("");
    let body = GitHub::new(&runner)
        .pr_body("#5", "## Summary\n\nShips lanes.\n")
        .unwrap();
    assert_eq!(body, "## Summary\n\nShips lanes.\n\nCloses #5\n");
}

#[test]
fn pr_body_keeps_a_body_that_already_closes_the_ticket() {
    let runner = FakeRunner::replying("");
    for body in ["Closes #5", "fixes #5.", "Some text\nResolved #5\n"] {
        assert_eq!(GitHub::new(&runner).pr_body("5", body).unwrap(), body);
    }
}

#[test]
fn pr_body_does_not_mistake_another_tickets_reference_for_this_ones() {
    let runner = FakeRunner::replying("");
    let body = GitHub::new(&runner)
        .pr_body("5", "Closes #50\nSee #5")
        .unwrap();
    assert_eq!(body, "Closes #50\nSee #5\n\nCloses #5\n");
}

const REPO: &str = "/home/ibedwi/github/bli";

#[test]
fn push_pushes_the_branch_to_origin_and_tracks_it() {
    let runner = FakeRunner::replying("");
    GitHub::new(&runner)
        .push(Path::new(REPO), "ticket/5-fan-in")
        .unwrap();
    assert_eq!(
        runner.calls(),
        vec![vec![
            "git",
            "-C",
            REPO,
            "push",
            "--quiet",
            "-u",
            "origin",
            "ticket/5-fan-in"
        ]]
    );
}

#[test]
fn push_reports_git_failure_with_its_stderr() {
    let runner = FakeRunner::with(Ok(Output {
        success: false,
        stdout: String::new(),
        stderr: "! [rejected] ticket/5-fan-in (non-fast-forward)".into(),
    }));
    let err = GitHub::new(&runner)
        .push(Path::new(REPO), "ticket/5-fan-in")
        .unwrap_err();
    assert!(err.to_string().contains("non-fast-forward"), "{err}");
}

#[test]
fn find_pr_lists_the_head_branchs_prs_in_any_state() {
    let runner = FakeRunner::replying(&fixture("pr_list_merged.json"));
    let pr = GitHub::new(&runner)
        .find_pr("ticket/3-fan-out-on-herdr")
        .unwrap();
    assert_eq!(
        runner.calls(),
        vec![vec![
            "gh",
            "pr",
            "list",
            "--head",
            "ticket/3-fan-out-on-herdr",
            "--state",
            "all",
            "--json",
            "number,url,state",
        ]]
    );
    assert_eq!(
        pr,
        Some(PullRequest {
            number: 12,
            url: "https://github.com/ibedwi/bli/pull/12".into(),
            state: PrState::Merged,
        })
    );
}

#[test]
fn find_pr_finds_nothing_for_a_branch_without_a_pr() {
    let runner = FakeRunner::replying("[]\n");
    assert_eq!(GitHub::new(&runner).find_pr("ticket/5-x").unwrap(), None);
}

#[test]
fn find_pr_prefers_an_open_pr_over_a_closed_one() {
    let runner = FakeRunner::replying(
        r#"[{"number":14,"state":"CLOSED","url":"https://github.com/o/r/pull/14"},
            {"number":13,"state":"OPEN","url":"https://github.com/o/r/pull/13"}]"#,
    );
    let pr = GitHub::new(&runner).find_pr("ticket/5-x").unwrap().unwrap();
    assert_eq!(pr.number, 13);
    assert_eq!(pr.state, PrState::Open);
}

#[test]
fn open_pr_creates_the_pr_against_the_base_and_reads_its_url() {
    let runner = FakeRunner::replying("https://github.com/ibedwi/bli/pull/13\n");
    let pr = GitHub::new(&runner)
        .open_pr(&NewPullRequest {
            head: "ticket/5-fan-in".into(),
            base: "release".into(),
            title: "Fan-in ships finished lanes".into(),
            body: "Closes #5\n".into(),
        })
        .unwrap();
    assert_eq!(
        runner.calls(),
        vec![vec![
            "gh",
            "pr",
            "create",
            "--head",
            "ticket/5-fan-in",
            "--base",
            "release",
            "--title",
            "Fan-in ships finished lanes",
            "--body",
            "Closes #5\n",
        ]]
    );
    assert_eq!(
        pr,
        PullRequest {
            number: 13,
            url: "https://github.com/ibedwi/bli/pull/13".into(),
            state: PrState::Open,
        }
    );
}

fn output(success: bool, stdout: &str, stderr: &str) -> Output {
    Output {
        success,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    }
}

#[test]
fn checks_pass_when_gh_is_installed_and_logged_in() {
    let runner = FakeRunner::replying("").then_in_turn(vec![
        output(true, "gh version 2.81.0 (2025-10-01)\n", ""),
        output(
            true,
            "",
            "github.com\n  ✓ Logged in to github.com account ibedwi\n",
        ),
    ]);
    let checks = GitHub::new(&runner).checks();
    assert_eq!(
        runner.calls(),
        vec![vec!["gh", "--version"], vec!["gh", "auth", "status"]]
    );
    let summary: Vec<_> = checks.iter().map(|c| (c.name.as_str(), c.status)).collect();
    assert_eq!(
        summary,
        [("gh", CheckStatus::Pass), ("gh auth", CheckStatus::Pass)]
    );
    assert_eq!(checks[0].detail, "gh version 2.81.0 (2025-10-01)");
}

#[test]
fn checks_warn_with_an_install_hint_when_gh_is_missing() {
    let runner = FakeRunner::with(Err(io::Error::new(io::ErrorKind::NotFound, "no gh")));
    let checks = GitHub::new(&runner).checks();
    assert_eq!(checks[0].name, "gh");
    assert_eq!(checks[0].status, CheckStatus::Warn);
    assert!(
        checks[0]
            .hint
            .as_deref()
            .unwrap()
            .contains("cli.github.com"),
        "{checks:?}"
    );
    assert_eq!(checks[1].name, "gh auth");
    assert_eq!(checks[1].status, CheckStatus::Warn);
}

#[test]
fn checks_warn_with_a_login_hint_when_gh_credentials_fail() {
    let runner = FakeRunner::replying("").then_in_turn(vec![
        output(true, "gh version 2.81.0 (2025-10-01)\n", ""),
        output(
            false,
            "",
            "You are not logged into any GitHub hosts. To log in, run: gh auth login\n",
        ),
    ]);
    let checks = GitHub::new(&runner).checks();
    assert_eq!(checks[0].status, CheckStatus::Pass);
    assert_eq!(checks[1].name, "gh auth");
    assert_eq!(checks[1].status, CheckStatus::Warn);
    assert!(checks[1].detail.contains("not logged into"), "{checks:?}");
    assert!(
        checks[1].hint.as_deref().unwrap().contains("gh auth login"),
        "{checks:?}"
    );
}
