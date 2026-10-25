//! Linear over its GraphQL API. Shared by the Inbox and the board sync.
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use super::{
    Account, BoardLabel, BoardProvider, ChangeHint, CommentRef, Container, IssueDraft, IssueFilter,
    IssuePatch, Member, MoveTarget, Page, ProviderError, ProviderKind, ProviderResult, RemoteIssue,
    StateGroup, SyncBatch, SyncCursor, WorkflowState,
};

const LINEAR_API: &str = "https://api.linear.app/graphql";
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_PAGE: u32 = 100;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinearTeam {
    pub id: String,
    pub key: String,
    pub name: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinearLabel {
    pub name: String,
    pub color: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinearAssignee {
    pub login: String,
    pub avatar_url: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssue {
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
    pub labels: Vec<LinearLabel>,
    pub assignees: Vec<LinearAssignee>,
    pub draft: bool,
    pub repo: String,
    pub team_id: String,
    pub team_name: String,
    pub project_id: String,
    pub project_name: String,
    pub project_path: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueDetails {
    pub body: String,
    pub author: String,
    pub author_avatar_url: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueComment {
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
    pub replies: Vec<LinearIssueComment>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueThread {
    pub comments: Vec<LinearIssueComment>,
    pub truncated: bool,
    pub review_decision: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
}

/// A Linear connection authenticated with a personal API key.
#[derive(Clone)]
pub struct LinearProvider {
    token: String,
}

impl LinearProvider {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.trim().to_string(),
        }
    }

    fn graphql(&self, query: &str, variables: Value) -> Result<Value, String> {
        graphql_with_token(&self.token, query, variables)
    }

    /// The cheapest call that proves the key works.
    pub fn verify(&self) -> Result<(), String> {
        self.graphql(VIEWER_QUERY, json!({})).map(|_| ())
    }

    pub fn teams(&self) -> Result<Vec<LinearTeam>, String> {
        let data = self.graphql(TEAMS_QUERY, json!({}))?;
        parse_linear_teams(&data)
    }

    /// The Inbox list: `state` is `"all"` or anything else for open issues only.
    pub fn inbox_issues(
        &self,
        assigned_to_me: bool,
        state: &str,
        team_ids: &[String],
        limit: u32,
    ) -> Result<Vec<LinearIssue>, String> {
        let filter = issue_filter(assigned_to_me, state, team_ids);
        let data = self.graphql(ISSUES_QUERY, json!({ "first": limit, "filter": filter }))?;
        parse_linear_issues(&data)
    }

    pub fn issue_details(&self, id: &str) -> Result<LinearIssueDetails, String> {
        let id = id.trim();
        if id.is_empty() {
            return Err("Missing Linear issue".into());
        }
        let data = self.graphql(ISSUE_QUERY, json!({ "id": id }))?;
        parse_linear_issue_details(&data)
    }

    pub fn issue_thread(&self, id: &str) -> Result<LinearIssueThread, String> {
        let id = id.trim();
        if !valid_linear_id(id) {
            return Err("Missing Linear issue".into());
        }
        let data = self.graphql(ISSUE_COMMENTS_QUERY, json!({ "id": id }))?;
        parse_linear_issue_thread(&data)
    }

    /// Posts a comment (a reply when `parent_id` is set) and returns its URL.
    pub fn post_comment(&self, id: &str, body: &str, parent_id: &str) -> Result<String, String> {
        let input = comment_input(id, body, parent_id)?;
        let data = self.graphql(COMMENT_CREATE_MUTATION, json!({ "input": input }))?;
        parse_linear_comment_create(&data)
    }
}

const VIEWER_QUERY: &str = "query { viewer { id } }";
const TEAMS_QUERY: &str = r#"
query {
  teams(first: 50) {
    nodes { id key name }
  }
}
"#;
const ISSUES_QUERY: &str = r#"
query InboxIssues($first: Int!, $filter: IssueFilter) {
  issues(first: $first, filter: $filter, orderBy: updatedAt) {
    nodes {
      id
      identifier
      number
      title
      url
      updatedAt
      state { name type }
      team { id key name }
      project { id name }
      labels { nodes { name color } }
      assignee { name displayName avatarUrl }
    }
  }
}
"#;
const ISSUE_QUERY: &str = r#"
query InboxIssue($id: String!) {
  issue(id: $id) {
    description
    creator { name displayName avatarUrl }
    assignee { name displayName avatarUrl }
  }
}
"#;
const ISSUE_COMMENTS_QUERY: &str = r#"
query InboxIssueComments($id: String!) {
  issue(id: $id) {
    comments(first: 50) {
      pageInfo { hasNextPage }
      nodes {
        id
        body
        createdAt
        url
        user { name displayName avatarUrl }
        parent { id }
      }
    }
  }
}
"#;
const COMMENT_CREATE_MUTATION: &str = r#"
mutation InboxCommentCreate($input: CommentCreateInput!) {
  commentCreate(input: $input) {
    success
    comment { id url }
  }
}
"#;

fn issue_filter(assigned_to_me: bool, state: &str, team_ids: &[String]) -> Value {
    let mut filter = serde_json::Map::new();
    if assigned_to_me {
        filter.insert("assignee".into(), json!({ "isMe": { "eq": true } }));
    }
    let ids: Vec<String> = team_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if !ids.is_empty() {
        filter.insert("team".into(), json!({ "id": { "in": ids } }));
    }
    if !state.trim().eq_ignore_ascii_case("all") {
        filter.insert(
            "state".into(),
            json!({ "type": { "nin": ["completed", "canceled"] } }),
        );
    }
    Value::Object(filter)
}

fn linear_authorization(token: &str) -> String {
    let trimmed = token.trim();
    trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

fn graphql_with_token(token: &str, query: &str, variables: Value) -> Result<Value, String> {
    let authorization = linear_authorization(token);
    let agent = ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).build();
    let payload = serde_json::to_string(&json!({ "query": query, "variables": variables }))
        .map_err(|error| error.to_string())?;
    let result = agent
        .post(LINEAR_API)
        .set("Authorization", &authorization)
        .set("Content-Type", "application/json")
        .send_string(&payload);
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => {
            return Err("Linear API key is invalid".into());
        }
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            return Err(linear_http_error(status, &body));
        }
        Err(_) => return Err("Could not reach Linear".into()),
    };
    let status = response.status();
    let body = response
        .into_string()
        .map_err(|_| "Linear returned an unreadable response".to_string())?;
    if status == 401 || status == 403 {
        return Err("Linear API key is invalid".into());
    }
    if !(200..300).contains(&status) {
        return Err(linear_http_error(status, &body));
    }
    parse_graphql_data(&body)
}

fn linear_http_error(status: u16, body: &str) -> String {
    graphql_error_message(body).unwrap_or_else(|| format!("Linear request failed ({status})"))
}

fn parse_graphql_data(body: &str) -> Result<Value, String> {
    let parsed: Value =
        serde_json::from_str(body).map_err(|_| "Linear returned invalid JSON".to_string())?;
    if let Some(message) = graphql_error_message_from_value(&parsed) {
        if message.to_ascii_lowercase().contains("auth")
            || message.to_ascii_lowercase().contains("unauthor")
        {
            return Err("Linear API key is invalid".into());
        }
        return Err(message);
    }
    parsed
        .get("data")
        .cloned()
        .ok_or_else(|| "Linear returned no data".to_string())
}

fn graphql_error_message(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    graphql_error_message_from_value(&parsed)
}

fn graphql_error_message_from_value(parsed: &Value) -> Option<String> {
    parsed
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(|message| message.trim().to_string())
        .filter(|message| !message.is_empty())
}

fn parse_linear_teams(data: &Value) -> Result<Vec<LinearTeam>, String> {
    let nodes = data
        .pointer("/teams/nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "Linear did not return teams".to_string())?;
    Ok(nodes
        .iter()
        .filter_map(|node| {
            let id = string_field(node, "id")?;
            let key = string_field(node, "key").unwrap_or_default();
            let name = string_field(node, "name").unwrap_or_else(|| key.clone());
            if id.is_empty() {
                return None;
            }
            Some(LinearTeam { id, key, name })
        })
        .collect())
}

fn parse_linear_issues(data: &Value) -> Result<Vec<LinearIssue>, String> {
    let nodes = data
        .pointer("/issues/nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "Linear did not return issues".to_string())?;
    Ok(nodes.iter().filter_map(parse_linear_issue).collect())
}

fn parse_linear_issue(node: &Value) -> Option<LinearIssue> {
    let id = string_field(node, "id")?;
    if id.is_empty() {
        return None;
    }
    let identifier = string_field(node, "identifier").unwrap_or_default();
    let number = node.get("number").and_then(Value::as_i64).unwrap_or(0);
    let team = node.get("team");
    let team_id = team
        .and_then(|value| string_field(value, "id"))
        .unwrap_or_default();
    let team_key = team
        .and_then(|value| string_field(value, "key"))
        .unwrap_or_default();
    let team_name = team
        .and_then(|value| string_field(value, "name"))
        .unwrap_or_else(|| team_key.clone());
    let project = node.get("project");
    let project_id = project
        .and_then(|value| string_field(value, "id"))
        .unwrap_or_default();
    let project_name = project
        .and_then(|value| string_field(value, "name"))
        .unwrap_or_default();
    let state = node.get("state");
    Some(LinearIssue {
        provider: "linear".into(),
        kind: "linear".into(),
        id,
        identifier,
        number,
        title: string_field(node, "title").unwrap_or_default(),
        url: string_field(node, "url").unwrap_or_default(),
        state: state
            .and_then(|value| string_field(value, "name"))
            .unwrap_or_else(|| "Open".into()),
        state_type: state
            .and_then(|value| string_field(value, "type"))
            .unwrap_or_default(),
        updated_at: string_field(node, "updatedAt").unwrap_or_default(),
        labels: parse_labels(node),
        assignees: parse_assignees(node),
        draft: false,
        repo: team_key,
        team_id,
        team_name,
        project_id,
        project_name,
        project_path: String::new(),
    })
}

fn parse_linear_issue_details(data: &Value) -> Result<LinearIssueDetails, String> {
    let issue = data
        .get("issue")
        .ok_or_else(|| "Linear did not return that issue".to_string())?;
    let creator = person_fields(issue.get("creator"));
    let assignee = person_fields(issue.get("assignee"));
    let (author, author_avatar_url) = if creator.0.is_some() {
        creator
    } else {
        assignee
    };
    Ok(LinearIssueDetails {
        body: string_field(issue, "description").unwrap_or_default(),
        author: author.unwrap_or_default(),
        author_avatar_url,
    })
}

fn parse_linear_issue_thread(data: &Value) -> Result<LinearIssueThread, String> {
    let issue = data
        .get("issue")
        .ok_or_else(|| "Linear did not return that issue".to_string())?;
    let comments_field = issue
        .get("comments")
        .ok_or_else(|| "Linear did not return comments".to_string())?;
    let truncated = comments_field
        .pointer("/pageInfo/hasNextPage")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let nodes = comments_field
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "Linear did not return comments".to_string())?;
    let rows: Vec<ParsedLinearComment> = nodes.iter().filter_map(parse_linear_comment).collect();
    Ok(LinearIssueThread {
        comments: nest_linear_comments(rows),
        truncated,
        review_decision: String::new(),
        base_ref_name: String::new(),
        head_ref_name: String::new(),
    })
}

struct ParsedLinearComment {
    id: String,
    parent_id: String,
    comment: LinearIssueComment,
}

fn parse_linear_comment(node: &Value) -> Option<ParsedLinearComment> {
    let id = string_field(node, "id").filter(|value| !value.is_empty())?;
    let (author, author_avatar_url) = person_fields(node.get("user"));
    let parent_id = node
        .get("parent")
        .and_then(|parent| string_field(parent, "id"))
        .unwrap_or_default();
    Some(ParsedLinearComment {
        id: id.clone(),
        parent_id,
        comment: LinearIssueComment {
            id,
            kind: "comment".into(),
            author: author.unwrap_or_default(),
            author_avatar_url,
            body: string_field(node, "body").unwrap_or_default(),
            created_at: string_field(node, "createdAt").unwrap_or_default(),
            url: string_field(node, "url").unwrap_or_default(),
            state: String::new(),
            path: String::new(),
            line: None,
            resolved: false,
            thread_id: String::new(),
            replies: Vec::new(),
        },
    })
}

fn nest_linear_comments(mut rows: Vec<ParsedLinearComment>) -> Vec<LinearIssueComment> {
    rows.sort_by(|left, right| left.comment.created_at.cmp(&right.comment.created_at));
    let ids: HashSet<String> = rows.iter().map(|row| row.id.clone()).collect();
    let mut parent_of = HashMap::new();
    for row in &rows {
        if !row.parent_id.is_empty() && ids.contains(&row.parent_id) {
            parent_of.insert(row.id.clone(), row.parent_id.clone());
        }
    }
    let mut top = Vec::new();
    let mut replies: HashMap<String, Vec<LinearIssueComment>> = HashMap::new();
    for row in rows {
        let root = linear_comment_root(&row.id, &parent_of);
        if root == row.id {
            top.push(row.comment);
        } else {
            replies.entry(root).or_default().push(row.comment);
        }
    }
    for comment in &mut top {
        if let Some(children) = replies.remove(&comment.id) {
            comment.replies = children;
        }
    }
    top
}

fn linear_comment_root(id: &str, parent_of: &HashMap<String, String>) -> String {
    let mut current = id.to_string();
    let mut seen = HashSet::new();
    while let Some(parent) = parent_of.get(&current) {
        if !seen.insert(current.clone()) {
            break;
        }
        current = parent.clone();
    }
    current
}

/// Validates a comment and builds its `CommentCreateInput`.
fn comment_input(id: &str, body: &str, parent_id: &str) -> Result<Value, String> {
    let id = id.trim();
    if !valid_linear_id(id) {
        return Err("Missing Linear issue".into());
    }
    let body = body.trim();
    if body.is_empty() {
        return Err("Comment cannot be empty".into());
    }
    let parent = parent_id.trim();
    if !parent.is_empty() && !valid_linear_id(parent) {
        return Err("Invalid Linear comment".into());
    }
    let mut input = serde_json::Map::new();
    input.insert("issueId".into(), json!(id));
    input.insert("body".into(), json!(body));
    if !parent.is_empty() {
        input.insert("parentId".into(), json!(parent));
    }
    Ok(Value::Object(input))
}

// The Inbox has always accepted a success with no comment, showing no link.
fn parse_linear_comment_create(data: &Value) -> Result<String, String> {
    let payload = comment_payload(data)?;
    Ok(payload
        .get("comment")
        .and_then(|comment| string_field(comment, "url"))
        .unwrap_or_default())
}

/// Boards need the comment's id, so a success without one is a failure.
fn parse_comment_ref(data: &Value) -> Result<CommentRef, String> {
    let comment = comment_payload(data)?
        .get("comment")
        .filter(|comment| comment.is_object())
        .ok_or_else(|| "Linear did not return a comment".to_string())?;
    let id = string_field(comment, "id").unwrap_or_default();
    if id.is_empty() {
        return Err("Linear did not return a comment".into());
    }
    Ok(CommentRef {
        id,
        url: string_field(comment, "url").unwrap_or_default(),
    })
}

fn comment_payload(data: &Value) -> Result<&Value, String> {
    let payload = data
        .get("commentCreate")
        .ok_or_else(|| "Linear did not return a comment".to_string())?;
    let success = payload
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !success {
        return Err("Could not post Linear comment".into());
    }
    Ok(payload)
}

fn valid_linear_id(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty()
        && id.len() < 128
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
}

fn parse_labels(node: &Value) -> Vec<LinearLabel> {
    node.pointer("/labels/nodes")
        .and_then(Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|label| {
                    let name = string_field(label, "name")?;
                    let color = string_field(label, "color")
                        .unwrap_or_default()
                        .trim_start_matches('#')
                        .to_string();
                    Some(LinearLabel { name, color })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_assignees(node: &Value) -> Vec<LinearAssignee> {
    let (name, avatar_url) = person_fields(node.get("assignee"));
    name.map(|login| vec![LinearAssignee { login, avatar_url }])
        .unwrap_or_default()
}

fn person_fields(node: Option<&Value>) -> (Option<String>, String) {
    let Some(node) = node else {
        return (None, String::new());
    };
    (
        display_name(Some(node)),
        string_field(node, "avatarUrl").unwrap_or_default(),
    )
}

fn display_name(node: Option<&Value>) -> Option<String> {
    let node = node?;
    string_field(node, "displayName")
        .filter(|value| !value.is_empty())
        .or_else(|| string_field(node, "name"))
        .filter(|value| !value.is_empty())
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|text| text.trim().to_string())
}

const VIEWER_ACCOUNT_QUERY: &str = "query { viewer { id name displayName email } }";
const TEAM_STATES_QUERY: &str = r#"
query BoardTeamStates($id: String!) {
  team(id: $id) {
    states(first: 100) { nodes { id name type position } }
  }
}
"#;
const TEAM_LABELS_QUERY: &str = r#"
query BoardTeamLabels($id: String!, $after: String) {
  team(id: $id) {
    labels(first: 100, after: $after) {
      pageInfo { hasNextPage endCursor }
      nodes { id name color }
    }
  }
}
"#;
const TEAM_MEMBERS_QUERY: &str = r#"
query BoardTeamMembers($id: String!, $after: String) {
  team(id: $id) {
    members(first: 100, after: $after) {
      pageInfo { hasNextPage endCursor }
      nodes { id name displayName avatarUrl }
    }
  }
}
"#;
// Separate from the Inbox query so the Inbox request stays exactly as it was.
const BOARD_ISSUES_QUERY: &str = r#"
query BoardIssues($first: Int!, $after: String, $filter: IssueFilter) {
  issues(first: $first, after: $after, filter: $filter, orderBy: updatedAt) {
    pageInfo { hasNextPage endCursor }
    nodes {
      id
      identifier
      title
      url
      updatedAt
      state { id name type }
      team { id }
      labels { nodes { id name color } }
      assignee { id name displayName avatarUrl }
    }
  }
}
"#;
const BOARD_ISSUE_QUERY: &str = r#"
query BoardIssue($id: String!) {
  issue(id: $id) {
    id
    identifier
    title
    url
    updatedAt
    description
    state { id name type }
    team { id }
    labels { nodes { id name color } }
    assignee { id name displayName avatarUrl }
  }
}
"#;

fn require_id(id: &str, what: &str) -> ProviderResult<String> {
    let id = id.trim();
    if valid_linear_id(id) {
        Ok(id.to_string())
    } else {
        Err(ProviderError::InvalidInput(format!(
            "Missing Linear {what}"
        )))
    }
}

fn not_supported<T>(operation: &'static str) -> ProviderResult<T> {
    Err(ProviderError::NotSupported {
        provider: ProviderKind::Linear,
        operation,
    })
}

fn parse_viewer(data: &Value) -> Result<Account, String> {
    let viewer = data
        .get("viewer")
        .filter(|viewer| viewer.is_object())
        .ok_or_else(|| "Linear did not return the current user".to_string())?;
    Ok(Account {
        id: string_field(viewer, "id").unwrap_or_default(),
        name: display_name(Some(viewer)).unwrap_or_default(),
        email: string_field(viewer, "email").unwrap_or_default(),
    })
}

fn team_field<'a>(data: &'a Value, connection: &str) -> Result<&'a Value, String> {
    data.pointer(connection)
        .filter(|value| value.is_object())
        .ok_or_else(|| "Linear did not return that team".to_string())
}

fn parse_states(data: &Value) -> Result<Vec<WorkflowState>, String> {
    let nodes = team_field(data, "/team/states")?
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut states: Vec<(f64, WorkflowState)> = nodes
        .iter()
        .filter_map(|node| {
            let id = string_field(node, "id").filter(|id| !id.is_empty())?;
            let position = node.get("position").and_then(Value::as_f64).unwrap_or(0.0);
            Some((
                position,
                WorkflowState {
                    id,
                    name: string_field(node, "name").unwrap_or_default(),
                    group: StateGroup::from_linear_type(
                        &string_field(node, "type").unwrap_or_default(),
                    ),
                },
            ))
        })
        .collect();
    states.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(states.into_iter().map(|(_, state)| state).collect())
}

/// Collects `nodes` from every page of the connection at `connection`.
fn follow_pages(
    mut fetch: impl FnMut(Option<String>) -> Result<Value, String>,
    connection: &str,
) -> Result<Vec<Value>, String> {
    let mut nodes = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let data = fetch(after.clone())?;
        let page = team_field(&data, connection)?;
        if let Some(rows) = page.get("nodes").and_then(Value::as_array) {
            nodes.extend(rows.iter().cloned());
        }
        if page
            .pointer("/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Ok(nodes);
        }
        let next = page
            .pointer("/pageInfo/endCursor")
            .and_then(Value::as_str)
            .map(str::to_string);
        if next.is_none() || next == after {
            return Err("Linear pagination did not advance".into());
        }
        after = next;
    }
}

fn parse_board_label(node: &Value) -> Option<BoardLabel> {
    Some(BoardLabel {
        id: string_field(node, "id").filter(|id| !id.is_empty())?,
        name: string_field(node, "name").unwrap_or_default(),
        color: string_field(node, "color")
            .unwrap_or_default()
            .trim_start_matches('#')
            .to_string(),
    })
}

fn parse_member(node: &Value) -> Option<Member> {
    Some(Member {
        id: string_field(node, "id").filter(|id| !id.is_empty())?,
        name: display_name(Some(node)).unwrap_or_default(),
        avatar_url: string_field(node, "avatarUrl").unwrap_or_default(),
    })
}

fn parse_remote_issue(node: &Value) -> Option<RemoteIssue> {
    let id = string_field(node, "id").filter(|id| !id.is_empty())?;
    let state = node.get("state");
    Some(RemoteIssue {
        id,
        key: string_field(node, "identifier").unwrap_or_default(),
        title: string_field(node, "title").unwrap_or_default(),
        url: string_field(node, "url").unwrap_or_default(),
        container_id: node
            .get("team")
            .and_then(|team| string_field(team, "id"))
            .unwrap_or_default(),
        updated_at: string_field(node, "updatedAt").unwrap_or_default(),
        state: WorkflowState {
            id: state
                .and_then(|value| string_field(value, "id"))
                .unwrap_or_default(),
            name: state
                .and_then(|value| string_field(value, "name"))
                .unwrap_or_default(),
            group: StateGroup::from_linear_type(
                &state
                    .and_then(|value| string_field(value, "type"))
                    .unwrap_or_default(),
            ),
        },
        labels: node
            .pointer("/labels/nodes")
            .and_then(Value::as_array)
            .map(|nodes| nodes.iter().filter_map(parse_board_label).collect())
            .unwrap_or_default(),
        assignees: node
            .get("assignee")
            .and_then(parse_member)
            .into_iter()
            .collect(),
        // Linear descriptions are already Markdown.
        description: node
            .get("description")
            .map(|value| value.as_str().unwrap_or_default().to_string()),
    })
}

fn parse_issue_page(data: &Value) -> Result<Page<RemoteIssue>, String> {
    let issues = data
        .get("issues")
        .ok_or_else(|| "Linear did not return issues".to_string())?;
    let items = issues
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "Linear did not return issues".to_string())?
        .iter()
        .filter_map(parse_remote_issue)
        .collect();
    let more = issues
        .pointer("/pageInfo/hasNextPage")
        .and_then(Value::as_bool)
        == Some(true);
    let cursor = issues
        .pointer("/pageInfo/endCursor")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty());
    let next_cursor = match (more, cursor) {
        (false, _) => None,
        (true, Some(cursor)) => Some(cursor.to_string()),
        // Same rule as `follow_pages`: more pages with no way to reach them.
        (true, None) => return Err("Linear pagination did not advance".into()),
    };
    Ok(Page { items, next_cursor })
}

