//! `bli init`: writes a repo's `bli.toml` from a short interview.

use std::path::Path;

use serde::Serialize;

use crate::branch::{BranchPattern, DEFAULT_PATTERN};
use crate::config::{self, FILE_NAME};
use crate::error::Error;
use crate::tracker::OnMerge;
use crate::Providers;

/// One option of a choice question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
}

/// Checks a typed answer, returning why it is refused.
pub type Validator = fn(&str) -> Result<(), String>;

/// The user cancelled the interview, with Esc or Ctrl-C, or the terminal went away. The
/// prompter has already said so on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

/// Asks the interview's questions. The real one draws prompts in the terminal; tests script it.
pub trait Prompter {
    fn intro(&self, title: &str);
    /// One of `choices`, by value; `default` is the value picked on enter.
    fn select(
        &self,
        prompt: &str,
        choices: &[Choice],
        default: &str,
    ) -> Result<&'static str, Cancelled>;
    /// A line of text, re-asked until `validate` accepts it. With an empty `default`, the
    /// answer is optional when `validate` accepts an empty one, and then empty means none.
    fn input(&self, prompt: &str, default: &str, validate: Validator) -> Result<String, Cancelled>;
    fn confirm(&self, prompt: &str, default: bool) -> Result<bool, Cancelled>;
    fn note(&self, title: &str, body: &str);
    fn outro(&self, message: &str);
    fn outro_cancel(&self, message: &str);
}

impl<T: Prompter + ?Sized> Prompter for &T {
    fn intro(&self, title: &str) {
        (**self).intro(title)
    }

    fn select(
        &self,
        prompt: &str,
        choices: &[Choice],
        default: &str,
    ) -> Result<&'static str, Cancelled> {
        (**self).select(prompt, choices, default)
    }

    fn input(&self, prompt: &str, default: &str, validate: Validator) -> Result<String, Cancelled> {
        (**self).input(prompt, default, validate)
    }

    fn confirm(&self, prompt: &str, default: bool) -> Result<bool, Cancelled> {
        (**self).confirm(prompt, default)
    }

    fn note(&self, title: &str, body: &str) {
        (**self).note(title, body)
    }

    fn outro(&self, message: &str) {
        (**self).outro(message)
    }

    fn outro_cancel(&self, message: &str) {
        (**self).outro_cancel(message)
    }
}

/// Why `bli init` can't ask its questions.
pub fn not_a_terminal() -> Error {
    Error::new("bli init asks its questions in a terminal, and this is not one").with_hint(
        "run it in a terminal, or run `bli init --yes` to write the defaults for GitHub Issues and herdr",
    )
}

const SKIP_HINT: &str = "none; press enter to skip";

/// The placeholder offering to skip a question: only one with no default whose validator
/// takes an empty answer.
fn skip_hint(default: &str, validate: Validator) -> Option<&'static str> {
    (default.is_empty() && validate("").is_ok()).then_some(SKIP_HINT)
}

/// The prompts drawn on the terminal, on stderr.
pub struct Terminal;

impl Prompter for Terminal {
    fn intro(&self, title: &str) {
        let _ = cliclack::intro(title);
    }

    fn select(
        &self,
        prompt: &str,
        choices: &[Choice],
        default: &str,
    ) -> Result<&'static str, Cancelled> {
        let mut select = cliclack::select(prompt);
        for choice in choices {
            select = select.item(choice.value, choice.label, choice.hint);
        }
        if let Some(choice) = choices.iter().find(|c| c.value == default) {
            select = select.initial_value(choice.value);
        }
        select.interact().map_err(|_| Cancelled)
    }

    fn input(&self, prompt: &str, default: &str, validate: Validator) -> Result<String, Cancelled> {
        let mut input = cliclack::input(prompt).validate(move |s: &String| validate(s));
        input = if default.is_empty() {
            // Not cliclack's own "required" check: the validator says why an answer is needed.
            input = input.required(false);
            match skip_hint(default, validate) {
                Some(hint) => input.placeholder(hint),
                None => input,
            }
        } else {
            input.default_input(default)
        };
        input.interact().map_err(|_| Cancelled)
    }

    fn confirm(&self, prompt: &str, default: bool) -> Result<bool, Cancelled> {
        cliclack::confirm(prompt)
            .initial_value(default)
            .interact()
            .map_err(|_| Cancelled)
    }

    fn note(&self, title: &str, body: &str) {
        let _ = cliclack::note(title, body);
    }

    fn outro(&self, message: &str) {
        let _ = cliclack::outro(message);
    }

    fn outro_cancel(&self, message: &str) {
        let _ = cliclack::outro_cancel(message);
    }
}

