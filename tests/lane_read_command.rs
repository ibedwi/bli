//! Seam 1: the command layer, for `bli lane read`. A fake lane host and real git.

mod common;

use bli::host::StatusHint;
use common::*;
use serde_json::json;

fn fakes_with_output(output: &str) -> Fakes {
    Fakes {
        host: FakeHost::default()
            .with_checkout("ticket/5-fan-in", StatusHint::Idle)
            .with_agent_output("5", output),
        ..Default::default()
    }
}

fn fakes() -> Fakes {
    fakes_with_output("All done. Committed 3 changes.\n")
}

/// Agent output of `n` numbered lines.
fn numbered(n: u32) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

#[test]
fn lane_read_returns_the_lane_agents_recent_output() {
    let repo = Repo::new(GITHUB);

    let out = bli(&["lane", "read", "5", "--json"], repo.path(), &fakes());

    assert_eq!(
        json(&out),
        json!({
            "ticket": "5",
            "branch": "ticket/5-fan-in",
            "status": "idle",
            "output": "All done. Committed 3 changes.\n",
        })
    );
}

#[test]
fn lane_read_prints_the_output_as_is() {
    let repo = Repo::new(GITHUB);

    let out = bli(&["lane", "read", "5"], repo.path(), &fakes());

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "All done. Committed 3 changes.\n");
}

#[test]
fn lane_read_reads_the_last_80_lines_by_default() {
    let repo = Repo::new(GITHUB);

    let out = bli(
        &["lane", "read", "5"],
        repo.path(),
        &fakes_with_output(&numbered(100)),
    );

    assert_eq!(out.stdout.lines().count(), 80);
    assert_eq!(out.stdout.lines().next(), Some("line 21"));
}

#[test]
fn lane_read_takes_the_number_of_lines() {
    let repo = Repo::new(GITHUB);

    let out = bli(
        &["lane", "read", "5", "--lines", "20"],
        repo.path(),
        &fakes_with_output(&numbered(100)),
    );

    assert_eq!(out.stdout.lines().count(), 20);
    assert_eq!(out.stdout.lines().next(), Some("line 81"));
}

#[test]
fn lane_read_fails_for_a_ticket_without_a_lane() {
    let repo = Repo::new(GITHUB);

    let out = bli(&["lane", "read", "9"], repo.path(), &fakes());

    assert_ne!(out.code, 0);
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.contains("no lane for ticket 9"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("bli lane list"), "{}", out.stderr);
}
