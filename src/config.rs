//! `bli.toml`: the per-repo choice of lane host and issue tracker.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};

use crate::error::Error;

pub const FILE_NAME: &str = "bli.toml";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Config {
    pub tracker: TrackerConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TrackerConfig {
    pub kind: TrackerKind,
    /// The default parent ticket for `bli frontier`.
    #[serde(default, deserialize_with = "ticket_id")]
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackerKind {
    Github,
    Clickup,
}

impl Config {
    /// Loads the `bli.toml` of the checkout containing `cwd`.
    pub fn load(cwd: &Path) -> Result<Config, Error> {
        let path = find(cwd).ok_or_else(|| {
            Error::new(format!("no {FILE_NAME} found in this repo")).with_hint(format!(
                "create {FILE_NAME} at the repo root with at least:\n\n[tracker]\nkind = \"github\""
            ))
        })?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| Error::new(format!("could not read {}: {e}", path.display())))?;
        toml::from_str(&text).map_err(|e| {
            Error::new(format!("invalid {}: {}", path.display(), e.message())).with_hint(
                "bli.toml needs a [tracker] table naming its kind, e.g. kind = \"github\"",
            )
        })
    }
}

/// Walks up from `cwd` to the checkout root (the directory holding `.git`) looking for `bli.toml`.
fn find(cwd: &Path) -> Option<PathBuf> {
    for dir in cwd.ancestors() {
        let candidate = dir.join(FILE_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        if dir.join(".git").exists() {
            return None;
        }
    }
    None
}

/// Ticket IDs may be written as a string or, for numeric IDs like GitHub's, a bare integer.
fn ticket_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Text(String),
        Number(u64),
    }
    Ok(Option::<Id>::deserialize(d)?.map(|id| match id {
        Id::Text(s) => s,
        Id::Number(n) => n.to_string(),
    }))
}