const HOSTS: [Choice; 2] = [
    Choice {
        value: "herdr",
        label: "herdr",
        hint: "lanes are herdr worktrees, with agents in herdr panes",
    },
    Choice {
        value: "superset",
        label: "Superset",
        hint: "lanes are Superset workspaces; the repo must be a Superset project",
    },
];

const TRACKERS: [Choice; 2] = [
    Choice {
        value: "github",
        label: "GitHub Issues",
        hint: "through gh",
    },
    Choice {
        value: "clickup",
        label: "ClickUp",
        hint: "tasks and subtasks, with CLICKUP_API_TOKEN in the environment or .env",
    },
];

const LEAVE: Choice = Choice {
    value: "none",
    label: "Leave it",
    hint: "leave the ticket as it is",
};

const GITHUB_ON_MERGE: [Choice; 2] = [
    Choice {
        value: "close",
        label: "Close it",
        hint: "close the issue, if the PR's `Closes #<id>` didn't",
    },
    LEAVE,
];

const CLICKUP_ON_MERGE: [Choice; 3] = [
    Choice {
        value: "close",
        label: "Close it",
        hint: "move the task to its list's closed status",
    },
    LEAVE,
    Choice {
        value: "status",
        label: "Move it to a status",
        hint: "move the task to a status you name",
    },
];

/// `bli.toml` as `bli init` writes it, in the order a reader expects.
#[derive(Serialize)]
struct Document {
    lane: Lane,
    tracker: Tracker,
}

#[derive(Serialize)]
struct Lane {
    host: &'static str,
    base: String,
    branch: String,
    prompt: String,
}

#[derive(Serialize)]
struct Tracker {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<Parent>,
    on_merge: OnMerge,
    #[serde(skip_serializing_if = "Option::is_none")]
    clickup: Option<ClickUp>,
}

/// A GitHub issue number is written bare; other tracker IDs as strings.
#[derive(Serialize)]
#[serde(untagged)]
enum Parent {
    Number(u64),
    Text(String),
}

/// `[tracker.clickup]`, without `api_base`, whose default is the real API.
#[derive(Serialize)]
struct ClickUp {
    workspace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    list_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    space_id: Option<String>,
}

const DEFAULT_PROMPT: &str = "/implement {ticket_url}";

pub(crate) struct Options {
    /// Replace an existing `bli.toml`.
    pub force: bool,
    /// Write the defaults without asking.
    pub yes: bool,
}

/// Runs the interview from `cwd`, or with `yes` takes the defaults, and writes `bli.toml`
/// at the checkout root; with `force`, over an existing one.
pub(crate) fn run(
    cwd: &Path,
    providers: &dyn Providers,
    options: Options,
) -> Result<String, Error> {
    let Options { force, yes } = options;
    // The same root `Config::load` stops its search at.
    let root = cwd
        .ancestors()
        .find(|d| d.join(".git").exists())
        .ok_or_else(|| {
            Error::new(format!(
                "not in a git checkout: {FILE_NAME} belongs at the root of one"
            ))
            .with_hint("run `bli init` inside the repo, or `git init` first")
        })?;
    let path = root.join(FILE_NAME);
    if path.exists() && !force {
        return Err(already_exists(&path));
    }
    if yes {
        write(&path, &render(&defaults())?, force)?;
        return Ok(format!("wrote {}\n", path.display()));
    }
    let prompter = providers.prompter()?;
    let prompter = prompter.as_ref();
    let cancelled = || Error::new("cancelled; no bli.toml was written");
    // A cancelled prompt has already closed the interview on screen.
    let document = interview(prompter).map_err(|Cancelled| cancelled())?;
    let text = render(&document)?;
    prompter.note(&path.display().to_string(), text.trim_end());
    if !prompter
        .confirm("Write bli.toml?", true)
        .map_err(|Cancelled| cancelled())?
    {
        prompter.outro_cancel("Nothing written");
        return Err(cancelled());
    }
    write(&path, &text, force)?;
    let token = if document.tracker.clickup.is_some() {
        format!(
            "set {} in the environment or in .env (never in bli.toml), then ",
            crate::clickup::TOKEN_VAR
        )
    } else {
        String::new()
    };
    prompter.outro(&format!(
        "Wrote {}. Next, {token}run `bli doctor` to check the setup",
        path.display()
    ));
    Ok(String::new())
}

