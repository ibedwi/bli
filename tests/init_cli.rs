//! `bli init` as the real binary runs it: no terminal, so no questions.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Runs the real `bli` in `dir` with stdin closed and output captured, failing if it
/// is still running after ten seconds, as it would be if it waited for an answer.
fn bli(dir: &Path, args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bli"))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            panic!("bli {args:?} hung without a terminal");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}

fn checkout() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(status.success());
    dir
}

#[test]
fn without_a_terminal_init_exits_at_once_and_writes_nothing() {
    let dir = checkout();

    let out = bli(dir.path(), &["init"]);

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.starts_with("error: bli init asks its questions in a terminal"),
        "{stderr}"
    );
    assert!(stderr.contains("bli init --yes"), "{stderr}");
    assert!(!dir.path().join("bli.toml").exists());
}

#[test]
fn yes_writes_a_config_the_other_commands_load_without_a_terminal() {
    let dir = checkout();

    let out = bli(dir.path(), &["init", "--yes"]);

    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = std::fs::read_to_string(dir.path().join("bli.toml")).unwrap();
    assert!(text.contains("kind = \"github\""), "{text}");
    bli::config::Config::load(dir.path()).unwrap();
}

#[test]
fn help_names_init_and_its_flags() {
    let dir = checkout();

    let top = String::from_utf8(bli(dir.path(), &["--help"]).stdout).unwrap();
    let init = String::from_utf8(bli(dir.path(), &["init", "--help"]).stdout).unwrap();

    assert!(top.contains("init"), "{top}");
    assert!(init.contains("--force"), "{init}");
    assert!(init.contains("-y, --yes"), "{init}");
}
