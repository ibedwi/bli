//! ClickUp as the issue tracker, through its HTTP API v2. A ticket is a task:
//! the frontier is a parent task's subtasks, "waiting on" dependencies are
//! blockers, and the assignee is the claim.

use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::check::Check;
use crate::config::ClickUpConfig;
use crate::error::Error;
use crate::tracker::{IssueTracker, OnMerge, Ticket, TicketDetails};

/// The environment variable, and `.env` key, holding the ClickUp API token.
pub const TOKEN_VAR: &str = "CLICKUP_API_TOKEN";

/// A ClickUp API token, and where it was found, for `bli doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub value: String,
    pub source: String,
}

pub struct ClickUp {
    settings: ClickUpConfig,
    token: Option<Token>,
    agent: ureq::Agent,
}

impl ClickUp {
    /// A client for the workspace in `settings`; without a token, every call fails saying so.
    pub fn new(settings: &ClickUpConfig, token: Option<Token>) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        ClickUp {
            settings: settings.clone(),
            token,
            agent,
        }
    }

    /// `GET`s `path` with `query` and parses the JSON reply.
    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T, Error> {
        let token = self.token()?;
        let request = self
            .agent
            .get(self.url(path))
            .header("Authorization", &token.value)
            .query_pairs(query.iter().copied());
        parse_reply(request.call(), &format!("GET {path}"))
    }

    /// `PUT`s `body` as JSON to `path` and parses the JSON reply.
    fn put<T: DeserializeOwned>(&self, path: &str, body: &serde_json::Value) -> Result<T, Error> {
        let token = self.token()?;
        let request = self
            .agent
            .put(self.url(path))
            .header("Authorization", &token.value)
            .header("Content-Type", "application/json");
        parse_reply(request.send(body.to_string()), &format!("PUT {path}"))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.settings.api_base.trim_end_matches('/'))
    }

    /// Every subtask of `parent`, closed ones included, across all pages.
    fn subtasks(&self, parent: &str) -> Result<Vec<Task>, Error> {
        let path = format!("/team/{}/task", self.settings.workspace_id);
        let mut tasks = Vec::new();
        for page in 0.. {
            let page = page.to_string();
            let mut query = vec![
                ("parent", parent),
                ("subtasks", "true"),
                ("include_closed", "true"),
                ("page", page.as_str()),
            ];
            if let Some(list) = &self.settings.list_id {
                query.push(("list_ids[]", list));
            }
            if let Some(space) = &self.settings.space_id {
                query.push(("space_ids[]", space));
            }
            let reply: TaskPage = self.get(&path, &query)?;
            let last = reply.last_page || reply.tasks.is_empty();
            tasks.extend(reply.tasks);
            if last {
                break;
            }
        }
        Ok(tasks)
    }

    fn task(&self, id: &str) -> Result<Task, Error> {
        self.get(&format!("/task/{id}"), &[])
    }

    /// Who the token logs in as, and that it can see the configured workspace.
    fn whoami(&self) -> Result<String, Error> {
        let me: Me = self.get("/user", &[])?;
        let teams: Teams = self.get("/team", &[])?;
        let workspace = &self.settings.workspace_id;
        let team = teams
            .teams
            .iter()
            .find(|t| &t.id == workspace)
            .ok_or_else(|| {
                Error::new(format!(
                    "the token's user {} cannot see workspace {workspace}",
                    me.user.name()
                ))
                .with_hint("check `workspace_id` under [tracker.clickup] in bli.toml")
            })?;
        Ok(format!(
            "logged in as {}, in workspace {}",
            me.user.name(),
            team.name
        ))
    }

    fn token(&self) -> Result<&Token, Error> {
        self.token.as_ref().ok_or_else(|| {
            Error::new("no ClickUp API token found").with_hint(format!(
                "set {TOKEN_VAR} in the environment, or in the .env of this checkout or the main checkout"
            ))
        })
    }
}

