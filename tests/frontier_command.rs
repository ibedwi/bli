//! Seam 1: the command layer. Runs `bli` commands against a fake issue tracker,
//! in a temporary repo with a real `bli.toml`, and checks output and exit code.

use std::collections::HashMap;
use std::path::Path;

use bli::config::Config;
use bli::error::Error;
use bli::tracker::{IssueTracker, Ticket};
use bli::{Outcome, Providers};
use serde_json::json;
use tempfile::TempDir;

/// An issue tracker whose frontier under each parent is fixed up front.
#[derive(Default)]
struct FakeTracker {
    frontiers: HashMap<String, Vec<Ticket>>,
}

impl FakeTracker {
    fn with_frontier(mut self, parent: &str, tickets: &[(&str, &str)]) -> Self {
        let tickets = tickets
            .iter()
            .map(|(id, title)| Ticket {
                id: id.to_string(),
                title: title.to_string(),
                url: format!("https://tracker.test/{id}"),
            })
            .collect();
        self.frontiers.insert(parent.to_string(), tickets);
        self
    }
}

impl IssueTracker for FakeTracker {
    fn frontier(&self, parent: &str) -> Result<Vec<Ticket>, Error> {
        self.frontiers
            .get(parent)
            .cloned()
            .ok_or_else(|| Error::new(format!("no ticket {parent}")))
    }
}

impl Providers for FakeTracker {
    fn issue_tracker(&self, _config: &Config) -> Result<Box<dyn IssueTracker + '_>, Error> {
        Ok(Box::new(self))
    }
}

/// A repo checkout with the given `bli.toml`, or none.
fn repo(bli_toml: Option<&str>) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    if let Some(toml) = bli_toml {
        std::fs::write(dir.path().join("bli.toml"), toml).unwrap();
    }
    dir
}

const GITHUB: &str = "[tracker]\nkind = \"github\"\n";

fn bli(args: &[&str], cwd: &Path, tracker: &FakeTracker) -> Outcome {
    bli::run(args, cwd, tracker)
}

fn json(out: &Outcome) -> serde_json::Value {
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    serde_json::from_str(&out.stdout).unwrap()
}

#[test]
fn frontier_json_lists_each_tickets_id_title_and_url() {
    let repo = repo(Some(GITHUB));
    let tracker = FakeTracker::default().with_frontier("1", &[("3", "Fan-out"), ("7", "Doctor")]);

    let out = bli(
        &["frontier", "--parent", "1", "--json"],
        repo.path(),
        &tracker,
    );

    assert_eq!(
        json(&out),
        json!({
            "parent": "1",
            "tickets": [
                {"id": "3", "title": "Fan-out", "url": "https://tracker.test/3"},
                {"id": "7", "title": "Doctor", "url": "https://tracker.test/7"},
            ]
        })
    );
    assert_eq!(out.stderr, "");
}

#[test]
fn frontier_prints_a_readable_list() {
    let repo = repo(Some(GITHUB));
    let tracker = FakeTracker::default().with_frontier("1", &[("3", "Fan-out"), ("7", "Doctor")]);

    let out = bli(&["frontier", "--parent", "1"], repo.path(), &tracker);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "3  Fan-out\n    https://tracker.test/3\n7  Doctor\n    https://tracker.test/7\n"
    );
}

#[test]
fn frontier_says_so_when_nothing_can_start() {
    let repo = repo(Some(GITHUB));
    let tracker = FakeTracker::default().with_frontier("1", &[]);

    let out = bli(&["frontier", "--parent", "1"], repo.path(), &tracker);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "No tickets on the frontier of 1.\n");
}

#[test]
fn frontier_uses_the_configured_parent_when_none_is_given() {
    let repo = repo(Some("[tracker]\nkind = \"github\"\nparent = \"17\"\n"));
    let tracker = FakeTracker::default()
        .with_frontier("1", &[("3", "Wrong effort")])
        .with_frontier("17", &[("20", "Right effort")]);

    let out = bli(&["frontier", "--json"], repo.path(), &tracker);

    assert_eq!(json(&out)["parent"], "17");
    assert_eq!(json(&out)["tickets"][0]["id"], "20");
}

