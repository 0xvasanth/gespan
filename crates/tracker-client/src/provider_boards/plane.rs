//! Plane over its REST API (Cloud or self-hosted). Reads a work item by its
//! key, comments on it and moves it between states.
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::format::{markdown_to_plane_comment_html, plane_html_to_markdown};
use super::{
    Account, BoardLabel, BoardProvider, ChangeHint, CommentRef, Container, IssueDraft, IssueFilter,
    IssuePatch, Member, MoveTarget, Page, ProviderError, ProviderKind, ProviderResult, RemoteIssue,
    StateGroup, SyncBatch, SyncCursor, WorkflowState,
};

const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RATE_LIMIT_WAIT: u64 = 60;
const MAX_LIST_PAGES: usize = 20;

/// The stored secret for one Plane connection.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlaneConfig {
    pub base_url: String,
    pub workspace: String,
    pub api_key: String,
}

impl fmt::Debug for PlaneConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlaneConfig")
            .field("base_url", &self.base_url)
            .field("workspace", &self.workspace)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

/// Status, response headers and parsed JSON body.
pub type PlaneReply = (u16, Vec<(String, String)>, Value);

/// Sends one Plane request and returns the status, the response headers and
/// the parsed JSON body (`null` when empty). A non-2xx status is a reply, not
/// an error: only a failure to get any reply is `Err`. Swapped out in tests.
pub trait PlaneTransport: Send + Sync {
    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<PlaneReply, String>;
}

struct HttpTransport {
    config: PlaneConfig,
}

impl HttpTransport {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.config.base_url)
    }
}

impl PlaneTransport for HttpTransport {
    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<PlaneReply, String> {
        let agent = ureq::AgentBuilder::new()
            .timeout(HTTP_TIMEOUT)
            .redirects(0)
            .build();
        let request = agent
            .request(method, &self.url(path))
            .set("X-API-Key", &self.config.api_key)
            .set("Accept", "application/json");
        let result = match body {
            Some(body) => {
                let payload = serde_json::to_string(&body).map_err(|error| error.to_string())?;
                request
                    .set("Content-Type", "application/json")
                    .send_string(&payload)
            }
            None => request.call(),
        };
        let response = match result {
            Ok(response) | Err(ureq::Error::Status(_, response)) => response,
            Err(_) => return Err("Could not reach Plane".into()),
        };
        let status = response.status();
        let headers = response
            .headers_names()
            .into_iter()
            .filter_map(|name| {
                let value = response.header(&name)?.to_string();
                Some((name, value))
            })
            .collect();
        let text = response
            .into_string()
            .map_err(|_| "Plane returned an unreadable response".to_string())?;
        let body = if text.trim().is_empty() {
            Value::Null
        } else {
            match serde_json::from_str(&text) {
                Ok(body) => body,
                // Error pages from a proxy in front of Plane are not JSON.
                Err(_) if !(200..300).contains(&status) => Value::Null,
                Err(_) => return Err("Plane returned invalid JSON".into()),
            }
        };
        Ok((status, headers, body))
    }
}

/// Wall clock and sleeper, so tests wait for a rate limit without waiting.
#[derive(Clone)]
struct Clock {
    now_epoch: Arc<dyn Fn() -> u64 + Send + Sync>,
    sleep: Arc<dyn Fn(Duration) + Send + Sync>,
}

impl Clock {
    fn system() -> Self {
        Self {
            now_epoch: Arc::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_secs())
            }),
            sleep: Arc::new(std::thread::sleep),
        }
    }
}

/// A Plane workspace, authenticated with a personal API key.
#[derive(Clone)]
pub struct PlaneProvider {
    config: PlaneConfig,
    transport: Arc<dyn PlaneTransport>,
    clock: Clock,
}

impl PlaneProvider {
    pub fn config(&self) -> &PlaneConfig {
        &self.config
    }

