//! Jira Cloud over REST v3. Shared by the Inbox and the board sync.
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    Account, BoardLabel, BoardProvider, ChangeHint, CommentRef, Container, IssueDraft, IssueFilter,
    IssuePatch, Member, MoveTarget, Page, ProviderError, ProviderKind, ProviderResult, RemoteIssue,
    StateGroup, SyncBatch, SyncCursor, WorkflowState,
};

const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const COMMENT_LIMIT: u32 = 50;
const ISSUE_FIELDS: [&str; 6] = [
    "summary", "status", "updated", "labels", "assignee", "project",
];

const MAX_PAGE: u32 = 100;
const USER_PAGE: usize = 100;

/// The `jira-config.json` file format.
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct JiraConfig {
    pub(crate) site: String,
    pub(crate) email: String,
    pub(crate) token: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JiraProject {
    pub id: String,
    pub key: String,
    pub name: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JiraLabel {
    pub name: String,
    pub color: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JiraAssignee {
    pub login: String,
    pub avatar_url: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JiraIssue {
    pub provider: String,
    pub kind: String,
    pub id: String,
    pub identifier: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub state_type: String,
    pub updated_at: String,
    pub labels: Vec<JiraLabel>,
    pub assignees: Vec<JiraAssignee>,
    pub draft: bool,
    pub repo: String,
    pub team_id: String,
    pub team_name: String,
    pub project_path: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JiraIssueDetails {
    pub body: String,
    pub author: String,
    pub author_avatar_url: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JiraIssueComment {
    pub id: String,
    pub kind: String,
    pub author: String,
    pub author_avatar_url: String,
    pub body: String,
    pub created_at: String,
    pub url: String,
    pub state: String,
    pub path: String,
    pub line: Option<i64>,
    pub resolved: bool,
    pub thread_id: String,
    pub replies: Vec<JiraIssueComment>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JiraIssueThread {
    pub comments: Vec<JiraIssueComment>,
    pub truncated: bool,
    pub review_decision: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
}

/// A Jira Cloud site, authenticated with an Atlassian email and API token.
#[derive(Clone)]
pub struct JiraProvider {
    config: JiraConfig,
}

impl JiraProvider {
    pub fn new(site: &str, email: &str, token: &str) -> Result<Self, String> {
        let email = email.trim().to_string();
        if email.is_empty() || !email.contains('@') {
            return Err("Enter the email address of your Atlassian account".into());
        }
        Ok(Self {
            config: JiraConfig {
                site: normalize_jira_site(site)?,
                email,
                token: token.trim().to_string(),
            },
        })
    }

    pub(crate) fn from_config(config: JiraConfig) -> Self {
        Self { config }
    }

    pub(crate) fn config(&self) -> &JiraConfig {
        &self.config
    }

    pub fn site(&self) -> &str {
        &self.config.site
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        jira_get(&self.config, path)
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        jira_post(&self.config, path, body)
    }

    /// Proves the credentials work.
    pub fn verify(&self) -> Result<(), String> {
        parse_myself(&self.get("/rest/api/3/myself")?).map(drop)
    }

    pub fn projects(&self) -> Result<Vec<JiraProject>, String> {
        fetch_jira_projects(|start| {
            self.get(&format!(
                "/rest/api/3/project/search?maxResults=100&orderBy=name&startAt={start}"
            ))
        })
    }

    /// The Inbox list: `state` is `"all"` or anything else for open issues only.
    pub fn inbox_issues(
        &self,
        assigned_to_me: bool,
        state: &str,
        project_ids: &[String],
        limit: u32,
    ) -> Result<Vec<JiraIssue>, String> {
        let jql = issue_jql(assigned_to_me, state, project_ids);
        let data = self.post(
            "/rest/api/3/search/jql",
            &json!({ "jql": jql, "maxResults": limit, "fields": ISSUE_FIELDS }),
        )?;
        parse_jira_issues(&data, self.site())
    }

    pub fn issue_details(&self, key: &str) -> Result<JiraIssueDetails, String> {
        let key = require_issue_key(key)?;
        let data = self.get(&format!(
            "/rest/api/3/issue/{key}?fields=description,reporter,creator,assignee"
        ))?;
        parse_jira_issue_details(&data)
    }

    pub fn issue_thread(&self, key: &str) -> Result<JiraIssueThread, String> {
        let key = require_issue_key(key)?;
        // Newest first so a long thread keeps its latest comments; re-sorted below.
        let data = self.get(&format!(
            "/rest/api/3/issue/{key}/comment?maxResults={COMMENT_LIMIT}&orderBy=-created"
        ))?;
        parse_jira_issue_thread(&data, self.site(), key)
    }

    pub fn post_comment(&self, key: &str, body: &str) -> Result<CommentRef, String> {
        let (key, body) = validate_comment(key, body)?;
        let data = self.post(
            &format!("/rest/api/3/issue/{key}/comment"),
            &json!({ "body": text_to_adf(body) }),
        )?;
        let id = string_field(&data, "id").unwrap_or_default();
        if id.is_empty() {
            return Err("Could not post Jira comment".into());
        }
        Ok(CommentRef {
            url: comment_url(self.site(), key, &id),
            id,
        })
    }
}

fn validate_comment<'a>(key: &'a str, body: &'a str) -> Result<(&'a str, &'a str), String> {
    let key = require_issue_key(key)?;
    let body = body.trim();
    if body.is_empty() {
        return Err("Comment cannot be empty".into());
    }
    Ok((key, body))
}

fn issue_jql(assigned_to_me: bool, state: &str, project_ids: &[String]) -> String {
    let mut clauses = Vec::new();
    if assigned_to_me {
        clauses.push("assignee = currentUser()".to_string());
    }
    let ids: Vec<&str> = project_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty() && id.chars().all(|ch| ch.is_ascii_digit()))
        .collect();
    if !ids.is_empty() {
        clauses.push(format!("project in ({})", ids.join(", ")));
    }
    if !state.trim().eq_ignore_ascii_case("all") {
        clauses.push("statusCategory != Done".to_string());
    }
    // The search endpoint rejects unbounded JQL, so an unfiltered query is
    // limited to the last year of activity.
    if clauses.is_empty() {
        clauses.push("updated >= -365d".to_string());
    }
    format!("{} ORDER BY updated DESC", clauses.join(" AND "))
}

fn jira_authorization(config: &JiraConfig) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!(
        "{}:{}",
        config.email.trim(),
        config.token.trim()
    ));
    format!("Basic {encoded}")
}

fn jira_get(config: &JiraConfig, path: &str) -> Result<Value, String> {
    let agent = ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).build();
    let result = agent
        .get(&format!("{}{path}", config.site))
        .set("Authorization", &jira_authorization(config))
        .set("Accept", "application/json")
        .call();
    read_jira_response(result)
}

fn jira_post(config: &JiraConfig, path: &str, body: &Value) -> Result<Value, String> {
    let agent = ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).build();
    let payload = serde_json::to_string(body).map_err(|error| error.to_string())?;
    let result = agent
        .post(&format!("{}{path}", config.site))
        .set("Authorization", &jira_authorization(config))
        .set("Accept", "application/json")
        .set("Content-Type", "application/json")
        .send_string(&payload);
    read_jira_response(result)
}

fn read_jira_response(result: Result<ureq::Response, ureq::Error>) -> Result<Value, String> {
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(401, _)) => {
            return Err("Jira email or API token is invalid".into());
        }
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            return Err(jira_http_error(status, &body));
        }
        Err(_) => return Err("Could not reach Jira".into()),
    };
    let status = response.status();
    let body = response
        .into_string()
        .map_err(|_| "Jira returned an unreadable response".to_string())?;
    if !(200..300).contains(&status) {
        return Err(jira_http_error(status, &body));
    }
    serde_json::from_str(&body).map_err(|_| "Jira returned invalid JSON".to_string())
}