/// The answers taken by pressing enter at every question.
fn defaults() -> Document {
    Document {
        lane: Lane {
            host: "herdr",
            base: "main".into(),
            branch: DEFAULT_PATTERN.into(),
            prompt: DEFAULT_PROMPT.into(),
        },
        tracker: Tracker {
            kind: "github",
            parent: None,
            on_merge: OnMerge::Close,
            clickup: None,
        },
    }
}

fn write(path: &Path, text: &str, force: bool) -> Result<(), Error> {
    if force {
        replace(path, text)
    } else {
        write_new(path, text)
    }
}

fn already_exists(path: &Path) -> Error {
    Error::new(format!("{} already exists", path.display()))
        .with_hint("edit it by hand, or run `bli init --force` to replace it")
}

/// Writes `text` to `path`, refusing a file made while the questions were being answered.
fn write_new(path: &Path, text: &str) -> Result<(), Error> {
    use std::io::Write;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(already_exists(path))
        }
        Err(e) => {
            return Err(Error::new(format!(
                "could not write {}: {e}",
                path.display()
            )))
        }
    };
    file.write_all(text.as_bytes())
        .map_err(|e| Error::new(format!("could not write {}: {e}", path.display())))
}

/// Replaces `path` with `text` in one step, so a failed write leaves the old file whole.
fn replace(path: &Path, text: &str) -> Result<(), Error> {
    use std::io::Write;
    let fail = |e: std::io::Error| Error::new(format!("could not write {}: {e}", path.display()));
    let (temp, mut file) = temp_file(path).map_err(fail)?;
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| std::fs::rename(&temp, path));
    drop(file);
    written.map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        fail(e)
    })
}

