//! Seam 2: the ClickUp adapter, against a local stub of the ClickUp API v2
//! served through the configurable API base. Checks that each operation makes
//! the right requests and that the API's JSON parses into the interface's types.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use bli::check::CheckStatus;
use bli::clickup::{find_token, ClickUp, Token};
use bli::config::ClickUpConfig;
use bli::tracker::{IssueTracker, OnMerge, Ticket, TicketDetails};
use serde_json::{json, Value};

/// One request the stub received.
#[derive(Debug, Clone)]
struct Request {
    method: String,
    /// The path and query, e.g. `/task/abc?include_subtasks=true`.
    url: String,
    authorization: Option<String>,
    body: Option<Value>,
}

impl Request {
    fn path(&self) -> &str {
        self.url.split('?').next().unwrap()
    }
}

struct Route {
    method: &'static str,
    path: String,
    /// Fragments the query must contain for this route to answer.
    query: Vec<String>,
    status: u16,
    body: Value,
}

/// A stub ClickUp API answering canned JSON by method and path, recording every request.
struct Stub {
    base: String,
    routes: Arc<Mutex<Vec<Route>>>,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Stub {
    fn start() -> Stub {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let routes: Arc<Mutex<Vec<Route>>> = Arc::default();
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let (r, q) = (routes.clone(), requests.clone());
        thread::spawn(move || {
            for mut req in server.incoming_requests() {
                let mut text = String::new();
                req.as_reader().read_to_string(&mut text).unwrap();
                let recorded = Request {
                    method: req.method().to_string(),
                    url: req.url().to_string(),
                    authorization: req
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Authorization"))
                        .map(|h| h.value.to_string()),
                    body: serde_json::from_str(&text).ok(),
                };
                let query = recorded
                    .url
                    .split_once('?')
                    .map_or("", |(_, q)| q)
                    .to_string();
                let (status, body) = r
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|route| {
                        route.method == recorded.method
                            && route.path == recorded.path()
                            && route.query.iter().all(|f| query.contains(f.as_str()))
                    })
                    .map(|route| (route.status, route.body.clone()))
                    .unwrap_or((404, json!({"err": "no stub route", "ECODE": "STUB_404"})));
                q.lock().unwrap().push(recorded);
                let header =
                    tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap();
                let response = tiny_http::Response::from_string(body.to_string())
                    .with_status_code(status)
                    .with_header(header);
                let _ = req.respond(response);
            }
        });
        Stub {
            base,
            routes,
            requests,
        }
    }

    fn on(self, method: &'static str, path: &str, body: Value) -> Self {
        self.on_status(method, path, 200, body)
    }

    fn on_status(self, method: &'static str, path: &str, status: u16, body: Value) -> Self {
        self.on_query(method, path, &[], status, body)
    }

