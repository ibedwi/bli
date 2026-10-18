//! `bli.toml`: the per-repo choice of lane host and issue tracker.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};

use crate::branch::BranchPattern;
use crate::error::Error;
use crate::tracker::OnMerge;

pub const FILE_NAME: &str = "bli.toml";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub lane: LaneConfig,
    pub tracker: TrackerConfig,
}

/// How lanes are made: which host runs them, what they branch from, and what the agent is told.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LaneConfig {
    pub host: HostKind,
    /// The default base branch, on the `origin` remote.
    pub base: String,
    pub branch: BranchPattern,
    /// The agent's first prompt; `{ticket_url}` is replaced with the ticket's URL.
    pub prompt: String,
}

impl Default for LaneConfig {
    fn default() -> Self {
        LaneConfig {
            host: HostKind::Herdr,
            base: "main".into(),
            branch: BranchPattern::default(),
            prompt: "/implement {ticket_url}".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostKind {
    Herdr,
    Superset,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TrackerConfig {
    pub kind: TrackerKind,
    /// The default parent ticket for `bli frontier`.
    #[serde(default, deserialize_with = "optional_id")]
    pub parent: Option<String>,
    /// What happens to a ticket once its lane's pull request is merged.
    #[serde(default)]
    pub on_merge: OnMerge,
    /// ClickUp's settings, required when `kind = "clickup"`.
    pub clickup: Option<ClickUpConfig>,
}

/// `[tracker.clickup]`: where the tickets live. The API token is never read from here.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClickUpConfig {
    #[serde(deserialize_with = "required_id")]
    pub workspace_id: String,
    /// Narrows the frontier search to subtasks in this list.
    #[serde(default, deserialize_with = "optional_id")]
    pub list_id: Option<String>,
    /// Narrows the frontier search to subtasks in this space.
    #[serde(default, deserialize_with = "optional_id")]
    pub space_id: Option<String>,
    /// The API v2 base address, configurable so tests can point it at a stub.
    #[serde(default = "default_api_base")]
    pub api_base: String,
}

pub const CLICKUP_API_BASE: &str = "https://api.clickup.com/api/v2";

fn default_api_base() -> String {
    CLICKUP_API_BASE.to_string()
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
        let invalid = |why: &str| Error::new(format!("invalid {}: {why}", path.display()));
        if let Some(key) = clickup_secret(&text) {
            return Err(invalid(&format!(
                "`{key}` under [tracker.clickup]: the ClickUp API token must not be in bli.toml"
            ))
            .with_hint(format!(
                "remove it, and set {} in the environment, or in the .env of this checkout or the main checkout",
                crate::clickup::TOKEN_VAR
            )));
        }
        let config: Config = toml::from_str(&text).map_err(|e| {
            invalid(e.message()).with_hint(
                "bli.toml needs a [tracker] table naming its kind, e.g. kind = \"github\"",
            )
        })?;
        if config.tracker.kind == TrackerKind::Clickup && config.tracker.clickup.is_none() {
            return Err(
                invalid("`kind = \"clickup\"` needs a [tracker.clickup] table")
                    .with_hint("add [tracker.clickup] with at least workspace_id = \"...\""),
            );
        }
        Ok(config)
    }
}

/// A token-like key in `[tracker.clickup]`, which is refused: bli.toml is committed,
/// so secrets never go there.
fn clickup_secret(text: &str) -> Option<String> {
    // A file that doesn't parse has its syntax error reported by the typed parse.
    let raw = toml::from_str::<toml::Table>(text).ok()?;
    let clickup = raw.get("tracker")?.get("clickup")?.as_table()?;
    clickup.keys().find(|k| k.contains("token")).cloned()
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

/// IDs may be written as a string or, for numeric IDs like GitHub's issue numbers
/// or ClickUp's workspace IDs, a bare integer.
#[derive(Deserialize)]
#[serde(untagged)]
enum Id {
    Text(String),
    Number(u64),
}

impl From<Id> for String {
    fn from(id: Id) -> String {
        match id {
            Id::Text(s) => s,
            Id::Number(n) => n.to_string(),
        }
    }
}

fn optional_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<Id>::deserialize(d)?.map(String::from))
}

fn required_id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Id::deserialize(d).map(String::from)
}
