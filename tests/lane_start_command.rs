//! Seam 1: the command layer, for `bli lane start`. A fake lane host and issue
//! tracker, and real git in a temporary repo with a local bare remote.

mod common;

use bli::host::{NewLane, StatusHint};
use common::*;
use serde_json::json;

fn fakes(tickets: &[(&str, &str)]) -> Fakes {
    let tracker = tickets
        .iter()
        .fold(FakeTracker::default(), |t, (id, title)| {
            t.with_ticket(id, title)
        });
    Fakes {
        tracker,
        ..Default::default()
    }
}

#[test]
fn lane_start_creates_a_lane_starts_the_agent_and_claims_the_ticket() {
    let repo = Repo::new(GITHUB);
    let fakes = fakes(&[("3", "Fan-out on herdr: lane start, lane list")]);

    let out = bli(&["lane", "start", "3", "--json"], repo.path(), &fakes);

    let branch = "ticket/3-fan-out-on-herdr-lane-start";
    assert_eq!(
        json(&out),
        json!({
            "base": "main",
            "lanes": [{
                "ticket": "3",
                "title": "Fan-out on herdr: lane start, lane list",
                "url": "https://tracker.test/3",
                "outcome": "started",
                "branch": branch,
                "base": "main",
                "path": "/lanes/ticket-3-fan-out-on-herdr-lane-start",
                "workspace": "w10",
                "status": "working",
            }]
        })
    );
    assert_eq!(
        fakes.host.created(),
        vec![(
            repo.root.clone(),
            NewLane {
                branch: branch.into(),
                base_ref: "origin/main".into(),
                label: "#3 fan-out-on-herdr-lane-start".into(),
            }
        )]
    );
    assert_eq!(
        fakes.host.started(),
        vec![(
            "3".to_string(),
            "/implement https://tracker.test/3".to_string()
        )]
    );
    assert_eq!(fakes.tracker.claimed(), vec!["3"]);
    assert_eq!(
        git(
            repo.path(),
            &["config", &format!("branch.{branch}.bliBase")]
        ),
        "main"
    );
}

#[test]
fn lane_start_fetches_so_lanes_branch_from_the_latest_remote_base() {
    let repo = Repo::new(GITHUB);
    let latest = repo.push_elsewhere("main");
    let fakes = fakes(&[("3", "Fan-out")]);

    let out = bli(&["lane", "start", "3", "--json"], repo.path(), &fakes);

    assert_eq!(json(&out)["lanes"][0]["outcome"], "started");
    assert_eq!(git(repo.path(), &["rev-parse", "origin/main"]), latest);
    assert_eq!(fakes.host.created()[0].1.base_ref, "origin/main");
}

#[test]
fn lane_start_uses_the_base_flag_and_records_it() {
    let repo = Repo::new(GITHUB);
    repo.push_elsewhere("release");
    let fakes = fakes(&[("3", "Fan-out")]);

    let out = bli(
        &["lane", "start", "3", "--base", "release", "--json"],
        repo.path(),
        &fakes,
    );

    assert_eq!(json(&out)["lanes"][0]["base"], "release");
    assert_eq!(fakes.host.created()[0].1.base_ref, "origin/release");
    assert_eq!(
        git(repo.path(), &["config", "branch.ticket/3-fan-out.bliBase"]),
        "release"
    );
}

#[test]
fn lane_start_uses_the_configured_base_and_prompt() {
    let repo = Repo::new(&format!(
        "[lane]\nbase = \"release\"\nprompt = \"/build {{ticket_url}} now\"\n{GITHUB}"
    ));
    repo.push_elsewhere("release");
    let fakes = fakes(&[("3", "Fan-out")]);

    let out = bli(&["lane", "start", "3", "--json"], repo.path(), &fakes);

    assert_eq!(json(&out)["base"], "release");
    assert_eq!(fakes.host.created()[0].1.base_ref, "origin/release");
    assert_eq!(
        fakes.host.started()[0].1,
        "/build https://tracker.test/3 now"
    );
}

#[test]
fn lane_start_from_inside_a_lane_creates_lanes_from_the_main_checkout() {
    let repo = Repo::new(GITHUB);
    let lane = repo.add_worktree("ticket/9-other");
    let fakes = fakes(&[("3", "Fan-out")]);

    let out = bli(&["lane", "start", "3", "--json"], &lane, &fakes);

    assert_eq!(json(&out)["lanes"][0]["outcome"], "started");
    assert_eq!(fakes.host.created()[0].0, repo.root);
}

#[test]
fn lane_start_fails_outside_the_lane_host_without_touching_anything() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        host: FakeHost::outside(),
        ..fakes(&[("3", "Fan-out")])
    };

    let out = bli(&["lane", "start", "3", "--json"], repo.path(), &fakes);

    assert_ne!(out.code, 0);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("not inside fakehost"), "{}", out.stderr);
    assert!(out.stderr.contains("hint:"), "{}", out.stderr);
    assert!(fakes.host.created().is_empty());
    assert!(fakes.tracker.claimed().is_empty());
}

#[test]
fn lane_start_fails_when_the_base_is_not_on_the_remote() {
    let repo = Repo::new(GITHUB);
    git(repo.path(), &["branch", "local-only"]);
    let fakes = fakes(&[("3", "Fan-out")]);

    let out = bli(
        &["lane", "start", "3", "--base", "local-only"],
        repo.path(),
        &fakes,
    );

    assert_ne!(out.code, 0);
    assert!(out.stderr.contains("local-only"), "{}", out.stderr);
    assert!(out.stderr.contains("origin"), "{}", out.stderr);
    assert!(fakes.host.created().is_empty());
}

