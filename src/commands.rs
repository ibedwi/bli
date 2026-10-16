//! The workflow. Depends only on the provider interfaces, never on an adapter.

use serde_json::json;

use crate::error::Error;
use crate::tracker::IssueTracker;

pub fn frontier(
    tracker: &dyn IssueTracker,
    parent: Option<&str>,
    default_parent: Option<&str>,
    json: bool,
) -> Result<String, Error> {
    let parent = parent.or(default_parent).ok_or_else(|| {
        Error::new("no parent ticket given")
            .with_hint("pass --parent <id>, or set `parent` under [tracker] in bli.toml")
    })?;
    let tickets = tracker.frontier(parent)?;
    if json {
        return Ok(format!(
            "{}\n",
            json!({ "parent": parent, "tickets": tickets })
        ));
    }
    if tickets.is_empty() {
        return Ok(format!("No tickets on the frontier of {parent}.\n"));
    }
    let mut out = String::new();
    for t in &tickets {
        out.push_str(&format!("{}  {}\n    {}\n", t.id, t.title, t.url));
    }
    Ok(out)
}
