//! Seam 1: the command layer. Runs `bli` commands against a fake issue tracker,
//! in a temporary git repo with a real `bli.toml`, and checks output and exit code.

mod common;

use bli::host::StatusHint;
use common::*;
use serde_json::json;

/// A repo with the given `bli.toml`, or none.
fn repo(bli_toml: Option<&str>) -> Repo {
    let repo = Repo::new(bli_toml.unwrap_or(GITHUB));
    if bli_toml.is_none() {
        std::fs::remove_file(repo.path().join("bli.toml")).unwrap();
    }
    repo
}

fn with_tracker(tracker: FakeTracker) -> Fakes {
    Fakes {
        tracker,
        ..Default::default()
    }
}

#[test]
fn frontier_json_lists_each_tickets_id_title_and_url() {
    let repo = repo(Some(GITHUB));
    let fakes = with_tracker(
        FakeTracker::default().with_frontier("1", &[("3", "Fan-out"), ("7", "Doctor")]),
    );

    let out = bli(
        &["frontier", "--parent", "1", "--json"],
        repo.path(),
        &fakes,
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
    let fakes = with_tracker(
        FakeTracker::default().with_frontier("1", &[("3", "Fan-out"), ("7", "Doctor")]),
    );

    let out = bli(&["frontier", "--parent", "1"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "3  Fan-out\n    https://tracker.test/3\n7  Doctor\n    https://tracker.test/7\n"
    );
}

#[test]
fn frontier_says_so_when_nothing_can_start() {
    let repo = repo(Some(GITHUB));
    let fakes = with_tracker(FakeTracker::default().with_frontier("1", &[]));

    let out = bli(&["frontier", "--parent", "1"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "No tickets on the frontier of 1.\n");
}

#[test]
fn frontier_uses_the_configured_parent_when_none_is_given() {
    let repo = repo(Some("[tracker]\nkind = \"github\"\nparent = \"17\"\n"));
    let fakes = with_tracker(
        FakeTracker::default()
            .with_frontier("1", &[("3", "Wrong effort")])
            .with_frontier("17", &[("20", "Right effort")]),
    );

    let out = bli(&["frontier", "--json"], repo.path(), &fakes);

    assert_eq!(json(&out)["parent"], "17");
    assert_eq!(json(&out)["tickets"][0]["id"], "20");
}

#[test]
fn frontier_accepts_a_numeric_configured_parent() {
    let repo = repo(Some("[tracker]\nkind = \"github\"\nparent = 17\n"));
    let fakes = with_tracker(FakeTracker::default().with_frontier("17", &[("20", "Right effort")]));

    let out = bli(&["frontier", "--json"], repo.path(), &fakes);

    assert_eq!(json(&out)["parent"], "17");
}

#[test]
fn frontier_parent_flag_overrides_the_configured_parent() {
    let repo = repo(Some("[tracker]\nkind = \"github\"\nparent = \"17\"\n"));
    let fakes = with_tracker(
        FakeTracker::default()
            .with_frontier("1", &[("3", "Chosen")])
            .with_frontier("17", &[("20", "Default")]),
    );

    let out = bli(
        &["frontier", "--parent", "1", "--json"],
        repo.path(),
        &fakes,
    );

    assert_eq!(json(&out)["tickets"][0]["id"], "3");
}

#[test]
fn frontier_without_any_parent_fails_with_a_hint() {
    let repo = repo(Some(GITHUB));
    let fakes = with_tracker(FakeTracker::default());

    let out = bli(&["frontier", "--json"], repo.path(), &fakes);

    assert_ne!(out.code, 0);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("--parent"), "{}", out.stderr);
    assert!(out.stderr.contains("[tracker]"), "{}", out.stderr);
}

#[test]
fn any_command_without_bli_toml_fails_with_a_hint() {
    let repo = repo(None);
    let fakes = with_tracker(FakeTracker::default().with_frontier("1", &[]));

    let out = bli(&["frontier", "--parent", "1"], repo.path(), &fakes);

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
    let fakes = with_tracker(FakeTracker::default().with_frontier("1", &[("3", "Fan-out")]));

    let out = bli(&["frontier", "--parent", "1", "--json"], &sub, &fakes);

    assert_eq!(json(&out)["tickets"][0]["id"], "3");
}

#[test]
fn bli_toml_outside_the_checkout_is_not_used() {
    let outer = repo(Some(GITHUB));
    let inner = outer.path().join("nested-repo");
    git(outer.path(), &["init", "-q", "nested-repo"]);
    let fakes = with_tracker(FakeTracker::default().with_frontier("1", &[]));

    let out = bli(&["frontier", "--parent", "1"], &inner, &fakes);

    assert_ne!(out.code, 0);
    assert!(out.stderr.contains("bli.toml"), "{}", out.stderr);
}

#[test]
fn an_unknown_tracker_kind_is_rejected() {
    let repo = repo(Some("[tracker]\nkind = \"jira\"\n"));
    let fakes = with_tracker(FakeTracker::default().with_frontier("1", &[]));

    let out = bli(&["frontier", "--parent", "1"], repo.path(), &fakes);

    assert_ne!(out.code, 0);
    assert!(out.stderr.contains("jira"), "{}", out.stderr);
}

#[test]
fn tracker_errors_go_to_stderr_with_a_nonzero_exit_even_with_json() {
    let repo = repo(Some(GITHUB));
    let fakes = with_tracker(FakeTracker::default());

    let out = bli(
        &["frontier", "--parent", "404", "--json"],
        repo.path(),
        &fakes,
    );

    assert_ne!(out.code, 0);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "error: no ticket 404\n");
}

#[test]
fn unknown_arguments_fail_on_stderr() {
    let repo = repo(Some(GITHUB));
    let fakes = with_tracker(FakeTracker::default());

    let out = bli(&["frontier", "--bogus"], repo.path(), &fakes);

    assert_ne!(out.code, 0);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("--bogus"), "{}", out.stderr);
}

#[test]
fn bli_toml_without_a_tracker_fails_with_a_hint() {
    let repo = repo(Some("[lane]\nbase = \"main\"\n"));
    let fakes = with_tracker(FakeTracker::default().with_frontier("1", &[]));

    let out = bli(&["frontier", "--parent", "1"], repo.path(), &fakes);

    assert_ne!(out.code, 0);
    assert!(out.stderr.contains("hint:"), "{}", out.stderr);
    assert!(out.stderr.contains("[tracker]"), "{}", out.stderr);
}

#[test]
fn frontier_leaves_out_tickets_that_already_have_a_lane() {
    let repo = repo(Some(GITHUB));
    let fakes = Fakes {
        tracker: FakeTracker::default().with_frontier("1", &[("3", "Fan-out"), ("7", "Doctor")]),
        host: FakeHost::default()
            .with_main_checkout("main")
            .with_checkout("ticket/3-fan-out", StatusHint::Working)
            .with_checkout("scratch/7-not-a-lane", StatusHint::Idle),
        ..Default::default()
    };

    let out = bli(
        &["frontier", "--parent", "1", "--json"],
        repo.path(),
        &fakes,
    );

    assert_eq!(
        json(&out)["tickets"],
        json!([
            {"id": "7", "title": "Doctor", "url": "https://tracker.test/7"}
        ])
    );
}