fn board_issue_variables(filter: &IssueFilter, cursor: Option<&str>) -> ProviderResult<Value> {
    let team_ids = filter
        .container_ids
        .iter()
        .map(|id| require_id(id, "team"))
        .collect::<ProviderResult<Vec<_>>>()?;
    let state = if filter.include_closed { "all" } else { "open" };
    Ok(json!({
        "first": filter.limit.clamp(1, MAX_PAGE),
        "after": cursor,
        "filter": issue_filter(filter.assigned_to_me, state, &team_ids),
    }))
}

impl LinearProvider {
    fn team_nodes(
        &self,
        query: &str,
        team_id: &str,
        connection: &str,
    ) -> ProviderResult<Vec<Value>> {
        let id = require_id(team_id, "team")?;
        Ok(follow_pages(
            |after| self.graphql(query, json!({ "id": id, "after": after })),
            connection,
        )?)
    }
}

impl BoardProvider for LinearProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Linear
    }

    fn whoami(&self) -> ProviderResult<Account> {
        let data = self.graphql(VIEWER_ACCOUNT_QUERY, json!({}))?;
        Ok(parse_viewer(&data)?)
    }

    fn list_containers(&self) -> ProviderResult<Vec<Container>> {
        Ok(self
            .teams()?
            .into_iter()
            .map(|team| Container {
                id: team.id,
                key: team.key,
                name: team.name,
            })
            .collect())
    }

    fn list_states(&self, container_id: &str) -> ProviderResult<Vec<WorkflowState>> {
        let id = require_id(container_id, "team")?;
        let data = self.graphql(TEAM_STATES_QUERY, json!({ "id": id }))?;
        Ok(parse_states(&data)?)
    }

    fn list_labels(&self, container_id: &str) -> ProviderResult<Vec<BoardLabel>> {
        let nodes = self.team_nodes(TEAM_LABELS_QUERY, container_id, "/team/labels")?;
        Ok(nodes.iter().filter_map(parse_board_label).collect())
    }

    fn list_members(&self, container_id: &str) -> ProviderResult<Vec<Member>> {
        let nodes = self.team_nodes(TEAM_MEMBERS_QUERY, container_id, "/team/members")?;
        Ok(nodes.iter().filter_map(parse_member).collect())
    }

    fn list_issues(
        &self,
        filter: &IssueFilter,
        cursor: Option<&str>,
    ) -> ProviderResult<Page<RemoteIssue>> {
        let variables = board_issue_variables(filter, cursor)?;
        let data = self.graphql(BOARD_ISSUES_QUERY, variables)?;
        Ok(parse_issue_page(&data)?)
    }

    fn get_issue(&self, id: &str) -> ProviderResult<RemoteIssue> {
        let id = require_id(id, "issue")?;
        let data = self.graphql(BOARD_ISSUE_QUERY, json!({ "id": id }))?;
        data.get("issue")
            .and_then(parse_remote_issue)
            .ok_or_else(|| ProviderError::Remote("Linear did not return that issue".into()))
    }

    // Linear writes and delta sync land in #98; webhook parsing waits on #109's fixtures.
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
        let input = comment_input(issue_id, body, reply_to.unwrap_or_default())
            .map_err(ProviderError::InvalidInput)?;
        let data = self.graphql(COMMENT_CREATE_MUTATION, json!({ "input": input }))?;
        Ok(parse_comment_ref(&data)?)
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

    #[test]
    fn parses_issue_fields_and_trims_label_hash() {
        let data = json!({ "issues": { "nodes": [
            {
                "id": "abc-1",
                "identifier": "ENG-7",
                "number": 7,
                "title": "Fix login",
                "url": "https://linear.app/x/issue/ENG-7",
                "updatedAt": "2026-03-01T10:00:00.000Z",
                "state": { "id": "s1", "name": "In Progress", "type": "started" },
                "team": { "id": "t1", "key": "ENG", "name": "Engineering" },
                "project": { "id": "p1", "name": "Auth" },
                "labels": { "nodes": [{ "id": "l1", "name": "bug", "color": "#ff0000" }] },
                "assignee": { "name": "ada", "displayName": "Ada", "avatarUrl": "https://a/ada.png" }
            },
            { "identifier": "ENG-8", "title": "No id" }
        ]}});
        let issues = parse_linear_issues(&data).unwrap();
        assert_eq!(issues.len(), 1);
        let issue = &issues[0];
        assert_eq!(issue.identifier, "ENG-7");
        assert_eq!(issue.state, "In Progress");
        assert_eq!(issue.state_type, "started");
        assert_eq!(issue.repo, "ENG");
        assert_eq!(issue.team_name, "Engineering");
        assert_eq!(issue.project_name, "Auth");
        assert_eq!(issue.labels[0].color, "ff0000");
        assert_eq!(issue.assignees[0].login, "Ada");
        assert!(parse_linear_issues(&json!({})).is_err());
    }

    #[test]
    fn nests_replies_under_their_root_comment() {
        let comment = |id: &str, parent: Option<&str>, at: &str| {
            json!({
                "id": id,
                "body": id,
                "createdAt": at,
                "user": { "name": "u" },
                "parent": parent.map(|p| json!({ "id": p })),
            })
        };
        let data = json!({ "issue": { "comments": {
            "pageInfo": { "hasNextPage": true },
            "nodes": [
                comment("reply2", Some("reply1"), "2026-01-03"),
                comment("root", None, "2026-01-01"),
                comment("reply1", Some("root"), "2026-01-02"),
                comment("orphan", Some("gone"), "2026-01-04"),
            ]
        }}});
        let thread = parse_linear_issue_thread(&data).unwrap();
        assert!(thread.truncated);
        let top: Vec<&str> = thread.comments.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(top, ["root", "orphan"]);
        let replies: Vec<&str> = thread.comments[0]
            .replies
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(replies, ["reply1", "reply2"]);
    }

    #[test]
    fn auth_graphql_errors_become_invalid_key() {
        let auth = r#"{"errors":[{"message":"Authentication required, not authenticated"}]}"#;
        assert_eq!(
            parse_graphql_data(auth).unwrap_err(),
            "Linear API key is invalid"
        );
        let other = r#"{"errors":[{"message":"Entity not found"}]}"#;
        assert_eq!(parse_graphql_data(other).unwrap_err(), "Entity not found");
        assert_eq!(
            parse_graphql_data(r#"{"data":{"viewer":{"id":"1"}}}"#).unwrap()["viewer"]["id"],
            "1"
        );
        assert_eq!(
            parse_graphql_data("<html>").unwrap_err(),
            "Linear returned invalid JSON"
        );
    }

    #[test]
    fn issue_filter_matches_inbox_request() {
        let filter = issue_filter(true, "open", &[" t1 ".into(), "".into()]);
        assert_eq!(
            filter,
            json!({
                "assignee": { "isMe": { "eq": true } },
                "team": { "id": { "in": ["t1"] } },
                "state": { "type": { "nin": ["completed", "canceled"] } },
            })
        );
        assert_eq!(issue_filter(false, "ALL", &[]), json!({}));
    }

    #[test]
    fn comment_create_requires_success() {
        let ok = json!({ "commentCreate": { "success": true, "comment": { "id": "c", "url": "https://l/c" } } });
        assert_eq!(parse_linear_comment_create(&ok).unwrap(), "https://l/c");
        let failed = json!({ "commentCreate": { "success": false } });
        assert_eq!(
            parse_linear_comment_create(&failed).unwrap_err(),
            "Could not post Linear comment"
        );
    }

    #[test]
    fn bearer_prefix_is_stripped_from_the_key() {
        assert_eq!(linear_authorization(" Bearer lin_api_x "), "lin_api_x");
        assert_eq!(linear_authorization("lin_api_x"), "lin_api_x");
    }

    fn issue_node() -> Value {
        json!({
            "id": "abc-1",
            "identifier": "ENG-7",
            "title": "Fix login",
            "url": "https://linear.app/x/issue/ENG-7",
            "updatedAt": "2026-03-01T10:00:00.000Z",
            "description": "Steps:\n1. Log in",
            "state": { "id": "s1", "name": "In Progress", "type": "started" },
            "team": { "id": "t1" },
            "labels": { "nodes": [{ "id": "l1", "name": "bug", "color": "#ff0000" }] },
            "assignee": { "id": "u1", "name": "ada", "displayName": "Ada", "avatarUrl": "https://a/ada.png" }
        })
    }

    #[test]
    fn viewer_becomes_account() {
        let data = json!({ "viewer": { "id": "u1", "name": "ada", "displayName": "", "email": "ada@example.com" } });
        assert_eq!(
            parse_viewer(&data).unwrap(),
            Account {
                id: "u1".into(),
                name: "ada".into(),
                email: "ada@example.com".into(),
            }
        );
        assert!(parse_viewer(&json!({ "viewer": null })).is_err());
    }

    #[test]
    fn states_are_ordered_by_position_and_grouped() {
        let data = json!({ "team": { "states": { "nodes": [
            { "id": "d", "name": "Done", "type": "completed", "position": 3.0 },
            { "id": "t", "name": "Todo", "type": "unstarted", "position": 1.0 },
            { "id": "x", "name": "Dropped", "type": "canceled", "position": 4.5 },
            { "id": "p", "name": "Doing", "type": "started", "position": 2 },
        ]}}});
        let states = parse_states(&data).unwrap();
        let order: Vec<(&str, StateGroup)> =
            states.iter().map(|s| (s.id.as_str(), s.group)).collect();
        assert_eq!(
            order,
            [
                ("t", StateGroup::Unstarted),
                ("p", StateGroup::Started),
                ("d", StateGroup::Completed),
                ("x", StateGroup::Cancelled),
            ]
        );
        assert_eq!(
            parse_states(&json!({ "team": null })).unwrap_err(),
            "Linear did not return that team"
        );
    }

    #[test]
    fn linear_paging_follows_end_cursor_and_rejects_stuck_pages() {
        let pages = [
            json!({ "team": { "labels": {
                "pageInfo": { "hasNextPage": true, "endCursor": "c1" },
                "nodes": [{ "id": "l1", "name": "bug", "color": "#f00" }]
            }}}),
            json!({ "team": { "labels": {
                "pageInfo": { "hasNextPage": false, "endCursor": "c2" },
                "nodes": [{ "id": "l2", "name": "ux", "color": "0f0" }]
            }}}),
        ];
        let mut seen = Vec::new();
        let nodes = follow_pages(
            |after| {
                seen.push(after.clone());
                Ok(pages[seen.len() - 1].clone())
            },
            "/team/labels",
        )
        .unwrap();
        assert_eq!(seen, [None, Some("c1".to_string())]);
        let labels: Vec<BoardLabel> = nodes.iter().filter_map(parse_board_label).collect();
        assert_eq!(labels[0].color, "f00");
        assert_eq!(labels[1].name, "ux");

        let stuck = json!({ "team": { "labels": {
            "pageInfo": { "hasNextPage": true, "endCursor": "same" },
            "nodes": []
        }}});
        let error = follow_pages(|_| Ok(stuck.clone()), "/team/labels").unwrap_err();
        assert_eq!(error, "Linear pagination did not advance");
        let no_cursor = json!({ "team": { "labels": {
            "pageInfo": { "hasNextPage": true, "endCursor": null },
            "nodes": []
        }}});
        assert!(follow_pages(|_| Ok(no_cursor.clone()), "/team/labels").is_err());
    }

    #[test]
    fn members_use_display_name_and_avatar() {
        let node = json!({ "id": "u1", "name": "ada", "displayName": "Ada L", "avatarUrl": "https://a/1.png" });
        assert_eq!(
            parse_member(&node).unwrap(),
            Member {
                id: "u1".into(),
                name: "Ada L".into(),
                avatar_url: "https://a/1.png".into(),
            }
        );
        assert_eq!(parse_member(&json!({ "name": "no id" })), None);
    }

    #[test]
    fn issue_node_maps_to_remote_issue() {
        let issue = parse_remote_issue(&issue_node()).unwrap();
        assert_eq!(issue.id, "abc-1");
        assert_eq!(issue.key, "ENG-7");
        assert_eq!(issue.container_id, "t1");
        assert_eq!(
            issue.state,
            WorkflowState {
                id: "s1".into(),
                name: "In Progress".into(),
                group: StateGroup::Started,
            }
        );
        assert_eq!(issue.labels[0].id, "l1");
        assert_eq!(issue.labels[0].color, "ff0000");
        assert_eq!(issue.assignees[0].id, "u1");
        assert_eq!(issue.description.as_deref(), Some("Steps:\n1. Log in"));

        let mut listed = issue_node();
        listed.as_object_mut().unwrap().remove("description");
        listed["assignee"] = Value::Null;
        let issue = parse_remote_issue(&listed).unwrap();
        assert_eq!(issue.description, None);
        assert!(issue.assignees.is_empty());
    }

    #[test]
    fn list_issues_page_exposes_end_cursor_only_when_more() {
        let page = |more: bool| {
            json!({ "issues": {
                "pageInfo": { "hasNextPage": more, "endCursor": "cur" },
                "nodes": [issue_node()]
            }})
        };
        let first = parse_issue_page(&page(true)).unwrap();
        assert_eq!(first.items.len(), 1);
        assert_eq!(first.next_cursor.as_deref(), Some("cur"));
        assert_eq!(parse_issue_page(&page(false)).unwrap().next_cursor, None);
    }

    #[test]
    fn board_issue_filter_reuses_the_inbox_filter() {
        let filter = IssueFilter {
            container_ids: vec!["t1".into()],
            assigned_to_me: false,
            include_closed: true,
            limit: 500,
        };
        assert_eq!(
            board_issue_variables(&filter, Some("cur")).unwrap(),
            json!({ "first": 100, "after": "cur", "filter": { "team": { "id": { "in": ["t1"] } } } })
        );
        let open = IssueFilter::default();
        let variables = board_issue_variables(&open, None).unwrap();
        assert_eq!(variables["after"], Value::Null);
        assert_eq!(variables["filter"]["state"]["type"]["nin"][1], "canceled");
        let bad = IssueFilter {
            container_ids: vec!["t1 OR 1=1".into()],
            ..Default::default()
        };
        assert!(matches!(
            board_issue_variables(&bad, None),
            Err(ProviderError::InvalidInput(_))
        ));
    }

    #[test]
    fn write_operations_are_not_supported_yet() {
        let linear = LinearProvider::new("lin_api_unused");
        let unsupported = |result: ProviderResult<()>, operation: &str| match result {
            Err(ProviderError::NotSupported {
                provider: ProviderKind::Linear,
                operation: actual,
            }) => assert_eq!(actual, operation),
            other => panic!("{operation}: {other:?}"),
        };
        unsupported(
            linear.create_issue("t1", &IssueDraft::default()).map(drop),
            "create_issue",
        );
        unsupported(
            linear.update_issue("abc", &IssuePatch::default()).map(drop),
            "update_issue",
        );
        unsupported(
            linear.move_issue("abc", &MoveTarget::default()).map(drop),
            "move_issue",
        );
        unsupported(linear.sync_since("t1", None).map(drop), "sync_since");
        unsupported(linear.parse_webhook(&[], b"{}").map(drop), "parse_webhook");
    }

    #[test]
    fn malformed_ids_are_rejected_before_any_request() {
        let linear = LinearProvider::new("lin_api_unused");
        let invalid = |result: ProviderResult<()>| {
            assert!(
                matches!(result, Err(ProviderError::InvalidInput(_))),
                "{result:?}"
            )
        };
        invalid(linear.list_states("").map(drop));
        invalid(linear.list_labels("a b").map(drop));
        invalid(linear.list_members("x\"y").map(drop));
        invalid(linear.get_issue("../etc").map(drop));
        invalid(linear.comment("abc", "  ", None).map(drop));
        invalid(linear.comment("abc", "Hi", Some("x y")).map(drop));
        assert_eq!(linear.kind(), ProviderKind::Linear);
    }

    #[test]
    fn issue_page_with_more_but_no_cursor_is_an_error() {
        for cursor in [Value::Null, json!("")] {
            let data = json!({ "issues": {
                "pageInfo": { "hasNextPage": true, "endCursor": cursor },
                "nodes": [issue_node()]
            }});
            assert_eq!(
                parse_issue_page(&data).unwrap_err(),
                "Linear pagination did not advance"
            );
        }
    }

    #[test]
    fn comment_without_a_comment_object_is_a_failure_for_boards() {
        let empty = json!({ "commentCreate": { "success": true, "comment": null } });
        assert_eq!(
            parse_comment_ref(&empty).unwrap_err(),
            "Linear did not return a comment"
        );
        // The Inbox keeps its old behaviour: success with no URL.
        assert_eq!(parse_linear_comment_create(&empty).unwrap(), "");
    }
}
