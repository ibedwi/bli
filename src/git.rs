//! The git operations `bli` does itself, rather than through a lane host.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

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

    /// Fetches the remote, pruning tracking refs of branches it no longer has, so they
    /// don't count commits as pushed after the remote branch is gone.
    pub fn fetch(&self) -> Result<(), Error> {
        self.git(&["fetch", "--quiet", "--prune", REMOTE]).map(drop)
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

    /// The files changed in the checkout at `path`, split into tracked, untracked and ignored.
    pub fn changes(&self, path: &Path) -> Result<Changes, Error> {
        let out = run(
            path,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?;
        let mut changes = Changes::default();
        let mut entries = out.split('\0').filter(|e| !e.is_empty());
        while let Some(entry) = entries.next() {
            let (code, file) = entry.split_at(3.min(entry.len()));
            if code.starts_with("??") {
                changes.untracked.push(file.to_string());
            } else {
                changes.modified.push(file.to_string());
                // A rename or copy is followed by its original path.
                if code.starts_with('R') || code.starts_with('C') {
                    entries.next();
                }
            }
        }
        // Ignored files are listed apart, with whole ignored directories collapsed, so a
        // build directory is one entry rather than thousands.
        let ignored = run(
            path,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--ignored=matching",
                "--untracked-files=normal",
            ],
        )?;
        changes.ignored = ignored
            .split('\0')
            .filter_map(|e| e.strip_prefix("!! "))
            .map(str::to_string)
            .collect();
        Ok(changes)
    }

    /// The commits on `branch` that no remote-tracking branch has, newest first.
    pub fn unpushed(&self, branch: &str) -> Result<Vec<Commit>, Error> {
        let reference = format!("refs/heads/{branch}");
        let out = self.git(&["log", "--format=%h%x09%s", &reference, "--not", "--remotes"])?;
        Ok(out
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(sha, subject)| Commit {
                sha: sha.to_string(),
                subject: subject.to_string(),
            })
            .collect())
    }

    /// The upstream `branch` tracks, such as `origin/ticket/4-x`, if any.
    pub fn upstream(&self, branch: &str) -> Result<Option<String>, Error> {
        let reference = format!("{branch}@{{upstream}}");
        let out = output(
            &self.main,
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                &reference,
            ],
        )?;
        // `rev-parse` fails when the branch has no upstream.
        Ok(out.success.then(|| out.stdout.trim().to_string()))
    }

    /// Deletes local branch `branch`, whether or not it is merged. Records of checkouts
    /// whose directory is gone are pruned first, since git won't delete a branch they hold.
    pub fn delete_branch(&self, branch: &str) -> Result<(), Error> {
        self.git(&["worktree", "prune"])?;
        self.git(&["branch", "--quiet", "-D", branch]).map(drop)
    }

    fn git(&self, args: &[&str]) -> Result<String, Error> {
        run(&self.main, args)
    }
}

/// Files changed in a checkout, as paths relative to its root.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Changes {
    pub modified: Vec<String>,
    pub untracked: Vec<String>,
    /// Files git ignores, such as `.env` or build output; an ignored directory is one entry.
    pub ignored: Vec<String>,
}

impl Changes {
    /// Whether nothing git tracks, or would track, has changed. Ignored files don't count.
    pub fn is_empty(&self) -> bool {
        self.modified.is_empty() && self.untracked.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
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
