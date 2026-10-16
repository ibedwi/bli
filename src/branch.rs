//! Lane branch names: rendering them from the configured pattern, and reading
//! the ticket ID back out of them (ADR 0001).

use std::fmt;

use serde::Deserialize;

use crate::error::Error;

pub const DEFAULT_PATTERN: &str = "ticket/{id}-{slug}";

/// A branch pattern such as `ticket/{id}-{slug}`: literal text with an `{id}`
/// placeholder and an optional `{slug}` one.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct BranchPattern {
    text: String,
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Literal(String),
    Id,
    Slug,
}

impl BranchPattern {
    pub fn parse(text: &str) -> Result<BranchPattern, Error> {
        let mut parts = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            let (part, len) = if rest.starts_with("{id}") {
                (Part::Id, 4)
            } else if rest.starts_with("{slug}") {
                (Part::Slug, 6)
            } else {
                let len = rest
                    .char_indices()
                    .skip(1)
                    .find(|&(_, c)| c == '{')
                    .map_or(rest.len(), |(i, _)| i);
                (Part::Literal(rest[..len].to_string()), len)
            };
            let is_placeholder = |p: &Part| matches!(p, Part::Id | Part::Slug);
            if is_placeholder(&part) && parts.last().is_some_and(is_placeholder) {
                return Err(invalid(text, "placeholders must be separated by text"));
            }
            parts.push(part);
            rest = &rest[len..];
        }
        let count = |p: &Part| parts.iter().filter(|q| *q == p).count();
        if count(&Part::Id) != 1 {
            return Err(invalid(text, "it must contain `{id}` exactly once"));
        }
        if count(&Part::Slug) > 1 {
            return Err(invalid(text, "it may contain `{slug}` at most once"));
        }
        Ok(BranchPattern {
            text: text.to_string(),
            parts,
        })
    }

    /// The branch for ticket `id` with `slug`.
    pub fn render(&self, id: &str, slug: &str) -> String {
        self.parts
            .iter()
            .map(|p| match p {
                Part::Literal(s) => s.as_str(),
                Part::Id => id,
                Part::Slug => slug,
            })
            .collect()
    }

    /// The ticket ID in `branch`, when `branch` matches the pattern.
    pub fn ticket_id(&self, branch: &str) -> Option<String> {
        let mut rest = branch;
        let mut id = None;
        for (i, part) in self.parts.iter().enumerate() {
            match part {
                Part::Literal(s) => rest = rest.strip_prefix(s.as_str())?,
                Part::Id | Part::Slug => {
                    // A placeholder runs up to the next literal, or to the end.
                    let end = match self.parts.get(i + 1) {
                        Some(Part::Literal(next)) => rest.find(next.as_str())?,
                        _ => rest.len(),
                    };
                    let value = &rest[..end];
                    let valid = match part {
                        Part::Id => !value.is_empty() && value.chars().all(is_id_char),
                        _ => is_slug(value),
                    };
                    if !valid {
                        return None;
                    }
                    if *part == Part::Id {
                        id = Some(value.to_string());
                    }
                    rest = &rest[end..];
                }
            }
        }
        if rest.is_empty() {
            id
        } else {
            None
        }
    }
}

impl Default for BranchPattern {
    fn default() -> Self {
        BranchPattern::parse(DEFAULT_PATTERN).expect("the default pattern is valid")
    }
}

impl TryFrom<String> for BranchPattern {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        BranchPattern::parse(&text).map_err(|e| e.to_string())
    }
}

impl fmt::Display for BranchPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

fn invalid(text: &str, why: &str) -> Error {
    Error::new(format!("branch pattern `{text}` is invalid: {why}"))
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// Whether `s` is a slug: non-empty lowercase kebab-case.
pub fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A branch slug from a ticket title: lowercase kebab-case of its first five
/// words, with non-alphanumerics dropped (a word's own hyphens are kept).
pub fn slugify(title: &str) -> String {
    title
        .split_whitespace()
        .map(|word| {
            let kept: String = word
                .to_ascii_lowercase()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            kept.split('-')
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("-")
        })
        .filter(|w| !w.is_empty())
        .take(5)
        .collect::<Vec<_>>()
        .join("-")
}
