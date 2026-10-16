//! Seam 2: the herdr adapter. Checks that each lane host operation becomes the
//! right `herdr` command lines and that saved real herdr output parses correctly.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use bli::herdr::Herdr;
use bli::host::{CreatedLane, HostCheckout, LaneHost, NewLane, StatusHint};
use bli::process::{Output, ProcessRunner};

/// Records every command and answers each `herdr <group> <verb>` with its canned output.
#[derive(Default)]
struct FakeRunner {
    replies: HashMap<String, io::Result<Output>>,
    calls: RefCell<Vec<Vec<String>>>,
}

impl FakeRunner {
    fn replying(mut self, command: &str, stdout: &str) -> Self {
        self.replies.insert(
            command.to_string(),
            Ok(Output {
                success: true,
                stdout: stdout.to_string(),
                stderr: String::new(),
            }),
        );
        self
    }

    fn failing(mut self, command: &str, stderr: &str) -> Self {
        self.replies.insert(
            command.to_string(),
            Ok(Output {
                success: false,
                stdout: String::new(),
                stderr: stderr.to_string(),
            }),
        );
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
        self.calls.borrow_mut().push(call);
        let key = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
        match self.replies.get(&key) {
            Some(Ok(out)) => Ok(out.clone()),
            Some(Err(e)) => Err(io::Error::new(e.kind(), e.to_string())),
            None => Err(io::Error::new(io::ErrorKind::NotFound, "no herdr")),
        }
    }
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/herdr/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn herdr(runner: &FakeRunner) -> Herdr<&FakeRunner> {
    Herdr::new(runner, Some("1"))
}

/// Shaped per `herdr api schema --json`; only success matters to the adapter.
const AGENT_STARTED: &str = r#"{"id":"cli:agent:start","result":{"type":"agent_started","argv":["claude","--permission-mode","bypassPermissions"],"agent":{}}}"#;
const AGENT_PROMPTED: &str =
    r#"{"id":"cli:agent:prompt","result":{"type":"agent_prompted","agent":{}}}"#;

const REPO: &str = "/home/ibedwi/github/bli";

fn lane() -> CreatedLane {
    CreatedLane {
        path: PathBuf::from("/home/ibedwi/.herdr/worktrees/bli/ticket-3-fan-out"),
        workspace: "w3".into(),
        handle: "w3:p1".into(),
    }
}

#[test]
fn is_inside_only_when_herdr_env_is_set() {
    let runner = FakeRunner::default();
    assert!(Herdr::new(&runner, Some("1")).is_inside());
    assert!(!Herdr::new(&runner, Some("0")).is_inside());
    assert!(!Herdr::new(&runner, None).is_inside());
    assert_eq!(Herdr::new(&runner, None).name(), "herdr");
}

#[test]
fn create_lane_creates_a_worktree_from_the_main_checkout_without_focusing_it() {
    let runner =
        FakeRunner::default().replying("worktree create", &fixture("worktree_create.json"));

    herdr(&runner)
        .create_lane(
            Path::new(REPO),
            &NewLane {
                branch: "ticket/3-fan-out".into(),
                base_ref: "origin/main".into(),
                label: "#3 fan-out".into(),
            },
        )
        .unwrap();

    assert_eq!(
        runner.calls(),
        vec![vec![
            "herdr",
            "worktree",
            "create",
            "--cwd",
            REPO,
            "--branch",
            "ticket/3-fan-out",
            "--base",
            "origin/main",
            "--label",
            "#3 fan-out",
            "--no-focus",
            "--trust-repository",
        ]]
    );
}

#[test]
fn create_lane_reads_the_checkout_workspace_and_pane() {
    let runner =
        FakeRunner::default().replying("worktree create", &fixture("worktree_create.json"));

    let lane = herdr(&runner)
        .create_lane(
            Path::new(REPO),
            &NewLane {
                branch: "scratch/bli-fixture".into(),
                base_ref: "origin/main".into(),
                label: "#0 bli-fixture".into(),
            },
        )
        .unwrap();

    assert_eq!(
        lane,
        CreatedLane {
            path: PathBuf::from("/home/ibedwi/.herdr/worktrees/bli/scratch-bli-fixture"),
            workspace: "w3".into(),
            handle: "w3:p1".into(),
        }
    );
}

#[test]
fn create_lane_reports_herdrs_error_message_and_code() {
    let runner = FakeRunner::default().failing(
        "worktree create",
        r#"{"error":{"code":"linked_worktree_source","message":"New and open worktree actions start from the repo parent workspace."},"id":"cli:worktree:create"}"#,
    );

    let err = herdr(&runner)
        .create_lane(
            Path::new(REPO),
            &NewLane {
                branch: "ticket/3-x".into(),
                base_ref: "origin/main".into(),
                label: "#3 x".into(),
            },
        )
        .unwrap_err();

    let text = err.to_string();
    assert!(
        text.contains("start from the repo parent workspace"),
        "{text}"
    );
    assert!(text.contains("linked_worktree_source"), "{text}");
}

#[test]
fn start_agent_starts_claude_in_bypass_permissions_mode_prompts_it_and_waits_for_it() {
    let runner = FakeRunner::default()
        .replying("agent start", AGENT_STARTED)
        .replying("agent prompt", AGENT_PROMPTED)
        .replying("agent wait", &fixture("agent_wait.json"));

    let status = herdr(&runner)
        .start_agent(&lane(), "3", "/implement https://github.com/o/r/issues/3")
        .unwrap();

    assert_eq!(status, StatusHint::Working);
    assert_eq!(
        runner.calls(),
        vec![
            vec![
                "herdr",
                "agent",
                "start",
                "t3",
                "--kind",
                "claude",
                "--pane",
                "w3:p1",
                "--",
                "--permission-mode",
                "bypassPermissions",
            ],
            vec![
                "herdr",
                "agent",
                "prompt",
                "t3",
                "/implement https://github.com/o/r/issues/3",
            ],
            vec![
                "herdr",
                "agent",
                "wait",
                "t3",
                "--until",
                "working",
                "--until",
                "blocked",
                "--timeout",
                "20000",
            ],
        ]
    );
}

#[test]
fn start_agent_reports_an_agent_that_has_not_started_working_yet_as_unknown() {
    let runner = FakeRunner::default()
        .replying("agent start", AGENT_STARTED)
        .replying("agent prompt", AGENT_PROMPTED)
        .failing("agent wait", &fixture("error_timeout.json"));

    let status = herdr(&runner).start_agent(&lane(), "3", "go").unwrap();

    assert_eq!(status, StatusHint::Unknown);
}

#[test]
fn start_agent_fails_without_prompting_when_the_agent_does_not_start() {
    let runner = FakeRunner::default().failing(
        "agent start",
        r#"{"error":{"code":"agent_not_ready","message":"agent was not ready"},"id":"cli:agent:start"}"#,
    );

    let err = herdr(&runner).start_agent(&lane(), "3", "go").unwrap_err();

    assert!(err.to_string().contains("agent_not_ready"), "{err}");
    assert_eq!(runner.calls().len(), 1);
}

#[test]
fn list_reads_every_checkout_with_its_agents_status() {
    let runner = FakeRunner::default()
        .replying("worktree list", &fixture("worktree_list.json"))
        .replying("agent list", &fixture("agent_list.json"));

    let checkouts = herdr(&runner).list(Path::new(REPO)).unwrap();

    assert_eq!(
        runner.calls(),
        vec![
            vec!["herdr", "worktree", "list", "--cwd", REPO],
            vec!["herdr", "agent", "list"],
        ]
    );
    assert_eq!(
        checkouts,
        vec![
            HostCheckout {
                path: PathBuf::from(REPO),
                branch: Some("main".into()),
                is_main: true,
                workspace: Some("w1".into()),
                status: StatusHint::Unknown,
            },
            HostCheckout {
                path: PathBuf::from("/home/ibedwi/.herdr/worktrees/bli/scratch-bli-fixture"),
                branch: Some("scratch/bli-fixture".into()),
                is_main: false,
                workspace: Some("w3".into()),
                status: StatusHint::Unknown,
            },
            HostCheckout {
                path: PathBuf::from("/home/ibedwi/.herdr/worktrees/bli/ticket-3-fan-out-on-herdr"),
                branch: Some("ticket/3-fan-out-on-herdr".into()),
                is_main: false,
                workspace: Some("w2".into()),
                status: StatusHint::Working,
            },
        ]
    );
}

#[test]
fn list_maps_herdr_agent_states_to_status_hints() {
    for (herdr_status, hint) in [
        ("working", StatusHint::Working),
        ("blocked", StatusHint::Blocked),
        ("idle", StatusHint::Idle),
        ("done", StatusHint::Idle),
        ("unknown", StatusHint::Unknown),
    ] {
        let agents = fixture("agent_list.json").replace(
            r#""agent_status":"working""#,
            &format!(r#""agent_status":"{herdr_status}""#),
        );
        let runner = FakeRunner::default()
            .replying("worktree list", &fixture("worktree_list.json"))
            .replying("agent list", &agents);

        let checkouts = herdr(&runner).list(Path::new(REPO)).unwrap();

        assert_eq!(checkouts[2].status, hint, "herdr status {herdr_status}");
    }
}

#[test]
fn list_reads_a_closed_workspace_and_a_detached_checkout() {
    let worktrees = fixture("worktree_list.json")
        .replace(r#""open_workspace_id":"w2""#, r#""open_workspace_id":null"#)
        .replace(r#""branch":"scratch/bli-fixture""#, r#""branch":null"#);
    let runner = FakeRunner::default()
        .replying("worktree list", &worktrees)
        .replying("agent list", &fixture("agent_list.json"));

    let checkouts = herdr(&runner).list(Path::new(REPO)).unwrap();

    assert_eq!(checkouts[1].branch, None);
    assert_eq!(checkouts[2].workspace, None);
    assert_eq!(checkouts[2].status, StatusHint::Unknown);
}

#[test]
fn a_missing_herdr_is_reported_with_a_hint() {
    let runner = FakeRunner::default();

    let err = herdr(&runner).list(Path::new(REPO)).unwrap_err();

    assert!(err.hint().unwrap().contains("herdr"), "{err:?}");
}

#[test]
fn start_agent_fails_when_waiting_for_the_agent_fails_for_another_reason() {
    let runner = FakeRunner::default()
        .replying("agent start", AGENT_STARTED)
        .replying("agent prompt", AGENT_PROMPTED)
        .failing("agent wait", &fixture("error_agent_not_found.json"));

    let err = herdr(&runner).start_agent(&lane(), "3", "go").unwrap_err();

    assert!(err.to_string().contains("agent_not_found"), "{err}");
}
