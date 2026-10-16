//! The issue tracker interface. The command layer depends only on this.

use serde::Serialize;

use crate::error::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    pub url: String,
}

pub trait IssueTracker {
    /// The child tickets of `parent` that are open, have no open blockers and are unclaimed.
    fn frontier(&self, parent: &str) -> Result<Vec<Ticket>, Error>;
}

impl<T: IssueTracker + ?Sized> IssueTracker for &T {
    fn frontier(&self, parent: &str) -> Result<Vec<Ticket>, Error> {
        (**self).frontier(parent)
    }
}
