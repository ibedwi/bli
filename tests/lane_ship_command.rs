//! Seam 1: the command layer, for `bli lane ship`. A fake lane host, issue
//! tracker and code host, and real git in a temporary repo with a local bare remote.

mod common;

use bli::code_host::{NewPullRequest, PrState};
use bli::host::StatusHint;
use common::*;
use serde_json::json;

const BRANCH: &str = "ticket/5-fan-in";

fn fakes() -> Fakes {
    Fakes {
        tracker: FakeTracker::default().with_ticket("5", "Fan-in ships finished lanes"),
        host: FakeHost::default().with_checkout(BRANCH, StatusHint::Idle),
        ..Default::default()
    }
}

fn ship(repo: &Repo, fakes: &Fakes, extra: &[&str]) -> bli::Outcome {
    let body = repo.write_file("body.md", "## Summary\n\nShips lanes.\n");
    let mut args = vec!["lane", "ship", "5", "--body-file", body.to_str().unwrap()];
    args.extend_from_slice(extra);
    bli(&args, repo.path(), fakes)
}

#[test]
fn lane_ship_pushes_the_branch_and_opens_a_pr_against_the_recorded_base() {
    let repo = Repo::new(GITHUB);
    repo.push_elsewhere("release");
    repo.add_lane_with_commit(BRANCH);
    git(
        repo.path(),
        &["config", &format!("branch.{BRANCH}.bliBase"), "release"],
    );
    let fakes = fakes();

    let out = ship(&repo, &fakes, &["--json"]);

    assert_eq!(
        json(&out),
        json!({
            "ticket": "5",
            "branch": BRANCH,
            "base": "release",
            "outcome": "opened",
            "pr": {"number": 100, "url": "https://code.test/pull/100", "state": "open"},
        })
    );
    assert_eq!(
        fakes.code.pushed(),
        vec![(repo.root.clone(), BRANCH.into())]
    );
    assert_eq!(
        fakes.code.opened(),
        vec![NewPullRequest {
            head: BRANCH.into(),
            base: "release".into(),
            title: "Fan-in ships finished lanes".into(),
            body: "## Summary\n\nShips lanes.\n[closes 5]\n".into(),
        }]
    );
}

#[test]
fn lane_ship_targets_the_configured_base_when_none_was_recorded() {
    let repo = Repo::new(GITHUB);
    repo.add_lane_with_commit(BRANCH);
    let fakes = fakes();

    let out = ship(&repo, &fakes, &["--json"]);

    assert_eq!(json(&out)["base"], "main");
    assert_eq!(fakes.code.opened()[0].base, "main");
}

#[test]
fn lane_ship_takes_a_title() {
    let repo = Repo::new(GITHUB);
    repo.add_lane_with_commit(BRANCH);
    let fakes = fakes();

    let out = ship(&repo, &fakes, &["--title", "Ship lanes from fan-in"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(fakes.code.opened()[0].title, "Ship lanes from fan-in");
}

#[test]
fn lane_ship_refuses_a_lane_with_no_commits_beyond_its_base() {
    let repo = Repo::new(GITHUB);
    repo.add_worktree(BRANCH);
    let fakes = fakes();

    let out = ship(&repo, &fakes, &["--json"]);

    assert_ne!(out.code, 0);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("no commits beyond"), "{}", out.stderr);
    assert!(out.stderr.contains("origin/main"), "{}", out.stderr);
    assert!(fakes.code.pushed().is_empty());
    assert!(fakes.code.opened().is_empty());
}

#[test]
fn lane_ship_counts_commits_against_the_latest_remote_base() {
    let repo = Repo::new(GITHUB);
    repo.add_lane_with_commit(BRANCH);
    // The lane's work landed on main some other way, which this checkout hasn't fetched yet.
    let lane_head = git(repo.path(), &["rev-parse", BRANCH]);
    git(
        repo.path(),
        &[
            "push",
            "-q",
            repo.remote.to_str().unwrap(),
            &format!("{lane_head}:refs/heads/main"),
        ],
    );
    let fakes = fakes();

    let out = ship(&repo, &fakes, &[]);

    assert_ne!(out.code, 0, "stdout: {}", out.stdout);
    assert!(out.stderr.contains("no commits beyond"), "{}", out.stderr);
}

#[test]
fn lane_ship_rerun_reports_the_existing_pr_instead_of_opening_another() {
    let repo = Repo::new(GITHUB);
    repo.add_lane_with_commit(BRANCH);
    let fakes = fakes();

    ship(&repo, &fakes, &["--json"]);
    let out = ship(&repo, &fakes, &["--json"]);

    let report = json(&out);
    assert_eq!(report["outcome"], "existing");
    assert_eq!(report["pr"]["number"], 100);
    assert_eq!(fakes.code.opened().len(), 1);
    assert_eq!(fakes.code.pushed().len(), 1);
}

#[test]
fn lane_ship_reports_a_merged_pr_without_pushing_even_with_no_commits_left() {
    let repo = Repo::new(GITHUB);
    repo.add_worktree(BRANCH);
    let fakes = Fakes {
        code: FakeCodeHost::default().with_pr(BRANCH, 12, PrState::Merged),
        ..fakes()
    };

    let out = ship(&repo, &fakes, &["--json"]);

    let report = json(&out);
    assert_eq!(report["outcome"], "existing");
    assert_eq!(
        report["pr"],
        json!({"number": 12, "url": "https://code.test/pull/12", "state": "merged"})
    );
    assert!(fakes.code.pushed().is_empty());
    assert!(fakes.code.opened().is_empty());
}

#[test]
fn lane_ship_prints_a_readable_report() {
    let repo = Repo::new(GITHUB);
    repo.add_lane_with_commit(BRANCH);

    let out = ship(&repo, &fakes(), &[]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "5  opened  https://code.test/pull/100  (ticket/5-fan-in into main)\n"
    );
}

#[test]
fn lane_ship_fails_for_a_ticket_without_a_lane() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        tracker: FakeTracker::default().with_ticket("9", "Other"),
        ..fakes()
    };
    let body = repo.write_file("body.md", "body");

    let out = bli(
        &["lane", "ship", "9", "--body-file", body.to_str().unwrap()],
        repo.path(),
        &fakes,
    );

    assert_ne!(out.code, 0);
    assert!(
        out.stderr.contains("no lane for ticket 9"),
        "{}",
        out.stderr
    );
    assert!(fakes.code.pushed().is_empty());
}

#[test]
fn lane_ship_fails_without_pushing_when_the_body_file_cannot_be_read() {
    let repo = Repo::new(GITHUB);
    repo.add_lane_with_commit(BRANCH);
    let fakes = fakes();

    let out = bli(
        &["lane", "ship", "5", "--body-file", "/no/such/body.md"],
        repo.path(),
        &fakes,
    );

    assert_ne!(out.code, 0);
    assert!(out.stderr.contains("/no/such/body.md"), "{}", out.stderr);
    assert!(fakes.code.pushed().is_empty());
}

#[test]
fn lane_ship_needs_a_body_file() {
    let repo = Repo::new(GITHUB);

    let out = bli(&["lane", "ship", "5"], repo.path(), &fakes());

    assert_ne!(out.code, 0);
    assert!(out.stderr.contains("--body-file"), "{}", out.stderr);
}
