//! Seam 2: the Superset adapter. Checks that each lane host operation becomes the
//! right `superset` (and `git`) command lines and that Superset's JSON parses correctly.

use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};

use bli::check::{Check, CheckStatus};
use bli::host::{CreatedLane, HostCheckout, LaneHost, NewLane, StatusHint};
use bli::process::{Output, ProcessRunner};
use bli::superset::Superset;

/// Records every command and answers each with the canned output of the longest
/// registered prefix of its command line, e.g. `superset workspaces create`.
#[derive(Default)]
struct FakeRunner {
    replies: Vec<(String, Output)>,
    calls: RefCell<Vec<Vec<String>>>,
}

impl FakeRunner {
    fn replying(self, command: &str, stdout: &str) -> Self {
        self.reply(command, true, stdout, "")
    }

    fn failing(self, command: &str, stderr: &str) -> Self {
        self.reply(command, false, "", stderr)
    }

    fn reply(mut self, command: &str, success: bool, stdout: &str, stderr: &str) -> Self {
        self.replies.push((
            command.to_string(),
            Output {
                success,
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
            },
        ));
        self
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.borrow().clone()
    }
}

impl ProcessRunner for FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<Output> {
        let mut call = vec![program.to_string()];
        call.extend(args.iter().map(|a| a.to_string()));
        let line = call.join(" ");
        self.calls.borrow_mut().push(call);
        self.replies
            .iter()
            .filter(|(prefix, _)| line.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(_, out)| out.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no {program}")))
    }
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/superset/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

const REPO: &str = "/home/ibedwi/github/bli";
const PROJECT: &str = "5b0f6c2e-8f4a-4d0e-9c1b-2a7d3e9f1c40";
const LANE_WS: &str = "0d4e2b7a-9c1f-4a3e-8b6d-5f0e7c2a1d93";
const LANE_PATH: &str = "/home/ibedwi/.superset/worktrees/bli/ticket/3-fan-out";
const AGENT_TERMINAL: &str = "b2e0c4a7-5d1f-4b8e-9a3c-7f6d0e1b2a54";

fn superset(runner: &FakeRunner) -> Superset<&FakeRunner> {
    Superset::new(runner, Some("c2f7a9e1-4d3b-4c8e-9a0f-6b1d2e5c7a38"))
}

fn new_lane() -> NewLane {
    NewLane {
        branch: "ticket/3-fan-out".into(),
        base_ref: "origin/main".into(),
        label: "#3 fan-out".into(),
    }
}

fn creating() -> FakeRunner {
    FakeRunner::default()
        .replying("git", "")
        .replying("superset projects list", &fixture("projects_list.json"))
        .replying(
            "superset workspaces create",
            &fixture("workspaces_create.json"),
        )
        .replying("superset workspaces get", &fixture("workspaces_get.json"))
}

#[test]
fn is_inside_only_in_a_superset_terminal() {
    let runner = FakeRunner::default();
    assert!(superset(&runner).is_inside());
    assert!(!Superset::new(&runner, None).is_inside());
    assert!(!Superset::new(&runner, Some("")).is_inside());
    assert_eq!(Superset::new(&runner, None).name(), "superset");
}

#[test]
fn current_workspace_is_the_one_superset_says_this_terminal_is_in() {
    let runner = FakeRunner::default();
    assert_eq!(
        superset(&runner).current_workspace(),
        Some("c2f7a9e1-4d3b-4c8e-9a0f-6b1d2e5c7a38")
    );
    assert_eq!(Superset::new(&runner, None).current_workspace(), None);
}

#[test]
fn create_lane_cuts_the_exact_branch_from_the_base_then_opens_a_workspace_on_it() {
    let runner = creating();

    superset(&runner)
        .create_lane(Path::new(REPO), &new_lane())
        .unwrap();

    assert_eq!(
        runner.calls(),
        vec![
            vec!["superset", "projects", "list", "--local", "--json"],
            vec![
                "git",
                "-C",
                REPO,
                "branch",
                "--no-track",
                "ticket/3-fan-out",
                "origin/main",
            ],
            vec![
                "superset",
                "workspaces",
                "create",
                "--local",
                "--project",
                PROJECT,
                "--name",
                "#3 fan-out",
                "--branch",
                "ticket/3-fan-out",
                "--skip-branch-prefix",
                "--json",
            ],
            vec![
                "superset",
                "workspaces",
                "get",
                LANE_WS,
                "--local",
                "--json"
            ],
        ]
    );
}

#[test]
fn create_lane_reads_the_checkout_and_workspace() {
    let runner = creating();

    let lane = superset(&runner)
        .create_lane(Path::new(REPO), &new_lane())
        .unwrap();

    assert_eq!(
        lane,
        CreatedLane {
            path: PathBuf::from(LANE_PATH),
            workspace: LANE_WS.into(),
            handle: LANE_WS.into(),
        }
    );
}

#[test]
fn create_lane_deletes_the_branch_it_cut_when_superset_fails_and_reports_supersets_hint() {
    let runner = creating().failing(
        "superset workspaces create",
        &fixture("error_not_logged_in.txt"),
    );

    let err = superset(&runner)
        .create_lane(Path::new(REPO), &new_lane())
        .unwrap_err();

    assert_eq!(
        err.to_string(),
        "`superset workspaces create` failed: Not logged in"
    );
    assert_eq!(
        err.hint(),
        Some("Run: superset auth login (or set SUPERSET_API_KEY)")
    );
    assert_eq!(
        runner.calls().last().unwrap(),
        &vec!["git", "-C", REPO, "branch", "-D", "ticket/3-fan-out"]
    );
}

#[test]
fn create_lane_fails_with_a_hint_when_the_repo_is_not_a_superset_project() {
    let runner = creating();

    let err = superset(&runner)
        .create_lane(Path::new("/home/ibedwi/github/other"), &new_lane())
        .unwrap_err();

    assert!(err.to_string().contains("not a Superset project"), "{err}");
    assert!(
        err.hint().unwrap().contains("superset projects create"),
        "{err:?}"
    );
    assert_eq!(runner.calls().len(), 1, "nothing created");
}

#[test]
fn a_missing_superset_is_reported_with_a_hint() {
    let err = superset(&FakeRunner::default())
        .create_lane(Path::new(REPO), &new_lane())
        .unwrap_err();

    assert!(err.hint().unwrap().contains("superset.sh"), "{err:?}");
}

fn created() -> CreatedLane {
    CreatedLane {
        path: PathBuf::from(LANE_PATH),
        workspace: LANE_WS.into(),
        handle: LANE_WS.into(),
    }
}

fn starting() -> FakeRunner {
    FakeRunner::default()
        .replying("superset agents create", &fixture("agents_create.json"))
        .replying("superset agents wait", &fixture("agents_wait.json"))
}

#[test]
fn start_agent_launches_supersets_claude_preset_with_the_prompt_and_waits_for_it() {
    let runner = starting();

    let status = superset(&runner)
        .start_agent(
            &created(),
            "3",
            "/implement https://github.com/o/r/issues/3",
        )
        .unwrap();

    assert_eq!(status, StatusHint::Working);
    assert_eq!(
        runner.calls(),
        vec![
            vec![
                "superset",
                "agents",
                "create",
                "--local",
                "--workspace",
                LANE_WS,
                "--agent",
                "claude",
                "--prompt",
                "/implement https://github.com/o/r/issues/3",
                "--json",
            ],
            vec![
                "superset",
                "agents",
                "wait",
                "--local",
                "--workspace",
                LANE_WS,
                "--terminal",
                AGENT_TERMINAL,
                "--until",
                "working",
                "--timeout",
                "20",
                "--json",
            ],
        ]
    );
}

#[test]
fn start_agent_reports_the_state_an_agent_that_never_started_working_was_left_in() {
    let runner = starting().failing("superset agents wait", &fixture("error_wait_timeout.txt"));

    let status = superset(&runner)
        .start_agent(&created(), "3", "go")
        .unwrap();

    assert_eq!(status, StatusHint::Blocked);
}

#[test]
fn start_agent_fails_without_waiting_when_the_agent_does_not_launch() {
    let runner = starting().failing(
        "superset agents create",
        &fixture("error_not_logged_in.txt"),
    );

    let err = superset(&runner)
        .start_agent(&created(), "3", "go")
        .unwrap_err();

    assert!(err.to_string().contains("Not logged in"), "{err}");
    assert_eq!(runner.calls().len(), 1);
}

#[test]
fn start_agent_fails_when_the_agent_exits_while_waiting_for_it() {
    let runner = starting().failing(
        "superset agents wait",
        &format!("Error: No agent is running in terminal {AGENT_TERMINAL}\nHint: It exited, or the terminal was closed\n"),
    );

    let err = superset(&runner)
        .start_agent(&created(), "3", "go")
        .unwrap_err();

    assert!(err.to_string().contains("No agent is running"), "{err}");
}

fn listing(snapshot: &str) -> FakeRunner {
    FakeRunner::default()
        .replying("superset projects list", &fixture("projects_list.json"))
        .replying("superset workspaces list", &fixture("workspaces_list.json"))
        .replying("superset snapshot", snapshot)
}

#[test]
fn list_reads_every_workspace_of_the_project_with_its_agents_status() {
    let runner = listing(&fixture("snapshot.json"));

    let checkouts = superset(&runner).list(Path::new(REPO)).unwrap();

    assert_eq!(
        runner.calls(),
        vec![
            vec!["superset", "projects", "list", "--local", "--json"],
            vec![
                "superset",
                "workspaces",
                "list",
                "--local",
                "--project",
                PROJECT,
                "--json"
            ],
            vec!["superset", "snapshot", "--json"],
        ]
    );
    assert_eq!(
        checkouts,
        vec![
            HostCheckout {
                path: PathBuf::from(REPO),
                branch: Some("main".into()),
                is_main: true,
                workspace: Some("c2f7a9e1-4d3b-4c8e-9a0f-6b1d2e5c7a38".into()),
                status: StatusHint::Unknown,
            },
            HostCheckout {
                path: PathBuf::from(LANE_PATH),
                branch: Some("ticket/3-fan-out".into()),
                is_main: false,
                workspace: Some(LANE_WS.into()),
                status: StatusHint::Working,
            },
            HostCheckout {
                path: PathBuf::from("/home/ibedwi/.superset/worktrees/bli/spike/try-superset"),
                branch: Some("spike/try-superset".into()),
                is_main: false,
                workspace: Some("9a6b3d0e-1f2c-4e7a-8d5b-c4f1a0e2b796".into()),
                status: StatusHint::Unknown,
            },
        ]
    );
}

#[test]
fn list_maps_superset_agent_states_to_status_hints() {
    for (state, hint) in [
        ("working", StatusHint::Working),
        ("blocked", StatusHint::Blocked),
        ("idle", StatusHint::Idle),
        ("starting", StatusHint::Unknown),
    ] {
        let snapshot = fixture("snapshot.json")
            .replace(r#""state": "working""#, &format!(r#""state": "{state}""#));
        let runner = listing(&snapshot);

        let checkouts = superset(&runner).list(Path::new(REPO)).unwrap();

        assert_eq!(checkouts[1].status, hint, "superset state {state}");
    }
}

#[test]
fn read_agent_reads_the_screen_of_the_lanes_agent_terminal() {
    let runner = FakeRunner::default()
        .replying("superset snapshot", &fixture("snapshot.json"))
        .replying("superset terminals read", &fixture("terminals_read.json"));

    let output = superset(&runner)
        .read_agent("3", Some(LANE_WS), 60)
        .unwrap();

    assert_eq!(
        runner.calls(),
        vec![
            vec!["superset", "snapshot", "--workspace", LANE_WS, "--json"],
            vec![
                "superset",
                "terminals",
                "read",
                "--local",
                "--workspace",
                LANE_WS,
                "--terminal",
                AGENT_TERMINAL,
                "--max-lines",
                "60",
                "--json",
            ],
        ]
    );
    assert_eq!(output, "● Running the test suite\n  ⎿  cargo test\n\n> \n");
}

#[test]
fn read_agent_fails_for_a_lane_with_no_agent() {
    let runner = FakeRunner::default().replying("superset snapshot", &fixture("snapshot.json"));

    let err = superset(&runner)
        .read_agent("9", Some("9a6b3d0e-1f2c-4e7a-8d5b-c4f1a0e2b796"), 60)
        .unwrap_err();

    assert!(err.to_string().contains("no agent"), "{err}");
}

#[test]
fn read_agent_fails_for_a_lane_with_no_workspace() {
    let err = superset(&FakeRunner::default())
        .read_agent("3", None, 60)
        .unwrap_err();

    assert!(err.to_string().contains("no Superset workspace"), "{err}");
}

fn removing() -> FakeRunner {
    FakeRunner::default().replying("git", "").replying(
        "superset workspaces archive",
        &fixture("workspaces_archive.json"),
    )
}

#[test]
fn remove_lane_archives_a_clean_lanes_workspace_which_removes_its_checkout() {
    let runner = removing();

    superset(&runner)
        .remove_lane(Path::new(REPO), Path::new(LANE_PATH), Some(LANE_WS), false)
        .unwrap();

    assert_eq!(
        runner.calls(),
        vec![
            vec![
                "git",
                "-C",
                LANE_PATH,
                "status",
                "--porcelain",
                "--untracked-files=all"
            ],
            vec![
                "superset",
                "workspaces",
                "archive",
                LANE_WS,
                "--local",
                "--json"
            ],
        ]
    );
}

#[test]
fn remove_lane_refuses_a_lane_with_uncommitted_or_untracked_files_without_force() {
    let runner = removing().replying("git", " M src/lib.rs\n?? notes.txt\n");

    let err = superset(&runner)
        .remove_lane(Path::new(REPO), Path::new(LANE_PATH), Some(LANE_WS), false)
        .unwrap_err();

    assert!(
        err.to_string().contains("uncommitted or untracked"),
        "{err}"
    );
    assert_eq!(runner.calls().len(), 1, "nothing archived");
}

#[test]
fn remove_lane_with_force_archives_without_looking_at_the_checkout() {
    let runner = removing();

    superset(&runner)
        .remove_lane(Path::new(REPO), Path::new(LANE_PATH), Some(LANE_WS), true)
        .unwrap();

    assert_eq!(
        runner.calls(),
        vec![vec![
            "superset",
            "workspaces",
            "archive",
            LANE_WS,
            "--local",
            "--json"
        ]]
    );
}

#[test]
fn remove_lane_fails_when_superset_leaves_the_checkout_behind() {
    let runner = removing().replying(
        "superset workspaces archive",
        &format!(
            r#"{{"deleted":["{LANE_WS}"],"warnings":["{LANE_WS}: Skipped worktree removal at {LANE_PATH}: project metadata is missing"]}}"#
        ),
    );

    let err = superset(&runner)
        .remove_lane(Path::new(REPO), Path::new(LANE_PATH), Some(LANE_WS), true)
        .unwrap_err();

    assert!(
        err.to_string().contains("Skipped worktree removal"),
        "{err}"
    );
}

#[test]
fn remove_lane_ignores_warnings_that_leave_nothing_behind() {
    let runner = removing().replying(
        "superset workspaces archive",
        &format!(r#"{{"deleted":["{LANE_WS}"],"warnings":["{LANE_WS}: 1 terminal(s) may still be running"]}}"#),
    );

    superset(&runner)
        .remove_lane(Path::new(REPO), Path::new(LANE_PATH), Some(LANE_WS), true)
        .unwrap();
}

#[test]
fn remove_lane_fails_for_a_lane_with_no_workspace() {
    let err = superset(&removing())
        .remove_lane(Path::new(REPO), Path::new(LANE_PATH), None, true)
        .unwrap_err();

    assert!(err.to_string().contains("no Superset workspace"), "{err}");
}

fn check_summary(checks: &[Check]) -> Vec<(&str, CheckStatus)> {
    checks.iter().map(|c| (c.name.as_str(), c.status)).collect()
}

fn checking(agents: &str) -> FakeRunner {
    FakeRunner::default()
        .replying("superset --version", "1.38.0\n")
        .replying("superset status", &fixture("status_running.json"))
        .replying("superset agents list", agents)
}

#[test]
fn checks_pass_when_superset_runs_and_its_claude_preset_bypasses_permissions() {
    let runner = checking(&fixture("agents_list.json"));

    let checks = superset(&runner).checks();

    assert_eq!(
        runner.calls(),
        vec![
            vec!["superset", "--version"],
            vec!["superset", "status", "--json"],
            vec!["superset", "agents", "list", "--local", "--json"],
        ]
    );
    assert_eq!(
        check_summary(&checks),
        [
            ("superset", CheckStatus::Pass),
            ("superset host", CheckStatus::Pass),
            ("permission mode", CheckStatus::Pass),
        ]
    );
    assert_eq!(checks[0].detail, "superset 1.38.0");
    assert!(
        checks[2].detail.contains("--dangerously-skip-permissions"),
        "{checks:?}"
    );
}

#[test]
fn the_permission_mode_check_accepts_the_bypass_permissions_mode_flag() {
    for args in [
        r#"["--permission-mode", "bypassPermissions"]"#,
        r#"["--permission-mode=bypassPermissions"]"#,
    ] {
        let agents = fixture("agents_list.json").replace(
            r#""args": ["--dangerously-skip-permissions"]"#,
            &format!(r#""args": {args}"#),
        );

        let checks = superset(&checking(&agents)).checks();

        assert_eq!(checks[2].status, CheckStatus::Pass, "{args}: {checks:?}");
    }
}

#[test]
fn the_permission_mode_check_warns_when_the_claude_preset_does_not_bypass_permissions() {
    let agents = fixture("agents_list.json").replace(
        r#""args": ["--dangerously-skip-permissions"]"#,
        r#""args": ["--permission-mode", "acceptEdits"]"#,
    );

    let checks = superset(&checking(&agents)).checks();

    assert_eq!(checks[2].name, "permission mode");
    assert_eq!(checks[2].status, CheckStatus::Warn);
    assert!(checks[2].detail.contains("Claude preset"), "{checks:?}");
    assert!(
        checks[2]
            .hint
            .as_deref()
            .unwrap()
            .contains("--dangerously-skip-permissions"),
        "{checks:?}"
    );
}

#[test]
fn the_permission_mode_check_warns_when_there_is_no_claude_preset() {
    let agents = r#"[{"id":"x","presetId":"codex","iconId":null,"label":"Codex","command":"codex","args":[],"promptTransport":"argv","promptArgs":[],"resumeArgs":[],"forkArgs":[],"env":{},"order":0}]"#;

    let checks = superset(&checking(agents)).checks();

    assert_eq!(checks[2].status, CheckStatus::Warn);
    assert!(checks[2].detail.contains("no Claude preset"), "{checks:?}");
}

#[test]
fn the_host_check_warns_when_supersets_host_service_is_not_running() {
    let runner = checking(&fixture("agents_list.json"))
        .replying("superset status", &fixture("status_not_running.json"));

    let checks = superset(&runner).checks();

    assert_eq!(checks[1].name, "superset host");
    assert_eq!(checks[1].status, CheckStatus::Warn);
    assert!(checks[1].detail.contains("not running"), "{checks:?}");
    assert!(checks[1].hint.is_some(), "{checks:?}");
}

#[test]
fn checks_warn_with_an_install_hint_when_superset_is_missing() {
    let checks = superset(&FakeRunner::default()).checks();

    assert_eq!(
        check_summary(&checks),
        [
            ("superset", CheckStatus::Warn),
            ("superset host", CheckStatus::Warn),
            ("permission mode", CheckStatus::Warn),
        ]
    );
    assert!(
        checks[0].hint.as_deref().unwrap().contains("superset.sh"),
        "{checks:?}"
    );
}