    /// Answers `method path` only when the query contains every one of `query`'s fragments.
    fn on_query(
        self,
        method: &'static str,
        path: &str,
        query: &[&str],
        status: u16,
        body: Value,
    ) -> Self {
        self.routes.lock().unwrap().push(Route {
            method,
            path: path.to_string(),
            query: query.iter().map(|f| f.to_string()).collect(),
            status,
            body,
        });
        self
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    /// The requests that changed something, as (method, path, body).
    fn writes(&self) -> Vec<(String, String, Value)> {
        self.requests()
            .into_iter()
            .filter(|r| r.method != "GET")
            .map(|r| {
                let path = r.path().to_string();
                (r.method, path, r.body.unwrap_or(Value::Null))
            })
            .collect()
    }

    fn settings(&self) -> ClickUpConfig {
        ClickUpConfig {
            workspace_id: "9001".into(),
            list_id: None,
            space_id: None,
            api_base: self.base.clone(),
        }
    }

    /// A ClickUp adapter talking to this stub with token `pk_test`.
    fn clickup(&self) -> ClickUp {
        ClickUp::new(&self.settings(), Some(token("pk_test")))
    }
}

fn token(value: &str) -> Token {
    Token {
        value: value.into(),
        source: "CLICKUP_API_TOKEN".into(),
    }
}

/// A task as the API returns it, with status `status` of type `kind`.
fn task(id: &str, name: &str, kind: &str) -> Value {
    json!({
        "id": id,
        "custom_id": null,
        "name": name,
        "status": {"status": if kind == "closed" { "complete" } else { "to do" }, "type": kind, "color": "#d3d3d3", "orderindex": 0},
        "assignees": [],
        "dependencies": [],
        "parent": "86parent",
        "list": {"id": "901", "name": "Backlog", "access": true},
        "url": format!("https://app.clickup.com/t/{id}"),
    })
}

fn with(mut task: Value, field: &str, value: Value) -> Value {
    task[field] = value;
    task
}

fn user(id: u64, username: &str) -> Value {
    json!({"id": id, "username": username, "email": format!("{username}@example.com"), "color": "#000000", "initials": "XX", "profilePicture": null})
}

/// `task_id` waits on `depends_on`.
fn waiting_on(task_id: &str, depends_on: &str) -> Value {
    json!({"task_id": task_id, "depends_on": depends_on, "type": 1, "date_created": "1700000000000", "userid": "1", "workspace_id": "9001", "chat_view_id": null})
}

fn page(tasks: Vec<Value>, last: bool) -> Value {
    json!({"tasks": tasks, "last_page": last})
}

fn clickup_ticket(id: &str, title: &str) -> Ticket {
    Ticket {
        id: id.into(),
        title: title.into(),
        url: format!("https://app.clickup.com/t/{id}"),
    }
}

#[test]
fn frontier_is_the_parents_open_subtasks_with_no_open_blocker_and_no_assignee() {
    let stub = Stub::start().on(
        "GET",
        "/team/9001/task",
        page(
            vec![
                task("86ready", "Ready one", "open"),
                task("86custom", "In a custom status", "custom"),
                task("86done", "Already done", "closed"),
                task("86donetype", "In a done status", "done"),
                with(
                    task("86taken", "Taken", "open"),
                    "assignees",
                    json!([user(7, "someone")]),
                ),
                with(
                    task("86blocked", "Blocked by an open sibling", "open"),
                    "dependencies",
                    json!([waiting_on("86blocked", "86ready")]),
                ),
                with(
                    task("86unblocked", "Waiting on a closed sibling", "open"),
                    "dependencies",
                    json!([waiting_on("86unblocked", "86done")]),
                ),
                // A task waiting on 86blocking does not block it.
                with(
                    task("86blocking", "Blocks another", "open"),
                    "dependencies",
                    json!([waiting_on("86other", "86blocking")]),
                ),
            ],
            true,
        ),
    );

    let tickets = stub.clickup().frontier("86parent").unwrap();

    assert_eq!(
        tickets,
        vec![
            clickup_ticket("86ready", "Ready one"),
            clickup_ticket("86custom", "In a custom status"),
            clickup_ticket("86unblocked", "Waiting on a closed sibling"),
            clickup_ticket("86blocking", "Blocks another"),
        ]
    );
    let requests = stub.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let query = requests[0].url.split_once('?').unwrap().1;
    for fragment in [
        "parent=86parent",
        "subtasks=true",
        "include_closed=true",
        "page=0",
    ] {
        assert!(query.contains(fragment), "{query}");
    }
    assert_eq!(requests[0].authorization.as_deref(), Some("pk_test"));
}

#[test]
fn frontier_reads_every_page_of_subtasks() {
    let stub = Stub::start()
        .on_query(
            "GET",
            "/team/9001/task",
            &["page=0"],
            200,
            page(vec![task("86one", "One", "open")], false),
        )
        .on_query(
            "GET",
            "/team/9001/task",
            &["page=1"],
            200,
            page(vec![task("86two", "Two", "open")], true),
        );

    let tickets = stub.clickup().frontier("86parent").unwrap();

    assert_eq!(
        tickets,
        vec![
            clickup_ticket("86one", "One"),
            clickup_ticket("86two", "Two")
        ]
    );
}

#[test]
fn frontier_looks_up_blockers_outside_the_parent() {
    let stub = Stub::start()
        .on(
            "GET",
            "/team/9001/task",
            page(
                vec![
                    with(
                        task("86waits", "Waits on an open outsider", "open"),
                        "dependencies",
                        json!([waiting_on("86waits", "86outopen")]),
                    ),
                    with(
                        task("86free", "Waits on a closed outsider", "open"),
                        "dependencies",
                        json!([waiting_on("86free", "86outdone")]),
                    ),
                ],
                true,
            ),
        )
        .on(
            "GET",
            "/task/86outopen",
            task("86outopen", "Elsewhere", "open"),
        )
        .on(
            "GET",
            "/task/86outdone",
            task("86outdone", "Elsewhere", "closed"),
        );

    let tickets = stub.clickup().frontier("86parent").unwrap();

    assert_eq!(
        tickets,
        vec![clickup_ticket("86free", "Waits on a closed outsider")]
    );
}

#[test]
fn frontier_is_narrowed_to_the_configured_list_and_space() {
    let stub = Stub::start().on("GET", "/team/9001/task", page(vec![], true));
    let mut settings = stub.settings();
    settings.list_id = Some("901".into());
    settings.space_id = Some("55".into());

    ClickUp::new(&settings, Some(token("pk_test")))
        .frontier("86parent")
        .unwrap();

    let url = &stub.requests()[0].url;
    assert!(url.contains("list_ids%5B%5D=901"), "{url}");
    assert!(url.contains("space_ids%5B%5D=55"), "{url}");
}

#[test]
fn ticket_reads_a_task_by_its_native_id() {
    let stub = Stub::start().on(
        "GET",
        "/task/86abc",
        with(
            task("86abc", "Add the frontier", "custom"),
            "assignees",
            json!([user(7, "ibedwi")]),
        ),
    );

    let details = stub.clickup().ticket("86abc").unwrap();

    assert_eq!(
        details,
        TicketDetails {
            ticket: clickup_ticket("86abc", "Add the frontier"),
            open: true,
            claimed_by: vec!["ibedwi".into()],
        }
    );
}

#[test]
fn ticket_in_a_closed_status_is_not_open() {
    let stub = Stub::start().on("GET", "/task/86abc", task("86abc", "Done", "closed"));

    assert!(!stub.clickup().ticket("86abc").unwrap().open);
}

#[test]
fn ticket_ids_that_are_not_native_task_ids_are_refused_without_a_request() {
    let stub = Stub::start();

    let e = stub.clickup().ticket("../team").unwrap_err();

    assert!(e.to_string().contains("not a ClickUp task ID"), "{e}");
    assert!(stub.requests().is_empty());
}

#[test]
fn claim_assigns_the_task_to_the_tokens_user() {
    let stub = Stub::start()
        .on("GET", "/user", json!({"user": user(42, "ibedwi")}))
        .on("PUT", "/task/86abc", task("86abc", "Claimed", "open"));

    stub.clickup().claim("86abc").unwrap();

    assert_eq!(
        stub.writes(),
        vec![(
            "PUT".into(),
            "/task/86abc".into(),
            json!({"assignees": {"add": [42], "rem": []}})
        )]
    );
}

#[test]
fn a_failed_request_reports_clickups_error() {
    let stub = Stub::start().on_status(
        "GET",
        "/task/86abc",
        401,
        json!({"err": "Token invalid", "ECODE": "OAUTH_025"}),
    );

    let e = stub.clickup().ticket("86abc").unwrap_err();

    assert!(e.to_string().contains("401"), "{e}");
    assert!(e.to_string().contains("Token invalid"), "{e}");
    assert!(e.hint().unwrap().contains("CLICKUP_API_TOKEN"), "{e:?}");
}

#[test]
fn without_a_token_every_call_fails_with_a_hint_and_sends_nothing() {
    let stub = Stub::start();
    let clickup = ClickUp::new(&stub.settings(), None);

    let e = clickup.ticket("86abc").unwrap_err();

    assert!(e.to_string().contains("no ClickUp API token"), "{e}");
    assert!(e.hint().unwrap().contains("CLICKUP_API_TOKEN"), "{e:?}");
    assert!(stub.requests().is_empty());
}

#[test]
fn pr_body_is_left_as_written_since_a_pull_request_cannot_close_a_clickup_task() {
    let stub = Stub::start();

    let body = stub
        .clickup()
        .pr_body("86abc", "Adds the frontier.\n")
        .unwrap();

    assert_eq!(body, "Adds the frontier.\n");
}

fn statuses_list() -> Value {
    json!({
        "id": "901",
        "name": "Backlog",
        "statuses": [
            {"status": "to do", "type": "open", "orderindex": 0, "color": "#d3d3d3"},
            {"status": "in review", "type": "custom", "orderindex": 1, "color": "#ffcc00"},
            {"status": "shipped", "type": "done", "orderindex": 2, "color": "#00ff00"},
            {"status": "complete", "type": "closed", "orderindex": 3, "color": "#6bc950"},
        ],
    })
}

#[test]
fn on_merge_close_moves_the_task_to_its_lists_closed_status() {
    let stub = Stub::start()
        .on("GET", "/task/86abc", task("86abc", "Merged", "custom"))
        .on("GET", "/list/901", statuses_list())
        .on("PUT", "/task/86abc", task("86abc", "Merged", "closed"));

    stub.clickup().on_merge("86abc", &OnMerge::Close).unwrap();

    assert_eq!(
        stub.writes(),
        vec![(
            "PUT".into(),
            "/task/86abc".into(),
            json!({"status": "complete"})
        )]
    );
}

#[test]
fn on_merge_close_leaves_a_closed_task_alone() {
    let stub = Stub::start().on("GET", "/task/86abc", task("86abc", "Merged", "closed"));

    stub.clickup().on_merge("86abc", &OnMerge::Close).unwrap();

    assert_eq!(stub.writes(), vec![]);
}

#[test]
fn on_merge_close_fails_with_a_hint_when_the_list_has_no_closed_status() {
    let stub = Stub::start()
        .on("GET", "/task/86abc", task("86abc", "Merged", "open"))
        .on(
            "GET",
            "/list/901",
            json!({"id": "901", "statuses": [{"status": "to do", "type": "open", "orderindex": 0}]}),
        );

    let e = stub
        .clickup()
        .on_merge("86abc", &OnMerge::Close)
        .unwrap_err();

    assert!(e.to_string().contains("closed status"), "{e}");
    assert!(e.hint().unwrap().contains("status"), "{e:?}");
    assert_eq!(stub.writes(), vec![]);
}

#[test]
fn on_merge_status_moves_the_task_to_the_named_status() {
    let stub = Stub::start()
        .on("GET", "/task/86abc", task("86abc", "Merged", "open"))
        .on("PUT", "/task/86abc", task("86abc", "Merged", "custom"));

    stub.clickup()
        .on_merge("86abc", &OnMerge::Status("in review".into()))
        .unwrap();

    assert_eq!(
        stub.writes(),
        vec![(
            "PUT".into(),
            "/task/86abc".into(),
            json!({"status": "in review"})
        )]
    );
}

#[test]
fn on_merge_status_leaves_a_task_already_in_that_status_alone() {
    // ClickUp reports status names in lower case.
    let stub = Stub::start().on(
        "GET",
        "/task/86abc",
        with(
            task("86abc", "Merged", "custom"),
            "status",
            json!({"status": "in review", "type": "custom", "orderindex": 1}),
        ),
    );

    stub.clickup()
        .on_merge("86abc", &OnMerge::Status("In Review".into()))
        .unwrap();

    assert_eq!(stub.writes(), vec![]);
}

#[test]
fn on_merge_none_sends_nothing() {
    let stub = Stub::start();

    stub.clickup().on_merge("86abc", &OnMerge::None).unwrap();

    assert!(stub.requests().is_empty());
}

#[test]
fn doctor_checks_the_token_is_found_and_works_in_the_workspace() {
    let stub = Stub::start()
        .on("GET", "/user", json!({"user": user(42, "ibedwi")}))
        .on(
            "GET",
            "/team",
            json!({"teams": [{"id": "1234", "name": "Other"}, {"id": "9001", "name": "Liburan"}]}),
        );

    let checks = stub.clickup().checks();

    let summary: Vec<_> = checks
        .iter()
        .map(|c| (c.name.as_str(), c.status, c.detail.as_str()))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("clickup token", CheckStatus::Pass, "from CLICKUP_API_TOKEN"),
            (
                "clickup auth",
                CheckStatus::Pass,
                "logged in as ibedwi, in workspace Liburan"
            ),
        ]
    );
}