/// A new file beside `path` with a name not taken before. Creating it exclusively means
/// whatever already sits at a name, a symlink included, is never opened.
fn temp_file(path: &Path) -> std::io::Result<(std::path::PathBuf, std::fs::File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let mut last = None;
    for _ in 0..100 {
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        let temp = path.with_file_name(format!(
            ".{FILE_NAME}.{}.{nanos:x}.{n}.tmp",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => return Ok((temp, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.expect("tried at least once"))
}

/// The TOML for `document`, with `on_merge = { status = "..." }` inline, as the README writes it.
fn render(document: &Document) -> Result<String, Error> {
    let fail = |e: &dyn std::fmt::Display| Error::new(format!("could not write {FILE_NAME}: {e}"));
    let text = toml::to_string(document).map_err(|e| fail(&e))?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| fail(&e))?;
    let tracker = doc["tracker"].as_table_mut().expect("[tracker] is a table");
    if tracker
        .get("on_merge")
        .is_some_and(toml_edit::Item::is_table)
    {
        if let Some(toml_edit::Item::Table(table)) = tracker.remove("on_merge") {
            tracker.insert("on_merge", toml_edit::value(table.into_inline_table()));
        }
    }
    Ok(doc.to_string())
}

/// Asks every question.
fn interview(prompter: &dyn Prompter) -> Result<Document, Cancelled> {
    prompter.intro("bli init");
    let host = prompter.select("Which lane host runs your lanes?", &HOSTS, "herdr")?;
    let kind = prompter.select("Where do your tickets live?", &TRACKERS, "github")?;
    let clickup = if kind == "clickup" {
        Some(ClickUp {
            workspace_id: prompter
                .input(
                    "ClickUp workspace ID (the number after app.clickup.com/)",
                    "",
                    check_workspace_id,
                )?
                .trim()
                .to_string(),
            list_id: optional(prompter.input(
                "Only look for subtasks in this list ID",
                "",
                check_optional_number,
            )?),
            space_id: optional(prompter.input(
                "Only look for subtasks in this space ID",
                "",
                check_optional_number,
            )?),
        })
    } else {
        None
    };
    let parent = if clickup.is_some() {
        let parent = prompter.input(
            "Default parent task ID for `bli frontier`",
            "",
            check_clickup_parent,
        )?;
        clickup_task(&parent).map(|id| Parent::Text(id.to_string()))
    } else {
        let parent = prompter.input(
            "Default parent issue for `bli frontier`",
            "",
            check_github_parent,
        )?;
        github_issue(&parent).map(Parent::Number)
    };
    let choices: &[Choice] = if clickup.is_some() {
        &CLICKUP_ON_MERGE
    } else {
        &GITHUB_ON_MERGE
    };
    let on_merge = match prompter.select(
        "What happens to a ticket when its pull request merges?",
        choices,
        "close",
    )? {
        "none" => OnMerge::None,
        "status" => {
            let status = prompter.input("Status to move it to", "", check_status)?;
            OnMerge::Status(status.trim().to_string())
        }
        _ => OnMerge::Close,
    };
    let base = prompter.input("Base branch lanes are cut from", "main", config::check_base)?;
    let branch = prompter.input("Lane branch pattern", DEFAULT_PATTERN, check_branch)?;
    let prompt = prompter.input("The agent's first prompt", DEFAULT_PROMPT, check_prompt)?;
    Ok(Document {
        lane: Lane {
            host,
            base,
            branch,
            prompt,
        },
        tracker: Tracker {
            kind,
            parent,
            on_merge,
            clickup,
        },
    })
}

/// The issue number in `12` or `#12`.
fn github_issue(text: &str) -> Option<u64> {
    text.trim().trim_start_matches('#').parse().ok()
}

fn check_github_parent(text: &str) -> Result<(), String> {
    match text.trim() {
        "" => Ok(()),
        t if github_issue(t).is_some_and(|n| n > 0) => Ok(()),
        _ => Err("a GitHub parent is an issue number, like 12 or #12".into()),
    }
}

fn check_branch(text: &str) -> Result<(), String> {
    BranchPattern::parse(text)
        .map(drop)
        .map_err(|e| e.to_string())
}

fn optional(text: String) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn check_workspace_id(text: &str) -> Result<(), String> {
    match text.trim() {
        "" => Err(
            "ClickUp needs a workspace ID: the number after app.clickup.com/ in its URLs".into(),
        ),
        t => check_number(t),
    }
}

fn check_optional_number(text: &str) -> Result<(), String> {
    match text.trim() {
        "" => Ok(()),
        t => check_number(t),
    }
}

fn check_number(text: &str) -> Result<(), String> {
    if text.chars().all(|c| c.is_ascii_digit()) {
        Ok(())
    } else {
        Err(format!(
            "`{text}` is not a ClickUp ID: those are numbers, like 9012345"
        ))
    }
}

/// The task ID in `86c1x2y3z` or its URL `https://app.clickup.com/t/86c1x2y3z`, allowing
/// a trailing slash, query and fragment; `None` for anything else, an empty answer included.
fn clickup_task(text: &str) -> Option<&str> {
    let text = text.trim();
    let id = match text.strip_prefix("https://app.clickup.com/t/") {
        Some(rest) => {
            let path = rest.split(['?', '#']).next().unwrap_or_default();
            path.strip_suffix('/').unwrap_or(path)
        }
        None => text,
    };
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then_some(id)
}

fn check_clickup_parent(text: &str) -> Result<(), String> {
    if text.trim().is_empty() || clickup_task(text).is_some() {
        Ok(())
    } else {
        Err(format!(
            "`{}` is not a ClickUp task ID: use the <id> in https://app.clickup.com/t/<id>",
            text.trim()
        ))
    }
}

fn check_status(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        Err("name the status, as ClickUp shows it, e.g. Shipped".into())
    } else {
        Ok(())
    }
}

fn check_prompt(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        Err(format!(
            "the agent needs a first prompt, e.g. {DEFAULT_PROMPT}"
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_question_whose_validator_takes_an_empty_answer_offers_to_skip_it() {
        assert_eq!(skip_hint("", check_optional_number), Some(SKIP_HINT));
        assert_eq!(skip_hint("", check_clickup_parent), Some(SKIP_HINT));
        assert_eq!(skip_hint("", check_workspace_id), None);
        assert_eq!(skip_hint("", check_status), None);
        assert_eq!(skip_hint("main", config::check_base), None);
    }
}
