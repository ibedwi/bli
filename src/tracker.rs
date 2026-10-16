//! The issue tracker interface. The command layer depends only on this.

use serde::Serialize;

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
    pub assignees: Vec<String>,
}

pub trait IssueTracker {
    /// The child tickets of `parent` that are open, have no open blockers and are unclaimed.
    fn frontier(&self, parent: &str) -> Result<Vec<Ticket>, Error>;

    /// Looks up one ticket by ID.
    fn ticket(&self, id: &str) -> Result<TicketDetails, Error>;

    /// Claims the ticket for the current user.
    fn claim(&self, id: &str) -> Result<(), Error>;
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
}
