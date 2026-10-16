//! The code host interface: where lane branches are pushed and their pull
//! requests live. The command layer depends only on this. There is no
//! operation to merge a pull request: merging stays the developer's decision.

use std::path::Path;

use serde::Serialize;

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

impl PrState {
    pub fn as_str(self) -> &'static str {
        match self {
            PrState::Open => "open",
            PrState::Merged => "merged",
            PrState::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub state: PrState,
}

/// A pull request to open from branch `head` into branch `base`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPullRequest {
    pub head: String,
    pub base: String,
    pub title: String,
    pub body: String,
}

pub trait CodeHost {
    /// Pushes local branch `branch` of the repo whose main checkout is `repo`, tracking it.
    fn push(&self, repo: &Path, branch: &str) -> Result<(), Error>;

    /// The pull request from branch `head`, in any state: the open one, else the newest.
    fn find_pr(&self, head: &str) -> Result<Option<PullRequest>, Error>;

    fn open_pr(&self, pr: &NewPullRequest) -> Result<PullRequest, Error>;
}

impl<T: CodeHost + ?Sized> CodeHost for &T {
    fn push(&self, repo: &Path, branch: &str) -> Result<(), Error> {
        (**self).push(repo, branch)
    }

    fn find_pr(&self, head: &str) -> Result<Option<PullRequest>, Error> {
        (**self).find_pr(head)
    }

    fn open_pr(&self, pr: &NewPullRequest) -> Result<PullRequest, Error> {
        (**self).open_pr(pr)
    }
}