    pub fn new(base_url: &str, workspace: &str, api_key: &str) -> Result<Self, String> {
        let workspace = workspace.trim();
        if !is_path_safe(workspace) {
            return Err("Enter the slug of your Plane workspace".into());
        }
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err("Enter a Plane API key".into());
        }
        let config = PlaneConfig {
            base_url: normalize_base_url(base_url)?,
            workspace: workspace.to_string(),
            api_key: api_key.to_string(),
        };
        let transport = Arc::new(HttpTransport {
            config: config.clone(),
        });
        Ok(Self {
            config,
            transport,
            clock: Clock::system(),
        })
    }

    #[cfg(test)]
    fn with_transport(
        config: PlaneConfig,
        transport: Arc<dyn PlaneTransport>,
        clock: Clock,
    ) -> Self {
        Self {
            config,
            transport,
            clock,
        }
    }

    /// Where a person opens the work item. Plane Cloud's API host differs from
    /// its web host; a self-hosted site serves both from one origin.
    fn web_url(&self, key: &str) -> String {
        let origin = match self.config.base_url.as_str() {
            "https://api.plane.so" => "https://app.plane.so",
            other => other,
        };
        format!("{origin}/{}/browse/{key}/", self.config.workspace)
    }

    fn workspace_path(&self, rest: &str) -> String {
        format!("/api/v1/workspaces/{}{rest}", self.config.workspace)
    }

    /// One request, retried once after a rate limit. Plane reports when the
    /// limit lifts in `X-RateLimit-Reset` (epoch seconds).
    fn send(&self, method: &str, path: &str, body: Option<Value>) -> ProviderResult<Value> {
        let mut retried = false;
        loop {
            let (status, headers, reply) = self.transport.request(method, path, body.clone())?;
            if status == 429 {
                let wait = rate_limit_wait(&headers, (self.clock.now_epoch)());
                if retried {
                    return Err(ProviderError::Remote(format!(
                        "Plane rate limit: try again in {wait} s"
                    )));
                }
                retried = true;
                (self.clock.sleep)(Duration::from_secs(wait));
                continue;
            }
            return match status {
                // Redirects are never followed (the key would go along), so a
                // 3xx means the base URL is not where Plane lives.
                300..=399 => Err(remote("Plane redirected the request; check the base URL")),
                200..=299 => Ok(reply),
                401 | 403 => Err(ProviderError::Remote(
                    "Plane API key is invalid or has no access to this workspace".into(),
                )),
                _ => Err(ProviderError::Remote(plane_http_error(status, &reply))),
            };
        }
    }

    /// Every item of a list endpoint, whether Plane answers with a bare array
    /// or a cursor-paginated `{ results, next_page_results, next_cursor }`.
    fn list_all(&self, path: &str) -> ProviderResult<Vec<Value>> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let page_path = match &cursor {
                Some(cursor) => format!("{path}?cursor={cursor}"),
                None => path.to_string(),
            };
            let reply = self.send("GET", &page_path, None)?;
            if let Some(array) = reply.as_array() {
                items.extend(array.iter().cloned());
                return Ok(items);
            }
            let results = reply
                .get("results")
                .and_then(Value::as_array)
                .ok_or_else(|| remote("Plane returned an unexpected list"))?;
            items.extend(results.iter().cloned());
            let next = reply
                .get("next_cursor")
                .and_then(Value::as_str)
                .filter(|_| reply["next_page_results"].as_bool() == Some(true));
            match next {
                Some(next) if next.chars().all(is_cursor_char) => cursor = Some(next.to_string()),
                Some(_) => return Err(remote("Plane returned an unusable page cursor")),
                None => return Ok(items),
            }
        }
        Err(remote("Plane returned too many pages"))
    }

    /// The work item behind a key, as Plane's identifier lookup returns it.
    fn lookup(&self, key: &str) -> ProviderResult<(String, WorkItem)> {
        let key = valid_key(key)?;
        let path = self.workspace_path(&format!("/work-items/{key}/"));
        let item = self.send("GET", &path, None).map_err(|error| match error {
            ProviderError::Remote(message) if message.starts_with("Plane could not find") => {
                ProviderError::Remote(format!("Plane has no work item {key}"))
            }
            other => other,
        })?;
        let project_id = item
            .get("project")
            .or_else(|| item.get("project_id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let id = str_field(&item, "id");
        if !is_path_safe(&project_id) || !is_path_safe(&id) {
            return Err(remote(&format!("Plane returned no project for {key}")));
        }
        Ok((
            key,
            WorkItem {
                id,
                project_id,
                raw: item,
            },
        ))
    }

    fn item_path(&self, item: &WorkItem, rest: &str) -> String {
        self.workspace_path(&format!(
            "/projects/{}/work-items/{}{rest}",
            item.project_id, item.id
        ))
    }

    /// `known` spares a states request when the caller already listed them.
    fn remote_issue(
        &self,
        key: &str,
        item: &WorkItem,
        known: Option<&[WorkflowState]>,
    ) -> ProviderResult<RemoteIssue> {
        let raw = &item.raw;
        let state = match &raw["state"] {
            Value::Object(_) => parse_state(&raw["state"]),
            Value::String(id) => {
                let listed;
                let states = match known {
                    Some(states) => states,
                    None => {
                        listed = self.list_states(&item.project_id)?;
                        &listed
                    }
                };
                states
                    .iter()
                    .find(|state| &state.id == id)
                    .cloned()
                    .unwrap_or_else(|| WorkflowState {
                        id: id.clone(),
                        ..Default::default()
                    })
            }
            _ => WorkflowState::default(),
        };
        Ok(RemoteIssue {
            id: item.id.clone(),
            key: key.to_string(),
            title: str_field(raw, "name"),
            url: self.web_url(key),
            container_id: item.project_id.clone(),
            updated_at: str_field(raw, "updated_at"),
            state,
            labels: Vec::new(),
            assignees: Vec::new(),
            description: Some(plane_html_to_markdown(&str_field(raw, "description_html"))),
        })
    }
}

struct WorkItem {
    id: String,
    project_id: String,
    raw: Value,
}

fn remote(message: &str) -> ProviderError {
    ProviderError::Remote(message.into())
}

fn str_field(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn parse_state(value: &Value) -> WorkflowState {
    WorkflowState {
        id: str_field(value, "id"),
        name: str_field(value, "name"),
        group: StateGroup::from_plane_group(&str_field(value, "group")),
    }
}

/// Ids and slugs go straight into request paths.
fn is_path_safe(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_cursor_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_')
}

/// `PROJ-7`: a project identifier and a sequence number.
fn valid_key(key: &str) -> ProviderResult<String> {
    let key = key.trim();
    let valid = key.rsplit_once('-').is_some_and(|(identifier, number)| {
        !identifier.is_empty()
            && identifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !number.is_empty()
            && number.chars().all(|c| c.is_ascii_digit())
    });
    if valid {
        Ok(key.to_string())
    } else {
        Err(ProviderError::InvalidInput(format!(
            "\"{key}\" is not a Plane work item key such as PROJ-7"
        )))
    }
}

/// Seconds until `X-RateLimit-Reset`, at least 1 and at most a minute. Without
/// the header, a short wait is as good a guess as any.
fn rate_limit_wait(headers: &[(String, String)], now: u64) -> u64 {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-ratelimit-reset"))
        .and_then(|(_, value)| value.trim().parse::<u64>().ok())
        .map_or(1, |reset| reset.saturating_sub(now))
        .clamp(1, MAX_RATE_LIMIT_WAIT)
}

fn plane_http_error(status: u16, body: &Value) -> String {
    if status == 404 {
        return "Plane could not find that".into();
    }
    let message = ["error", "detail", "message"]
        .iter()
        .find_map(|field| body.get(field).and_then(Value::as_str))
        .unwrap_or_default();
    if message.is_empty() {
        format!("Plane returned HTTP {status}")
    } else {
        format!("Plane returned HTTP {status}: {message}")
    }
}

/// The origin of a Plane site. Plain `http` is for a local dev instance only,
/// so the API key never crosses a network unencrypted.
fn normalize_base_url(raw: &str) -> Result<String, String> {
    const INVALID: &str = "Enter the address of your Plane site, such as https://api.plane.so";
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(INVALID.into());
    }
    let (scheme, rest) = match raw.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("https".to_string(), raw),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(INVALID.into());
    }
    let authority = authority.to_ascii_lowercase();
    let host = authority.split(':').next().unwrap_or_default();
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    let local = host == "localhost" || host == "127.0.0.1";
    match scheme.as_str() {
        "https" if host_ok => Ok(format!("https://{authority}")),
        "http" if local => Ok(format!("http://{authority}")),
        "http" => {
            Err("Plane must be reached over https (http is only allowed for localhost)".into())
        }
        _ => Err(INVALID.into()),
    }
}

