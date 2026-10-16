//! Seam 1: the command layer, for `bli lane read`. A fake lane host and real git.

mod common;

use bli::host::StatusHint;
use common::*;
use serde_json::json;

fn fakes() -> Fakes {
    Fakes {
        host: FakeHost::default()
            .with_checkout("ticket/5-fan-in", StatusHint::Idle)
            .with_agent_output("5", "All done. Committed 3 changes.\n"),
        ..Default::default()
    }
}

#[test]
fn lane_read_returns_the_lane_agents_recent_output() {
    let repo = Repo::new(GITHUB);
    let fakes = fakes();

    let out = bli(&["lane", "read", "5", "--json"], repo.path(), &fakes);

    assert_eq!(
        json(&out),
        json!({
            "ticket": "5",
            "branch": "ticket/5-fan-in",
            "status": "idle",
            "output": "All done. Committed 3 changes.\n",
        })
    );
    assert_eq!(fakes.host.read(), vec![("5".to_string(), 80)]);
}

#[test]
fn lane_read_prints_the_output_as_is() {
    let repo = Repo::new(GITHUB);

    let out = bli(
        &["lane", "read", "5", "--lines", "20"],
        repo.path(),
        &fakes(),
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "All done. Committed 3 changes.\n");
}

#[test]
fn lane_read_takes_the_number_of_lines() {
    let repo = Repo::new(GITHUB);
    let fakes = fakes();

    bli(&["lane", "read", "5", "--lines", "20"], repo.path(), &fakes);

    assert_eq!(fakes.host.read(), vec![("5".to_string(), 20)]);
}

#[test]
fn lane_read_fails_for_a_ticket_without_a_lane() {
    let repo = Repo::new(GITHUB);
    let fakes = fakes();

    let out = bli(&["lane", "read", "9"], repo.path(), &fakes);

    assert_ne!(out.code, 0);
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.contains("no lane for ticket 9"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("bli lane list"), "{}", out.stderr);
    assert!(fakes.host.read().is_empty());
}
