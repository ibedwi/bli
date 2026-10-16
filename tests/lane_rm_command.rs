//! Seam 1: the command layer, for `bli lane rm`. A fake lane host, and real git
//! in a temporary repo with a local bare remote.

mod common;

use bli::host::StatusHint;
use common::*;
use serde_json::json;

const BRANCH: &str = "ticket/4-remove-lane";

/// A repo with one real lane on [`BRANCH`], known to the fake host.
fn lane() -> (Repo, std::path::PathBuf, Fakes) {
    let repo = Repo::new(GITHUB);
    let path = repo.add_worktree(BRANCH);
    git(
        repo.path(),
        &["config", &format!("branch.{BRANCH}.bliBase"), "main"],
    );
    let fakes = Fakes {
        host: FakeHost::default()
            .with_main_checkout_at(repo.path(), "main")
            .with_checkout_at(&path, BRANCH, StatusHint::Idle),
        ..Default::default()
    };
    (repo, path, fakes)
}

fn branch_exists(repo: &Repo, branch: &str) -> bool {
    !git(repo.path(), &["branch", "--list", branch]).is_empty()
}

#[test]
fn lane_rm_removes_the_lane_through_the_host_and_keeps_its_branch() {
    let (repo, path, fakes) = lane();

    let out = bli(&["lane", "rm", "4", "--json"], repo.path(), &fakes);

    assert_eq!(
        json(&out),
        json!({
            "ticket": "4",
            "branch": BRANCH,
            "path": path,
            "workspace": "w11",
            "forced": false,
            "branch_deleted": false,
        })
    );
    assert_eq!(
        fakes.host.removed(),
        vec![Removal {
            repo: repo.root.clone(),
            path: path.clone(),
            workspace: Some("w11".into()),
            force: false,
        }]
    );
    assert!(!path.exists());
    assert!(branch_exists(&repo, BRANCH));
    assert_eq!(
        git(
            repo.path(),
            &["config", &format!("branch.{BRANCH}.bliBase")]
        ),
        "main"
    );
}

#[test]
fn lane_rm_with_delete_branch_deletes_the_branch_and_its_recorded_base() {
    let (repo, path, fakes) = lane();
    git(&path, &["push", "-q", "origin", BRANCH]);

    let out = bli(
        &["lane", "rm", BRANCH, "--delete-branch", "--json"],
        repo.path(),
        &fakes,
    );

    assert_eq!(json(&out)["branch_deleted"], true);
    assert!(!branch_exists(&repo, BRANCH));
    assert!(!git(repo.path(), &["config", "--list"]).contains("blibase"));
}

/// Asserts `out` is a refusal mentioning `why`, and that nothing was removed.
fn assert_refused(out: &bli::Outcome, fakes: &Fakes, why: &str) {
    assert_eq!(out.code, 1, "stdout: {}", out.stdout);
    assert!(out.stderr.contains(why), "{}", out.stderr);
    assert_eq!(fakes.host.removed(), vec![]);
}

#[test]
fn lane_rm_refuses_uncommitted_changes_without_force() {
    let (repo, path, fakes) = lane();
    std::fs::write(path.join("bli.toml"), "changed").unwrap();

    let out = bli(&["lane", "rm", "4"], repo.path(), &fakes);

    assert_refused(&out, &fakes, "bli.toml");
    assert!(out.stderr.contains("--force"), "{}", out.stderr);
    assert!(path.exists());
}

#[test]
fn lane_rm_refuses_untracked_files_without_force() {
    let (repo, path, fakes) = lane();
    std::fs::write(path.join("notes.md"), "x").unwrap();

    let out = bli(&["lane", "rm", "4"], repo.path(), &fakes);

    assert_refused(&out, &fakes, "notes.md");
    assert!(path.exists());
}

#[test]
fn lane_rm_with_force_removes_a_lane_with_changes() {
    let (repo, path, fakes) = lane();
    std::fs::write(path.join("bli.toml"), "changed").unwrap();
    std::fs::write(path.join("notes.md"), "x").unwrap();

    let out = bli(
        &["lane", "rm", "4", "--force", "--json"],
        repo.path(),
        &fakes,
    );

    assert_eq!(json(&out)["forced"], true);
    assert!(fakes.host.removed()[0].force);
    assert!(!path.exists());
}

