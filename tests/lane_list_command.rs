//! Seam 1: the command layer, for `bli lane list`. A fake lane host and real git.

mod common;

use bli::host::StatusHint;
use common::*;
use serde_json::json;

#[test]
fn lane_list_shows_each_lane_with_its_ticket_branch_base_and_status_hint() {
    let repo = Repo::new(GITHUB);
    git(
        repo.path(),
        &["config", "branch.ticket/7-doctor.bliBase", "release"],
    );
    let fakes = Fakes {
        host: FakeHost::default()
            .with_main_checkout("main")
            .with_checkout("ticket/3-fan-out", StatusHint::Working)
            .with_checkout("ticket/7-doctor", StatusHint::Blocked),
        ..Default::default()
    };

    let out = bli(&["lane", "list", "--json"], repo.path(), &fakes);

    assert_eq!(
        json(&out),
        json!({
            "lanes": [
                {"ticket": "3", "branch": "ticket/3-fan-out", "base": "main",
                 "path": "/lanes/ticket-3-fan-out", "workspace": "w11", "status": "working"},
                {"ticket": "7", "branch": "ticket/7-doctor", "base": "release",
                 "path": "/lanes/ticket-7-doctor", "workspace": "w12", "status": "blocked"},
            ]
        })
    );
}

#[test]
fn lane_list_from_inside_a_lane_asks_the_host_about_the_main_checkout() {
    let repo = Repo::new(GITHUB);
    let lane = repo.add_worktree("ticket/3-fan-out");
    let fakes = Fakes::default();

    let out = bli(&["lane", "list", "--json"], &lane, &fakes);

    assert_eq!(json(&out), json!({"lanes": []}));
    assert_eq!(fakes.host.listed_from(), vec![repo.root.clone()]);
}

#[test]
fn lane_list_finds_lanes_by_the_configured_branch_pattern() {
    let repo = Repo::new(&format!(
        "[lane]\nbranch = \"work/{{slug}}.{{id}}\"\n{GITHUB}"
    ));
    let fakes = Fakes {
        host: FakeHost::default()
            .with_checkout("ticket/3-fan-out", StatusHint::Working)
            .with_checkout("work/doctor.CU42", StatusHint::Idle),
        ..Default::default()
    };

    let out = bli(&["lane", "list", "--json"], repo.path(), &fakes);

    let lanes = &json(&out)["lanes"];
    assert_eq!(lanes.as_array().unwrap().len(), 1, "{lanes}");
    assert_eq!(lanes[0]["ticket"], "CU42");
    assert_eq!(lanes[0]["branch"], "work/doctor.CU42");
}

#[test]
fn lane_list_ignores_the_main_checkout_even_on_a_lane_branch() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        host: FakeHost::default().with_main_checkout("ticket/3-fan-out"),
        ..Default::default()
    };

    let out = bli(&["lane", "list", "--json"], repo.path(), &fakes);

    assert_eq!(json(&out), json!({"lanes": []}));
}

#[test]
fn lane_list_prints_a_readable_list() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        host: FakeHost::default().with_checkout("ticket/3-fan-out", StatusHint::Blocked),
        ..Default::default()
    };

    let out = bli(&["lane", "list"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "3  ticket/3-fan-out  (base main, blocked)\n    /lanes/ticket-3-fan-out\n"
    );
}

#[test]
fn a_branch_pattern_without_an_id_is_rejected() {
    let repo = Repo::new(&format!("[lane]\nbranch = \"work/{{slug}}\"\n{GITHUB}"));

    let out = bli(&["lane", "list"], repo.path(), &Fakes::default());

    assert_ne!(out.code, 0);
    assert!(out.stderr.contains("{id}"), "{}", out.stderr);
}
