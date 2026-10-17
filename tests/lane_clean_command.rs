//! Seam 1: the command layer, for `bli lane clean`. A fake lane host, issue
//! tracker and code host, and real git in a temporary repo with a local bare remote.

mod common;

use bli::code_host::PrState;
use bli::host::StatusHint;
use bli::tracker::OnMerge;
use common::*;
use serde_json::json;

const BRANCH: &str = "ticket/6-lane-clean";

/// A repo with one real lane on [`BRANCH`] holding one commit, known to the fake host.
fn lane(bli_toml: &str) -> (Repo, std::path::PathBuf) {
    let repo = Repo::new(bli_toml);
    let path = repo.add_lane_with_commit(BRANCH);
    git(
        repo.path(),
        &["config", &format!("branch.{BRANCH}.bliBase"), "main"],
    );
    (repo, path)
}

/// Pushes the lane, then squash-merges it into `main` on origin and deletes its
/// remote branch, as GitHub does, leaving its commits on no remote.
fn squash_merge(repo: &Repo, path: &std::path::Path) {
    git(path, &["push", "-q", "origin", BRANCH]);
    repo.push_elsewhere("main");
    git(path, &["push", "-q", "origin", "--delete", BRANCH]);
}

fn fakes(repo: &Repo, path: &std::path::Path, pr: PrState) -> Fakes {
    Fakes {
        host: FakeHost::default()
            .with_main_checkout_at(repo.path(), "main")
            .with_checkout_at(path, BRANCH, StatusHint::Idle),
        code: FakeCodeHost::default().with_pr(BRANCH, 12, pr),
        ..Default::default()
    }
}

fn branch_exists(repo: &Repo, branch: &str) -> bool {
    !git(repo.path(), &["branch", "--list", branch]).is_empty()
}

#[test]
fn lane_clean_removes_a_squash_merged_lane_whose_remote_branch_was_deleted() {
    let (repo, path) = lane(GITHUB);
    squash_merge(&repo, &path);
    let fakes = fakes(&repo, &path, PrState::Merged);

    let out = bli(&["lane", "clean", "--json"], repo.path(), &fakes);

    assert_eq!(
        json(&out)["lanes"],
        json!([{
            "ticket": "6",
            "branch": BRANCH,
            "base": "main",
            "path": path,
            "workspace": "w11",
            "status": "idle",
            "pr": {"number": 12, "url": "https://code.test/pull/12", "state": "merged"},
            "outcome": "cleaned",
            "on_merge": "close",
        }])
    );
    assert_eq!(fakes.host.removed().len(), 1);
    assert!(!path.exists());
    assert!(!branch_exists(&repo, BRANCH));
    assert!(!git(repo.path(), &["config", "--list"]).contains("blibase"));
    assert_eq!(
        fakes.tracker.on_merged(),
        vec![("6".to_string(), OnMerge::Close)]
    );
}

/// Cleans a squash-merged lane under `bli_toml` and returns the on-merge actions the tracker received.
fn on_merge_under(bli_toml: &str) -> (serde_json::Value, Vec<(String, OnMerge)>) {
    let (repo, path) = lane(bli_toml);
    squash_merge(&repo, &path);
    let fakes = fakes(&repo, &path, PrState::Merged);
    let out = bli(&["lane", "clean", "--json"], repo.path(), &fakes);
    (
        json(&out)["lanes"][0]["on_merge"].clone(),
        fakes.tracker.on_merged(),
    )
}

#[test]
fn lane_clean_closes_the_ticket_when_on_merge_is_close() {
    let (reported, applied) = on_merge_under(&format!("{GITHUB}on_merge = \"close\"\n"));
    assert_eq!(reported, json!("close"));
    assert_eq!(applied, vec![("6".to_string(), OnMerge::Close)]);
}

#[test]
fn lane_clean_passes_none_to_the_tracker_when_on_merge_is_none() {
    let (reported, applied) = on_merge_under(&format!("{GITHUB}on_merge = \"none\"\n"));
    assert_eq!(reported, json!("none"));
    assert_eq!(applied, vec![("6".to_string(), OnMerge::None)]);
}

#[test]
fn lane_clean_sets_the_named_status_when_on_merge_is_a_status() {
    let (reported, applied) =
        on_merge_under(&format!("{GITHUB}on_merge = {{ status = \"review\" }}\n"));
    assert_eq!(reported, json!({"status": "review"}));
    assert_eq!(
        applied,
        vec![("6".to_string(), OnMerge::Status("review".into()))]
    );
}

#[test]
fn an_unknown_on_merge_action_is_rejected() {
    let (repo, path) = lane(&format!("{GITHUB}on_merge = \"archive\"\n"));
    let fakes = fakes(&repo, &path, PrState::Merged);

    let out = bli(&["lane", "clean"], repo.path(), &fakes);

    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("archive"), "{}", out.stderr);
    assert_eq!(fakes.host.removed(), vec![]);
}

#[test]
fn lane_clean_reports_a_lane_closed_without_merging_and_leaves_it_alone() {
    let (repo, path) = lane(GITHUB);
    git(&path, &["push", "-q", "origin", BRANCH]);
    let fakes = fakes(&repo, &path, PrState::Closed);

    let out = bli(&["lane", "clean", "--json"], repo.path(), &fakes);

    let lanes = &json(&out)["lanes"];
    assert_eq!(lanes[0]["outcome"], "left");
    assert_eq!(lanes[0]["pr"]["state"], "closed");
    assert_eq!(lanes[0].get("on_merge"), None);
    assert_eq!(fakes.host.removed(), vec![]);
    assert_eq!(fakes.tracker.on_merged(), vec![]);
    assert!(path.exists());
    assert!(branch_exists(&repo, BRANCH));
}