fn jira_http_error(status: u16, body: &str) -> String {
    if let Some(message) = jira_error_message(body) {
        return message;
    }
    match status {
        403 => "Jira denied access. Check the API token's permissions".into(),
        404 => "Jira could not find that issue".into(),
        _ => format!("Jira request failed ({status})"),
    }
}

fn jira_error_message(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let from_list = parsed
        .get("errorMessages")
        .and_then(Value::as_array)
        .and_then(|messages| messages.iter().filter_map(Value::as_str).next())
        .map(str::to_string);
    let from_map = || {
        parsed
            .get("errors")
            .and_then(Value::as_object)
            .and_then(|errors| errors.values().filter_map(Value::as_str).next())
            .map(str::to_string)
    };
    from_list
        .or_else(from_map)
        .map(|message| message.trim().to_string())
        .filter(|message| !message.is_empty())
}

fn parse_jira_projects(data: &Value) -> Result<Vec<JiraProject>, String> {
    let values = data
        .get("values")
        .and_then(Value::as_array)
        .ok_or_else(|| "Jira did not return projects".to_string())?;
    Ok(values
        .iter()
        .filter_map(|value| {
            let id = string_field(value, "id").filter(|id| !id.is_empty())?;
            let key = string_field(value, "key").unwrap_or_default();
            let name = string_field(value, "name")
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| key.clone());
            Some(JiraProject { id, key, name })
        })
        .collect())
}

// Follow every page: a partial project list would silently exclude visible
// projects when converting hidden projects into the JQL allowlist.
fn fetch_jira_projects(
    mut fetch: impl FnMut(u64) -> Result<Value, String>,
) -> Result<Vec<JiraProject>, String> {
    let mut start = 0;
    let mut projects = Vec::new();
    loop {
        let data = fetch(start)?;
        let page = parse_jira_projects(&data)?;
        let count = data["values"].as_array().map_or(0, Vec::len) as u64;
        projects.extend(page);
        if data["isLast"].as_bool() == Some(true) {
            break;
        }
        let next = data["startAt"].as_u64().unwrap_or(start) + count;
        if data["total"].as_u64().is_some_and(|total| next >= total) {
            break;
        }
        if count == 0 || next <= start {
            if data["isLast"].as_bool() == Some(false) {
                return Err("Jira project pagination did not advance".into());
            }
            break;
        }
        start = next;
    }
    let mut seen = std::collections::HashSet::new();
    projects.retain(|project| seen.insert(project.id.clone()));
    Ok(projects)
}

fn parse_jira_issues(data: &Value, site: &str) -> Result<Vec<JiraIssue>, String> {
    let issues = data
        .get("issues")
        .and_then(Value::as_array)
        .ok_or_else(|| "Jira did not return issues".to_string())?;
    Ok(issues
        .iter()
        .filter_map(|issue| parse_jira_issue(issue, site))
        .collect())
}