#[test]
fn doctor_warns_when_the_token_cannot_see_the_configured_workspace() {
    let stub = Stub::start()
        .on("GET", "/user", json!({"user": user(42, "ibedwi")}))
        .on(
            "GET",
            "/team",
            json!({"teams": [{"id": "1234", "name": "Other"}]}),
        );

    let checks = stub.clickup().checks();

    assert_eq!(checks[1].status, CheckStatus::Warn);
    assert!(checks[1].detail.contains("9001"), "{:?}", checks[1]);
    assert!(checks[1].hint.as_deref().unwrap().contains("workspace_id"));
}

#[test]
fn doctor_warns_when_the_token_is_rejected() {
    let stub = Stub::start().on_status(
        "GET",
        "/user",
        401,
        json!({"err": "Token invalid", "ECODE": "OAUTH_025"}),
    );

    let checks = stub.clickup().checks();

    assert_eq!(checks[1].name, "clickup auth");
    assert_eq!(checks[1].status, CheckStatus::Warn);
    assert!(
        checks[1].detail.contains("Token invalid"),
        "{:?}",
        checks[1]
    );
}

#[test]
fn doctor_warns_when_there_is_no_token_and_skips_the_api() {
    let stub = Stub::start();

    let checks = ClickUp::new(&stub.settings(), None).checks();

    assert_eq!(checks.len(), 1, "{checks:?}");
    assert_eq!(checks[0].name, "clickup token");
    assert_eq!(checks[0].status, CheckStatus::Warn);
    assert!(checks[0]
        .hint
        .as_deref()
        .unwrap()
        .contains("CLICKUP_API_TOKEN"));
    assert!(stub.requests().is_empty());
}