#[test]
fn frontier_accepts_a_numeric_configured_parent() {
    let repo = repo(Some("[tracker]\nkind = \"github\"\nparent = 17\n"));
    let tracker = FakeTracker::default().with_frontier("17", &[("20", "Right effort")]);

    let out = bli(&["frontier", "--json"], repo.path(), &tracker);

    assert_eq!(json(&out)["parent"], "17");
}

#[test]
fn frontier_parent_flag_overrides_the_configured_parent() {
    let repo = repo(Some("[tracker]\nkind = \"github\"\nparent = \"17\"\n"));
    let tracker = FakeTracker::default()
        .with_frontier("1", &[("3", "Chosen")])
        .with_frontier("17", &[("20", "Default")]);

    let out = bli(
        &["frontier", "--parent", "1", "--json"],
        repo.path(),
        &tracker,
    );

    assert_eq!(json(&out)["tickets"][0]["id"], "3");
}

#[test]
fn frontier_without_any_parent_fails_with_a_hint() {
    let repo = repo(Some(GITHUB));
    let tracker = FakeTracker::default();

    let out = bli(&["frontier", "--json"], repo.path(), &tracker);

    assert_ne!(out.code, 0);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("--parent"), "{}", out.stderr);
    assert!(out.stderr.contains("[tracker]"), "{}", out.stderr);
}

#[test]
fn any_command_without_bli_toml_fails_with_a_hint() {
    let repo = repo(None);
    let tracker = FakeTracker::default().with_frontier("1", &[]);

    let out = bli(&["frontier", "--parent", "1"], repo.path(), &tracker);

    assert_ne!(out.code, 0);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("bli.toml"), "{}", out.stderr);
    assert!(out.stderr.contains("hint:"), "{}", out.stderr);
}

#[test]
fn bli_toml_is_found_from_a_subdirectory_of_the_checkout() {
    let repo = repo(Some(GITHUB));
    let sub = repo.path().join("src/deep");
    std::fs::create_dir_all(&sub).unwrap();
    let tracker = FakeTracker::default().with_frontier("1", &[("3", "Fan-out")]);

    let out = bli(&["frontier", "--parent", "1", "--json"], &sub, &tracker);

    assert_eq!(json(&out)["tickets"][0]["id"], "3");
}

#[test]
fn bli_toml_outside_the_checkout_is_not_used() {
    let outer = repo(Some(GITHUB));
    let inner = outer.path().join("nested-repo");
    std::fs::create_dir_all(inner.join(".git")).unwrap();
    let tracker = FakeTracker::default().with_frontier("1", &[]);

    let out = bli(&["frontier", "--parent", "1"], &inner, &tracker);

    assert_ne!(out.code, 0);
    assert!(out.stderr.contains("bli.toml"), "{}", out.stderr);
}

#[test]
fn an_unknown_tracker_kind_is_rejected() {
    let repo = repo(Some("[tracker]\nkind = \"jira\"\n"));
    let tracker = FakeTracker::default().with_frontier("1", &[]);

    let out = bli(&["frontier", "--parent", "1"], repo.path(), &tracker);

    assert_ne!(out.code, 0);
    assert!(out.stderr.contains("jira"), "{}", out.stderr);
}

#[test]
fn tracker_errors_go_to_stderr_with_a_nonzero_exit_even_with_json() {
    let repo = repo(Some(GITHUB));
    let tracker = FakeTracker::default();

    let out = bli(
        &["frontier", "--parent", "404", "--json"],
        repo.path(),
        &tracker,
    );

    assert_ne!(out.code, 0);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "error: no ticket 404\n");
}

#[test]
fn unknown_arguments_fail_on_stderr() {
    let repo = repo(Some(GITHUB));
    let tracker = FakeTracker::default();

    let out = bli(&["frontier", "--bogus"], repo.path(), &tracker);

    assert_ne!(out.code, 0);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("--bogus"), "{}", out.stderr);
}

#[test]
fn bli_toml_without_a_tracker_fails_with_a_hint() {
    let repo = repo(Some("[lane]\nbase = \"main\"\n"));
    let tracker = FakeTracker::default().with_frontier("1", &[]);

    let out = bli(&["frontier", "--parent", "1"], repo.path(), &tracker);

    assert_ne!(out.code, 0);
    assert!(out.stderr.contains("hint:"), "{}", out.stderr);
    assert!(out.stderr.contains("[tracker]"), "{}", out.stderr);
}
