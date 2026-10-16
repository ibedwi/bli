//! A setup check, as reported by `bli doctor`. Each adapter contributes its own.

use serde::Serialize;

use crate::error::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    /// What was found: a version, an account, or why the check failed.
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Warn,
}

impl CheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckStatus::Pass => "pass",
            CheckStatus::Warn => "warn",
        }
    }
}

impl Check {
    pub fn pass(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Check {
            name: name.into(),
            status: CheckStatus::Pass,
            detail: detail.into(),
            hint: None,
        }
    }

    /// A failed check, saying what went wrong and, when `error` has one, how to fix it.
    pub fn warn(name: impl Into<String>, error: &Error) -> Self {
        Check {
            name: name.into(),
            status: CheckStatus::Warn,
            detail: error.to_string(),
            hint: error.hint().map(str::to_string),
        }
    }

    /// Passes with `detail` when `result` is `Ok`, else warns with its error.
    pub fn from_result(name: impl Into<String>, result: Result<String, Error>) -> Self {
        match result {
            Ok(detail) => Check::pass(name, detail),
            Err(e) => Check::warn(name, &e),
        }
    }

    pub fn passed(&self) -> bool {
        self.status == CheckStatus::Pass
    }
}