#[test]
fn lane_start_slug_flag_overrides_the_title_slug() {
    let repo = Repo::new(GITHUB);
    let fakes = fakes(&[("3", "A very long and unwieldy title indeed")]);

    let out = bli(
        &["lane", "start", "3", "--slug", "short-name", "--json"],
        repo.path(),
        &fakes,
    );

    assert_eq!(json(&out)["lanes"][0]["branch"], "ticket/3-short-name");
}

#[test]
fn lane_start_rejects_a_slug_that_is_not_kebab_case() {
    let repo = Repo::new(GITHUB);
    let fakes = fakes(&[("3", "Fan-out")]);

    let out = bli(
        &["lane", "start", "3", "--slug", "Not A Slug"],
        repo.path(),
        &fakes,
    );

    assert_ne!(out.code, 0);
    assert!(out.stderr.contains("Not A Slug"), "{}", out.stderr);
    assert!(fakes.host.created().is_empty());
}

#[test]
fn lane_start_rejects_a_slug_for_more_than_one_ticket() {
    let repo = Repo::new(GITHUB);
    let fakes = fakes(&[("3", "Fan-out"), ("4", "Remove")]);

    let out = bli(
        &["lane", "start", "3", "4", "--slug", "same"],
        repo.path(),
        &fakes,
    );

    assert_ne!(out.code, 0);
    assert!(out.stderr.contains("--slug"), "{}", out.stderr);
    assert!(fakes.host.created().is_empty());
}

#[test]
fn lane_start_skips_tickets_that_already_have_a_lane_or_a_claim() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        tracker: FakeTracker::default()
            .with_ticket("3", "Has a lane")
            .with_assigned_ticket("4", "Claimed", &["someone"])
            .with_ticket("5", "Free"),
        host: FakeHost::default().with_checkout("ticket/3-old-slug", StatusHint::Idle),
    };

    let out = bli(
        &["lane", "start", "3", "4", "5", "--json"],
        repo.path(),
        &fakes,
    );

    let lanes = json(&out)["lanes"].clone();
    assert_eq!(lanes[0]["outcome"], "skipped");
    assert_eq!(lanes[0]["branch"], "ticket/3-old-slug");
    assert!(
        lanes[0]["reason"].as_str().unwrap().contains("lane"),
        "{lanes}"
    );
    assert_eq!(lanes[1]["outcome"], "skipped");
    assert!(
        lanes[1]["reason"].as_str().unwrap().contains("someone"),
        "{lanes}"
    );
    assert_eq!(lanes[2]["outcome"], "started");
    let created: Vec<String> = fakes
        .host
        .created()
        .into_iter()
        .map(|(_, l)| l.branch)
        .collect();
    assert_eq!(created, vec!["ticket/5-free"]);
    assert_eq!(fakes.tracker.claimed(), vec!["5"]);
}

#[test]
fn lane_start_reports_each_lanes_outcome_and_keeps_going_after_a_failure() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        tracker: FakeTracker::default()
            .with_ticket("3", "First")
            .with_ticket("4", "Second"),
        host: FakeHost::default()
            .failing_to_create("ticket/3-first")
            .starting_agents_as(StatusHint::Blocked),
    };

    let out = bli(
        &["lane", "start", "3", "404", "4", "--json"],
        repo.path(),
        &fakes,
    );

    let lanes = json(&out)["lanes"].clone();
    assert_eq!(lanes[0]["ticket"], "3");
    assert_eq!(lanes[0]["outcome"], "failed");
    assert_eq!(lanes[0]["error"], "could not create ticket/3-first");
    assert_eq!(lanes[1]["ticket"], "404");
    assert_eq!(lanes[1]["outcome"], "failed");
    assert_eq!(lanes[1]["error"], "no ticket 404");
    assert_eq!(lanes[2]["outcome"], "started");
    assert_eq!(lanes[2]["status"], "blocked");
    assert_eq!(fakes.tracker.claimed(), vec!["4"]);
}

#[test]
fn lane_start_prints_a_readable_report() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        tracker: FakeTracker::default()
            .with_ticket("3", "Fan-out")
            .with_assigned_ticket("4", "Claimed", &["someone"]),
        ..Default::default()
    };

    let out = bli(&["lane", "start", "3", "4"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "3  started  ticket/3-fan-out  (working)\n4  skipped  already claimed by someone\n"
    );
}

#[test]
fn lane_start_needs_at_least_one_ticket() {
    let repo = Repo::new(GITHUB);

    let out = bli(&["lane", "start"], repo.path(), &Fakes::default());

    assert_ne!(out.code, 0);
    assert_eq!(out.stdout, "");
}

#[test]
fn lane_start_reports_a_lane_whose_agent_failed_to_start_with_its_branch() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        host: FakeHost::default().failing_to_start_agents(),
        ..fakes(&[("3", "Fan-out")])
    };

    let out = bli(&["lane", "start", "3", "--json"], repo.path(), &fakes);

    let lane = json(&out)["lanes"][0].clone();
    assert_eq!(lane["outcome"], "failed");
    assert_eq!(lane["error"], "agent_not_ready");
    assert_eq!(lane["branch"], "ticket/3-fan-out");
    assert_eq!(lane["path"], "/lanes/ticket-3-fan-out");
}

#[test]
fn lane_start_starts_a_ticket_named_twice_only_once() {
    let repo = Repo::new(GITHUB);
    let fakes = fakes(&[("3", "Fan-out")]);

    let out = bli(&["lane", "start", "3", "3", "--json"], repo.path(), &fakes);

    let lanes = json(&out)["lanes"].clone();
    assert_eq!(lanes[0]["outcome"], "started");
    assert_eq!(lanes[1]["outcome"], "skipped");
    assert_eq!(fakes.host.created().len(), 1);
}
