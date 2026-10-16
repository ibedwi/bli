//! The lane host interface. The command layer depends only on this.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Error;

/// The host's view of what a lane's agent is doing. Advisory: `bli` only reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusHint {
    Working,
    Blocked,
    Idle,
    Unknown,
}

impl StatusHint {
    pub fn as_str(self) -> &'static str {
        match self {
            StatusHint::Working => "working",
            StatusHint::Blocked => "blocked",
            StatusHint::Idle => "idle",
            StatusHint::Unknown => "unknown",
        }
    }
}

/// One checkout the host knows about, whether or not it is a lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCheckout {
    pub path: PathBuf,
    /// `None` when the checkout is on a detached HEAD.
    pub branch: Option<String>,
    /// True for the repo's main checkout, which is never a lane.
    pub is_main: bool,
    /// The host's handle for the checkout's workspace, when one is open.
    pub workspace: Option<String>,
    pub status: StatusHint,
}

/// What to create: a checkout of a new `branch` from `base_ref`, in a workspace labelled `label`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLane {
    pub branch: String,
    pub base_ref: String,
    pub label: String,
}

/// A lane the host just created. `handle` is opaque, for passing back to [`LaneHost::start_agent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedLane {
    pub path: PathBuf,
    pub workspace: String,
    pub handle: String,
}

pub trait LaneHost {
    /// The host's name as written in `bli.toml`, for messages.
    fn name(&self) -> &str;

    /// Whether `bli` is running inside this host, so host commands have something to act on.
    fn is_inside(&self) -> bool;

    /// Creates a lane for the repo whose main checkout is `repo`.
    fn create_lane(&self, repo: &Path, lane: &NewLane) -> Result<CreatedLane, Error>;

    /// Starts the agent for ticket `ticket_id` in `lane`, sends it `prompt`, and reports its status.
    fn start_agent(
        &self,
        lane: &CreatedLane,
        ticket_id: &str,
        prompt: &str,
    ) -> Result<StatusHint, Error>;

    /// The last `lines` lines of output from ticket `ticket_id`'s agent.
    fn read_agent(&self, ticket_id: &str, lines: u32) -> Result<String, Error>;

    /// Every checkout of the repo whose main checkout is `repo`, as the host sees them now.
    fn list(&self, repo: &Path) -> Result<Vec<HostCheckout>, Error>;
}

impl<T: LaneHost + ?Sized> LaneHost for &T {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn is_inside(&self) -> bool {
        (**self).is_inside()
    }

    fn create_lane(&self, repo: &Path, lane: &NewLane) -> Result<CreatedLane, Error> {
        (**self).create_lane(repo, lane)
    }

    fn start_agent(
        &self,
        lane: &CreatedLane,
        ticket_id: &str,
        prompt: &str,
    ) -> Result<StatusHint, Error> {
        (**self).start_agent(lane, ticket_id, prompt)
    }

    fn read_agent(&self, ticket_id: &str, lines: u32) -> Result<String, Error> {
        (**self).read_agent(ticket_id, lines)
    }

    fn list(&self, repo: &Path) -> Result<Vec<HostCheckout>, Error> {
        (**self).list(repo)
    }
}
