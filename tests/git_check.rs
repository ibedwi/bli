//! Seam 2: the git check `bli doctor` runs, against a fake process runner.

use std::io;

use bli::check::CheckStatus;
use bli::git;
use bli::process::{Output, ProcessRunner};

struct FakeRunner(fn() -> io::Result<Output>);

impl ProcessRunner for FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<Output> {
        assert_eq!((program, args), ("git", &["--version"][..]));
        (self.0)()
    }
}

#[test]
fn the_git_check_passes_with_gits_version() {
    let check = git::check(&FakeRunner(|| {
        Ok(Output {
            success: true,
            stdout: "git version 2.51.0\n".into(),
            stderr: String::new(),
        })
    }));
    assert_eq!(check.name, "git");
    assert_eq!(check.status, CheckStatus::Pass);
    assert_eq!(check.detail, "git version 2.51.0");
}

#[test]
fn the_git_check_warns_with_an_install_hint_when_git_is_missing() {
    let check = git::check(&FakeRunner(|| {
        Err(io::Error::new(io::ErrorKind::NotFound, "no git"))
    }));
    assert_eq!(check.status, CheckStatus::Warn);
    assert_eq!(check.detail, "could not run `git`: it is not installed");
    assert!(
        check.hint.as_deref().unwrap().contains("install git"),
        "{check:?}"
    );
}
