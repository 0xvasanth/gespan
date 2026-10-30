//! Native Inbox services. Provider adapters preserve the Tauri API's requests,
//! validation, response parsers and saved credential formats.
mod avatar;
mod media;
pub use media::init_video_dir as init_media_video_dir;
pub use media::png as media_png;
#[doc(hidden)]
pub use media::remember_video;
pub use media::video as media_video;
pub mod azure_devops;
pub use avatar::png as avatar_png;
pub(crate) mod github;
pub mod gitlab;
pub mod jira;
pub mod linear;
pub use gespan_tracker_client::credentials::plane;

use crate::{
    desktop_preferences::DesktopPreferences, files::remote as remote_workspace,
    session_groups::GithubWorkItem, store::Store,
};
use chrono::{Datelike, Local, TimeZone};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
};

pub const PROVIDERS: [&str; 5] = ["github", "linear", "jira", "gitlab", "azuredevops"];
pub fn provider_label(provider: &str) -> &'static str {
    match provider {
        "linear" => "Linear",
        "jira" => "Jira",
        "plane" => "Plane",
        "gitlab" => "GitLab",
        "azuredevops" => "ADO",
        _ => "GitHub",
    }
}
pub fn is_tracker(provider: &str) -> bool {
    matches!(provider, "linear" | "jira")
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Label {
    pub name: String,
    pub color: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Person {
    pub login: String,
    pub avatar_url: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Item {
    pub provider: String,
    pub kind: String,
    pub id: String,
    pub identifier: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub state_type: String,
    pub state_reason: String,
    pub created_at: String,
    pub updated_at: String,
    pub labels: Vec<Label>,
    pub assignees: Vec<Person>,
    pub draft: bool,
    pub repo: String,
    pub team_id: String,
    pub team_name: String,
    pub project_id: String,
    pub project_name: String,
    pub project_path: String,
    pub attention_reason: String,
}
impl Item {
    pub fn key(&self) -> String {
        let identity = if self.provider == "linear" {
            let id = if self.identifier.trim().is_empty() {
                self.id.trim()
            } else {
                self.identifier.trim()
            };
            if id.is_empty() {
                format!("linear:{}", self.number)
            } else {
                id.to_lowercase()
            }
        } else if self.provider == "jira" {
            let id = if self.id.trim().is_empty() {
                self.identifier.trim()
            } else {
                self.id.trim()
            };
            if id.is_empty() {
                format!("jira:{}", self.number)
            } else {
                id.to_lowercase()
            }
        } else if !self.repo.trim().is_empty() {
            format!(
                "{}:{}:{}",
                self.repo.trim().to_lowercase(),
                self.kind,
                self.number
            )
        } else if !self.url.trim().is_empty() {
            self.url.trim().to_lowercase()
        } else {
            format!("{}:{}", self.kind, self.number)
        };
        format!("{}:{identity}", self.provider)
    }
    pub fn reference(&self) -> String {
        if self.identifier.is_empty() {
            format!("#{}", self.number)
        } else {
            self.identifier.clone()
        }
    }
    pub fn updated_ms(&self) -> i64 {
        timestamp_ms(&self.updated_at)
    }
    pub fn status(&self) -> &'static str {
        if self.state.eq_ignore_ascii_case("merged") {
            "Merged"
        } else if matches!(
            self.state.to_lowercase().as_str(),
            "closed" | "done" | "resolved" | "removed" | "completed"
        ) || matches!(self.state_type.as_str(), "completed" | "canceled" | "done")
        {
            "Closed"
        } else if self.draft {
            "Draft"
        } else {
            "Open"
        }
    }
    pub fn work_item(&self) -> GithubWorkItem {
        GithubWorkItem {
            kind: self.kind.clone(),
            repo: self.repo.clone(),
            number: self.number.max(0) as u64,
            url: self.url.clone(),
        }
    }
    pub fn prompt(&self, body: &str, ask: bool) -> String {
        format!(
            "{}\n\n{} {}: {}\n{}\n\n{}",
            if ask {
                "Help me understand and discuss this work item."
            } else {
                "Work on this item. Read its description and discussion, inspect the project, and implement the required changes."
            },
            provider_label(&self.provider),
            self.reference(),
            self.title,
            self.url,
            body
        )
    }
}
pub fn timestamp_ms(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|at| at.timestamp_millis())
        .unwrap_or(0)
}
pub fn relative_time(value: &str) -> String {
    let at = timestamp_ms(value);
    if at == 0 {
        return String::new();
    }
    let seconds = ((crate::time::now_ms() as i64 - at).max(0)) / 1000;
    match seconds {
        0..60 => "now".into(),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        86400..604800 => format!("{}d", seconds / 86400),
        _ => chrono::DateTime::from_timestamp_millis(at)
            .map(|d| d.format("%b %-d").to_string())
            .unwrap_or_default(),
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Details {
    pub body: String,
    pub author: String,
    pub author_avatar_url: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub review_decision: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Comment {
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
    pub replies: Vec<Comment>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Commit {
    pub oid: String,
    pub message_headline: String,
    pub author: String,
    pub committed_date: String,
    pub url: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Thread {
    pub comments: Vec<Comment>,
    pub commits: Vec<Commit>,
    pub truncated: bool,
    pub review_decision: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DiffFile {
    pub path: String,
    pub additions: i64,
    pub deletions: i64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Diff {
    pub additions: i64,
    pub deletions: i64,
    pub files: Vec<DiffFile>,
    pub patch: String,
    pub truncated: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    pub workflow: String,
    pub state: String,
    pub url: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Checks {
    pub head_oid: String,
    pub checks: Vec<Check>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Status {
    pub connected: bool,
    pub installed: bool,
    pub authenticated: bool,
    pub url: String,
    pub site: String,
    pub email: String,
    pub organization: String,
    pub workspace: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OptionRow {
    pub id: String,
    pub name: String,
    pub key: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StatusFilter {
    pub open: bool,
    pub draft: bool,
    pub closed: bool,
    pub merged: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Filters {
    pub assigned_to_me: bool,
    pub hidden_projects: Vec<String>,
    pub hidden_linear_projects: Vec<String>,
    pub hidden_kinds: Vec<String>,
    pub time: String,
    pub status: StatusFilter,
}
impl Default for Filters {
    fn default() -> Self {
        Self {
            assigned_to_me: false,
            hidden_projects: vec![],
            hidden_linear_projects: vec![],
            hidden_kinds: vec![],
            time: "all".into(),
            status: StatusFilter::default(),
        }
    }
}
impl Filters {
    pub fn fetch_state(&self) -> &str {
        if self.status.closed || self.status.merged || !(self.status.open || self.status.draft) {
            "all"
        } else {
            "open"
        }
    }
    pub fn matches(&self, item: &Item, source: &str, search: &str) -> bool {
        if item.provider != source {
            return false;
        }
        let tracker = is_tracker(source);
        if !tracker
            && !((source == "gitlab" || source == "azuredevops") && self.assigned_to_me)
            && self.hidden_projects.contains(&item.project_path)
        {
            return false;
        }
        if !tracker && self.hidden_kinds.contains(&item.kind) {
            return false;
        }
        let linear_project = if item.project_id.is_empty() {
            "~none"
        } else {
            &item.project_id
        };
        if source == "linear"
            && self
                .hidden_linear_projects
                .iter()
                .any(|id| id == linear_project)
        {
            return false;
        }
        let s = &self.status;
        if (s.open || s.closed || (!tracker && (s.draft || s.merged)))
            && !match item.status() {
                "Open" => s.open,
                "Draft" => s.draft && !tracker,
                "Closed" => s.closed,
                "Merged" => s.merged && !tracker,
                _ => true,
            }
        {
            return false;
        }
        let now = Local::now();
        let start = match self.time.as_str() {
            "today" => Local
                .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
                .single()
                .map(|d| d.timestamp_millis())
                .unwrap_or(0),
            "7d" => now.timestamp_millis() - 7 * 86400000,
            "30d" => now.timestamp_millis() - 30 * 86400000,
            _ => 0,
        };
        if start > 0 && item.updated_ms() < start {
            return false;
        }
        let text = format!(
            "{} {} {} {} {} {}",
            item.title,
            item.reference(),
            item.number,
            item.repo,
            item.team_name,
            item.labels
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        )
        .to_lowercase();
        search
            .split_whitespace()
            .all(|word| text.contains(&word.to_lowercase()))
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Seen {
    pub seeded: bool,
    pub items: BTreeMap<String, i64>,
}
impl Seen {
    pub fn unseen(&self, item: &Item) -> bool {
        self.seeded
            && self
                .items
                .get(&item.key())
                .is_none_or(|at| item.updated_ms() > *at)
    }
    pub fn mark(&mut self, item: &Item) {
        let old = self.items.entry(item.key()).or_default();
        *old = (*old).max(item.updated_ms());
    }
    pub fn seed(&mut self, items: &[Item]) {
        if !self.seeded && !items.is_empty() {
            for item in items {
                self.mark(item);
            }
            self.seeded = true;
        }
    }
}
/// What a change Gespan made itself was made to. Empty fields match anything,
/// but at least one of `id`, `repo`, `number` or `project_path` must be set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelfTarget {
    pub provider: String,
    pub kind: Option<String>,
    pub repo: Option<String>,
    pub number: Option<i64>,
    pub id: Option<String>,
    pub project_path: Option<String>,
}
impl SelfTarget {
    /// The item a comment or action was made on.
    pub fn of(item: &Item) -> Self {
        let text = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_owned());
        Self {
            provider: item.provider.clone(),
            kind: text(&item.kind),
            repo: text(&item.repo),
            number: (item.number != 0).then_some(item.number),
            id: text(&item.id),
            project_path: None,
        }
    }
    fn matches(&self, item: &Item) -> bool {
        let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
        let path = |p: &str| p.trim_end_matches(['/', '\\']).to_owned();
        self.provider == item.provider
            && self.id.as_ref().is_none_or(|id| *id == item.id)
            && self.kind.as_ref().is_none_or(|kind| *kind == item.kind)
            && self.number.is_none_or(|number| number == item.number)
            && self.repo.as_ref().is_none_or(|repo| same(repo, &item.repo))
            && self
                .project_path
                .as_ref()
                .is_none_or(|p| path(p) == path(&item.project_path))
            && (self.id.is_some()
                || self.repo.is_some()
                || self.number.is_some()
                || self.project_path.is_some())
    }
}
/// inboxSelfActivity: changes Gespan made, so the update they cause is not
/// announced back to its author as unread. Each is kept ten minutes.
#[derive(Clone, Debug, Default)]
pub struct SelfActivity {
    pending: Vec<(SelfTarget, u64)>,
}
impl SelfActivity {
    const MAX_AGE_MS: u64 = 10 * 60_000;
    fn prune(&mut self, now: u64) {
        let oldest = now.saturating_sub(Self::MAX_AGE_MS);
        self.pending.retain(|(_, at)| *at >= oldest);
    }
    pub fn record(&mut self, target: SelfTarget, now: u64) {
        self.prune(now);
        self.pending.push((target, now));
    }
    /// Whether `item`'s new revision is one of ours. Consumes every match, so
    /// a later revision is announced as usual.
    pub fn consume(&mut self, item: &Item, now: u64) -> bool {
        self.prune(now);
        let before = self.pending.len();
        self.pending.retain(|(target, _)| !target.matches(item));
        self.pending.len() != before
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    pub source: String,
    pub filters: Filters,
    pub seen: Seen,
    pub hidden_linear_teams: Vec<String>,
    pub hidden_jira_projects: Vec<String>,
    pub list_width: f32,
}
impl Default for ViewState {
    fn default() -> Self {
        Self {
            source: "github".into(),
            filters: Filters::default(),
            seen: Seen::default(),
            hidden_linear_teams: vec![],
            hidden_jira_projects: vec![],
            list_width: 280.,
        }
    }
}
impl ViewState {
    pub fn fetch_scope(&self, projects: &[PathBuf]) -> String {
        let mut paths = projects
            .iter()
            .map(|p| crate::files::path_key(&p.to_string_lossy()))
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        let mut teams = self.hidden_linear_teams.clone();
        teams.sort();
        let mut jira = self.hidden_jira_projects.clone();
        jira.sort();
        format!(
            "{}:{}:{}:{}:{}",
            self.filters.assigned_to_me,
            self.filters.fetch_state(),
            paths.join("|"),
            teams.join(","),
            jira.join(",")
        )
    }
    pub fn load(store: &Store) -> Self {
        if let Some(state) = store
            .setting("native-inbox-state")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            return state;
        }
        let desktop = store
            .data_dir()
            .map(DesktopPreferences::load)
            .unwrap_or_default();
        let mut state = Self::default();
        if let Some(source) = desktop
            .get("gespan.inboxSource")
            .filter(|s| PROVIDERS.contains(s))
        {
            state.source = source.into();
        }
        if let Some(filters) = desktop
            .json("gespan.inboxFilters")
            .and_then(|v| serde_json::from_value(v).ok())
        {
            state.filters = filters;
        }
        if let Some(seen) = desktop.json("gespan.inboxSeen") {
            state.seen = serde_json::from_value(seen.clone()).unwrap_or_else(|_| Seen {
                seeded: true,
                items: serde_json::from_value(seen).unwrap_or_default(),
            });
        }
        state.hidden_linear_teams = desktop
            .json("gespan.linearHiddenTeams")
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        state.hidden_jira_projects = desktop
            .json("gespan.jiraHiddenProjects")
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        state
    }
    pub fn save(&self, store: &Store) -> Result<(), String> {
        store.save_setting(
            "native-inbox-state",
            &serde_json::to_string(self).map_err(|e| e.to_string())?,
        )
    }
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub items: Vec<Item>,
    pub errors: BTreeMap<String, String>,
    pub connections: BTreeMap<String, Status>,
    pub linear_teams: Vec<OptionRow>,
    pub jira_projects: Vec<OptionRow>,
    /// Repository scopes (see `repo_scope`) fetched in full by this poll.
    pub fetched: BTreeSet<String>,
    /// Repository scopes that failed while others of their provider loaded.
    pub repo_errors: BTreeMap<String, String>,
    /// `Item::key` to its repository scope, for repository-backed items.
    pub item_scopes: HashMap<String, String>,
}

/// One project's repository and work-item kind: the unit a forge fetch
/// succeeds or fails as.
fn repo_scope(provider: &str, project: &Path, repo: &str, kind: &str) -> String {
    format!(
        "{provider}:{}:{}:{kind}",
        crate::files::path_key(&project.to_string_lossy()),
        repo.to_lowercase()
    )
}

#[derive(Debug, Default)]
struct RepoFetch {
    items: Vec<Item>,
    fetched: BTreeSet<String>,
    errors: BTreeMap<String, String>,
    item_scopes: HashMap<String, String>,
}

/// Fetches issues and PRs for each project's repositories. One failing
/// repository, such as a fork with issues disabled, must not hide the rest;
/// like Tauri's collectInboxResults, the provider fails only when no fetch
/// succeeded.
/// Puts the active checkout first. Several checkouts of the same repository
/// share its rows, and the first one visited fetches them, so the active
/// checkout keeps them when the inbox is scoped to it.
fn active_first(projects: &[PathBuf], cwd: &Path) -> Vec<PathBuf> {
    let mut ordered = projects.to_vec();
    ordered.sort_by_key(|project| project.as_path() != cwd);
    ordered
}

/// What the host answers for a project that has no GitHub repository.
const NOT_ON_GITHUB: &str = "GitHub did not return a repository";

/// Each machine signs in to GitHub on its own, so one that can't read a
/// repository says nothing about another.
fn github_account(project: &Path) -> &str {
    project
        .to_str()
        .and_then(|path| path.strip_prefix(crate::remote::REMOTE_PATH_PREFIX))
        .and_then(|path| path.split('/').next())
        .unwrap_or("")
}

/// The projects GitHub can be asked about. Without a local `gh` account, the
/// projects on other machines still answer with theirs.
fn github_reachable(signed_in: bool, projects: &[PathBuf]) -> Vec<PathBuf> {
    projects
        .iter()
        .filter(|project| signed_in || remote_workspace::is_remote(project))
        .cloned()
        .collect()
}

/// Runs a GitHub Inbox operation with the project's own `gh`: a remote
/// project's checkout and account are on its machine.
fn github_call<T: DeserializeOwned, L: Serialize>(
    project: &Path,
    command: &str,
    mut args: Value,
    local: impl FnOnce() -> Result<L, String>,
) -> Result<T, String> {
    if remote_workspace::is_remote(project) {
        args["cwd"] = json!(project);
        return remote_workspace::command(command, args);
    }
    convert(local()?)
}

fn github_repositories(project: &Path) -> Result<Vec<String>, String> {
    github_call(project, "git_github_repositories", json!({}), || {
        github::git_github_repositories_for(project)
    })
}

fn github_work_items(
    project: &Path,
    repo: &str,
    kind: &str,
    assigned_to_me: bool,
    state: &str,
    limit: u32,
) -> Result<Vec<Item>, String> {
    github_call(
        project,
        "git_github_work_items",
        json!({
            "repo": repo,
            "kind": kind,
            "assignedToMe": assigned_to_me,
            "state": state,
            "search": "",
            "limit": limit,
        }),
        || github::git_github_work_items_for(project, repo, kind, assigned_to_me, state, "", limit),
    )
}

fn collect_repo_items(
    provider: &str,
    projects: &[PathBuf],
    repos_for: impl Fn(&Path) -> Result<Vec<String>, String>,
    mut fetch: impl FnMut(&Path, &str, &str) -> Result<Vec<Item>, String>,
) -> Result<RepoFetch, String> {
    let mut out = RepoFetch::default();
    let mut visited = HashSet::new();
    for project in projects {
        let repos = match repos_for(project) {
            Ok(repos) => repos,
            // A folder that isn't on this forge has nothing to list, but a
            // machine that can't answer would otherwise just go quiet.
            Err(error) if remote_workspace::is_remote(project) && error != NOT_ON_GITHUB => {
                out.errors
                    .insert(repo_scope(provider, project, "", "repositories"), error);
                continue;
            }
            Err(_) => continue,
        };
        for repo in repos {
            let account = if provider == "github" {
                github_account(project)
            } else {
                ""
            };
            if !visited.insert(format!("{account}:{}", repo.to_lowercase())) {
                continue;
            }
            for kind in ["issue", "pr"] {
                let scope = repo_scope(provider, project, &repo, kind);
                match fetch(project, &repo, kind) {
                    Ok(rows) => {
                        for mut item in rows {
                            item.provider = provider.into();
                            item.project_path = project.to_string_lossy().into_owned();
                            out.item_scopes
                                .entry(item.key())
                                .or_insert_with(|| scope.clone());
                            out.items.push(item);
                        }
                        out.fetched.insert(scope);
                    }
                    Err(error) => {
                        out.errors.insert(scope, error);
                    }
                }
            }
        }
    }
    match out.errors.values().next() {
        Some(first) if out.fetched.is_empty() => Err(first.clone()),
        _ => Ok(out),
    }
}

/// The Tauri InboxNotificationTracker: provider baselines and revisions are
/// separate from the user's read state. Startup and query changes never
/// trigger automations for an existing backlog.
#[derive(Default)]
pub struct NotificationTracker {
    revisions: HashMap<String, i64>,
    primed: HashSet<String>,
    /// Repository scopes with a successful baseline. A repository that
    /// failed earlier has none, so its backlog is not new when it recovers.
    primed_repos: HashSet<String>,
    scope: Option<String>,
}
impl NotificationTracker {
    pub fn observe(&mut self, snapshot: &Snapshot, scope: String) -> Vec<Item> {
        if self.scope.as_ref() != Some(&scope) {
            self.primed.clear();
            self.primed_repos.clear();
        }
        self.scope = Some(scope);
        let mut appeared = vec![];
        for item in &snapshot.items {
            if snapshot.errors.contains_key(&item.provider) {
                continue;
            }
            let at = item.updated_ms();
            if at == 0 {
                continue;
            }
            let project = match item.provider.as_str() {
                "linear" if !item.project_id.is_empty() => {
                    format!("linear:project:{}", item.project_id)
                }
                "linear" => format!("linear:team:{}:unassigned", item.team_id),
                "jira" => format!(
                    "jira:{}:project:{}",
                    item.url
                        .split('/')
                        .nth(2)
                        .unwrap_or("unknown")
                        .to_lowercase(),
                    if item.team_id.is_empty() {
                        &item.repo
                    } else {
                        &item.team_id
                    }
                ),
                _ if !item.project_path.is_empty() => crate::files::path_key(&item.project_path),
                _ => format!("{}:{}", item.provider, item.repo.to_lowercase()),
            };
            let id = if item.id.is_empty() {
                item.number.to_string()
            } else {
                item.id.clone()
            };
            let key = format!("{project}:{}:{id}", item.kind);
            let previous = self.revisions.get(&key).copied();
            let primed = match snapshot.item_scopes.get(&item.key()) {
                Some(repo) => self.primed_repos.contains(repo),
                None => self.primed.contains(&item.provider),
            };
            if primed && previous.is_none() {
                appeared.push(item.clone());
            }
            self.revisions.insert(key, previous.unwrap_or(0).max(at));
        }
        for provider in PROVIDERS {
            if !snapshot.errors.contains_key(provider) {
                self.primed.insert(provider.into());
            }
        }
        self.primed_repos.extend(snapshot.fetched.iter().cloned());
        appeared
    }
}

#[derive(Clone)]
pub struct Service {
    pub data_dir: PathBuf,
    pub cwd: PathBuf,
}
/// Plane is a connection only: tickets link to it, but it lists no issues.
fn plane_connection(status: plane::PlaneStatus) -> Result<Status, String> {
    Ok(Status {
        connected: status.connected,
        url: status.base_url,
        workspace: status.workspace,
        ..Default::default()
    })
}
fn convert<T: Serialize, U: DeserializeOwned>(value: T) -> Result<U, String> {
    serde_json::from_value(serde_json::to_value(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
impl Service {
    pub fn status(&self, provider: &str) -> Result<Status, String> {
        match provider {
            "linear" => convert(linear::linear_status(self.data_dir.clone())?),
            "jira" => convert(jira::jira_status(self.data_dir.clone())?),
            "gitlab" => convert(gitlab::gitlab_status(self.data_dir.clone())?),
            "azuredevops" => convert(azure_devops::azure_devops_status(self.data_dir.clone())?),
            "plane" => plane_connection(plane::plane_status(self.data_dir.clone())?),
            _ => {
                let installed = crate::scm::resolve_gui_binary("gh").is_some();
                let authenticated = installed
                    && run_gh(
                        &gh_root(&self.cwd),
                        &["auth", "status", "--active", "--hostname", "github.com"],
                        true,
                    )
                    .is_ok();
                Ok(Status {
                    installed,
                    authenticated,
                    connected: authenticated,
                    ..Default::default()
                })
            }
        }
    }
    pub fn connect(
        &self,
        provider: &str,
        url: String,
        email: String,
        token: String,
    ) -> Result<Status, String> {
        match provider {
            "linear" => convert(linear::linear_set_token(self.data_dir.clone(), token)?),
            "jira" => convert(jira::jira_set_config(
                self.data_dir.clone(),
                url,
                email,
                token,
            )?),
            "gitlab" => convert(gitlab::gitlab_set_config(
                self.data_dir.clone(),
                url,
                token,
            )?),
            "azuredevops" => convert(azure_devops::azure_devops_set_config(
                self.data_dir.clone(),
                url,
                token,
            )?),
            // Plane takes a workspace as well, which rides in the email slot.
            "plane" => plane_connection(plane::plane_set_config(
                self.data_dir.clone(),
                url,
                email,
                token,
            )?),
            _ => self.status("github"),
        }
    }
    pub fn list(&self, projects: &[PathBuf], state: &ViewState) -> Snapshot {
        let mut snapshot = Snapshot::default();
        let limit = if state.filters.fetch_state() == "all" {
            100
        } else {
            40
        };
        for provider in PROVIDERS {
            let status = match self.status(provider) {
                Ok(status) => status,
                Err(error) => {
                    snapshot.errors.insert(provider.into(), error);
                    continue;
                }
            };
            let connected = status.connected;
            snapshot.connections.insert(provider.into(), status);
            let github = (provider == "github").then(|| github_reachable(connected, projects));
            if !connected && github.as_ref().is_none_or(Vec::is_empty) {
                continue;
            }
            let result: Result<Vec<Item>, String> = (|| {
                if provider == "linear" {
                    let teams = linear::linear_list_teams(self.data_dir.clone())?;
                    let visible = teams
                        .iter()
                        .filter(|t| !state.hidden_linear_teams.contains(&t.id))
                        .map(|t| t.id.clone())
                        .collect::<Vec<_>>();
                    snapshot.linear_teams = convert(teams)?;
                    if !state.hidden_linear_teams.is_empty() && visible.is_empty() {
                        return Ok(vec![]);
                    }
                    return convert(linear::linear_list_issues(
                        self.data_dir.clone(),
                        state.filters.assigned_to_me,
                        state.filters.fetch_state().into(),
                        if state.hidden_linear_teams.is_empty() {
                            vec![]
                        } else {
                            visible
                        },
                        Some(limit),
                    )?);
                }
                if provider == "jira" {
                    let projects = jira::jira_list_projects(self.data_dir.clone())?;
                    let visible = projects
                        .iter()
                        .filter(|p| !state.hidden_jira_projects.contains(&p.id))
                        .map(|p| p.id.clone())
                        .collect::<Vec<_>>();
                    snapshot.jira_projects = convert(projects)?;
                    if !state.hidden_jira_projects.is_empty() && visible.is_empty() {
                        return Ok(vec![]);
                    }
                    return convert(jira::jira_list_issues(
                        self.data_dir.clone(),
                        state.filters.assigned_to_me,
                        state.filters.fetch_state().into(),
                        if state.hidden_jira_projects.is_empty() {
                            vec![]
                        } else {
                            visible
                        },
                        Some(limit),
                    )?);
                }
                let mut items = Vec::new();
                if state.filters.assigned_to_me && provider != "github" {
                    for kind in ["issue", "pr"] {
                        let rows: Vec<Item> = if provider == "gitlab" {
                            convert(gitlab::gitlab_list_todos(
                                self.data_dir.clone(),
                                kind.into(),
                                Some(limit),
                            )?)?
                        } else {
                            convert(azure_devops::azure_devops_list_todos(
                                self.data_dir.clone(),
                                kind.into(),
                                Some(limit),
                            )?)?
                        };
                        items.extend(rows.into_iter().map(|mut item| {
                            item.provider = provider.into();
                            item
                        }));
                    }
                    return Ok(items);
                }
                let fetch = collect_repo_items(
                    provider,
                    &active_first(github.as_deref().unwrap_or(projects), &self.cwd),
                    |project| match provider {
                        "github" => github_repositories(project),
                        "gitlab" => gitlab::gitlab_repo(
                            self.data_dir.clone(),
                            project.to_string_lossy().into_owned(),
                        )
                        .map(|r| vec![r]),
                        _ => azure_devops::azure_devops_repo(
                            self.data_dir.clone(),
                            project.to_string_lossy().into_owned(),
                        )
                        .map(|r| vec![r]),
                    },
                    |project, repo, kind| match provider {
                        "github" => github_work_items(
                            project,
                            repo,
                            kind,
                            state.filters.assigned_to_me,
                            state.filters.fetch_state(),
                            limit,
                        ),
                        "gitlab" => convert(gitlab::gitlab_list_work_items(
                            self.data_dir.clone(),
                            project.to_string_lossy().into_owned(),
                            kind.into(),
                            false,
                            state.filters.fetch_state().into(),
                            Some(limit),
                        )?),
                        _ => convert(azure_devops::azure_devops_list_work_items(
                            self.data_dir.clone(),
                            project.to_string_lossy().into_owned(),
                            kind.into(),
                            false,
                            state.filters.fetch_state().into(),
                            Some(limit),
                        )?),
                    },
                )?;
                if !fetch.fetched.is_empty() {
                    // Another machine's account is a connection too.
                    if let Some(status) = snapshot.connections.get_mut(provider) {
                        status.connected = true;
                    }
                }
                snapshot.fetched.extend(fetch.fetched);
                snapshot.repo_errors.extend(fetch.errors);
                snapshot.item_scopes.extend(fetch.item_scopes);
                Ok(fetch.items)
            })();
            match result {
                Ok(items) => snapshot.items.extend(items),
                Err(error) => {
                    snapshot.errors.insert(provider.into(), error);
                }
            }
        }
        match self.status("plane") {
            Ok(status) => {
                snapshot.connections.insert("plane".into(), status);
            }
            Err(error) => {
                snapshot.errors.insert("plane".into(), error);
            }
        }
        snapshot.items.sort_by(|a, b| {
            b.updated_ms()
                .cmp(&a.updated_ms())
                .then_with(|| a.title.cmp(&b.title))
        });
        let mut seen = HashSet::new();
        snapshot.items.retain(|i| seen.insert(i.key()));
        snapshot
    }
    pub fn details(&self, item: &Item) -> Result<Details, String> {
        match item.provider.as_str() {
            "linear" => convert(linear::linear_issue_details(
                self.data_dir.clone(),
                item.id.clone(),
            )?),
            "jira" => convert(jira::jira_issue_details(
                self.data_dir.clone(),
                item.identifier.clone(),
            )?),
            "gitlab" => convert(gitlab::gitlab_work_item_details(
                self.data_dir.clone(),
                item.repo.clone(),
                item.kind.clone(),
                item.number,
            )?),
            "azuredevops" => convert(azure_devops::azure_devops_work_item_details(
                self.data_dir.clone(),
                item.repo.clone(),
                item.kind.clone(),
                item.number,
            )?),
            _ => github_call(
                self.project(item),
                "git_github_work_item_details",
                json!({"repo": item.repo, "kind": item.kind, "number": item.number}),
                || {
                    github::git_github_work_item_details_for(
                        self.project(item),
                        &item.repo,
                        &item.kind,
                        item.number,
                    )
                },
            ),
        }
    }
    fn project<'a>(&'a self, item: &'a Item) -> &'a Path {
        if item.project_path.is_empty() {
            &self.cwd
        } else {
            Path::new(&item.project_path)
        }
    }
    pub fn thread(&self, item: &Item) -> Result<Thread, String> {
        match item.provider.as_str() {
            "linear" => convert(linear::linear_issue_thread(
                self.data_dir.clone(),
                item.id.clone(),
            )?),
            "jira" => convert(jira::jira_issue_thread(
                self.data_dir.clone(),
                item.identifier.clone(),
            )?),
            "gitlab" => convert(gitlab::gitlab_work_item_thread(
                self.data_dir.clone(),
                item.repo.clone(),
                item.kind.clone(),
                item.number,
            )?),
            "azuredevops" => convert(azure_devops::azure_devops_work_item_thread(
                self.data_dir.clone(),
                item.repo.clone(),
                item.kind.clone(),
                item.number,
            )?),
            _ => github_call(
                self.project(item),
                "git_github_work_item_thread",
                json!({"repo": item.repo, "kind": item.kind, "number": item.number}),
                || {
                    github::git_github_work_item_thread_for(
                        self.project(item),
                        &item.repo,
                        &item.kind,
                        item.number,
                    )
                },
            ),
        }
    }
    pub fn comment(&self, item: &Item, body: String, reply_to: String) -> Result<String, String> {
        match item.provider.as_str() {
            "linear" => {
                linear::linear_issue_comment(self.data_dir.clone(), item.id.clone(), body, reply_to)
            }
            "jira" => {
                jira::jira_issue_comment(self.data_dir.clone(), item.identifier.clone(), body)
            }
            "gitlab" => gitlab::gitlab_work_item_comment(
                self.data_dir.clone(),
                item.repo.clone(),
                item.kind.clone(),
                item.number,
                body,
            ),
            "azuredevops" => azure_devops::azure_devops_work_item_comment(
                self.data_dir.clone(),
                item.repo.clone(),
                item.kind.clone(),
                item.number,
                body,
            ),
            _ => github_call(
                self.project(item),
                "git_github_work_item_comment",
                json!({
                    "repo": item.repo,
                    "kind": item.kind,
                    "number": item.number,
                    "body": body,
                    "inReplyTo": reply_to,
                }),
                || {
                    github::git_github_work_item_comment_for(
                        self.project(item),
                        &item.repo,
                        &item.kind,
                        item.number,
                        &body,
                        &reply_to,
                    )
                },
            ),
        }
    }
    pub fn diff(&self, item: &Item) -> Result<Diff, String> {
        self.diff_context(item, false)
    }
    pub fn diff_context(&self, item: &Item, full_context: bool) -> Result<Diff, String> {
        match item.provider.as_str() {
            "gitlab" => convert(gitlab::gitlab_mr_diff(
                self.data_dir.clone(),
                item.repo.clone(),
                item.number,
            )?),
            "azuredevops" => convert(azure_devops::azure_devops_mr_diff(
                self.data_dir.clone(),
                item.repo.clone(),
                item.number,
            )?),
            "github" => github_call(
                self.project(item),
                "git_github_pr_diff",
                json!({"repo": item.repo, "number": item.number, "fullContext": full_context}),
                || {
                    github::git_github_pr_diff_for(
                        self.project(item),
                        &item.repo,
                        item.number,
                        full_context,
                    )
                },
            ),
            _ => Err("This issue has no pull request diff".into()),
        }
    }
    pub fn checks(&self, item: &Item) -> Result<Checks, String> {
        github_call(
            self.project(item),
            "git_github_pr_checks",
            json!({"repo": item.repo, "number": item.number}),
            || github::git_github_pr_checks_for(self.project(item), &item.repo, item.number),
        )
    }
    pub fn action(&self, item: &Item, action: &str) -> Result<Item, String> {
        let mut updated: Item = github_call(
            self.project(item),
            "git_github_pr_action",
            json!({"repo": item.repo, "number": item.number, "action": action}),
            || {
                github::git_github_pr_action_for(
                    self.project(item),
                    &item.repo,
                    item.number,
                    action,
                )
            },
        )?;
        updated.provider = item.provider.clone();
        updated.project_path = item.project_path.clone();
        Ok(updated)
    }
}
pub(super) fn run_git(root: &Path, args: &[&str]) -> Result<String, String> {
    run_command("git", root, args, true)
}
/// githubWorkItem: one GitHub issue or pull request a message links to,
/// looked up with `gh` from `cwd` (any folder, when the session's is gone).
pub fn github_work_item(cwd: &Path, repo: &str, kind: &str, number: i64) -> Result<Item, String> {
    let host = host_lookup(cwd, "git_github_work_item", repo, kind, number);
    let mut item: Item = match host {
        Some(item) => item,
        None => convert(github::git_github_work_item_for(
            &gh_root(cwd),
            repo,
            kind,
            number,
        )?)?,
    };
    item.provider = "github".into();
    Ok(item)
}
/// githubWorkItemDetails for [`github_work_item`]'s item.
pub fn github_work_item_details(
    cwd: &Path,
    repo: &str,
    kind: &str,
    number: i64,
) -> Result<Details, String> {
    let host = host_lookup(cwd, "git_github_work_item_details", repo, kind, number);
    match host {
        Some(details) => Ok(details),
        None => convert(github::git_github_work_item_details_for(
            &gh_root(cwd),
            repo,
            kind,
            number,
        )?),
    }
}
/// A link in a remote session, read by its machine's account. A host that
/// can't answer leaves it to the local `gh`, which still reads public links.
fn host_lookup<T: DeserializeOwned>(
    cwd: &Path,
    command: &str,
    repo: &str,
    kind: &str,
    number: i64,
) -> Option<T> {
    remote_workspace::is_remote(cwd)
        .then(|| {
            remote_workspace::command(
                command,
                json!({"cwd": cwd, "repo": repo, "kind": kind, "number": number}),
            )
            .ok()
        })
        .flatten()
}
fn gh_root(cwd: &Path) -> PathBuf {
    let cwd = crate::scm::expand_home(cwd);
    if cwd.is_dir() {
        cwd
    } else {
        crate::scm::shell::dirs_home()
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
    }
}
pub(super) fn run_gh(root: &Path, args: &[&str], allow_empty: bool) -> Result<String, String> {
    run_command("gh", root, args, allow_empty)
}
fn run_command(
    program: &str,
    root: &Path,
    args: &[&str],
    allow_empty: bool,
) -> Result<String, String> {
    let binary = crate::scm::resolve_gui_binary(program).ok_or_else(|| {
        format!(
            "{} is not installed. Install {} and check again.",
            if program == "gh" {
                "GitHub CLI (`gh`)"
            } else {
                program
            },
            program
        )
    })?;
    let mut command = Command::new(binary);
    command
        .current_dir(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_PAGER", "cat")
        .env("GIT_PAGER", "cat");
    crate::scm::shell::apply_gui_env(&mut command);
    crate::process::configure_child(&mut command);
    let output = command.output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() { stdout } else { stderr });
    }
    if stdout.is_empty() && !allow_empty {
        Err(format!("{program} returned no output"))
    } else {
        Ok(stdout)
    }
}

#[cfg(test)]
mod secret_tests {
    use super::*;
    use gespan_tracker_secrets::{testing, Tracker};
    use std::fs;

    fn data_dir() -> tempfile::TempDir {
        testing::use_memory_keychain();
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn linear_status_migrates_the_saved_token_file() {
        let dir = data_dir();
        fs::write(dir.path().join("linear-token"), "lin_api_saved\n").unwrap();

        assert!(linear::linear_status(dir.path().into()).unwrap().connected);
        assert!(!dir.path().join("linear-token").exists());
        assert_eq!(
            testing::stored(Tracker::Linear, dir.path()).as_deref(),
            Some("lin_api_saved\n")
        );
        assert!(linear::linear_status(dir.path().into()).unwrap().connected);
    }

    #[test]
    fn jira_status_migrates_the_saved_config_file() {
        let dir = data_dir();
        let saved =
            r#"{"site":"https://acme.atlassian.net","email":"me@acme.dev","token":"jira-secret"}"#;
        fs::write(dir.path().join("jira-config.json"), saved).unwrap();

        let status = jira::jira_status(dir.path().into()).unwrap();

        assert!(status.connected);
        assert_eq!(status.site, "https://acme.atlassian.net");
        assert_eq!(status.email, "me@acme.dev");
        assert!(!dir.path().join("jira-config.json").exists());
        assert_eq!(
            testing::stored(Tracker::Jira, dir.path()).as_deref(),
            Some(saved)
        );
    }

    #[test]
    fn gitlab_status_migrates_the_saved_config_file() {
        let dir = data_dir();
        let saved = r#"{"url":"https://gitlab.acme.dev","token":"glpat-secret"}"#;
        fs::write(dir.path().join("gitlab-config.json"), saved).unwrap();

        let status = gitlab::gitlab_status(dir.path().into()).unwrap();

        assert!(status.connected);
        assert_eq!(status.url, "https://gitlab.acme.dev");
        assert!(!dir.path().join("gitlab-config.json").exists());
        assert_eq!(
            testing::stored(Tracker::Gitlab, dir.path()).as_deref(),
            Some(saved)
        );
    }

    #[test]
    fn azure_devops_status_migrates_the_saved_config_file() {
        let dir = data_dir();
        let saved = r#"{"url":"https://dev.azure.com/acme","token":"ado-secret"}"#;
        fs::write(dir.path().join("azure-devops-config.json"), saved).unwrap();

        let status = azure_devops::azure_devops_status(dir.path().into()).unwrap();

        assert!(status.connected);
        assert_eq!(status.url, "https://dev.azure.com/acme");
        assert!(!dir.path().join("azure-devops-config.json").exists());
        assert_eq!(
            testing::stored(Tracker::AzureDevOps, dir.path()).as_deref(),
            Some(saved)
        );
    }

    #[test]
    fn unreadable_keychain_reports_never_connected_trackers_as_not_connected() {
        let dir = data_dir();
        for tracker in [
            Tracker::Linear,
            Tracker::Jira,
            Tracker::Gitlab,
            Tracker::AzureDevOps,
        ] {
            testing::fail_reads(tracker, dir.path(), true);
        }
        let service = Service {
            data_dir: dir.path().into(),
            cwd: dir.path().into(),
        };

        for provider in ["linear", "jira", "gitlab", "azuredevops"] {
            let status = service.status(provider);
            assert!(
                !status.as_ref().unwrap().connected,
                "{provider}: {status:?}"
            );
        }
    }

    #[test]
    fn disconnect_clears_the_keychain_entry() {
        let dir = data_dir();
        gespan_tracker_secrets::write(Tracker::Linear, dir.path(), "lin_api_saved").unwrap();

        let status = linear::linear_set_token(dir.path().into(), " ".into()).unwrap();

        assert!(!status.connected);
        assert_eq!(testing::stored(Tracker::Linear, dir.path()), None);
        assert!(!linear::linear_status(dir.path().into()).unwrap().connected);
    }

    #[test]
    fn disconnecting_jira_also_removes_an_unmigrated_file() {
        let dir = data_dir();
        fs::write(dir.path().join("jira-config.json"), "{}").unwrap();

        let status = jira::jira_set_config(
            dir.path().into(),
            String::new(),
            String::new(),
            String::new(),
        )
        .unwrap();

        assert!(!status.connected);
        assert!(!dir.path().join("jira-config.json").exists());
    }

    #[test]
    fn invalid_saved_config_is_reported_without_its_contents() {
        let dir = data_dir();
        fs::write(
            dir.path().join("gitlab-config.json"),
            "not json glpat-secret",
        )
        .unwrap();

        let error = gitlab::gitlab_status(dir.path().into()).err().unwrap();

        assert_eq!(error, "GitLab settings are invalid");
    }
}

#[cfg(test)]
mod notification_tests {
    use super::*;

    fn item(number: i64, provider: &str) -> Item {
        Item {
            provider: provider.into(),
            kind: "issue".into(),
            repo: "acme/app".into(),
            number,
            id: number.to_string(),
            updated_at: "2026-10-21T10:00:00Z".into(),
            ..Item::default()
        }
    }
    fn snapshot(items: Vec<Item>) -> Snapshot {
        Snapshot {
            items,
            ..Snapshot::default()
        }
    }

    #[test]
    fn first_baseline_and_existing_revisions_never_appear() {
        let mut tracker = NotificationTracker::default();
        assert!(tracker
            .observe(&snapshot(vec![item(1, "github")]), "scope".into())
            .is_empty());
        let mut updated = item(1, "github");
        updated.updated_at = "2026-10-21T11:00:00Z".into();
        let appeared = tracker.observe(&snapshot(vec![updated, item(2, "github")]), "scope".into());
        assert_eq!(
            appeared.iter().map(|i| i.number).collect::<Vec<_>>(),
            vec![2]
        );
        assert!(tracker
            .observe(&snapshot(vec![item(2, "github")]), "scope".into())
            .is_empty());
    }

    #[test]
    fn failed_provider_has_its_own_successful_baseline() {
        let mut tracker = NotificationTracker::default();
        let mut failed = snapshot(vec![item(1, "linear")]);
        failed.errors.insert("linear".into(), "offline".into());
        assert!(tracker.observe(&failed, "scope".into()).is_empty());
        assert!(tracker
            .observe(&snapshot(vec![item(2, "linear")]), "scope".into())
            .is_empty());
        let appeared = tracker.observe(
            &snapshot(vec![item(2, "linear"), item(3, "linear")]),
            "scope".into(),
        );
        assert_eq!(appeared.len(), 1);
        assert_eq!(appeared[0].number, 3);
    }

    #[test]
    fn changing_query_baselines_new_scope_without_forgetting_known_items() {
        let mut tracker = NotificationTracker::default();
        assert!(tracker
            .observe(&snapshot(vec![item(1, "github")]), "one".into())
            .is_empty());
        assert!(tracker
            .observe(&snapshot(vec![item(2, "github")]), "two".into())
            .is_empty());
        assert!(tracker
            .observe(
                &snapshot(vec![item(1, "github"), item(2, "github")]),
                "two".into()
            )
            .is_empty());
        assert_eq!(
            tracker
                .observe(&snapshot(vec![item(3, "github")]), "two".into())
                .len(),
            1
        );
    }

    fn repo_item(number: i64, repo: &str) -> Item {
        Item {
            repo: repo.into(),
            url: format!("https://github.com/{repo}/issues/{number}"),
            ..item(number, "github")
        }
    }
    /// The project folder as `collect_repo_items` stamps it on items.
    const PROJECT: &str = "/work/app";
    fn scoped(items: Vec<Item>, fetched: &[&str]) -> Snapshot {
        let project = PathBuf::from(PROJECT);
        let mut snapshot = snapshot(vec![]);
        for item in items {
            let scope = repo_scope("github", &project, &item.repo, &item.kind);
            snapshot.item_scopes.insert(item.key(), scope);
            snapshot.items.push(item);
        }
        snapshot.fetched = fetched
            .iter()
            .map(|repo| repo_scope("github", &project, repo, "issue"))
            .collect();
        snapshot
    }

    #[test]
    fn one_failing_repo_keeps_the_other_repos_items() {
        // The user's n8n fork has issues disabled; it hid every GitHub item,
        // so issue automations never saw a new issue.
        let projects = [PathBuf::from(PROJECT)];
        let fetch = collect_repo_items(
            "github",
            &projects,
            |_| Ok(vec!["acme/fork".into(), "acme/app".into()]),
            |_, repo, kind| match (repo, kind) {
                ("acme/fork", "issue") => {
                    Err("the 'acme/fork' repository has disabled issues".into())
                }
                ("acme/app", "issue") => Ok(vec![repo_item(7, "acme/app")]),
                _ => Ok(vec![]),
            },
        )
        .expect("one repo succeeding keeps the provider usable");
        assert_eq!(
            fetch.items.iter().map(|i| i.number).collect::<Vec<_>>(),
            vec![7]
        );
        assert_eq!(fetch.items[0].project_path, PROJECT);
        let fork = repo_scope("github", &projects[0], "acme/fork", "issue");
        assert!(fetch.errors[&fork].contains("disabled issues"));
        assert!(!fetch.fetched.contains(&fork));
        assert!(fetch
            .fetched
            .contains(&repo_scope("github", &projects[0], "acme/app", "issue")));
        assert_eq!(
            fetch.item_scopes[&fetch.items[0].key()],
            repo_scope("github", &projects[0], "acme/app", "issue")
        );
    }

    #[test]
    fn every_repo_failing_fails_the_provider() {
        let projects = [PathBuf::from(PROJECT)];
        let error = collect_repo_items(
            "github",
            &projects,
            |_| Ok(vec!["acme/app".into()]),
            |_, _, _| Err("gh: offline".into()),
        )
        .err();
        assert_eq!(error.as_deref(), Some("gh: offline"));
        // A project without a repository is skipped, not a failure.
        let empty = collect_repo_items(
            "github",
            &projects,
            |_| Err("not a git repository".into()),
            |_, _, _| unreachable!("no repository to fetch"),
        )
        .expect("nothing to fetch is not an error");
        assert!(empty.items.is_empty() && empty.errors.is_empty());
    }

    #[test]
    fn a_repo_that_recovers_does_not_flood_its_backlog() {
        let mut tracker = NotificationTracker::default();
        // First poll: app is fetched, tools fails before its baseline.
        assert!(tracker
            .observe(
                &scoped(vec![repo_item(1, "acme/app")], &["acme/app"]),
                "s".into()
            )
            .is_empty());
        // tools recovers with an old backlog while app gains one new issue.
        let appeared = tracker.observe(
            &scoped(
                vec![
                    repo_item(1, "acme/app"),
                    repo_item(2, "acme/app"),
                    repo_item(50, "acme/tools"),
                    repo_item(51, "acme/tools"),
                ],
                &["acme/app", "acme/tools"],
            ),
            "s".into(),
        );
        assert_eq!(
            appeared.iter().map(|i| i.number).collect::<Vec<_>>(),
            vec![2],
            "a backlog seen for the first time is a baseline, not new issues"
        );
        let appeared = tracker.observe(
            &scoped(
                vec![repo_item(50, "acme/tools"), repo_item(52, "acme/tools")],
                &["acme/app", "acme/tools"],
            ),
            "s".into(),
        );
        assert_eq!(
            appeared.iter().map(|i| i.number).collect::<Vec<_>>(),
            vec![52]
        );
    }

    #[test]
    fn a_repo_with_no_issues_yet_reports_its_first_one() {
        let mut tracker = NotificationTracker::default();
        assert!(tracker
            .observe(&scoped(vec![], &["acme/app"]), "s".into())
            .is_empty());
        let appeared = tracker.observe(
            &scoped(vec![repo_item(1, "acme/app")], &["acme/app"]),
            "s".into(),
        );
        assert_eq!(appeared.len(), 1);
    }

    #[test]
    fn seen_keys_match_desktop_identity_rules() {
        let mut github = item(12, "github");
        github.url = "https://GitHub.com/Acme/App/pull/12/".into();
        assert_eq!(github.key(), "github:acme/app:issue:12");
        let mut linear = item(7, "linear");
        linear.id = "stable-id".into();
        linear.identifier = "MON-7".into();
        assert_eq!(linear.key(), "linear:mon-7");
    }

    #[test]
    fn two_checkouts_of_one_repository_fetch_it_through_the_active_one() {
        let first = PathBuf::from("/work/app");
        let second = PathBuf::from("/work/app-review");
        let other = PathBuf::from("/work/site");
        let projects = [first.clone(), second.clone(), other.clone()];
        let repos = |project: &Path| {
            Ok(vec![if project == other {
                "acme/site".to_string()
            } else {
                "Acme/App".to_string()
            }])
        };
        for (cwd, owner) in [(&first, &first), (&second, &second), (&other, &first)] {
            let mut fetched = Vec::new();
            collect_repo_items(
                "github",
                &active_first(&projects, cwd),
                repos,
                |project, repo, _| {
                    fetched.push((project.to_path_buf(), repo.to_string()));
                    Ok(vec![])
                },
            )
            .unwrap();
            fetched.dedup();
            assert_eq!(
                fetched.len(),
                2,
                "each repository is fetched once: {fetched:?}"
            );
            assert!(
                fetched
                    .iter()
                    .any(|(project, repo)| project == owner && repo == "Acme/App"),
                "from {cwd:?}: {fetched:?}"
            );
        }
    }
}

#[cfg(test)]
mod plane_connection_tests {
    use super::*;
    use gespan_tracker_secrets::{self as secrets, testing, Tracker};

    #[test]
    fn plane_shows_as_a_connection_but_lists_no_issues_and_disconnects() {
        testing::use_memory_keychain();
        let dir = tempfile::tempdir().unwrap();
        let service = Service {
            data_dir: dir.path().to_path_buf(),
            cwd: dir.path().to_path_buf(),
        };
        assert!(!service.status("plane").unwrap().connected);
        secrets::write(
            Tracker::Plane,
            dir.path(),
            r#"{"base_url":"https://api.plane.so","workspace":"acme","api_key":"k"}"#,
        )
        .unwrap();
        let status = service.status("plane").unwrap();
        assert!(status.connected);
        assert_eq!(status.url, "https://api.plane.so");
        assert_eq!(status.workspace, "acme");

        let snapshot = service.list(&[], &ViewState::default());
        assert!(snapshot.connections["plane"].connected);
        assert!(snapshot.items.iter().all(|item| item.provider != "plane"));
        assert!(!PROVIDERS.contains(&"plane"));

        let status = service
            .connect("plane", String::new(), String::new(), "  ".into())
            .unwrap();
        assert!(!status.connected);
        assert!(!service.status("plane").unwrap().connected);
    }

    #[test]
    fn plane_connect_rejects_a_bad_workspace_before_any_request() {
        testing::use_memory_keychain();
        let dir = tempfile::tempdir().unwrap();
        let service = Service {
            data_dir: dir.path().to_path_buf(),
            cwd: dir.path().to_path_buf(),
        };
        let error = service
            .connect(
                "plane",
                "https://api.plane.so".into(),
                "bad/slug".into(),
                "k".into(),
            )
            .unwrap_err();
        assert!(error.contains("workspace"), "{error}");
    }
}

#[cfg(test)]
mod self_activity_tests {
    use super::*;

    fn pr(repo: &str, number: i64) -> Item {
        Item {
            provider: "github".into(),
            kind: "pr".into(),
            repo: repo.into(),
            number,
            project_path: "/work/app".into(),
            ..Default::default()
        }
    }

    #[test]
    fn our_change_is_consumed_once_and_only_by_its_item() {
        let mut ours = SelfActivity::default();
        ours.record(SelfTarget::of(&pr("Acme/App", 7)), 1_000);
        // Another number, repository or provider is not ours.
        assert!(!ours.consume(&pr("acme/app", 8), 2_000));
        assert!(!ours.consume(&pr("acme/other", 7), 2_000));
        let mut gitlab = pr("acme/app", 7);
        gitlab.provider = "gitlab".into();
        assert!(!ours.consume(&gitlab, 2_000));
        // The same item, its repository spelled in another case, is.
        assert!(ours.consume(&pr("acme/app", 7), 2_000));
        // The next revision is someone else's again.
        assert!(!ours.consume(&pr("acme/app", 7), 3_000));
    }

    #[test]
    fn several_changes_coalesce_into_one_revision_and_expire_after_ten_minutes() {
        let mut ours = SelfActivity::default();
        let target = SelfTarget::of(&pr("acme/app", 7));
        ours.record(target.clone(), 0);
        ours.record(target.clone(), 1_000);
        assert!(ours.consume(&pr("acme/app", 7), 2_000));
        assert!(!ours.consume(&pr("acme/app", 7), 2_000));
        ours.record(target, 0);
        assert!(!ours.consume(&pr("acme/app", 7), 10 * 60_000 + 1));
    }

    #[test]
    fn a_project_wide_change_matches_its_project_and_an_empty_target_nothing() {
        let mut ours = SelfActivity::default();
        ours.record(
            SelfTarget {
                provider: "github".into(),
                kind: Some("pr".into()),
                project_path: Some("/work/app/".into()),
                ..Default::default()
            },
            0,
        );
        let mut elsewhere = pr("acme/app", 1);
        elsewhere.project_path = "/work/other".into();
        assert!(!ours.consume(&elsewhere, 1));
        assert!(ours.consume(&pr("acme/app", 1), 1));
        ours.record(
            SelfTarget {
                provider: "github".into(),
                ..Default::default()
            },
            0,
        );
        assert!(!ours.consume(&pr("acme/app", 1), 1));
    }

    #[test]
    fn a_linear_or_jira_change_matches_by_id() {
        let mut ours = SelfActivity::default();
        let issue = Item {
            provider: "linear".into(),
            kind: "linear".into(),
            id: "abc".into(),
            ..Default::default()
        };
        ours.record(SelfTarget::of(&issue), 0);
        let mut other = issue.clone();
        other.id = "xyz".into();
        assert!(!ours.consume(&other, 1));
        assert!(ours.consume(&issue, 1));
    }
}

#[cfg(test)]
mod remote_github_tests {
    use super::*;
    use crate::files::remote::TEST_HOST;
    use std::{cell::RefCell, rc::Rc};

    const PROJECT: &str = "remote://env/home/me/web";

    type Calls = Rc<RefCell<Vec<(String, Value)>>>;

    /// Answers like the host's GitHub Inbox and records what it was asked.
    fn host(answer: impl Fn(&str, &Value) -> Result<Value, String> + 'static) -> Calls {
        let calls = Calls::default();
        let seen = calls.clone();
        TEST_HOST.set(Some(Box::new(move |environment, params| {
            assert_eq!(environment, "env");
            let command = params["command"].as_str().unwrap().to_owned();
            let args = params["args"].clone();
            seen.borrow_mut().push((command.clone(), args.clone()));
            answer(&command, &args)
        })));
        calls
    }

    fn host_item() -> Value {
        json!({
            "kind": "pr",
            "repo": "acme/web",
            "number": 42,
            "title": "Remote PR",
            "url": "https://github.com/acme/web/pull/42",
            "state": "open",
            "stateReason": "",
            "createdAt": "",
            "updatedAt": "2026-10-10",
            "draft": true,
            "labels": [{"name": "bug", "color": ""}],
            "assignees": [{"login": "remote-user", "avatarUrl": ""}],
        })
    }

    fn item() -> Item {
        Item {
            provider: "github".into(),
            kind: "pr".into(),
            repo: "acme/web".into(),
            number: 42,
            project_path: PROJECT.into(),
            ..Default::default()
        }
    }

    fn service() -> Service {
        Service {
            data_dir: std::env::temp_dir(),
            cwd: PathBuf::from("/nonexistent/local/checkout"),
        }
    }

    #[test]
    fn every_item_operation_runs_with_the_gh_of_the_items_machine() {
        let calls = host(|command, _| {
            Ok(match command {
                "git_github_work_item_details" => json!({
                    "body": "Hello", "author": "octo", "authorAvatarUrl": "",
                    "baseRefName": "main", "headRefName": "fix", "reviewDecision": "",
                }),
                "git_github_work_item_thread" => json!({
                    "comments": [{
                        "id": "c1", "kind": "review_comment", "author": "octo",
                        "authorAvatarUrl": "", "body": "Nit", "createdAt": "2026-10-10",
                        "url": "", "state": "", "path": "a.rs", "line": null,
                        "resolved": false, "threadId": "T_1", "replies": [],
                    }],
                    "commits": [],
                    "truncated": false,
                    "reviewDecision": "",
                    "baseRefName": "main",
                    "headRefName": "fix",
                }),
                "git_github_work_item_comment" => json!("https://github.com/c/1"),
                "git_github_pr_diff" => json!({
                    "files": [{"path": "a.rs", "additions": 1, "deletions": 0}],
                    "patch": "diff", "truncated": false, "additions": 1, "deletions": 0,
                }),
                "git_github_pr_checks" => json!({
                    "headOid": "abc",
                    "checks": [{
                        "name": "build", "workflow": "CI", "state": "fail",
                        "url": null, "startedAt": null, "completedAt": null,
                    }],
                }),
                "git_github_pr_action" => host_item(),
                other => panic!("unexpected command {other}"),
            })
        });
        let service = service();
        let item = item();
        assert_eq!(service.details(&item).unwrap().body, "Hello");
        assert_eq!(service.thread(&item).unwrap().comments[0].thread_id, "T_1");
        assert_eq!(
            service
                .comment(&item, "Thanks".into(), "T_1".into())
                .unwrap(),
            "https://github.com/c/1"
        );
        assert_eq!(service.diff_context(&item, true).unwrap().files.len(), 1);
        assert_eq!(service.checks(&item).unwrap().checks[0].state, "fail");
        let updated = service.action(&item, "ready").unwrap();
        TEST_HOST.set(None);
        // The item stays tied to its machine after a host round trip.
        assert_eq!(updated.provider, "github");
        assert_eq!(updated.project_path, PROJECT);
        assert!(updated.draft);

        let base = json!({"cwd": "/home/me/web", "repo": "acme/web", "number": 42});
        let with = |extra: Value| {
            let mut args = base.clone();
            args.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            args
        };
        assert_eq!(
            *calls.borrow(),
            [
                (
                    "git_github_work_item_details".to_owned(),
                    with(json!({"kind": "pr"}))
                ),
                (
                    "git_github_work_item_thread".to_owned(),
                    with(json!({"kind": "pr"}))
                ),
                (
                    "git_github_work_item_comment".to_owned(),
                    with(json!({"kind": "pr", "body": "Thanks", "inReplyTo": "T_1"}))
                ),
                (
                    "git_github_pr_diff".to_owned(),
                    with(json!({"fullContext": true}))
                ),
                ("git_github_pr_checks".to_owned(), base.clone()),
                (
                    "git_github_pr_action".to_owned(),
                    with(json!({"action": "ready"}))
                ),
            ]
        );
    }

    #[test]
    fn a_remote_project_lists_its_items_whatever_the_local_gh_account_says() {
        let calls = host(|command, args| {
            Ok(match command {
                "git_github_repositories" => json!(["acme/web"]),
                "git_github_work_items" if args["kind"] == "pr" => json!([host_item()]),
                "git_github_work_items" => json!([]),
                other => panic!("unexpected command {other}"),
            })
        });
        let dir = tempfile::tempdir().unwrap();
        let service = Service {
            data_dir: dir.path().to_path_buf(),
            cwd: PathBuf::from(PROJECT),
        };
        let snapshot = service.list(&[PathBuf::from(PROJECT)], &ViewState::default());
        TEST_HOST.set(None);
        assert_eq!(snapshot.errors.get("github"), None);
        assert!(snapshot.connections["github"].connected);
        let item = snapshot
            .items
            .iter()
            .find(|item| item.provider == "github")
            .expect("the remote pull request");
        assert_eq!(item.number, 42);
        assert_eq!(item.project_path, PROJECT);
        let calls = calls.borrow();
        assert_eq!(calls[0].1, json!({"cwd": "/home/me/web"}));
        assert_eq!(calls[1].1["repo"], "acme/web");
        assert_eq!(calls[1].1["cwd"], "/home/me/web");
        assert_eq!(calls[1].1["search"], "");
    }

    #[test]
    fn without_a_local_account_only_remote_projects_are_asked() {
        let local = PathBuf::from("/code/web");
        let remote = PathBuf::from(PROJECT);
        let projects = [local.clone(), remote.clone()];
        assert_eq!(github_reachable(true, &projects), projects);
        assert_eq!(github_reachable(false, &projects), [remote]);
        assert!(github_reachable(false, &[local]).is_empty());
    }

    #[test]
    fn a_repository_is_asked_once_per_machine_because_accounts_differ() {
        let local = PathBuf::from("/code/web");
        let projects = [
            local.clone(),
            PathBuf::from("/code/web-copy"),
            PathBuf::from(PROJECT),
            PathBuf::from("remote://env/home/me/web-copy"),
            PathBuf::from("remote://other/home/me/web"),
        ];
        let mut asked = Vec::new();
        let fetch = collect_repo_items(
            "github",
            &projects,
            |_| Ok(vec!["Acme/Web".into()]),
            |project, _, kind| {
                if kind == "pr" {
                    asked.push(project.to_path_buf());
                }
                Ok(vec![])
            },
        )
        .unwrap();
        assert_eq!(
            asked,
            [
                local,
                PathBuf::from(PROJECT),
                PathBuf::from("remote://other/home/me/web")
            ]
        );
        assert_eq!(fetch.fetched.len(), 6);
    }

    #[test]
    fn a_machine_that_cant_answer_is_reported_but_a_non_github_folder_is_not() {
        let quiet = collect_repo_items(
            "github",
            &[PathBuf::from(PROJECT)],
            |_| Err(NOT_ON_GITHUB.into()),
            |_, _, _| Ok(vec![]),
        )
        .unwrap();
        assert!(quiet.errors.is_empty());
        let error = collect_repo_items(
            "github",
            &[PathBuf::from(PROJECT)],
            |_| Err("Machine is no longer connected".into()),
            |_, _, _| Ok(vec![]),
        )
        .unwrap_err();
        assert_eq!(error, "Machine is no longer connected");
        // A local folder keeps being skipped, and one bad machine leaves the rest.
        let partial = collect_repo_items(
            "github",
            &[PathBuf::from("/code/web"), PathBuf::from(PROJECT)],
            |project| {
                if remote_workspace::is_remote(project) {
                    Err("Machine is no longer connected".into())
                } else {
                    Ok(vec!["acme/web".into()])
                }
            },
            |_, _, _| Ok(vec![]),
        )
        .unwrap();
        assert_eq!(partial.errors.len(), 1);
        assert_eq!(partial.fetched.len(), 2);
        let local = collect_repo_items(
            "github",
            &[PathBuf::from("/code/web")],
            |_| Err("no git remotes found".into()),
            |_, _, _| Ok(vec![]),
        )
        .unwrap();
        assert!(local.errors.is_empty());
    }

    #[test]
    fn an_outdated_host_is_named_on_the_item_fetch() {
        host(|_, _| Err("Unsupported workspace command".into()));
        let error = service().checks(&item()).unwrap_err();
        TEST_HOST.set(None);
        assert!(error.contains("Update Gespan Host"), "{error}");
        assert!(error.contains("GitHub Inbox"), "{error}");
    }

    #[test]
    fn a_link_in_a_remote_session_is_read_by_that_machines_account() {
        let calls = host(|command, _| {
            assert_eq!(command, "git_github_work_item");
            Ok(host_item())
        });
        let item = github_work_item(Path::new(PROJECT), "acme/web", "pr", 42).unwrap();
        TEST_HOST.set(None);
        assert_eq!(item.provider, "github");
        assert_eq!(item.title, "Remote PR");
        assert_eq!(
            calls.borrow()[0].1,
            json!({"cwd": "/home/me/web", "repo": "acme/web", "kind": "pr", "number": 42})
        );
    }
}