/// Finds the API token: `env` (the value of [`TOKEN_VAR`]), else the `.env` in
/// `checkout`, else the `.env` in `main_checkout`. Empty values don't count.
pub fn find_token(
    env: Option<String>,
    checkout: Option<&Path>,
    main_checkout: Option<&Path>,
) -> Option<Token> {
    if let Some(value) = env.filter(|v| !v.is_empty()) {
        return Some(Token {
            value,
            source: TOKEN_VAR.to_string(),
        });
    }
    [checkout, main_checkout]
        .into_iter()
        .flatten()
        .find_map(|dir| {
            let path = dir.join(".env");
            let text = std::fs::read_to_string(&path).ok()?;
            dotenv_value(&text, TOKEN_VAR).map(|value| Token {
                value,
                source: path.display().to_string(),
            })
        })
}

/// The non-empty value of `key` in a `.env` file's `KEY=value` lines, which may
/// start with `export` and quote their value.
fn dotenv_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (k, v) = line.split_once('=')?;
        if k.trim() != key {
            return None;
        }
        let v = v.trim();
        let unquoted = ['"', '\'']
            .iter()
            .find_map(|q| v.strip_prefix(*q)?.strip_suffix(*q))
            .unwrap_or(v);
        Some(unquoted.to_string()).filter(|v| !v.is_empty())
    })
}

/// Parses the JSON reply to `request`, turning a transport failure or a non-2xx status into an error.
fn parse_reply<T: DeserializeOwned>(
    reply: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    request: &str,
) -> Result<T, Error> {
    let mut reply =
        reply.map_err(|e| Error::new(format!("ClickUp `{request}` could not be sent: {e}")))?;
    let status = reply.status().as_u16();
    let text = reply.body_mut().read_to_string().map_err(|e| {
        Error::new(format!(
            "could not read ClickUp's reply to `{request}`: {e}"
        ))
    })?;
    if !(200..300).contains(&status) {
        // ClickUp explains failures as `{"err": "...", "ECODE": "..."}`.
        let why = serde_json::from_str::<ApiError>(&text).map_or(text, |e| e.err);
        let e = Error::new(format!("ClickUp `{request}` failed ({status}): {why}"));
        return Err(match status {
            401 => e.with_hint(format!(
                "check that {TOKEN_VAR} holds a valid ClickUp personal API token"
            )),
            _ => e,
        });
    }
    serde_json::from_str(&text).map_err(|e| {
        Error::new(format!(
            "could not read ClickUp's reply to `{request}`: {e}"
        ))
    })
}

impl IssueTracker for ClickUp {
    fn frontier(&self, parent: &str) -> Result<Vec<Ticket>, Error> {
        let parent = task_id(parent)?;
        let tasks = self.subtasks(parent)?;
        let mut candidates = Vec::new();
        // The listing can reach deeper than the parent's own subtasks.
        let children = tasks.iter().filter(|t| t.parent.as_deref() == Some(parent));
        for task in children.filter(|t| t.is_open() && t.assignees.is_empty()) {
            let mut blocked = false;
            for blocker in task.blockers() {
                // Closed siblings are listed too, so only blockers elsewhere need looking up.
                blocked = match tasks.iter().find(|t| t.id == blocker) {
                    Some(sibling) => sibling.is_open(),
                    None => self.task(blocker)?.is_open(),
                };
                if blocked {
                    break;
                }
            }
            if !blocked {
                candidates.push(task.ticket());
            }
        }
        Ok(candidates)
    }

    fn ticket(&self, id: &str) -> Result<TicketDetails, Error> {
        let task = self.task(task_id(id)?)?;
        Ok(TicketDetails {
            ticket: task.ticket(),
            open: task.is_open(),
            claimed_by: task.assignees.iter().map(User::name).collect(),
        })
    }

    /// On ClickUp, a ticket is claimed by assigning it to the token's user.
    fn claim(&self, id: &str) -> Result<(), Error> {
        let id = task_id(id)?;
        let me: Me = self.get("/user", &[])?;
        let body = serde_json::json!({"assignees": {"add": [me.user.id], "rem": []}});
        self.put::<serde_json::Value>(&format!("/task/{id}"), &body)
            .map(drop)
    }

    /// ClickUp can't read pull requests, so the body is left as written; `lane clean`'s
    /// on-merge action closes the ticket instead.
    fn pr_body(&self, _id: &str, body: &str) -> Result<String, Error> {
        Ok(body.to_string())
    }