fn parse_jira_issue(node: &Value, site: &str) -> Option<JiraIssue> {
    let id = string_field(node, "id").filter(|id| !id.is_empty())?;
    let key = string_field(node, "key").filter(|key| !key.is_empty())?;
    let fields = node.get("fields").cloned().unwrap_or(Value::Null);
    let project = fields.get("project");
    let status = fields.get("status");
    let labels = fields
        .get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(Value::as_str)
                .map(|name| JiraLabel {
                    name: name.to_string(),
                    color: String::new(),
                })
                .collect()
        })
        .unwrap_or_default();
    let (assignee, avatar_url) = person_fields(fields.get("assignee"));
    Some(JiraIssue {
        provider: "jira".into(),
        kind: "jira".into(),
        number: issue_number(&key),
        url: format!("{site}/browse/{key}"),
        title: string_field(&fields, "summary").unwrap_or_default(),
        state: status
            .and_then(|value| string_field(value, "name"))
            .unwrap_or_else(|| "Open".into()),
        state_type: status
            .and_then(|value| value.pointer("/statusCategory/key"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        updated_at: normalize_jira_time(&string_field(&fields, "updated").unwrap_or_default()),
        labels,
        assignees: assignee
            .map(|login| vec![JiraAssignee { login, avatar_url }])
            .unwrap_or_default(),
        draft: false,
        repo: project
            .and_then(|value| string_field(value, "key"))
            .unwrap_or_default(),
        team_id: project
            .and_then(|value| string_field(value, "id"))
            .unwrap_or_default(),
        team_name: project
            .and_then(|value| string_field(value, "name"))
            .unwrap_or_default(),
        project_path: String::new(),
        id,
        identifier: key,
    })
}

fn parse_jira_issue_details(data: &Value) -> Result<JiraIssueDetails, String> {
    let fields = data
        .get("fields")
        .ok_or_else(|| "Jira did not return that issue".to_string())?;
    let (author, author_avatar_url) = [fields.get("reporter"), fields.get("creator")]
        .into_iter()
        .map(person_fields)
        .find(|(name, _)| name.is_some())
        .unwrap_or_else(|| person_fields(fields.get("assignee")));
    Ok(JiraIssueDetails {
        body: rich_text(fields.get("description")),
        author: author.unwrap_or_default(),
        author_avatar_url,
    })
}

fn parse_jira_issue_thread(data: &Value, site: &str, key: &str) -> Result<JiraIssueThread, String> {
    let comments = data
        .get("comments")
        .and_then(Value::as_array)
        .ok_or_else(|| "Jira did not return comments".to_string())?;
    let total = data
        .get("total")
        .and_then(Value::as_u64)
        .unwrap_or(comments.len() as u64);
    let mut parsed: Vec<JiraIssueComment> = comments
        .iter()
        .filter_map(|comment| parse_jira_comment(comment, site, key))
        .collect();
    parsed.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    Ok(JiraIssueThread {
        truncated: total > comments.len() as u64,
        comments: parsed,
        review_decision: String::new(),
        base_ref_name: String::new(),
        head_ref_name: String::new(),
    })
}

fn parse_jira_comment(node: &Value, site: &str, key: &str) -> Option<JiraIssueComment> {
    let id = string_field(node, "id").filter(|id| !id.is_empty())?;
    let (author, author_avatar_url) = person_fields(node.get("author"));
    Some(JiraIssueComment {
        url: comment_url(site, key, &id),
        kind: "comment".into(),
        author: author.unwrap_or_default(),
        author_avatar_url,
        body: rich_text(node.get("body")),
        created_at: normalize_jira_time(&string_field(node, "created").unwrap_or_default()),
        state: String::new(),
        path: String::new(),
        line: None,
        resolved: false,
        thread_id: String::new(),
        replies: Vec::new(),
        id,
    })
}

fn comment_url(site: &str, key: &str, id: &str) -> String {
    format!("{site}/browse/{key}?focusedCommentId={id}")
}

fn issue_number(key: &str) -> i64 {
    key.rsplit('-')
        .next()
        .and_then(|number| number.parse().ok())
        .unwrap_or(0)
}

/// Jira sends `+0000` offsets, which WebKit's `Date.parse` rejects.
fn normalize_jira_time(value: &str) -> String {
    let bytes = value.as_bytes();
    let len = bytes.len();
    if len > 5
        && matches!(bytes[len - 5], b'+' | b'-')
        && bytes[len - 4..].iter().all(u8::is_ascii_digit)
    {
        return format!("{}:{}", &value[..len - 2], &value[len - 2..]);
    }
    value.to_string()
}

fn require_issue_key(key: &str) -> Result<&str, String> {
    let key = key.trim();
    let valid = !key.is_empty()
        && key.len() < 64
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'));
    if valid {
        Ok(key)
    } else {
        Err("Missing Jira issue".into())
    }
}

fn person_fields(node: Option<&Value>) -> (Option<String>, String) {
    let Some(node) = node.filter(|node| node.is_object()) else {
        return (None, String::new());
    };
    (
        string_field(node, "displayName").filter(|name| !name.is_empty()),
        node.pointer("/avatarUrls/48x48")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    )
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|text| text.trim().to_string())
}

// ---- Atlassian Document Format ----

/// Descriptions and comments arrive as ADF (v3) or plain text; the Inbox renders markdown.
fn rich_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(doc @ Value::Object(_)) => adf_to_markdown(doc),
        _ => String::new(),
    }
}

fn adf_to_markdown(doc: &Value) -> String {
    let mut blocks = Vec::new();
    for node in children(doc) {
        let block = adf_block(node, "");
        if !block.trim().is_empty() {
            blocks.push(block);
        }
    }
    blocks.join("\n\n").trim().to_string()
}

fn children(node: &Value) -> &[Value] {
    node.get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn node_type(node: &Value) -> &str {
    node.get("type").and_then(Value::as_str).unwrap_or_default()
}

fn attr<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    node.pointer(&format!("/attrs/{name}"))
}

fn attr_str<'a>(node: &'a Value, name: &str) -> &'a str {
    attr(node, name).and_then(Value::as_str).unwrap_or_default()
}

