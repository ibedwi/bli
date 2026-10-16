//! Seam 2: the GitHub adapter. Checks that each operation becomes the right
//! `gh` command line and that saved real `gh` output parses correctly.

use std::cell::RefCell;
use std::io;

use bli::github::GitHub;
use bli::process::{Output, ProcessRunner};
use bli::tracker::{IssueTracker, Ticket};

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
