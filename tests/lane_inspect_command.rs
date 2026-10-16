//! Seam 1: the command layer, for `bli lane inspect`. A fake lane host and code
//! host, and real git in a temporary repo with a local bare remote.

mod common;

use bli::code_host::PrState;
use bli::host::StatusHint;
use common::*;
use serde_json::json;

const BRANCH: &str = "ticket/4-remove-lane";

#[test]
fn lane_inspect_reports_a_clean_lane() {
    let repo = Repo::new(GITHUB);
    let path = repo.add_worktree(BRANCH);
    let fakes = Fakes {
        host: FakeHost::default()
            .with_main_checkout_at(repo.path(), "main")
            .with_checkout_at(&path, BRANCH, StatusHint::Idle),
        ..Default::default()
    };

    let out = bli(&["lane", "inspect", "4", "--json"], repo.path(), &fakes);

    assert_eq!(
        json(&out),
        json!({
            "ticket": "4",
            "branch": BRANCH,
            "base": "main",
            "path": path,
            "workspace": "w11",
            "status": "idle",
            "modified": [],
            "untracked": [],
            "unpushed": [],
            "upstream": null,
            "pr": null,
        })
    );
}

#[test]
fn lane_inspect_reports_changed_files_unpushed_commits_upstream_and_pr() {
    let repo = Repo::new(GITHUB);
    let path = repo.add_worktree(BRANCH);
    std::fs::write(path.join("pushed.txt"), "one").unwrap();
    git(&path, &["add", "pushed.txt"]);
    git(&path, &["commit", "-q", "-m", "Pushed work"]);
    git(&path, &["push", "-q", "-u", "origin", BRANCH]);
    git(
        &path,
        &["commit", "-q", "--allow-empty", "-m", "Local work"],
    );
    let local = git(&path, &["rev-parse", "--short", "HEAD"]);
    std::fs::write(path.join("pushed.txt"), "two").unwrap();
    std::fs::create_dir(path.join("notes")).unwrap();
    std::fs::write(path.join("notes/todo.md"), "x").unwrap();
    let fakes = Fakes {
        host: FakeHost::default().with_checkout_at(&path, BRANCH, StatusHint::Working),
        code: FakeCodeHost::default().with_pr(BRANCH, 21, PrState::Open),
        ..Default::default()
    };

    let out = bli(&["lane", "inspect", "4", "--json"], repo.path(), &fakes);

    let report = json(&out);
    assert_eq!(report["modified"], json!(["pushed.txt"]));
    assert_eq!(report["untracked"], json!(["notes/todo.md"]));
    assert_eq!(
        report["unpushed"],
        json!([{"sha": local, "subject": "Local work"}])
    );
    assert_eq!(report["upstream"], format!("origin/{BRANCH}"));
    assert_eq!(
        report["pr"],
        json!({"number": 21, "state": "open", "url": "https://code.test/pull/21"})
    );
    assert_eq!(report["status"], "working");
}

fn three_lanes() -> Fakes {
    Fakes {
        host: FakeHost::default()
            .with_main_checkout("main")
            .with_checkout("ticket/3-fan-out", StatusHint::Working)
            .with_checkout("ticket/4-remove-lane", StatusHint::Idle)
            .with_checkout("ticket/7-doctor", StatusHint::Blocked),
        ..Default::default()
    }
}

/// The branch named in `out`'s `lane inspect --json` report.
fn named_branch(out: &bli::Outcome) -> String {
    json(out)["branch"].as_str().unwrap().to_string()
}

#[test]
fn a_lane_can_be_named_by_ticket_branch_path_or_workspace() {
    let repo = Repo::new(GITHUB);
    let fakes = three_lanes();
    // The fake lanes' checkouts don't exist, so name them through `lane inspect`'s
    // resolution only: a missing checkout fails after the lane was found.
    for name in [
        "4",
        "#4",
        "ticket/4-remove-lane",
        "/lanes/ticket-4-remove-lane",
        "/lanes/ticket-4-remove-lane/",
        "w12",
    ] {
        let out = bli(&["lane", "inspect", name, "--json"], repo.path(), &fakes);
        assert!(
            out.stderr.contains("/lanes/ticket-4-remove-lane"),
            "{name}: {}",
            out.stderr
        );
        assert!(
            !out.stderr.contains("no lane matches"),
            "{name}: {}",
            out.stderr
        );
    }
}

