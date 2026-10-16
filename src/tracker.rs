//! The issue tracker interface. The command layer depends only on this.

use serde::Serialize;

use crate::check::Check;
use crate::error::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    pub url: String,
}

/// A ticket with the state `lane start` checks before starting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketDetails {
    /// `id` is the tracker's canonical form of the ID, e.g. `12` for `#12` on GitHub.
    pub ticket: Ticket,
    pub open: bool,
    /// Who has claimed the ticket; empty when it is unclaimed.
    pub claimed_by: Vec<String>,
}

pub trait IssueTracker {
    /// The child tickets of `parent` that are open, have no open blockers and are unclaimed.
    fn frontier(&self, parent: &str) -> Result<Vec<Ticket>, Error>;

    /// Looks up one ticket by ID.
    fn ticket(&self, id: &str) -> Result<TicketDetails, Error>;

    /// Claims the ticket for the current user.
    fn claim(&self, id: &str) -> Result<(), Error>;

    /// The body for a pull request shipping ticket `id`, given the `body` written for it:
    /// on a tracker that reads pull requests, one that closes the ticket on merge.
    fn pr_body(&self, id: &str, body: &str) -> Result<String, Error>;

    /// Checks the tracker's tools and credentials, for `bli doctor`.
    fn checks(&self) -> Vec<Check>;
}

impl<T: IssueTracker + ?Sized> IssueTracker for &T {
    fn frontier(&self, parent: &str) -> Result<Vec<Ticket>, Error> {
        (**self).frontier(parent)
    }

    fn ticket(&self, id: &str) -> Result<TicketDetails, Error> {
        (**self).ticket(id)
    }

    fn claim(&self, id: &str) -> Result<(), Error> {
        (**self).claim(id)
    }

    fn pr_body(&self, id: &str, body: &str) -> Result<String, Error> {
        (**self).pr_body(id, body)
    }

    fn checks(&self) -> Vec<Check> {
        (**self).checks()
    }
}