    fn on_merge(&self, id: &str, action: &OnMerge) -> Result<(), Error> {
        let id = task_id(id)?;
        let target = match action {
            OnMerge::None => return Ok(()),
            OnMerge::Status(status) => {
                if self.task(id)?.status.status.eq_ignore_ascii_case(status) {
                    return Ok(());
                }
                status.clone()
            }
            OnMerge::Close => {
                let task = self.task(id)?;
                if !task.is_open() {
                    return Ok(());
                }
                // Each list names its own closed status.
                let list: List = self.get(&format!("/list/{}", task.list.id), &[])?;
                list.statuses
                    .into_iter()
                    .find(|s| s.kind == "closed")
                    .map(|s| s.status)
                    .ok_or_else(|| {
                        Error::new(format!(
                            "cannot close task {id}: its list {} has no closed status",
                            task.list.id
                        ))
                        .with_hint(
                            "set `on_merge = { status = \"...\" }` under [tracker] in bli.toml",
                        )
                    })?
            }
        };
        let body = serde_json::json!({ "status": target });
        self.put::<serde_json::Value>(&format!("/task/{id}"), &body)
            .map(drop)
    }

    fn checks(&self) -> Vec<Check> {
        let token = match self.token() {
            Ok(token) => token,
            Err(e) => return vec![Check::warn("clickup token", &e)],
        };
        vec![
            Check::pass("clickup token", format!("from {}", token.source)),
            Check::from_result("clickup auth", self.whoami()),
        ]
    }
}

/// A ClickUp ticket ID is a native task ID, such as `86c1x2y3z`.
fn task_id(id: &str) -> Result<&str, Error> {
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric()) {
        Ok(id)
    } else {
        Err(Error::new(format!("`{id}` is not a ClickUp task ID"))
            .with_hint("use the task's native ID, as in its URL: https://app.clickup.com/t/<id>"))
    }
}

#[derive(Deserialize)]
struct TaskPage {
    tasks: Vec<Task>,
    #[serde(default)]
    last_page: bool,
}

#[derive(Deserialize)]
struct Task {
    id: String,
    name: String,
    url: String,
    status: Status,
    #[serde(default)]
    assignees: Vec<User>,
    #[serde(default)]
    dependencies: Vec<Dependency>,
    /// The task this one is a subtask of.
    #[serde(default)]
    parent: Option<String>,
    list: ListRef,
}

#[derive(Deserialize)]
struct ListRef {
    id: String,
}

/// `GET /list/{id}`: a list and the statuses its tasks move through.
#[derive(Deserialize)]
struct List {
    statuses: Vec<Status>,
}

impl Task {
    /// Tasks in a "closed" or "done" status are finished; "open" and custom statuses are not.
    fn is_open(&self) -> bool {
        !matches!(self.status.kind.as_str(), "closed" | "done")
    }

    /// The tasks this one is waiting on.
    fn blockers(&self) -> impl Iterator<Item = &str> {
        self.dependencies
            .iter()
            .filter(|d| d.task_id == self.id)
            .map(|d| d.depends_on.as_str())
    }

    fn ticket(&self) -> Ticket {
        Ticket {
            id: self.id.clone(),
            title: self.name.clone(),
            url: self.url.clone(),
        }
    }
}

#[derive(Deserialize)]
struct Status {
    status: String,
    #[serde(rename = "type")]
    kind: String,
}

/// An entry of a task's `dependencies`: `task_id` is waiting on `depends_on`.
#[derive(Deserialize)]
struct Dependency {
    task_id: String,
    depends_on: String,
}

#[derive(Deserialize)]
struct User {
    id: u64,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

impl User {
    /// How a user is shown: their username, else their email, else their ID.
    fn name(&self) -> String {
        self.username
            .clone()
            .or_else(|| self.email.clone())
            .unwrap_or_else(|| self.id.to_string())
    }
}

/// `GET /user`: the token's own user.
#[derive(Deserialize)]
struct Me {
    user: User,
}

#[derive(Deserialize)]
struct ApiError {
    err: String,
}

/// `GET /team`: the workspaces the token can see.
#[derive(Deserialize)]
struct Teams {
    teams: Vec<Team>,
}

#[derive(Deserialize)]
struct Team {
    id: String,
    name: String,
}
