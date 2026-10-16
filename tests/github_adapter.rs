//! Seam 2: the GitHub adapter. Checks that each operation becomes the right
//! `gh` command line and that saved real `gh` output parses correctly.

use std::cell::RefCell;
use std::io;

use bli::github::GitHub;
use bli::process::{Output, ProcessRunner};
use bli::tracker::{IssueTracker, Ticket, TicketDetails};

/// Records every command it is asked to run and answers with a canned output.
struct FakeRunner {
    output: io::Result<Output>,
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
            calls: RefCell::new(Vec::new()),
        }
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
            assignees: vec![],
        }
    );
}

#[test]
fn ticket_reads_a_claimed_issues_assignee_logins() {
    let runner = FakeRunner::replying(&fixture("issue_claimed.json"));
    let ticket = GitHub::new(&runner).ticket("3").unwrap();
    assert_eq!(ticket.assignees, vec!["ibedwi"]);
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