#[test]
fn lane_rm_refuses_to_delete_a_branch_with_unpushed_commits_without_force() {
    let (repo, path, fakes) = lane();
    git(
        &path,
        &["commit", "-q", "--allow-empty", "-m", "Local work"],
    );

    let out = bli(&["lane", "rm", "4", "--delete-branch"], repo.path(), &fakes);

    assert_refused(&out, &fakes, "Local work");
    assert!(out.stderr.contains("--force"), "{}", out.stderr);
    assert!(branch_exists(&repo, BRANCH));
}

#[test]
fn lane_rm_keeping_the_branch_removes_a_lane_with_unpushed_commits() {
    let (repo, path, fakes) = lane();
    git(
        &path,
        &["commit", "-q", "--allow-empty", "-m", "Local work"],
    );

    let out = bli(&["lane", "rm", "4", "--json"], repo.path(), &fakes);

    assert_eq!(json(&out)["branch_deleted"], false);
    assert!(!path.exists());
    assert!(branch_exists(&repo, BRANCH));
}

#[test]
fn lane_rm_with_force_deletes_a_branch_with_unpushed_commits() {
    let (repo, path, fakes) = lane();
    git(
        &path,
        &["commit", "-q", "--allow-empty", "-m", "Local work"],
    );

    let out = bli(
        &["lane", "rm", "4", "--delete-branch", "--force", "--json"],
        repo.path(),
        &fakes,
    );

    assert_eq!(json(&out)["branch_deleted"], true);
    assert!(!branch_exists(&repo, BRANCH));
}

#[test]
fn lane_rm_always_refuses_the_main_checkout() {
    let (repo, _path, fakes) = lane();

    for name in [repo.path().to_str().unwrap(), "main", "w1"] {
        let out = bli(&["lane", "rm", name, "--force"], repo.path(), &fakes);
        assert_refused(&out, &fakes, "main checkout");
    }
}

#[test]
fn lane_rm_refuses_the_lane_it_is_run_from() {
    let (_repo, path, fakes) = lane();
    let inside = path.join("src");
    std::fs::create_dir(&inside).unwrap();

    for cwd in [&path, &inside] {
        let out = bli(&["lane", "rm", "4", "--force"], cwd, &fakes);
        assert_refused(&out, &fakes, "running from");
    }
}

#[test]
fn lane_rm_removes_another_lane_when_run_from_a_lane() {
    let (repo, path, fakes) = lane();
    let other = repo.add_worktree("ticket/7-doctor");
    let fakes = Fakes {
        host: fakes
            .host
            .with_checkout_at(&other, "ticket/7-doctor", StatusHint::Idle),
        ..Default::default()
    };

    let out = bli(&["lane", "rm", "4", "--json"], &other, &fakes);

    assert_eq!(json(&out)["ticket"], "4");
    assert!(!path.exists());
}

#[test]
fn lane_rm_fails_outside_the_lane_host() {
    let repo = Repo::new(GITHUB);
    let fakes = Fakes {
        host: FakeHost::outside().with_checkout("ticket/4-remove-lane", StatusHint::Idle),
        ..Default::default()
    };

    let out = bli(&["lane", "rm", "4"], repo.path(), &fakes);

    assert_refused(&out, &fakes, "not inside fakehost");
}

#[test]
fn lane_rm_says_what_it_removed() {
    let (repo, path, fakes) = lane();

    let out = bli(&["lane", "rm", "4"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!(
            "Removed lane 4 at {}; its branch {BRANCH} is kept\n",
            path.display()
        )
    );
}

#[test]
fn lane_rm_counts_commits_whose_remote_branch_was_deleted_as_unpushed() {
    let (repo, path, fakes) = lane();
    git(
        &path,
        &["commit", "-q", "--allow-empty", "-m", "Pushed once"],
    );
    git(&path, &["push", "-q", "origin", BRANCH]);
    // The remote branch goes away, leaving this checkout's tracking ref stale.
    git(&repo.remote, &["branch", "-D", BRANCH]);

    let out = bli(&["lane", "rm", "4", "--delete-branch"], repo.path(), &fakes);

    assert_refused(&out, &fakes, "Pushed once");
    assert!(branch_exists(&repo, BRANCH));
}
