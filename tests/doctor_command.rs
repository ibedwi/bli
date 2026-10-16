//! Seam 1: the command layer, for `bli doctor`. Fake providers, a real repo, and a fake git for the git check.

mod common;

use bli::check::Check;
use bli::error::Error;
use common::*;
use serde_json::json;

#[test]
fn doctor_reports_git_then_the_trackers_then_the_hosts_checks() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        tracker: FakeTracker::default().with_check(Check::pass("tracker auth", "logged in")),
        host: FakeHost::default().with_check(Check::pass("host", "running")),
        ..Default::default()
    };

    let out = bli(&["doctor", "--json"], repo.path(), &fakes);

    let checks = &json(&out)["checks"];
    assert_eq!(checks[0]["name"], "git");
    assert_eq!(checks[0]["status"], "pass");
    assert_eq!(
        checks[1],
        json!({"name": "tracker auth", "status": "pass", "detail": "logged in"})
    );
    assert_eq!(
        checks[2],
        json!({"name": "host", "status": "pass", "detail": "running"})
    );
    assert_eq!(checks.as_array().unwrap().len(), 3, "{checks}");
}

#[test]
fn doctor_exits_non_zero_when_a_check_fails_but_still_reports_every_check() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        tracker: FakeTracker::default().with_check(Check::warn(
            "tracker auth",
            &Error::new("not logged in").with_hint("log in"),
        )),
        host: FakeHost::default().with_check(Check::pass("host", "running")),
        ..Default::default()
    };

    let out = bli(&["doctor", "--json"], repo.path(), &fakes);

    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "error: 1 of 3 checks failed\n");
    let checks: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(
        checks["checks"][1],
        json!({"name": "tracker auth", "status": "warn", "detail": "not logged in", "hint": "log in"})
    );
    assert_eq!(checks["checks"][2]["status"], "pass");
}

#[test]
fn doctor_prints_each_check_as_pass_or_warn_with_its_hint() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        tracker: FakeTracker::default().with_check(Check::pass("tracker auth", "logged in")),
        host: FakeHost::default().with_check(Check::warn(
            "host",
            &Error::new("not running").with_hint("start it"),
        )),
        ..Default::default()
    };

    let out = bli(&["doctor"], repo.path(), &fakes);

    assert_eq!(out.code, 1);
    assert_eq!(
        out.stdout,
        "pass  git  git version 2.51.0\n\
         pass  tracker auth  logged in\n\
         warn  host  not running\n      hint: start it\n"
    );
}

#[test]
fn doctor_warns_on_a_provider_bli_cannot_build_and_checks_the_rest() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        host: FakeHost::default().with_check(Check::pass("host", "running")),
        tracker_unsupported: true,
        ..Default::default()
    };

    let out = bli(&["doctor", "--json"], repo.path(), &fakes);

    assert_eq!(out.code, 1);
    let checks: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(checks["checks"][1]["name"], "issue tracker");
    assert_eq!(checks["checks"][1]["status"], "warn");
    assert_eq!(checks["checks"][1]["detail"], "unsupported issue tracker");
    assert_eq!(checks["checks"][2]["name"], "host");
}

#[test]
fn doctor_warns_with_an_install_hint_when_git_is_missing() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        git: FakeGit::missing(),
        ..Default::default()
    };

    let out = bli(&["doctor"], repo.path(), &fakes);

    assert_eq!(out.code, 1);
    assert_eq!(
        out.stdout,
        "warn  git  could not run `git`: it is not installed\n      hint: install git (https://git-scm.com)\n"
    );
}

#[test]
fn doctor_warns_on_a_missing_bli_toml_and_still_checks_git() {
    let repo = Repo::new(GITHUB);
    std::fs::remove_file(repo.path().join("bli.toml")).unwrap();
    let fakes = Fakes {
        tracker: FakeTracker::default().with_check(Check::pass("tracker auth", "logged in")),
        host: FakeHost::default().with_check(Check::pass("host", "running")),
        ..Default::default()
    };

    let out = bli(&["doctor", "--json"], repo.path(), &fakes);

    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "error: 1 of 2 checks failed\n");
    let checks: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let checks = checks["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 2, "{checks:?}");
    assert_eq!(
        checks[0],
        json!({"name": "git", "status": "pass", "detail": "git version 2.51.0"})
    );
    assert_eq!(checks[1]["name"], "bli.toml");
    assert_eq!(checks[1]["status"], "warn");
    assert_eq!(checks[1]["detail"], "no bli.toml found in this repo");
    assert!(
        checks[1]["hint"].as_str().unwrap().contains("[tracker]"),
        "{checks:?}"
    );
}

#[test]
fn doctor_indents_every_line_of_a_multi_line_hint() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        host: FakeHost::default().with_check(Check::warn(
            "host",
            &Error::new("not set up").with_hint("add this:\n\n[host]\nkind = \"x\""),
        )),
        ..Default::default()
    };

    let out = bli(&["doctor"], repo.path(), &fakes);

    assert_eq!(
        out.stdout,
        "pass  git  git version 2.51.0\n\
         warn  host  not set up\n      hint: add this:\n\n      [host]\n      kind = \"x\"\n"
    );
}
