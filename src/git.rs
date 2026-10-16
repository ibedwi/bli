//! The git operations `bli` does itself, rather than through a lane host.

use std::io;
use std::path::{Path, PathBuf};

use crate::check::Check;
use crate::error::Error;
use crate::process::{Output, ProcessRunner, SystemRunner};

/// The remote lanes branch from and ship to.
pub const REMOTE: &str = "origin";

/// The git config key, under `branch.<name>`, recording a lane's base.
const BASE_KEY: &str = "bliBase";

/// The repo containing `dir`, addressed through its main checkout.
pub struct Git {
    main: PathBuf,
}

impl Git {
    /// Finds the repo containing `dir`. From inside a lane, that is still the main checkout.
    pub fn open(dir: &Path) -> Result<Git, Error> {
        let common = run(
            dir,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .map_err(|e| e.with_hint("run bli from inside a git checkout"))?;
        let common = PathBuf::from(common.trim());
        let main = match common.file_name() {
            Some(name) if name == ".git" => common.parent().map(Path::to_path_buf),
            _ => None,
        }
        .ok_or_else(|| {
            Error::new(format!(
                "the repo at {} has no main checkout",
                common.display()
            ))
        })?;
        Ok(Git { main })
    }

    /// The repo's main checkout, which every linked checkout belongs to.
    pub fn main_checkout(&self) -> &Path {
        &self.main
    }

    pub fn fetch(&self) -> Result<(), Error> {
        self.git(&["fetch", "--quiet", REMOTE]).map(drop)
    }

    /// Whether `branch` exists on the remote, as of the last fetch.
    pub fn has_remote_branch(&self, branch: &str) -> Result<bool, Error> {
        let reference = format!("refs/remotes/{REMOTE}/{branch}");
        let out = output(&self.main, &["show-ref", "--verify", "--quiet", &reference])?;
        Ok(out.success)
    }

    /// How many commits local branch `branch` has that `base` on the remote doesn't, as of the last fetch.
    pub fn commits_ahead(&self, branch: &str, base: &str) -> Result<u64, Error> {
        let range = format!("{REMOTE}/{base}..refs/heads/{branch}");
        let count = self.git(&["rev-list", "--count", &range])?;
        count
            .trim()
            .parse()
            .map_err(|_| Error::new(format!("could not read the commit count of {range}")))
    }

    /// Records `base` as the base of lane branch `branch`.
    pub fn set_base(&self, branch: &str, base: &str) -> Result<(), Error> {
        self.git(&["config", &base_key(branch), base]).map(drop)
    }

    /// The recorded base of lane branch `branch`, if any.
    pub fn base(&self, branch: &str) -> Result<Option<String>, Error> {
        let out = output(&self.main, &["config", "--get", &base_key(branch)])?;
        // `git config --get` exits 1 when the key is unset.
        Ok(out.success.then(|| out.stdout.trim().to_string()))
    }

    fn git(&self, args: &[&str]) -> Result<String, Error> {
        run(&self.main, args)
    }
}

fn base_key(branch: &str) -> String {
    format!("branch.{branch}.{BASE_KEY}")
}

/// Runs git in `dir`, failing only when git could not be run at all.
fn output(dir: &Path, args: &[&str]) -> Result<Output, Error> {
    let dir = dir.to_string_lossy();
    let mut full = vec!["-C", dir.as_ref()];
    full.extend_from_slice(args);
    SystemRunner.run("git", &full).map_err(not_run)
}

/// Runs git in `dir` and returns its stdout, or an error carrying its stderr.
fn run(dir: &Path, args: &[&str]) -> Result<String, Error> {
    let out = output(dir, args)?;
    if !out.success {
        return Err(Error::new(format!(
            "`git {}` failed: {}",
            args.join(" "),
            out.stderr.trim()
        )));
    }
    Ok(out.stdout)
}

fn not_run(e: io::Error) -> Error {
    match e.kind() {
        io::ErrorKind::NotFound => Error::new("could not run `git`: it is not installed"),
        _ => Error::new(format!("could not run `git`: {e}")),
    }
}

/// Checks that git is installed, for `bli doctor`.
pub fn check(runner: &dyn ProcessRunner) -> Check {
    let version = runner
        .run("git", &["--version"])
        .map_err(|e| not_run(e).with_hint("install git (https://git-scm.com)"))
        .and_then(|out| {
            if out.success {
                Ok(out.stdout.trim().to_string())
            } else {
                Err(Error::new(format!(
                    "`git --version` failed: {}",
                    out.stderr.trim()
                )))
            }
        });
    Check::from_result("git", version)
}