fn not_supported<T>(operation: &'static str) -> ProviderResult<T> {
    Err(ProviderError::NotSupported {
        provider: ProviderKind::Plane,
        operation,
    })
}

impl BoardProvider for PlaneProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Plane
    }

    fn whoami(&self) -> ProviderResult<Account> {
        let user = self.send("GET", "/api/v1/users/me/", None)?;
        let email = str_field(&user, "email");
        let name = [
            str_field(&user, "display_name"),
            format!(
                "{} {}",
                str_field(&user, "first_name"),
                str_field(&user, "last_name")
            )
            .trim()
            .to_string(),
            email.clone(),
        ]
        .into_iter()
        .find(|name| !name.is_empty())
        .unwrap_or_default();
        Ok(Account {
            id: str_field(&user, "id"),
            name,
            email,
        })
    }

    fn list_containers(&self) -> ProviderResult<Vec<Container>> {
        let projects = self.list_all(&self.workspace_path("/projects/"))?;
        Ok(projects
            .iter()
            .map(|project| Container {
                id: str_field(project, "id"),
                key: str_field(project, "identifier"),
                name: str_field(project, "name"),
            })
            .collect())
    }

    fn list_states(&self, container_id: &str) -> ProviderResult<Vec<WorkflowState>> {
        let project_id = container_id.trim();
        if !is_path_safe(project_id) {
            return Err(ProviderError::InvalidInput("Missing Plane project".into()));
        }
        let states =
            self.list_all(&self.workspace_path(&format!("/projects/{project_id}/states/")))?;
        Ok(states.iter().map(parse_state).collect())
    }

    fn list_labels(&self, _: &str) -> ProviderResult<Vec<BoardLabel>> {
        not_supported("list_labels")
    }

    fn list_members(&self, _: &str) -> ProviderResult<Vec<Member>> {
        not_supported("list_members")
    }

    fn list_issues(&self, _: &IssueFilter, _: Option<&str>) -> ProviderResult<Page<RemoteIssue>> {
        not_supported("list_issues")
    }

    /// `key` is the human identifier, such as `PROJ-7`.
    fn get_issue(&self, key: &str) -> ProviderResult<RemoteIssue> {
        let (key, item) = self.lookup(key)?;
        self.remote_issue(&key, &item, None)
    }

    fn create_issue(&self, _: &str, _: &IssueDraft) -> ProviderResult<RemoteIssue> {
        not_supported("create_issue")
    }

    fn update_issue(&self, _: &str, _: &IssuePatch) -> ProviderResult<RemoteIssue> {
        not_supported("update_issue")
    }

    /// `target.state_id` is a state id or, failing that, a state name.
    fn move_issue(&self, key: &str, target: &MoveTarget) -> ProviderResult<RemoteIssue> {
        if !target.required_fields.is_empty() {
            return Err(ProviderError::InvalidInput(
                "Plane moves take no extra fields".into(),
            ));
        }
        let wanted = target.state_id.trim();
        if wanted.is_empty() {
            return Err(ProviderError::InvalidInput("Missing Plane state".into()));
        }
        let (key, item) = self.lookup(key)?;
        let states = self.list_states(&item.project_id)?;
        let state = match states.iter().find(|state| state.id == wanted) {
            Some(state) => state,
            None => {
                let mut named = states
                    .iter()
                    .filter(|state| state.name.trim().eq_ignore_ascii_case(wanted));
                match (named.next(), named.next()) {
                    (Some(state), None) => state,
                    (Some(_), Some(_)) => {
                        return Err(ProviderError::InvalidInput(format!(
                            "More than one Plane state is named \"{wanted}\": use its id"
                        )))
                    }
                    _ => {
                        return Err(ProviderError::InvalidInput(format!(
                            "Plane has no state \"{wanted}\" in this project"
                        )))
                    }
                }
            }
        };
        let patched = self.send(
            "PATCH",
            &self.item_path(&item, "/"),
            Some(json!({ "state": state.id })),
        )?;
        let moved = WorkItem {
            id: item.id,
            project_id: item.project_id,
            raw: patched,
        };
        self.remote_issue(&key, &moved, Some(&states))
    }

    fn comment(&self, key: &str, body: &str, reply_to: Option<&str>) -> ProviderResult<CommentRef> {
        if reply_to.is_some() {
            return not_supported("comment replies");
        }
        if body.trim().is_empty() {
            return Err(ProviderError::InvalidInput(
                "Plane comment cannot be empty".into(),
            ));
        }
        let (key, item) = self.lookup(key)?;
        let created = self.send(
            "POST",
            &self.item_path(&item, "/comments/"),
            Some(json!({ "comment_html": markdown_to_plane_comment_html(body) })),
        )?;
        Ok(CommentRef {
            id: str_field(&created, "id"),
            url: self.web_url(&key),
        })
    }

    fn sync_since(&self, _: &str, _: Option<&SyncCursor>) -> ProviderResult<SyncBatch> {
        not_supported("sync_since")
    }

    fn parse_webhook(&self, _: &[(String, String)], _: &[u8]) -> ProviderResult<Vec<ChangeHint>> {
        not_supported("parse_webhook")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    const BASE: &str = "https://plane.example.com";
    const NOW: u64 = 1_000_000;

    type Reply = Result<(u16, Vec<(String, String)>, Value), String>;

    struct Scripted {
        replies: Mutex<VecDeque<Reply>>,
        requests: Mutex<Vec<(String, String, Option<Value>)>>,
    }

    impl Scripted {
        fn requests(&self) -> Vec<(String, String, Option<Value>)> {
            self.requests.lock().unwrap().clone()
        }

        fn paths(&self) -> Vec<String> {
            self.requests()
                .into_iter()
                .map(|(method, path, _)| format!("{method} {path}"))
                .collect()
        }
    }

    impl PlaneTransport for Scripted {
        fn request(&self, method: &str, path: &str, body: Option<Value>) -> Reply {
            self.requests
                .lock()
                .unwrap()
                .push((method.into(), path.into(), body));
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("an unscripted Plane request")
        }
    }

    fn ok(body: Value) -> Reply {
        Ok((200, Vec::new(), body))
    }

    fn limited(reset: u64) -> Reply {
        Ok((
            429,
            vec![("x-ratelimit-reset".into(), reset.to_string())],
            json!({ "error": "Rate limit exceeded" }),
        ))
    }

    fn scripted(replies: Vec<Reply>) -> (PlaneProvider, Arc<Scripted>, Arc<Mutex<Vec<Duration>>>) {
        let transport = Arc::new(Scripted {
            replies: Mutex::new(replies.into()),
            requests: Mutex::default(),
        });
        let sleeps = Arc::new(Mutex::new(Vec::new()));
        let recorded = sleeps.clone();
        let clock = Clock {
            now_epoch: Arc::new(|| NOW),
            sleep: Arc::new(move |wait| recorded.lock().unwrap().push(wait)),
        };
        let config = PlaneConfig {
            base_url: BASE.into(),
            workspace: "acme".into(),
            api_key: "plane_api_secret".into(),
        };
        (
            PlaneProvider::with_transport(config, transport.clone(), clock),
            transport,
            sleeps,
        )
    }

    fn work_item(state: Value) -> Value {
        json!({
            "id": "item-1",
            "name": "Fix the login redirect",
            "sequence_id": 7,
            "project": "proj-1",
            "state": state,
            "description_html": "<p>Redirect loops on <strong>logout</strong>.</p>",
            "updated_at": "2026-10-20T09:30:00Z"
        })
    }

    fn expanded_state() -> Value {
        json!({ "id": "st-2", "name": "In Progress", "group": "started" })
    }

    fn states_page() -> Value {
        json!({
            "results": [
                { "id": "st-1", "name": "Todo", "group": "unstarted" },
                { "id": "st-2", "name": "In Progress", "group": "started" },
                { "id": "st-3", "name": "Done", "group": "completed" }
            ],
            "next_page_results": false
        })
    }

    const LOOKUP: &str = "GET /api/v1/workspaces/acme/work-items/PROJ-7/";
    const STATES: &str = "GET /api/v1/workspaces/acme/projects/proj-1/states/";

    #[test]
    fn connect_checks_the_key() {
        let (plane, transport, _) = scripted(vec![ok(json!({
            "id": "user-1", "display_name": "ada", "first_name": "Ada",
            "last_name": "Lovelace", "email": "ada@example.com"
        }))]);
        let account = plane.connect().unwrap();
        assert_eq!(
            account,
            Account {
                id: "user-1".into(),
                name: "ada".into(),
                email: "ada@example.com".into()
            }
        );
        assert_eq!(transport.paths(), ["GET /api/v1/users/me/"]);
    }

    #[test]
    fn connect_reports_a_rejected_key_without_echoing_it() {
        let (plane, _, _) = scripted(vec![Ok((401, vec![], json!({ "detail": "bad" })))]);
        let error = plane.connect().unwrap_err().to_string();
        assert!(error.contains("API key"), "{error}");
        assert!(!error.contains("plane_api_secret"), "{error}");
    }

    #[test]
    fn get_by_key_resolves_project_identifier() {
        let (plane, transport, _) = scripted(vec![ok(work_item(expanded_state()))]);
        let issue = plane.get_issue("PROJ-7").unwrap();
        assert_eq!(transport.paths(), [LOOKUP]);
        assert_eq!(issue.id, "item-1");
        assert_eq!(issue.key, "PROJ-7");
        assert_eq!(issue.title, "Fix the login redirect");
        assert_eq!(issue.container_id, "proj-1");
        assert_eq!(issue.url, "https://plane.example.com/acme/browse/PROJ-7/");
        assert_eq!(issue.updated_at, "2026-10-20T09:30:00Z");
        assert_eq!(
            issue.description.as_deref(),
            Some("Redirect loops on **logout**.")
        );
        assert_eq!(
            issue.state,
            WorkflowState {
                id: "st-2".into(),
                name: "In Progress".into(),
                group: StateGroup::Started
            }
        );
    }

    #[test]
    fn get_by_key_looks_up_a_state_that_comes_back_as_an_id() {
        let (plane, transport, _) = scripted(vec![ok(work_item(json!("st-3"))), ok(states_page())]);
        let issue = plane.get_issue("PROJ-7").unwrap();
        assert_eq!(transport.paths(), [LOOKUP, STATES]);
        assert_eq!(issue.state.name, "Done");
        assert_eq!(issue.state.group, StateGroup::Completed);
    }

    #[test]
    fn get_by_key_reports_a_missing_work_item() {
        let (plane, _, _) = scripted(vec![Ok((404, vec![], json!({ "error": "not found" })))]);
        let error = plane.get_issue("PROJ-99").unwrap_err().to_string();
        assert!(error.contains("PROJ-99"), "{error}");
    }

    #[test]
    fn keys_that_are_not_identifiers_are_rejected_before_any_request() {
        let (plane, transport, _) = scripted(vec![]);
        for key in [
            "",
            "PROJ",
            "PROJ-",
            "-7",
            "PROJ-7/../x",
            "PROJ-7?x=1",
            "PR OJ-7",
        ] {
            let error = plane.get_issue(key).unwrap_err();
            assert!(matches!(error, ProviderError::InvalidInput(_)), "{key}");
        }
        assert!(transport.requests().is_empty());
    }

    #[test]
    fn comment_posts_html() {
        let (plane, transport, _) = scripted(vec![
            ok(work_item(expanded_state())),
            Ok((201, vec![], json!({ "id": "comment-1" }))),
            ok(work_item(expanded_state())),
            Ok((201, vec![], json!({ "id": "comment-2" }))),
        ]);
        let comment = plane.comment("PROJ-7", "**b**", None).unwrap();
        assert_eq!(comment.id, "comment-1");
        assert_eq!(comment.url, "https://plane.example.com/acme/browse/PROJ-7/");

        let requests = transport.requests();
        let (method, path, body) = &requests[1];
        assert_eq!(method, "POST");
        assert_eq!(
            path,
            "/api/v1/workspaces/acme/projects/proj-1/work-items/item-1/comments/"
        );
        let html = body.as_ref().unwrap()["comment_html"].as_str().unwrap();
        assert!(html.contains("<strong>b</strong>"), "{html}");

        plane.comment("PROJ-7", "a\nb", None).unwrap();
        let requests = transport.requests();
        let html = requests[3].2.as_ref().unwrap()["comment_html"]
            .as_str()
            .unwrap();
        assert!(html.contains("a<br"), "{html}");
        assert!(html.split_once("a<br").unwrap().1.contains('b'), "{html}");
    }

    #[test]
    fn comment_rejects_a_blank_body_without_a_request() {
        let (plane, transport, _) = scripted(vec![]);
        let error = plane.comment("PROJ-7", "  \n", None).unwrap_err();
        assert!(matches!(error, ProviderError::InvalidInput(_)));
        assert!(transport.requests().is_empty());
    }

    #[test]
    fn comment_replies_are_not_supported() {
        let (plane, transport, _) = scripted(vec![]);
        let error = plane.comment("PROJ-7", "hi", Some("c1")).unwrap_err();
        assert!(matches!(error, ProviderError::NotSupported { .. }));
        assert!(transport.requests().is_empty());
    }

    #[test]
    fn move_by_state_name() {
        let patched = work_item(json!("st-3"));
        let (plane, transport, _) = scripted(vec![
            ok(work_item(expanded_state())),
            ok(states_page()),
            ok(patched),
        ]);
        let target = MoveTarget {
            state_id: " done ".into(),
            ..Default::default()
        };
        let issue = plane.move_issue("PROJ-7", &target).unwrap();

        let requests = transport.requests();
        assert_eq!(transport.paths()[..2], [LOOKUP, STATES]);
        let (method, path, body) = &requests[2];
        assert_eq!(method, "PATCH");
        assert_eq!(
            path,
            "/api/v1/workspaces/acme/projects/proj-1/work-items/item-1/"
        );
        assert_eq!(body, &Some(json!({ "state": "st-3" })));
        assert_eq!(issue.state.name, "Done");
        assert_eq!(issue.state.group, StateGroup::Completed);
        assert_eq!(issue.key, "PROJ-7");
    }

    #[test]
    fn move_by_state_id() {
        let (plane, transport, _) = scripted(vec![
            ok(work_item(expanded_state())),
            ok(states_page()),
            ok(work_item(json!("st-1"))),
        ]);
        let target = MoveTarget {
            state_id: "st-1".into(),
            ..Default::default()
        };
        let issue = plane.move_issue("PROJ-7", &target).unwrap();
        assert_eq!(transport.requests()[2].2, Some(json!({ "state": "st-1" })));
        assert_eq!(issue.state.name, "Todo");
    }

    #[test]
    fn move_to_an_unknown_state_sends_nothing() {
        let (plane, transport, _) =
            scripted(vec![ok(work_item(expanded_state())), ok(states_page())]);
        let target = MoveTarget {
            state_id: "Shipped".into(),
            ..Default::default()
        };
        let error = plane.move_issue("PROJ-7", &target).unwrap_err();
        assert!(matches!(&error, ProviderError::InvalidInput(m) if m.contains("Shipped")));
        assert_eq!(transport.requests().len(), 2);
    }

    #[test]
    fn move_by_an_ambiguous_state_name_sends_nothing() {
        let states = json!({ "results": [
            { "id": "st-1", "name": "Review", "group": "started" },
            { "id": "st-2", "name": "review", "group": "completed" }
        ]});
        let (plane, transport, _) = scripted(vec![ok(work_item(expanded_state())), ok(states)]);
        let target = MoveTarget {
            state_id: "REVIEW".into(),
            ..Default::default()
        };
        let error = plane.move_issue("PROJ-7", &target).unwrap_err();
        assert!(matches!(error, ProviderError::InvalidInput(_)));
        assert_eq!(transport.requests().len(), 2);
    }

    #[test]
    fn move_with_extra_fields_is_rejected_before_any_request() {
        let (plane, transport, _) = scripted(vec![]);
        let mut target = MoveTarget {
            state_id: "Done".into(),
            ..Default::default()
        };
        target.required_fields.insert("x".into(), json!(1));
        assert!(matches!(
            plane.move_issue("PROJ-7", &target).unwrap_err(),
            ProviderError::InvalidInput(_)
        ));
        assert!(transport.requests().is_empty());
    }

    #[test]
    fn rate_limited_once_then_ok() {
        let (plane, transport, sleeps) = scripted(vec![
            limited(NOW + 5),
            ok(json!({ "id": "user-1", "email": "ada@example.com" })),
        ]);
        assert_eq!(plane.whoami().unwrap().id, "user-1");
        assert_eq!(*sleeps.lock().unwrap(), [Duration::from_secs(5)]);
        assert_eq!(
            transport.paths(),
            ["GET /api/v1/users/me/", "GET /api/v1/users/me/"]
        );
    }

    #[test]
    fn rate_limit_wait_is_capped_at_a_minute() {
        let (plane, _, sleeps) =
            scripted(vec![limited(NOW + 3_600), ok(json!({ "id": "user-1" }))]);
        plane.whoami().unwrap();
        assert_eq!(*sleeps.lock().unwrap(), [Duration::from_secs(60)]);
    }

    #[test]
    fn rate_limited_twice_reports_wait() {
        let (plane, transport, sleeps) = scripted(vec![limited(NOW + 5), limited(NOW + 8)]);
        let error = plane.whoami().unwrap_err();
        assert_eq!(
            error,
            ProviderError::Remote("Plane rate limit: try again in 8 s".into())
        );
        assert_eq!(sleeps.lock().unwrap().len(), 1);
        assert_eq!(transport.requests().len(), 2);
    }

    #[test]
    fn rate_limit_without_a_reset_header_still_waits_and_retries_once() {
        let (plane, _, sleeps) = scripted(vec![
            Ok((429, vec![], Value::Null)),
            ok(json!({ "id": "user-1" })),
        ]);
        plane.whoami().unwrap();
        assert_eq!(*sleeps.lock().unwrap(), [Duration::from_secs(1)]);
    }

    #[test]
    fn self_hosted_base_url_is_kept() {
        let plane = PlaneProvider::new("https://plane.example.com/", "acme", "key").unwrap();
        assert_eq!(plane.config.base_url, "https://plane.example.com");
        let transport = HttpTransport {
            config: plane.config.clone(),
        };
        assert_eq!(
            transport.url("/api/v1/users/me/"),
            "https://plane.example.com/api/v1/users/me/"
        );
    }

    fn provider_at(base_url: &str) -> PlaneProvider {
        PlaneProvider::new(base_url, "acme", "key").unwrap()
    }

    #[test]
    fn web_url_uses_the_app_host_on_plane_cloud_only() {
        let cases = [
            (
                "https://api.plane.so",
                "https://app.plane.so/acme/browse/PROJ-7/",
            ),
            (
                "https://plane.example.com",
                "https://plane.example.com/acme/browse/PROJ-7/",
            ),
            (
                "http://localhost:8000",
                "http://localhost:8000/acme/browse/PROJ-7/",
            ),
        ];
        for (base, expected) in cases {
            assert_eq!(provider_at(base).web_url("PROJ-7"), expected, "{base}");
        }
    }

    #[test]
    fn a_redirect_is_an_error_and_is_not_followed() {
        let (plane, transport, sleeps) = scripted(vec![Ok((
            302,
            vec![("location".into(), "http://evil.example/steal".into())],
            Value::Null,
        ))]);
        let error = plane.whoami().unwrap_err();
        assert_eq!(
            error,
            ProviderError::Remote("Plane redirected the request; check the base URL".into())
        );
        assert!(!error.to_string().contains("evil"));
        assert_eq!(transport.requests().len(), 1);
        assert!(sleeps.lock().unwrap().is_empty());
    }

    #[test]
    fn the_http_transport_does_not_follow_redirects() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut seen = Vec::new();
            listener.set_nonblocking(false).unwrap();
            // The redirect target is this same server: a follower would call back.
            let (mut first, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 4096];
            let read = first.read(&mut buffer).unwrap();
            seen.push(String::from_utf8_lossy(&buffer[..read]).to_string());
            first
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            drop(first);
            listener.set_nonblocking(true).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            (seen, listener.accept().is_ok())
        });
        let plane = provider_at(&format!("http://127.0.0.1:{port}"));
        let error = plane.whoami().unwrap_err();
        assert_eq!(
            error,
            ProviderError::Remote("Plane redirected the request; check the base URL".into())
        );
        let (seen, followed) = server.join().unwrap();
        assert!(!followed, "the redirect was followed");
        assert!(seen[0].to_ascii_lowercase().contains("x-api-key: key"));
    }

    #[test]
    fn base_urls_normalise_to_an_origin() {
        let cases = [
            ("https://api.plane.so", "https://api.plane.so"),
            ("  api.plane.so  ", "https://api.plane.so"),
            (
                "https://plane.example.com:8443/spaces?x=1",
                "https://plane.example.com:8443",
            ),
            ("http://localhost:8000/", "http://localhost:8000"),
            ("http://127.0.0.1:8000", "http://127.0.0.1:8000"),
        ];
        for (raw, expected) in cases {
            let plane = PlaneProvider::new(raw, "acme", "key").unwrap();
            assert_eq!(plane.config.base_url, expected, "{raw}");
        }
    }

    #[test]
    fn insecure_or_malformed_base_urls_are_rejected() {
        for raw in [
            "",
            "http://plane.example.com",
            "ftp://plane.example.com",
            "https://",
            "https://user:pw@plane.example.com",
            "http://localhost.evil.com",
        ] {
            assert!(PlaneProvider::new(raw, "acme", "key").is_err(), "{raw}");
        }
    }

    #[test]
    fn workspace_and_key_are_required_and_path_safe() {
        assert!(PlaneProvider::new(BASE, "", "key").is_err());
        assert!(PlaneProvider::new(BASE, "a/b", "key").is_err());
        assert!(PlaneProvider::new(BASE, "acme", "  ").is_err());
        let plane = PlaneProvider::new(BASE, " acme ", " key ").unwrap();
        assert_eq!(plane.config.workspace, "acme");
        assert_eq!(plane.config.api_key, "key");
    }

    #[test]
    fn config_debug_hides_the_key() {
        let config = PlaneConfig {
            base_url: BASE.into(),
            workspace: "acme".into(),
            api_key: "plane_api_secret".into(),
        };
        assert!(!format!("{config:?}").contains("plane_api_secret"));
    }

    #[test]
    fn list_containers_follows_pages_and_uses_the_identifier_as_key() {
        let (plane, transport, _) = scripted(vec![
            ok(json!({
                "results": [{ "id": "proj-1", "identifier": "PROJ", "name": "Platform" }],
                "next_page_results": true, "next_cursor": "100:1:0"
            })),
            ok(json!({
                "results": [{ "id": "proj-2", "identifier": "WEB", "name": "Web" }],
                "next_page_results": false
            })),
        ]);
        let containers = plane.list_containers().unwrap();
        assert_eq!(
            containers,
            vec![
                Container {
                    id: "proj-1".into(),
                    key: "PROJ".into(),
                    name: "Platform".into()
                },
                Container {
                    id: "proj-2".into(),
                    key: "WEB".into(),
                    name: "Web".into()
                },
            ]
        );
        assert_eq!(
            transport.paths(),
            [
                "GET /api/v1/workspaces/acme/projects/",
                "GET /api/v1/workspaces/acme/projects/?cursor=100:1:0"
            ]
        );
    }

    #[test]
    fn list_states_accepts_a_plain_array_and_maps_groups() {
        let (plane, _, _) = scripted(vec![ok(json!([
            { "id": "st-1", "name": "Triage", "group": "triage" },
            { "id": "st-2", "name": "Dropped", "group": "cancelled" }
        ]))]);
        let states = plane.list_states("proj-1").unwrap();
        assert_eq!(states[0].group, StateGroup::Triage);
        assert_eq!(states[1].group, StateGroup::Cancelled);
    }

    #[test]
    fn list_states_rejects_a_project_id_that_is_not_path_safe() {
        let (plane, transport, _) = scripted(vec![]);
        assert!(plane.list_states("../x").is_err());
        assert!(transport.requests().is_empty());
    }

    #[test]
    fn unsupported_operations_say_so() {
        let (plane, _, _) = scripted(vec![]);
        assert!(matches!(
            plane.create_issue("proj-1", &IssueDraft::default()),
            Err(ProviderError::NotSupported {
                provider: ProviderKind::Plane,
                ..
            })
        ));
        assert!(matches!(
            plane.sync_since("proj-1", None),
            Err(ProviderError::NotSupported { .. })
        ));
        assert_eq!(plane.kind(), ProviderKind::Plane);
    }

    #[test]
    fn server_errors_surface_the_status_and_message() {
        let (plane, _, _) = scripted(vec![Ok((
            500,
            vec![],
            json!({ "error": "Something broke" }),
        ))]);
        let error = plane.whoami().unwrap_err().to_string();
        assert!(
            error.contains("500") && error.contains("Something broke"),
            "{error}"
        );
    }

    #[test]
    fn transport_failures_are_remote_errors() {
        let (plane, _, _) = scripted(vec![Err("Could not reach Plane".into())]);
        assert_eq!(
            plane.whoami().unwrap_err(),
            ProviderError::Remote("Could not reach Plane".into())
        );
    }
}