#[test]
fn a_lane_can_be_named_by_a_path_relative_to_the_current_directory() {
    let repo = Repo::new(GITHUB);
    let path = repo.add_worktree(BRANCH);
    let fakes = Fakes {
        host: FakeHost::default().with_checkout_at(&path, BRANCH, StatusHint::Idle),
        ..Default::default()
    };

    let out = bli(
        &["lane", "inspect", "../ticket-4-remove-lane", "--json"],
        repo.path(),
        &fakes,
    );

    assert_eq!(named_branch(&out), BRANCH);
}

#[test]
fn naming_a_lane_that_matches_nothing_fails_listing_every_lane() {
    let repo = Repo::new(GITHUB);

    let out = bli(&["lane", "inspect", "9"], repo.path(), &three_lanes());

    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("no lane matches `9`"), "{}", out.stderr);
    for branch in [
        "ticket/3-fan-out",
        "ticket/4-remove-lane",
        "ticket/7-doctor",
    ] {
        assert!(out.stderr.contains(branch), "{}", out.stderr);
    }
}

#[test]
fn naming_a_lane_when_there_are_none_says_so() {
    let repo = Repo::new(GITHUB);

    let out = bli(&["lane", "inspect", "9"], repo.path(), &Fakes::default());

    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("there are no lanes"), "{}", out.stderr);
}

#[test]
fn naming_more_than_one_lane_fails_listing_the_matches() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        host: FakeHost::default()
            .with_checkout("ticket/4-remove-lane", StatusHint::Idle)
            .with_checkout("ticket/4-remove-lane-again", StatusHint::Idle)
            .with_checkout("ticket/7-doctor", StatusHint::Idle),
        ..Default::default()
    };

    let out = bli(&["lane", "inspect", "4"], repo.path(), &fakes);

    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("`4` matches 2 lanes"), "{}", out.stderr);
    assert!(
        out.stderr.contains("ticket/4-remove-lane-again"),
        "{}",
        out.stderr
    );
    assert!(!out.stderr.contains("ticket/7-doctor"), "{}", out.stderr);
}

#[test]
fn the_main_checkout_is_never_a_lane() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        host: FakeHost::default().with_main_checkout_at(repo.path(), "ticket/4-remove-lane"),
        ..Default::default()
    };

    for name in [
        repo.path().to_str().unwrap(),
        ".",
        "ticket/4-remove-lane",
        "w1",
    ] {
        let out = bli(&["lane", "inspect", name], repo.path(), &fakes);
        assert_eq!(out.code, 1, "{name}");
        assert!(
            out.stderr.contains("main checkout"),
            "{name}: {}",
            out.stderr
        );
    }
}

#[test]
fn lane_inspect_prints_a_readable_report() {
    let repo = Repo::new(GITHUB);
    let path = repo.add_worktree(BRANCH);
    git(
        &path,
        &["commit", "-q", "--allow-empty", "-m", "Local work"],
    );
    let local = git(&path, &["rev-parse", "--short", "HEAD"]);
    std::fs::write(path.join("new.txt"), "x").unwrap();
    let fakes = Fakes {
        host: FakeHost::default().with_checkout_at(&path, BRANCH, StatusHint::Idle),
        code: FakeCodeHost::default().with_pr(BRANCH, 21, PrState::Merged),
        ..Default::default()
    };

    let out = bli(&["lane", "inspect", "4"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!(
            "4  {BRANCH}  (base main, idle)\n    {}\n\
             modified:  none\n\
             untracked: new.txt\n\
             unpushed:  1 commit\n    {local} Local work\n\
             upstream:  none\n\
             PR:        #21 merged  https://code.test/pull/21\n",
            path.display()
        )
    );
}