fn adf_block(node: &Value, indent: &str) -> String {
    match node_type(node) {
        "paragraph" => adf_inline(children(node)),
        "heading" => {
            let level = attr(node, "level")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 6) as usize;
            format!("{} {}", "#".repeat(level), adf_inline(children(node)))
        }
        "bulletList" => adf_list(node, indent, None),
        "orderedList" => {
            let start = attr(node, "order").and_then(Value::as_u64).unwrap_or(1);
            adf_list(node, indent, Some(start))
        }
        "codeBlock" => {
            let code: String = children(node)
                .iter()
                .filter_map(|child| child.get("text").and_then(Value::as_str))
                .collect();
            format!("```{}\n{}\n```", attr_str(node, "language"), code)
        }
        "blockquote" | "panel" => adf_blocks(children(node), indent)
            .lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
        "rule" => "---".into(),
        "table" => adf_table(node),
        "mediaSingle" | "mediaGroup" | "media" => String::new(),
        _ => {
            if children(node).is_empty() {
                adf_inline(std::slice::from_ref(node))
            } else {
                adf_blocks(children(node), indent)
            }
        }
    }
}

fn adf_blocks(nodes: &[Value], indent: &str) -> String {
    nodes
        .iter()
        .map(|node| adf_block(node, indent))
        .filter(|block| !block.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn adf_list(node: &Value, indent: &str, start: Option<u64>) -> String {
    let mut lines = Vec::new();
    for (index, item) in children(node).iter().enumerate() {
        let marker = match start {
            Some(start) => format!("{}. ", start + index as u64),
            None => "- ".into(),
        };
        let nested = format!("{indent}{}", " ".repeat(marker.len()));
        let mut first = true;
        for child in children(item) {
            let text = adf_block(child, &nested);
            if text.trim().is_empty() {
                continue;
            }
            if matches!(node_type(child), "bulletList" | "orderedList") {
                lines.push(text);
            } else if first {
                lines.push(format!(
                    "{indent}{marker}{}",
                    text.replace('\n', &format!("\n{nested}"))
                ));
                first = false;
            } else {
                lines.push(format!(
                    "{nested}{}",
                    text.replace('\n', &format!("\n{nested}"))
                ));
            }
        }
    }
    lines.join("\n")
}

fn adf_table(node: &Value) -> String {
    let rows: Vec<Vec<String>> = children(node)
        .iter()
        .map(|row| {
            children(row)
                .iter()
                .map(|cell| {
                    adf_blocks(children(cell), "")
                        .replace('\n', " ")
                        .replace('|', "\\|")
                })
                .collect()
        })
        .filter(|row: &Vec<String>| !row.is_empty())
        .collect();
    let Some(width) = rows.iter().map(Vec::len).max() else {
        return String::new();
    };
    let line = |cells: &[String]| {
        let mut padded = cells.to_vec();
        padded.resize(width, String::new());
        format!("| {} |", padded.join(" | "))
    };
    let mut lines = vec![line(&rows[0]), format!("|{}", " --- |".repeat(width))];
    lines.extend(rows[1..].iter().map(|row| line(row)));
    lines.join("\n")
}

fn adf_inline(nodes: &[Value]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node_type(node) {
            "text" => out.push_str(&adf_marked_text(node)),
            "hardBreak" => out.push('\n'),
            "mention" => {
                let text = attr_str(node, "text");
                if text.starts_with('@') {
                    out.push_str(text);
                } else if !text.is_empty() {
                    out.push('@');
                    out.push_str(text);
                }
            }
            "emoji" => {
                let text = attr_str(node, "text");
                out.push_str(if text.is_empty() {
                    attr_str(node, "shortName")
                } else {
                    text
                });
            }
            "inlineCard" | "blockCard" | "embedCard" => {
                let url = attr_str(node, "url");
                if !url.is_empty() {
                    out.push_str(&format!("<{url}>"));
                }
            }
            "status" => out.push_str(&format!("`{}`", attr_str(node, "text"))),
            "date" => {}
            _ => out.push_str(&adf_inline(children(node))),
        }
    }
    out
}

fn adf_marked_text(node: &Value) -> String {
    let mut text = node
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if text.is_empty() {
        return text;
    }
    let marks = node
        .get("marks")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if marks.iter().any(|mark| node_type(mark) == "code") {
        return format!("`{text}`");
    }
    for mark in marks {
        text = match node_type(mark) {
            "strong" => format!("**{text}**"),
            "em" => format!("_{text}_"),
            "strike" => format!("~~{text}~~"),
            "link" => {
                let href = attr_str(mark, "href");
                if href.is_empty() {
                    text
                } else {
                    format!("[{text}]({href})")
                }
            }
            _ => text,
        };
    }
    text
}

/// Posts plain text as ADF: blank lines split paragraphs, single newlines become hard breaks.
fn text_to_adf(text: &str) -> Value {
    let paragraphs: Vec<Value> = text
        .replace("\r\n", "\n")
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .map(|paragraph| {
            let mut content = Vec::new();
            for (index, line) in paragraph.lines().enumerate() {
                if index > 0 {
                    content.push(json!({ "type": "hardBreak" }));
                }
                if !line.is_empty() {
                    content.push(json!({ "type": "text", "text": line }));
                }
            }
            json!({ "type": "paragraph", "content": content })
        })
        .collect();
    json!({ "type": "doc", "version": 1, "content": paragraphs })
}

// ---- Config storage ----

pub(crate) fn normalize_jira_site(raw: &str) -> Result<String, String> {
    let raw = raw.trim().trim_end_matches('/');
    if raw.is_empty() {
        return Err("Enter your Jira site (e.g. yourteam.atlassian.net)".into());
    }
    // Bare `yourteam` becomes the Atlassian Cloud site.
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else if raw.contains('.') {
        format!("https://{raw}")
    } else {
        format!("https://{raw}.atlassian.net")
    };
    let url = url::Url::parse(&with_scheme).map_err(|_| "Jira site is invalid".to_string())?;
    // The API token travels on every request as Basic auth.
    if url.scheme() != "https" {
        return Err("Jira site must use HTTPS".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Jira site is invalid".into());
    }
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(|| "Jira site is invalid".to_string())?;
    Ok(match url.port() {
        Some(port) => format!("https://{}:{port}", host.to_ascii_lowercase()),
        None => format!("https://{}", host.to_ascii_lowercase()),
    })
}

// ---- Board provider ----

const BOARD_ISSUE_FIELDS: &str = "summary,status,updated,labels,assignee,project,description";

fn not_supported<T>(operation: &'static str) -> ProviderResult<T> {
    Err(ProviderError::NotSupported {
        provider: ProviderKind::Jira,
        operation,
    })
}

/// Board containers are Jira project IDs, which are always numeric.
fn require_project_id(id: &str) -> ProviderResult<&str> {
    let id = id.trim();
    if !id.is_empty() && id.len() < 20 && id.chars().all(|ch| ch.is_ascii_digit()) {
        Ok(id)
    } else {
        Err(ProviderError::InvalidInput("Missing Jira project".into()))
    }
}

fn parse_myself(data: &Value) -> Result<Account, String> {
    let id = string_field(data, "accountId").unwrap_or_default();
    if id.is_empty() {
        return Err("Jira did not return the current user".into());
    }
    Ok(Account {
        id,
        name: string_field(data, "displayName").unwrap_or_default(),
        email: string_field(data, "emailAddress").unwrap_or_default(),
    })
}

fn parse_status(status: Option<&Value>) -> WorkflowState {
    let field = |key| {
        status
            .and_then(|value| string_field(value, key))
            .unwrap_or_default()
    };
    WorkflowState {
        id: field("id"),
        name: field("name"),
        group: StateGroup::from_jira_category(
            status
                .and_then(|value| value.pointer("/statusCategory/key"))
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
    }
}

/// `/project/{id}/statuses` lists statuses per issue type; a board wants each once.
fn parse_project_statuses(data: &Value) -> Result<Vec<WorkflowState>, String> {
    let issue_types = data
        .as_array()
        .ok_or_else(|| "Jira did not return statuses".to_string())?;
    let mut seen = std::collections::HashSet::new();
    Ok(issue_types
        .iter()
        .filter_map(|issue_type| issue_type.get("statuses").and_then(Value::as_array))
        .flatten()
        .map(|status| parse_status(Some(status)))
        .filter(|state| !state.id.is_empty() && seen.insert(state.id.clone()))
        .collect())
}

/// Labels are site-wide in Jira and have no IDs, so the name is the ID.
fn fetch_jira_labels(
    mut fetch: impl FnMut(u64) -> Result<Value, String>,
) -> Result<Vec<BoardLabel>, String> {
    let mut start = 0;
    let mut labels = Vec::new();
    loop {
        let data = fetch(start)?;
        let values = data
            .get("values")
            .and_then(Value::as_array)
            .ok_or_else(|| "Jira did not return labels".to_string())?;
        labels.extend(
            values
                .iter()
                .filter_map(Value::as_str)
                .map(|name| BoardLabel {
                    id: name.to_string(),
                    name: name.to_string(),
                    color: String::new(),
                }),
        );
        if data["isLast"].as_bool() != Some(false) {
            return Ok(labels);
        }
        if values.is_empty() {
            return Err("Jira label pagination did not advance".into());
        }
        start = data["startAt"].as_u64().unwrap_or(start) + values.len() as u64;
    }
}

fn parse_jira_member(node: &Value) -> Option<Member> {
    if node.get("active").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let id = string_field(node, "accountId").filter(|id| !id.is_empty())?;
    let (name, avatar_url) = person_fields(Some(node));
    Some(Member {
        id,
        name: name.unwrap_or_default(),
        avatar_url,
    })
}

/// The assignable-users endpoint returns a bare array; a short page is the last.
fn fetch_assignable_users(
    mut fetch: impl FnMut(u64) -> Result<Value, String>,
) -> Result<Vec<Member>, String> {
    let mut start = 0;
    let mut members = Vec::new();
    loop {
        let data = fetch(start)?;
        let users = data
            .as_array()
            .ok_or_else(|| "Jira did not return users".to_string())?;
        members.extend(users.iter().filter_map(parse_jira_member));
        if users.len() < USER_PAGE {
            return Ok(members);
        }
        start += users.len() as u64;
    }
}

fn parse_remote_jira_issue(node: &Value, site: &str) -> Option<RemoteIssue> {
    let id = string_field(node, "id").filter(|id| !id.is_empty())?;
    let key = string_field(node, "key").filter(|key| !key.is_empty())?;
    let fields = node.get("fields").cloned().unwrap_or(Value::Null);
    Some(RemoteIssue {
        url: format!("{site}/browse/{key}"),
        title: string_field(&fields, "summary").unwrap_or_default(),
        container_id: fields
            .get("project")
            .and_then(|project| string_field(project, "id"))
            .unwrap_or_default(),
        updated_at: normalize_jira_time(&string_field(&fields, "updated").unwrap_or_default()),
        state: parse_status(fields.get("status")),
        labels: fields
            .get("labels")
            .and_then(Value::as_array)
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|name| BoardLabel {
                        id: name.to_string(),
                        name: name.to_string(),
                        color: String::new(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        assignees: fields
            .get("assignee")
            .and_then(parse_jira_member)
            .into_iter()
            .collect(),
        description: fields
            .get("description")
            .map(|description| rich_text(Some(description))),
        id,
        key,
    })
}

fn parse_search_page(data: &Value, site: &str) -> Result<Page<RemoteIssue>, String> {
    let items = data
        .get("issues")
        .and_then(Value::as_array)
        .ok_or_else(|| "Jira did not return issues".to_string())?
        .iter()
        .filter_map(|issue| parse_remote_jira_issue(issue, site))
        .collect();
    let next_cursor = data
        .get("nextPageToken")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string);
    Ok(Page { items, next_cursor })
}

fn board_search_body(filter: &IssueFilter, cursor: Option<&str>) -> ProviderResult<Value> {
    let project_ids = filter
        .container_ids
        .iter()
        .map(|id| require_project_id(id).map(str::to_string))
        .collect::<ProviderResult<Vec<_>>>()?;
    let state = if filter.include_closed { "all" } else { "open" };
    let mut body = json!({
        "jql": issue_jql(filter.assigned_to_me, state, &project_ids),
        "maxResults": filter.limit.clamp(1, MAX_PAGE),
        "fields": ISSUE_FIELDS,
    });
    if let Some(token) = cursor {
        body["nextPageToken"] = json!(token);
    }
    Ok(body)
}

impl BoardProvider for JiraProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Jira
    }

    fn whoami(&self) -> ProviderResult<Account> {
        Ok(parse_myself(&self.get("/rest/api/3/myself")?)?)
    }

    fn list_containers(&self) -> ProviderResult<Vec<Container>> {
        Ok(self
            .projects()?
            .into_iter()
            .map(|project| Container {
                id: project.id,
                key: project.key,
                name: project.name,
            })
            .collect())
    }

    fn list_states(&self, container_id: &str) -> ProviderResult<Vec<WorkflowState>> {
        let id = require_project_id(container_id)?;
        let data = self.get(&format!("/rest/api/3/project/{id}/statuses"))?;
        Ok(parse_project_statuses(&data)?)
    }

    fn list_labels(&self, container_id: &str) -> ProviderResult<Vec<BoardLabel>> {
        require_project_id(container_id)?;
        Ok(fetch_jira_labels(|start| {
            self.get(&format!(
                "/rest/api/3/label?startAt={start}&maxResults=1000"
            ))
        })?)
    }

    fn list_members(&self, container_id: &str) -> ProviderResult<Vec<Member>> {
        let id = require_project_id(container_id)?;
        Ok(fetch_assignable_users(|start| {
            self.get(&format!(
                "/rest/api/3/user/assignable/search?project={id}&startAt={start}&maxResults={USER_PAGE}"
            ))
        })?)
    }

    fn list_issues(
        &self,
        filter: &IssueFilter,
        cursor: Option<&str>,
    ) -> ProviderResult<Page<RemoteIssue>> {
        let body = board_search_body(filter, cursor)?;
        let data = self.post("/rest/api/3/search/jql", &body)?;
        Ok(parse_search_page(&data, self.site())?)
    }

    fn get_issue(&self, id: &str) -> ProviderResult<RemoteIssue> {
        let key = require_issue_key(id).map_err(ProviderError::InvalidInput)?;
        let data = self.get(&format!(
            "/rest/api/3/issue/{key}?fields={BOARD_ISSUE_FIELDS}"
        ))?;
        parse_remote_jira_issue(&data, self.site())
            .ok_or_else(|| ProviderError::Remote("Jira did not return that issue".into()))
    }

    // Jira writes and JQL delta sync land in #99; webhook parsing waits on #109's fixtures.
    fn create_issue(&self, _: &str, _: &IssueDraft) -> ProviderResult<RemoteIssue> {
        not_supported("create_issue")
    }

    fn update_issue(&self, _: &str, _: &IssuePatch) -> ProviderResult<RemoteIssue> {
        not_supported("update_issue")
    }

    fn move_issue(&self, _: &str, _: &MoveTarget) -> ProviderResult<RemoteIssue> {
        not_supported("move_issue")
    }

    fn comment(
        &self,
        issue_id: &str,
        body: &str,
        reply_to: Option<&str>,
    ) -> ProviderResult<CommentRef> {
        if reply_to.is_some() {
            return Err(ProviderError::InvalidInput(
                "Jira comments cannot be replies".into(),
            ));
        }
        validate_comment(issue_id, body).map_err(ProviderError::InvalidInput)?;
        Ok(self.post_comment(issue_id, body)?)
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
    use super::*;

    const SITE: &str = "https://acme.atlassian.net";

    fn issue_node() -> Value {
        json!({
            "id": "10042",
            "key": "ENG-42",
            "fields": {
                "summary": "Fix login",
                "updated": "2026-03-01T10:00:00.000+0000",
                "status": { "id": "3", "name": "In Progress", "statusCategory": { "key": "indeterminate" } },
                "labels": ["bug", "auth"],
                "assignee": { "accountId": "acc-1", "displayName": "Ada", "avatarUrls": { "48x48": "https://a/ada.png" } },
                "project": { "id": "100", "key": "ENG", "name": "Engineering" },
                "description": { "type": "doc", "version": 1, "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "Steps", "marks": [{ "type": "strong" }] }] }
                ]}
            }
        })
    }

    #[test]
    fn parses_issue_number_url_and_time() {
        let issues =
            parse_jira_issues(&json!({ "issues": [issue_node(), { "id": "1" }] }), SITE).unwrap();
        assert_eq!(issues.len(), 1);
        let issue = &issues[0];
        assert_eq!(issue.number, 42);
        assert_eq!(issue.url, "https://acme.atlassian.net/browse/ENG-42");
        assert_eq!(issue.updated_at, "2026-03-01T10:00:00.000+00:00");
        assert_eq!(issue.state_type, "indeterminate");
        assert_eq!(issue.repo, "ENG");
        assert_eq!(issue.labels[1].name, "auth");
        assert_eq!(issue.assignees[0].login, "Ada");
        assert_eq!(
            normalize_jira_time("2026-03-01T10:00:00Z"),
            "2026-03-01T10:00:00Z"
        );
    }

    #[test]
    fn jql_drops_non_numeric_projects_and_bounds_empty_queries() {
        assert_eq!(
            issue_jql(true, "open", &["100".into(), "ENG) OR (1=1".into()]),
            "assignee = currentUser() AND project in (100) AND statusCategory != Done ORDER BY updated DESC"
        );
        assert_eq!(
            issue_jql(false, "all", &[]),
            "updated >= -365d ORDER BY updated DESC"
        );
    }

    #[test]
    fn site_normalization() {
        assert_eq!(normalize_jira_site("acme").unwrap(), SITE);
        assert_eq!(normalize_jira_site(" Acme.Atlassian.net/ ").unwrap(), SITE);
        assert_eq!(
            normalize_jira_site("https://jira.example.com:8443").unwrap(),
            "https://jira.example.com:8443"
        );
        assert_eq!(
            normalize_jira_site("http://acme.atlassian.net").unwrap_err(),
            "Jira site must use HTTPS"
        );
        assert_eq!(
            normalize_jira_site("https://user:pw@acme.atlassian.net").unwrap_err(),
            "Jira site is invalid"
        );
        assert!(normalize_jira_site("  ").is_err());
    }

    #[test]
    fn project_pages_are_followed_and_deduplicated() {
        let pages = [
            json!({ "startAt": 0, "isLast": false, "values": [
                { "id": "1", "key": "A", "name": "Alpha" },
                { "id": "2", "key": "B", "name": "" }
            ]}),
            json!({ "startAt": 2, "isLast": true, "values": [
                { "id": "2", "key": "B", "name": "Beta" },
                { "id": "3", "key": "C", "name": "Gamma" }
            ]}),
        ];
        let mut starts = Vec::new();
        let projects = fetch_jira_projects(|start| {
            starts.push(start);
            Ok(pages[starts.len() - 1].clone())
        })
        .unwrap();
        assert_eq!(starts, [0, 2]);
        let keys: Vec<(&str, &str)> = projects
            .iter()
            .map(|p| (p.key.as_str(), p.name.as_str()))
            .collect();
        assert_eq!(keys, [("A", "Alpha"), ("B", "B"), ("C", "Gamma")]);

        let stuck = json!({ "startAt": 0, "isLast": false, "values": [] });
        assert_eq!(
            fetch_jira_projects(|_| Ok(stuck.clone())).unwrap_err(),
            "Jira project pagination did not advance"
        );
    }

    #[test]
    fn adf_renders_lists_code_and_links() {
        let doc = json!({ "type": "doc", "content": [
            { "type": "heading", "attrs": { "level": 2 }, "content": [{ "type": "text", "text": "Plan" }] },
            { "type": "bulletList", "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [
                    { "type": "text", "text": "docs", "marks": [{ "type": "link", "attrs": { "href": "https://d" } }] }
                ]}]}
            ]},
            { "type": "codeBlock", "attrs": { "language": "rust" }, "content": [{ "type": "text", "text": "fn main() {}" }] }
        ]});
        assert_eq!(
            rich_text(Some(&doc)),
            "## Plan\n\n- [docs](https://d)\n\n```rust\nfn main() {}\n```"
        );
        assert_eq!(rich_text(Some(&json!("  plain  "))), "plain");
        assert_eq!(rich_text(None), "");
    }

    #[test]
    fn text_to_adf_splits_paragraphs_and_breaks() {
        assert_eq!(
            text_to_adf("one\r\ntwo\n\nthree"),
            json!({ "type": "doc", "version": 1, "content": [
                { "type": "paragraph", "content": [
                    { "type": "text", "text": "one" },
                    { "type": "hardBreak" },
                    { "type": "text", "text": "two" }
                ]},
                { "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }
            ]})
        );
    }

    #[test]
    fn myself_becomes_account() {
        let data = json!({ "accountId": "acc-1", "displayName": "Ada", "emailAddress": "ada@example.com" });
        assert_eq!(
            parse_myself(&data).unwrap(),
            Account {
                id: "acc-1".into(),
                name: "Ada".into(),
                email: "ada@example.com".into(),
            }
        );
        assert_eq!(
            parse_myself(&json!({ "displayName": "No id" })).unwrap_err(),
            "Jira did not return the current user"
        );
    }

    #[test]
    fn jira_states_are_deduplicated_across_issue_types() {
        let status = |id: &str, name: &str, category: &str| json!({ "id": id, "name": name, "statusCategory": { "key": category } });
        let data = json!([
            { "id": "t1", "name": "Task", "statuses": [status("1", "To Do", "new"), status("3", "In Progress", "indeterminate")] },
            { "id": "t2", "name": "Bug", "statuses": [status("3", "In Progress", "indeterminate"), status("5", "Done", "done")] }
        ]);
        let states: Vec<(String, StateGroup)> = parse_project_statuses(&data)
            .unwrap()
            .into_iter()
            .map(|state| (state.id, state.group))
            .collect();
        assert_eq!(
            states,
            [
                ("1".to_string(), StateGroup::Unstarted),
                ("3".to_string(), StateGroup::Started),
                ("5".to_string(), StateGroup::Completed),
            ]
        );
        assert!(parse_project_statuses(&json!({})).is_err());
    }

    #[test]
    fn label_pages_are_followed() {
        let pages = [
            json!({ "startAt": 0, "maxResults": 2, "isLast": false, "values": ["auth", "bug"] }),
            json!({ "startAt": 2, "maxResults": 2, "isLast": true, "values": ["ux"] }),
        ];
        let mut starts = Vec::new();
        let labels = fetch_jira_labels(|start| {
            starts.push(start);
            Ok(pages[starts.len() - 1].clone())
        })
        .unwrap();
        assert_eq!(starts, [0, 2]);
        let names: Vec<&str> = labels.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["auth", "bug", "ux"]);
        assert_eq!(labels[0].id, "auth");

        let stuck = json!({ "startAt": 0, "isLast": false, "values": [] });
        assert!(fetch_jira_labels(|_| Ok(stuck.clone())).is_err());
    }

    #[test]
    fn inactive_members_are_dropped_and_pages_followed() {
        let user = |id: &str, active: bool| json!({ "accountId": id, "displayName": id.to_uppercase(), "active": active, "avatarUrls": { "48x48": "https://a/x.png" } });
        let full: Vec<Value> = (0..USER_PAGE)
            .map(|i| user(&format!("u{i}"), true))
            .collect();
        let pages = [
            Value::Array(full),
            json!([user("last", true), user("gone", false)]),
        ];
        let mut starts = Vec::new();
        let members = fetch_assignable_users(|start| {
            starts.push(start);
            Ok(pages[starts.len() - 1].clone())
        })
        .unwrap();
        assert_eq!(starts, [0, USER_PAGE as u64]);
        assert_eq!(members.len(), USER_PAGE + 1);
        assert_eq!(members.last().unwrap().id, "last");
        assert_eq!(members[0].name, "U0");
        assert_eq!(members[0].avatar_url, "https://a/x.png");
    }

    #[test]
    fn issue_maps_to_remote_issue_with_markdown_description() {
        let issue = parse_remote_jira_issue(&issue_node(), SITE).unwrap();
        assert_eq!(issue.id, "10042");
        assert_eq!(issue.key, "ENG-42");
        assert_eq!(issue.container_id, "100");
        assert_eq!(issue.url, "https://acme.atlassian.net/browse/ENG-42");
        assert_eq!(issue.updated_at, "2026-03-01T10:00:00.000+00:00");
        assert_eq!(
            issue.state,
            WorkflowState {
                id: "3".into(),
                name: "In Progress".into(),
                group: StateGroup::Started,
            }
        );
        assert_eq!(issue.labels[0].id, "bug");
        assert_eq!(issue.assignees[0].id, "acc-1");
        assert_eq!(issue.description.as_deref(), Some("**Steps**"));

        let mut listed = issue_node();
        listed["fields"]
            .as_object_mut()
            .unwrap()
            .remove("description");
        listed["fields"]["assignee"] = Value::Null;
        let issue = parse_remote_jira_issue(&listed, SITE).unwrap();
        assert_eq!(issue.description, None);
        assert!(issue.assignees.is_empty());
    }

    #[test]
    fn search_page_exposes_next_page_token() {
        let more = json!({ "issues": [issue_node()], "nextPageToken": "tok" });
        let page = parse_search_page(&more, SITE).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.next_cursor.as_deref(), Some("tok"));
        let last = json!({ "issues": [], "isLast": true });
        assert_eq!(parse_search_page(&last, SITE).unwrap().next_cursor, None);
    }

    #[test]
    fn board_search_body_validates_projects_and_carries_the_token() {
        let filter = IssueFilter {
            container_ids: vec!["100".into()],
            include_closed: true,
            limit: 500,
            ..Default::default()
        };
        let body = board_search_body(&filter, Some("tok")).unwrap();
        assert_eq!(body["jql"], "project in (100) ORDER BY updated DESC");
        assert_eq!(body["maxResults"], 100);
        assert_eq!(body["nextPageToken"], "tok");
        assert_eq!(body["fields"][1], "status");
        assert!(board_search_body(&IssueFilter::default(), None)
            .unwrap()
            .get("nextPageToken")
            .is_none());
        let bad = IssueFilter {
            container_ids: vec!["ENG".into()],
            ..Default::default()
        };
        assert!(matches!(
            board_search_body(&bad, None),
            Err(ProviderError::InvalidInput(_))
        ));
    }

    fn provider() -> JiraProvider {
        JiraProvider::new("acme", "ada@example.com", "token").unwrap()
    }

    #[test]
    fn new_validates_email_and_site() {
        assert_eq!(
            JiraProvider::new("acme", "ada", "t").err().unwrap(),
            "Enter the email address of your Atlassian account"
        );
        assert_eq!(
            JiraProvider::new("http://acme.atlassian.net", "a@b.c", "t")
                .err()
                .unwrap(),
            "Jira site must use HTTPS"
        );
        assert_eq!(provider().site(), SITE);
    }

    #[test]
    fn write_operations_are_not_supported_yet() {
        let jira = provider();
        let unsupported = |result: ProviderResult<()>, operation: &str| match result {
            Err(ProviderError::NotSupported {
                provider: ProviderKind::Jira,
                operation: actual,
            }) => assert_eq!(actual, operation),
            other => panic!("{operation}: {other:?}"),
        };
        unsupported(
            jira.create_issue("100", &IssueDraft::default()).map(drop),
            "create_issue",
        );
        unsupported(
            jira.update_issue("ENG-1", &IssuePatch::default()).map(drop),
            "update_issue",
        );
        unsupported(
            jira.move_issue("ENG-1", &MoveTarget::default()).map(drop),
            "move_issue",
        );
        unsupported(jira.sync_since("100", None).map(drop), "sync_since");
        unsupported(jira.parse_webhook(&[], b"{}").map(drop), "parse_webhook");
    }

    #[test]
    fn replies_and_malformed_input_are_rejected_before_any_request() {
        let jira = provider();
        let invalid = |result: ProviderResult<()>| {
            assert!(
                matches!(result, Err(ProviderError::InvalidInput(_))),
                "{result:?}"
            )
        };
        invalid(jira.comment("ENG-1", "Hi", Some("10")).map(drop));
        invalid(jira.comment("ENG-1", "   ", None).map(drop));
        invalid(jira.comment("ENG 1", "Hi", None).map(drop));
        invalid(jira.get_issue("../ENG-1").map(drop));
        invalid(jira.list_states("ENG/../x").map(drop));
        invalid(jira.list_members("").map(drop));
        assert_eq!(jira.kind(), ProviderKind::Jira);
    }
}