fn write_env(dir: &Path, text: &str) {
    std::fs::write(dir.join(".env"), text).unwrap();
}

#[test]
fn the_token_comes_from_the_environment_first() {
    let checkout = tempfile::tempdir().unwrap();
    write_env(checkout.path(), "CLICKUP_API_TOKEN=pk_dotenv\n");

    let found = find_token(Some("pk_env".into()), Some(checkout.path()), None);

    assert_eq!(found, Some(token("pk_env")));
}

#[test]
fn the_token_comes_from_the_checkouts_env_file_next() {
    let checkout = tempfile::tempdir().unwrap();
    let main = tempfile::tempdir().unwrap();
    write_env(
        checkout.path(),
        "# secrets\nOTHER=1\nexport CLICKUP_API_TOKEN=\"pk_checkout\"\n",
    );
    write_env(main.path(), "CLICKUP_API_TOKEN=pk_main\n");

    let found = find_token(None, Some(checkout.path()), Some(main.path())).unwrap();

    assert_eq!(found.value, "pk_checkout");
    assert_eq!(
        found.source,
        checkout.path().join(".env").display().to_string()
    );
}

#[test]
fn the_token_comes_from_the_main_checkouts_env_file_last() {
    let lane = tempfile::tempdir().unwrap();
    let main = tempfile::tempdir().unwrap();
    write_env(lane.path(), "OTHER=1\nCLICKUP_API_TOKEN=\n");
    write_env(main.path(), "CLICKUP_API_TOKEN='pk_main'\n");

    let found = find_token(Some(String::new()), Some(lane.path()), Some(main.path())).unwrap();

    assert_eq!(found.value, "pk_main");
    assert_eq!(found.source, main.path().join(".env").display().to_string());
}

#[test]
fn no_token_anywhere_is_none() {
    let lane = tempfile::tempdir().unwrap();

    assert_eq!(find_token(None, Some(lane.path()), Some(lane.path())), None);
}

#[test]
fn on_merge_close_leaves_a_task_in_a_done_status_alone() {
    let stub = Stub::start().on("GET", "/task/86abc", task("86abc", "Merged", "done"));

    stub.clickup().on_merge("86abc", &OnMerge::Close).unwrap();

    assert_eq!(stub.writes(), vec![]);
}

#[test]
fn frontier_leaves_out_subtasks_of_subtasks() {
    let stub = Stub::start().on(
        "GET",
        "/team/9001/task",
        page(
            vec![
                task("86child", "Child", "open"),
                with(
                    task("86grandchild", "Grandchild", "open"),
                    "parent",
                    json!("86child"),
                ),
            ],
            true,
        ),
    );

    let tickets = stub.clickup().frontier("86parent").unwrap();

    assert_eq!(tickets, vec![clickup_ticket("86child", "Child")]);
}