#[test]
fn lane_clean_leaves_out_lanes_with_an_open_pr_or_none() {
    let repo = Repo::new(GITHUB);
    let open = repo.add_lane_with_commit("ticket/7-open");
    let unshipped = repo.add_lane_with_commit("ticket/8-unshipped");
    let fakes = Fakes {
        host: FakeHost::default()
            .with_main_checkout_at(repo.path(), "main")
            .with_checkout_at(&open, "ticket/7-open", StatusHint::Idle)
            .with_checkout_at(&unshipped, "ticket/8-unshipped", StatusHint::Working),
        code: FakeCodeHost::default().with_pr("ticket/7-open", 13, PrState::Open),
        ..Default::default()
    };

    let out = bli(&["lane", "clean", "--json"], repo.path(), &fakes);

    assert_eq!(json(&out)["lanes"], json!([]));
    assert_eq!(fakes.host.removed(), vec![]);
    assert_eq!(fakes.tracker.on_merged(), vec![]);
}

#[test]
fn lane_clean_says_so_when_nothing_has_merged_or_closed() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        host: FakeHost::default().with_main_checkout_at(repo.path(), "main"),
        ..Default::default()
    };

    let out = bli(&["lane", "clean"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.contains("No merged or closed lanes"),
        "{}",
        out.stdout
    );
}

#[test]
fn lane_clean_refuses_a_merged_lane_with_uncommitted_files_and_leaves_its_ticket() {
    let (repo, path) = lane(GITHUB);
    squash_merge(&repo, &path);
    std::fs::write(path.join("notes.md"), "x").unwrap();
    let fakes = fakes(&repo, &path, PrState::Merged);

    let out = bli(&["lane", "clean", "--json"], repo.path(), &fakes);

    assert_eq!(out.code, 1);
    let report: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(report["lanes"][0]["outcome"], "failed");
    assert!(out.stderr.contains("notes.md"), "{}", out.stderr);
    assert!(out.stderr.contains("bli lane rm"), "{}", out.stderr);
    assert_eq!(fakes.host.removed(), vec![]);
    assert_eq!(fakes.tracker.on_merged(), vec![]);
    assert!(branch_exists(&repo, BRANCH));
}

#[test]
fn lane_clean_leaves_the_lane_it_is_running_from() {
    let (repo, path) = lane(GITHUB);
    squash_merge(&repo, &path);
    let fakes = fakes(&repo, &path, PrState::Merged);

    let out = bli(&["lane", "clean", "--json"], &path, &fakes);

    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("running from"), "{}", out.stderr);
    assert_eq!(fakes.host.removed(), vec![]);
}

#[test]
fn lane_clean_fast_forwards_the_main_checkout_on_the_base_despite_uncommitted_changes() {
    let (repo, path) = lane(GITHUB);
    squash_merge(&repo, &path);
    std::fs::write(repo.path().join("scratch.txt"), "x").unwrap();
    std::fs::write(repo.path().join("bli.toml"), format!("{GITHUB}# edited\n")).unwrap();
    let fakes = fakes(&repo, &path, PrState::Merged);

    let out = bli(&["lane", "clean", "--json"], repo.path(), &fakes);

    assert_eq!(json(&out)["fast_forwarded"], "main");
    assert_eq!(
        git(repo.path(), &["rev-parse", "HEAD"]),
        git(repo.path(), &["rev-parse", "origin/main"])
    );
    assert!(std::fs::read_to_string(repo.path().join("bli.toml"))
        .unwrap()
        .contains("# edited"));
    assert!(repo.path().join("scratch.txt").exists());
}

#[test]
fn lane_clean_leaves_the_main_checkout_alone_when_it_is_off_the_base() {
    let (repo, path) = lane(GITHUB);
    squash_merge(&repo, &path);
    git(repo.path(), &["checkout", "-q", "-b", "spike"]);
    let before = git(repo.path(), &["rev-parse", "HEAD"]);
    let fakes = fakes(&repo, &path, PrState::Merged);

    let out = bli(&["lane", "clean", "--json"], repo.path(), &fakes);

    assert_eq!(json(&out)["fast_forwarded"], json!(null));
    assert_eq!(git(repo.path(), &["rev-parse", "HEAD"]), before);
    assert_eq!(git(repo.path(), &["rev-parse", "main"]), before);
}

#[test]
fn lane_clean_leaves_the_main_checkout_alone_when_nothing_was_cleaned() {
    let (repo, path) = lane(GITHUB);
    repo.push_elsewhere("main");
    let before = git(repo.path(), &["rev-parse", "HEAD"]);
    let fakes = fakes(&repo, &path, PrState::Open);

    let out = bli(&["lane", "clean", "--json"], repo.path(), &fakes);

    assert_eq!(json(&out)["fast_forwarded"], json!(null));
    assert_eq!(git(repo.path(), &["rev-parse", "HEAD"]), before);
}

#[test]
fn lane_clean_reports_a_main_checkout_that_cannot_fast_forward() {
    let (repo, path) = lane(GITHUB);
    squash_merge(&repo, &path);
    git(
        repo.path(),
        &["commit", "-q", "--allow-empty", "-m", "local"],
    );
    let fakes = fakes(&repo, &path, PrState::Merged);

    let out = bli(&["lane", "clean", "--json"], repo.path(), &fakes);

    assert_eq!(out.code, 1);
    let report: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(report["lanes"][0]["outcome"], "cleaned");
    assert_eq!(report["fast_forwarded"], json!(null));
    assert!(out.stderr.contains("fast-forward"), "{}", out.stderr);
}
