//! Azure DevOps Inbox adapter. The saved credential format is the one older Tauri builds wrote.
use gespan_tracker_secrets::{self as secrets, Tracker};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::review::{self, CheckSummary, LineComment, Outcome, Review, Side, TtlCache, Verdict};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const DEFAULT_LIMIT: u32 = 40;
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
/// Per-file content cap for hunk generation; larger files stay listed.
const MAX_DIFF_FILE_BYTES: usize = 512 * 1024;
/// Hunks render for the first N files; the rest stay listed without hunks.
const MAX_DIFF_HUNK_FILES: usize = 40;
const USER_AGENT: &str = "Gespan";
const API_VERSION: &str = "7.1";
const CONNECTION_DATA_API_VERSION: &str = "7.1-preview.1";
const WIT_COMMENTS_API_VERSION: &str = "7.1-preview.4";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsStatus {
    pub connected: bool,
    pub url: String,
    pub organization: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct AzureDevOpsConfig {
    url: String,
    token: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsLabel {
    pub name: String,
    pub color: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsAssignee {
    pub login: String,
    pub avatar_url: String,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsWorkItem {
    pub kind: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub updated_at: String,
    pub labels: Vec<AzureDevOpsLabel>,
    pub assignees: Vec<AzureDevOpsAssignee>,
    pub draft: bool,
    pub repo: String,
    pub attention_reason: String,
    pub author: String,
    /// Pull requests (see [`super::Item`]): the review and check fields the
    /// Pull requests page groups by. Reviewers go by display name, as Azure DevOps
    /// shows them; diff sizes stay 0, which its lists don't carry.
    pub review_decision: String,
    pub reviews: Vec<Review>,
    pub requested_reviewers: Vec<String>,
    pub viewer: String,
    pub check_summary: CheckSummary,
    pub additions: i64,
    pub deletions: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsWorkItemDetails {
    pub body: String,
    pub author: String,
    pub author_avatar_url: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub review_decision: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsWorkItemComment {
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
    pub replies: Vec<AzureDevOpsWorkItemComment>,
    pub author_bot: bool,
    pub resolvable: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsWorkItemThread {
    pub comments: Vec<AzureDevOpsWorkItemComment>,
    pub truncated: bool,
    pub review_decision: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsMrFile {
    pub path: String,
    pub additions: i64,
    pub deletions: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsMrDiff {
    pub additions: i64,
    pub deletions: i64,
    pub files: Vec<AzureDevOpsMrFile>,
    pub patch: String,
    pub truncated: bool,
}

pub fn azure_devops_status(app: PathBuf) -> Result<AzureDevOpsStatus, String> {
    let config = read_config(&app)?;
    Ok(AzureDevOpsStatus {
        connected: config.is_some(),
        url: config
            .as_ref()
            .map(|config| config.url.clone())
            .unwrap_or_default(),
        organization: config
            .as_ref()
            .map(|config| organization_from_url(&config.url))
            .unwrap_or_default(),
    })
}

pub fn azure_devops_set_config(
    app: PathBuf,
    url: String,
    token: String,
) -> Result<AzureDevOpsStatus, String> {
    let url = normalize_azure_devops_url(&url)?;
    let token = token.trim().to_string();
    if token.is_empty() {
        delete_config(&app)?;
        let organization = organization_from_url(&url);
        return Ok(AzureDevOpsStatus {
            connected: false,
            url,
            organization,
        });
    }
    let config = AzureDevOpsConfig { url, token };
    // Validates the PAT and fails fast on 401/203 (non-visualstudio passthrough).
    let user_id = azure_current_user_id(&config)?;
    if user_id.trim().is_empty() {
        return Err("Azure DevOps did not return the current user".into());
    }
    write_config(&app, &config)?;
    Ok(AzureDevOpsStatus {
        connected: true,
        organization: organization_from_url(&config.url),
        url: config.url,
    })
}

pub fn azure_devops_repo(app: PathBuf, cwd: String) -> Result<String, String> {
    let config = require_config(&app)?;
    azure_devops_repo_for(&expand_home(&cwd), &config.url)
}

pub fn azure_devops_list_work_items(
    app: PathBuf,
    cwd: String,
    kind: String,
    assigned_to_me: bool,
    state: String,
    limit: Option<u32>,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let config = require_config(&app)?;
    let repo = azure_devops_repo_for(&expand_home(&cwd), &config.url)?;
    azure_devops_list_work_items_for(
        &config,
        &repo,
        &kind,
        assigned_to_me,
        &state,
        limit.unwrap_or(DEFAULT_LIMIT),
    )
}

pub fn azure_devops_list_todos(
    app: PathBuf,
    kind: String,
    limit: Option<u32>,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let config = require_config(&app)?;
    azure_devops_list_todos_for(&config, &kind, limit.unwrap_or(DEFAULT_LIMIT))
}

pub fn azure_devops_work_item_details(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
) -> Result<AzureDevOpsWorkItemDetails, String> {
    let config = require_config(&app)?;
    // Boards work items carry only the project name in `repo`, so per-kind
    // validation happens inside the helper instead of here.
    azure_devops_work_item_details_for(&config, &repo, &kind, number)
}

pub fn azure_devops_work_item_thread(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
) -> Result<AzureDevOpsWorkItemThread, String> {
    let config = require_config(&app)?;
    azure_devops_work_item_thread_for(&config, &repo, &kind, number)
}

/// A comment on a pull request or work item: in the pull request thread
/// `reply_to` names, or a new one when it is empty.
pub fn azure_devops_work_item_comment(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
    body: String,
    reply_to: String,
) -> Result<String, String> {
    let config = require_config(&app)?;
    let reply_to = reply_to.trim();
    if !reply_to.is_empty() && kind == "pr" {
        validate_item(&kind, number)?;
        let body = body.trim();
        if body.is_empty() {
            return Err("Comment cannot be empty".into());
        }
        let (project, repo_name) = split_repo(&repo)?;
        let (path, payload) = reply_request(&project, &repo_name, number, reply_to, body)?;
        azure_send_json(&config, "POST", &path, Some(payload))?;
        return Ok(pr_web_url(&config, &project, &repo_name, number));
    }
    azure_devops_work_item_comment_for(&config, &repo, &kind, number, &body)
}

/// Marks a pull request thread resolved ("fixed"), or active again.
pub fn azure_devops_resolve_thread(
    app: PathBuf,
    repo: String,
    number: i64,
    thread: String,
    resolved: bool,
) -> Result<(), String> {
    let config = require_config(&app)?;
    validate_item("pr", number)?;
    let (project, repo_name) = split_repo(&repo)?;
    let (path, payload) = resolve_request(&project, &repo_name, number, thread.trim(), resolved)?;
    azure_send_json(&config, "PATCH", &path, Some(payload))?;
    Ok(())
}

fn thread_number(thread: &str) -> Result<i64, String> {
    thread
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| "Invalid Azure DevOps thread".to_string())
}

/// Where a reply in a pull request thread is posted, and its body.
pub(super) fn reply_request(
    project: &str,
    repo: &str,
    number: i64,
    thread: &str,
    body: &str,
) -> Result<(String, Value), String> {
    let thread = thread_number(thread)?;
    Ok((
        format!(
            "/{}/_apis/git/repositories/{}/pullrequests/{number}/threads/{thread}/comments?api-version={}",
            encode_segment(project),
            encode_segment(repo),
            API_VERSION
        ),
        json!({ "content": body, "parentCommentId": 1, "commentType": 1 }),
    ))
}

/// Where a pull request thread's status is set, and its body.
pub(super) fn resolve_request(
    project: &str,
    repo: &str,
    number: i64,
    thread: &str,
    resolved: bool,
) -> Result<(String, Value), String> {
    let thread = thread_number(thread)?;
    Ok((
        format!(
            "/{}/_apis/git/repositories/{}/pullrequests/{number}/threads/{thread}?api-version={}",
            encode_segment(project),
            encode_segment(repo),
            API_VERSION
        ),
        json!({ "status": if resolved { "fixed" } else { "active" } }),
    ))
}

pub fn azure_devops_mr_diff(
    app: PathBuf,
    repo: String,
    number: i64,
) -> Result<AzureDevOpsMrDiff, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(&repo)?;
    azure_devops_mr_diff_for(&config, &repo, number)
}

fn azure_devops_list_work_items_for(
    config: &AzureDevOpsConfig,
    repo: &str,
    kind: &str,
    assigned_to_me: bool,
    state: &str,
    limit: u32,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    validate_kind(kind)?;
    let (project, repo_name) = split_repo(repo)?;
    let limit = limit.clamp(1, 100);
    if kind == "pr" {
        let me = if assigned_to_me {
            Some(azure_current_user_id(config)?)
        } else {
            None
        };
        return azure_list_prs_for(config, &project, &repo_name, me.as_deref(), state, limit);
    }
    azure_list_wit_for(config, Some(&project), assigned_to_me, state, limit)
}

fn azure_devops_list_todos_for(
    config: &AzureDevOpsConfig,
    kind: &str,
    limit: u32,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    validate_kind(kind)?;
    let limit = limit.clamp(1, 100);
    if kind == "pr" {
        return azure_list_pr_reviews_for(config, limit);
    }
    let mut items = azure_list_wit_for(config, None, true, "open", limit)?;
    for item in &mut items {
        if item.attention_reason.is_empty() {
            item.attention_reason = "assigned".into();
        }
    }
    Ok(items)
}

fn azure_devops_work_item_details_for(
    config: &AzureDevOpsConfig,
    repo: &str,
    kind: &str,
    number: i64,
) -> Result<AzureDevOpsWorkItemDetails, String> {
    validate_item(kind, number)?;
    if kind == "pr" {
        let (project, repo_name) = split_repo(repo)?;
        let path = format!(
            "/{}/_apis/git/repositories/{}/pullrequests/{}?api-version={}",
            encode_segment(&project),
            encode_segment(&repo_name),
            number,
            API_VERSION
        );
        let response = azure_get(config, &path)?;
        return parse_pr_details(&response.value);
    }
    let path = format!(
        "/_apis/wit/workitems/{}?$expand=relations&api-version={}",
        number, API_VERSION
    );
    let response = azure_get(config, &path)?;
    parse_wit_details(&response.value)
}

fn azure_devops_work_item_thread_for(
    config: &AzureDevOpsConfig,
    repo: &str,
    kind: &str,
    number: i64,
) -> Result<AzureDevOpsWorkItemThread, String> {
    validate_item(kind, number)?;
    if kind == "pr" {
        let (project, repo_name) = split_repo(repo)?;
        let path = format!(
            "/{}/_apis/git/repositories/{}/pullrequests/{}/threads?api-version={}",
            encode_segment(&project),
            encode_segment(&repo_name),
            number,
            API_VERSION
        );
        let response = azure_get(config, &path)?;
        let pr_url = pr_web_url(config, &project, &repo_name, number);
        return parse_pr_thread(&response.value, &pr_url, response.truncated);
    }
    let project = wit_project(repo)?;
    let path = format!(
        "/{}/_apis/wit/workitems/{}/comments?api-version={}",
        encode_segment(&project),
        number,
        WIT_COMMENTS_API_VERSION
    );
    let response = azure_get(config, &path)?;
    parse_wit_thread(&response.value, response.truncated)
}

fn azure_devops_work_item_comment_for(
    config: &AzureDevOpsConfig,
    repo: &str,
    kind: &str,
    number: i64,
    body: &str,
) -> Result<String, String> {
    validate_item(kind, number)?;
    let body = body.trim();
    if body.is_empty() {
        return Err("Comment cannot be empty".into());
    }
    if kind == "pr" {
        let (project, repo_name) = split_repo(repo)?;
        let path = format!(
            "/{}/_apis/git/repositories/{}/pullrequests/{}/threads?api-version={}",
            encode_segment(&project),
            encode_segment(&repo_name),
            number,
            API_VERSION
        );
        let payload = json!({
            "comments": [{ "parentCommentId": 0, "content": body, "commentType": 1 }],
            "status": 1,
        });
        azure_post_json(config, &path, payload)?;
        return Ok(pr_web_url(config, &project, &repo_name, number));
    }
    let project = wit_project(repo)?;
    let path = format!(
        "/{}/_apis/wit/workitems/{}/comments?api-version={}",
        encode_segment(&project),
        number,
        WIT_COMMENTS_API_VERSION
    );
    let response = azure_post_json(config, &path, json!({ "text": body }))?;
    Ok(string_field(&response.value, "url").unwrap_or_default())
}

fn azure_devops_mr_diff_for(
    config: &AzureDevOpsConfig,
    repo: &str,
    number: i64,
) -> Result<AzureDevOpsMrDiff, String> {
    validate_item("pr", number)?;
    let (project, repo_name) = split_repo(repo)?;
    let iterations_path = format!(
        "/{}/_apis/git/repositories/{}/pullrequests/{}/iterations?api-version={}",
        encode_segment(&project),
        encode_segment(&repo_name),
        number,
        API_VERSION
    );
    let iterations = azure_get(config, &iterations_path)?;
    let Some((iteration_id, source_sha, base_sha)) = latest_iteration_commits(&iterations.value)
    else {
        return Ok(AzureDevOpsMrDiff {
            additions: 0,
            deletions: 0,
            files: Vec::new(),
            patch: String::new(),
            truncated: false,
        });
    };
    let changes_path = format!(
        "/{}/_apis/git/repositories/{}/pullrequests/{}/iterations/{}/changes?$top=100&api-version={}",
        encode_segment(&project),
        encode_segment(&repo_name),
        number,
        iteration_id,
        API_VERSION
    );
    let response = azure_get(config, &changes_path)?;
    let changes = parse_iteration_change_list(&response.value);
    let mut files: Vec<AzureDevOpsMrFile> = changes
        .iter()
        .map(|change| AzureDevOpsMrFile {
            path: change.path.clone(),
            additions: 0,
            deletions: 0,
        })
        .collect();
    let mut patch = String::new();
    let mut additions = 0_i64;
    let mut deletions = 0_i64;
    let mut truncated = response.truncated;
    let mut hunked = 0_usize;
    for (index, change) in changes.iter().enumerate() {
        if hunked >= MAX_DIFF_HUNK_FILES {
            truncated = true;
            break;
        }
        match diff_iteration_file(config, &project, &repo_name, change, &source_sha, &base_sha) {
            Ok(FileDiff::Ready {
                additions: file_adds,
                deletions: file_dels,
                block,
            }) => {
                hunked += 1;
                additions += file_adds;
                deletions += file_dels;
                if let Some(file) = files.get_mut(index) {
                    file.additions = file_adds;
                    file.deletions = file_dels;
                }
                if patch.len() + block.len() > MAX_DIFF_BYTES {
                    truncated = true;
                    continue;
                }
                patch.push_str(&block);
            }
            Ok(FileDiff::Unchanged) => {
                hunked += 1;
            }
            Ok(FileDiff::Skipped { too_large }) => {
                truncated = truncated || too_large;
            }
            Err(_) => {
                // A single unreadable file must not fail the whole diff; the
                // file stays listed without hunks.
            }
        }
    }
    Ok(AzureDevOpsMrDiff {
        additions,
        deletions,
        files,
        patch,
        truncated,
    })
}

// ---- Pull requests ----

fn azure_list_prs_for(
    config: &AzureDevOpsConfig,
    project: &str,
    repo_name: &str,
    assigned_user_id: Option<&str>,
    state: &str,
    limit: u32,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let repo = format!("{project}/{repo_name}");
    let reviewer = assigned_user_id.map(str::trim).filter(|id| !id.is_empty());
    if state.trim().eq_ignore_ascii_case("all") {
        let mut active = azure_fetch_prs(
            config,
            project,
            repo_name,
            &repo,
            "active",
            limit * 2,
            reviewer,
        )?;
        let mut completed = azure_fetch_prs(
            config,
            project,
            repo_name,
            &repo,
            "completed",
            limit * 2,
            reviewer,
        )?;
        // Abandoned PRs surface through the "all" view too, like closed GitHub PRs.
        let mut abandoned = azure_fetch_prs(
            config,
            project,
            repo_name,
            &repo,
            "abandoned",
            limit * 2,
            reviewer,
        )?;
        let mut items = Vec::new();
        items.append(&mut active);
        items.append(&mut completed);
        items.append(&mut abandoned);
        items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        items.truncate(limit as usize);
        return Ok(items);
    }
    azure_fetch_prs(config, project, repo_name, &repo, "active", limit, reviewer)
}

fn azure_fetch_prs(
    config: &AzureDevOpsConfig,
    project: &str,
    repo_name: &str,
    repo: &str,
    status: &str,
    limit: u32,
    reviewer_id: Option<&str>,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let reviewer = reviewer_id
        .map(|id| format!("&searchCriteria.reviewerId={}", encode_segment(id)))
        .unwrap_or_default();
    let path = format!(
        "/{}/_apis/git/repositories/{}/pullrequests?searchCriteria.status={}{}&$top={}&api-version={}",
        encode_segment(project),
        encode_segment(repo_name),
        status,
        reviewer,
        limit.clamp(1, 100),
        API_VERSION
    );
    let response = azure_get(config, &path)?;
    let mut items = parse_pr_list(&response.value, repo, &config.url)?;
    if status == "active" {
        enrich_prs(config, project, &response.value, &mut items);
    }
    Ok(items)
}

/// Active pull requests whose policy checks are looked up per list, at most
/// this many, with this many requests at once.
const MAX_ENRICHED: usize = 30;
const ENRICH_WORKERS: usize = 6;
const POLICY_API_VERSION: &str = "7.1-preview.1";
/// Policy types whose evaluations are checks: build validation, and status
/// checks posted by other services.
const BUILD_POLICY: &str = "0609b952-1397-4640-95ec-e00a01b2c241";
const STATUS_POLICY: &str = "cbdc66da-9728-4af8-aada-9a5a32e4a226";

/// Fills the viewer and the policy checks of a list's active pull requests.
/// Each needs its own request, so a few run at once and are kept for a
/// minute per source commit; one that fails keeps its checks empty.
fn enrich_prs(
    config: &AzureDevOpsConfig,
    project: &str,
    rows: &Value,
    items: &mut [AzureDevOpsWorkItem],
) {
    let viewer = azure_viewer(config)
        .map(|(_, name)| name)
        .unwrap_or_default();
    let mut targets = std::collections::HashMap::new();
    for row in rows
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(number) = row.get("pullRequestId").and_then(Value::as_i64) else {
            continue;
        };
        let project_id = row
            .pointer("/repository/project/id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let commit = row
            .pointer("/lastMergeSourceCommit/commitId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        targets.insert(number, (project_id, commit));
    }
    for item in items.iter_mut() {
        item.viewer = viewer.clone();
    }
    let work: Vec<(&mut AzureDevOpsWorkItem, (String, String))> = items
        .iter_mut()
        .filter(|item| item.state == "open")
        .filter_map(|item| {
            let target = targets.get(&item.number).cloned()?;
            (!target.0.is_empty()).then_some((item, target))
        })
        .take(MAX_ENRICHED)
        .collect();
    review::for_each_bounded(work, ENRICH_WORKERS, |(item, (project_id, commit))| {
        static CHECKS: TtlCache<CheckSummary> =
            TtlCache::new(Duration::from_secs(60), Duration::from_secs(60));
        let key = format!("{}|{project_id}|{}|{commit}", config.url, item.number);
        if let Ok(summary) = CHECKS.get(&key, || {
            policy_evaluations(config, project, &project_id, item.number).map(|v| policy_checks(&v))
        }) {
            item.check_summary = summary;
        }
    });
}

fn policy_evaluations(
    config: &AzureDevOpsConfig,
    project: &str,
    project_id: &str,
    number: i64,
) -> Result<Value, String> {
    let artifact = format!("vstfs:///CodeReview/CodeReviewId/{project_id}/{number}");
    let path = format!(
        "/{}/_apis/policy/evaluations?artifactId={}&api-version={POLICY_API_VERSION}",
        encode_segment(project),
        encode_segment(&artifact)
    );
    Ok(azure_get(config, &path)?.value)
}

/// A build or status policy evaluation, with its name and how it counts.
fn policy_check(evaluation: &Value) -> Option<(String, Outcome, bool)> {
    let configuration = evaluation.get("configuration")?;
    if configuration.get("isEnabled").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let kind = configuration.get("type");
    let type_id = kind
        .and_then(|t| string_field(t, "id"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let type_name = kind
        .and_then(|t| string_field(t, "displayName"))
        .unwrap_or_default();
    let build = type_id == BUILD_POLICY || type_name.eq_ignore_ascii_case("Build");
    if !(build || type_id == STATUS_POLICY || type_name.eq_ignore_ascii_case("Status")) {
        return None;
    }
    let settings = configuration.get("settings");
    let name = settings
        .and_then(|s| string_field(s, "displayName"))
        .filter(|n| !n.is_empty())
        .or_else(|| {
            settings
                .and_then(|s| string_field(s, "statusName"))
                .filter(|n| !n.is_empty())
        })
        .unwrap_or(type_name);
    let outcome = match string_field(evaluation, "status")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "approved" => Outcome::Passed,
        "rejected" | "broken" => Outcome::Failed,
        "notapplicable" => Outcome::Ignored,
        _ => Outcome::Pending,
    };
    Some((name, outcome, build))
}

fn policy_checks(value: &Value) -> CheckSummary {
    CheckSummary::tally(
        value
            .get("value")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(policy_check)
            .map(|(name, outcome, _)| (name, outcome)),
    )
}

/// The failed build validations, which can be queued again.
fn failed_builds(value: &Value) -> Vec<String> {
    value
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|evaluation| {
            policy_check(evaluation)
                .is_some_and(|(_, outcome, build)| build && outcome == Outcome::Failed)
        })
        .filter_map(|evaluation| string_field(evaluation, "evaluationId"))
        .filter(|id| !id.is_empty())
        .collect()
}

/// Reviewer votes as reviews: 10 and 5 approve, -5 (waiting for author)
/// and -10 (rejected) request changes, 0 is still asked. Returns the
/// verdict, the reviews and who is still asked.
fn pr_reviews(row: &Value) -> (String, Vec<Review>, Vec<String>) {
    let mut reviews = Vec::new();
    let mut requested = Vec::new();
    let mut required_waiting = false;
    for reviewer in row
        .get("reviewers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = string_field(reviewer, "displayName").filter(|n| !n.is_empty()) else {
            continue;
        };
        let vote = reviewer.get("vote").and_then(Value::as_i64).unwrap_or(0);
        let state = match vote {
            v if v > 0 => "approved",
            v if v < 0 => "changes_requested",
            _ => {
                if reviewer.get("hasDeclined").and_then(Value::as_bool) != Some(true) {
                    required_waiting |=
                        reviewer.get("isRequired").and_then(Value::as_bool) == Some(true);
                    requested.push(name);
                }
                continue;
            }
        };
        reviews.push(Review {
            login: name,
            state: state.into(),
        });
    }
    let decision = if reviews.iter().any(|r| r.state == "changes_requested") {
        "changes_requested"
    } else if required_waiting {
        "review_required"
    } else if reviews.iter().any(|r| r.state == "approved") {
        "approved"
    } else if !requested.is_empty() {
        "review_required"
    } else {
        ""
    };
    (decision.into(), reviews, requested)
}

/// The connected account's id and display name, asked at most every few
/// minutes.
fn azure_viewer(config: &AzureDevOpsConfig) -> Result<(String, String), String> {
    static VIEWERS: TtlCache<(String, String)> =
        TtlCache::new(Duration::from_secs(300), Duration::from_secs(60));
    VIEWERS.get(&config.url, || {
        let response = azure_get(
            config,
            &format!("/_apis/connectionData?api-version={CONNECTION_DATA_API_VERSION}"),
        )?;
        parse_viewer(&response.value)
    })
}

fn parse_viewer(value: &Value) -> Result<(String, String), String> {
    let user = value
        .get("authenticatedUser")
        .or_else(|| value.get("authorizedUser"))
        .ok_or_else(|| "Azure DevOps did not return the current user".to_string())?;
    let id = string_field(user, "id").unwrap_or_default();
    if id.is_empty() {
        return Err("Azure DevOps did not return the current user".into());
    }
    let name = string_field(user, "providerDisplayName")
        .filter(|n| !n.is_empty())
        .or_else(|| string_field(user, "customDisplayName"))
        .unwrap_or_default();
    Ok((id, name))
}

/// The connected organization's name: the last part of its URL.
fn organization_name(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// `project/repository`, from that or `organization/project/repository` of
/// the connected organization.
fn added_repo(config_url: &str, repo: &str) -> Result<String, String> {
    let repo = repo.trim().trim_matches('/');
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() == 3 {
        if !parts[0].eq_ignore_ascii_case(&organization_name(config_url)) {
            return Err(format!(
                "{} is not the connected Azure DevOps organization",
                parts[0]
            ));
        }
        return validate_repo(&parts[1..].join("/"));
    }
    validate_repo(repo)
}

/// The active pull requests of a repository added by hand.
pub fn azure_devops_list_repo_prs(
    app: PathBuf,
    repo: &str,
    limit: u32,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let config = require_config(&app)?;
    let repo = added_repo(&config.url, repo)?;
    let (project, name) = split_repo(&repo)?;
    azure_list_prs_for(&config, &project, &name, None, "open", limit.clamp(1, 100))
}

/// Active pull requests across the organization where you are a reviewer
/// and have not voted: one request.
pub fn azure_devops_review_requests(app: PathBuf) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let config = require_config(&app)?;
    let (me, viewer) = azure_viewer(&config)?;
    let path = format!(
        "/_apis/git/pullrequests?searchCriteria.status=active&searchCriteria.reviewerId={}&$top=50&api-version={}",
        encode_segment(&me),
        API_VERSION
    );
    let mut items = parse_review_requests(&azure_get(&config, &path)?.value, &me, &config.url)?;
    for item in &mut items {
        item.viewer = viewer.clone();
    }
    Ok(items)
}

fn parse_review_requests(
    value: &Value,
    me: &str,
    base_url: &str,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let rows = value
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return pull requests".to_string())?;
    Ok(rows
        .iter()
        .filter(|row| {
            // Asked through a group counts; having voted does not.
            !row.get("reviewers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|r| {
                    string_field(r, "id").is_some_and(|id| id.eq_ignore_ascii_case(me))
                        && r.get("vote").and_then(Value::as_i64).unwrap_or(0) != 0
                })
        })
        .filter_map(|row| {
            let (Some(project), Some(name)) = pr_repo_parts(row) else {
                return None;
            };
            let mut item = parse_pr(row, &format!("{project}/{name}"), base_url)?;
            item.attention_reason = "review_requested".into();
            Some(item)
        })
        .collect())
}

/// The organization's repositories whose name matches `query`. Azure DevOps
/// can't search them, so the list is read once a minute and filtered here.
pub fn azure_devops_search_repos(
    app: PathBuf,
    query: &str,
) -> Result<Vec<(String, String)>, String> {
    static REPOS: TtlCache<Value> = TtlCache::new(Duration::from_secs(60), Duration::from_secs(30));
    let config = require_config(&app)?;
    let value = REPOS.get(&config.url, || {
        Ok(azure_get(
            &config,
            &format!("/_apis/git/repositories?api-version={API_VERSION}"),
        )?
        .value)
    })?;
    Ok(parse_repo_hits(&value, query))
}

fn parse_repo_hits(value: &Value, query: &str) -> Vec<(String, String)> {
    let query = query.trim().to_lowercase();
    value
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("isDisabled").and_then(Value::as_bool) != Some(true))
        .filter_map(|row| {
            let name = string_field(row, "name")?;
            let project = row.get("project").and_then(|p| string_field(p, "name"))?;
            let repo = format!("{project}/{name}");
            (validate_repo(&repo).is_ok() && repo.to_lowercase().contains(&query))
                .then(|| (repo, format!("{project} project")))
        })
        .take(20)
        .collect()
}

/// Sets your vote (10 approve, -5 waiting for author; a comment leaves it)
/// and posts `body`, if any, as a thread.
pub fn azure_devops_submit_review(
    app: PathBuf,
    repo: &str,
    number: i64,
    verdict: Verdict,
    body: &str,
    comments: &[LineComment],
) -> Result<(), String> {
    let config = require_config(&app)?;
    validate_item("pr", number)?;
    let (project, name) = split_repo(repo)?;
    let threads = format!(
        "/{}/_apis/git/repositories/{}/pullrequests/{number}/threads?api-version={API_VERSION}",
        encode_segment(&project),
        encode_segment(&name),
    );
    for comment in comments {
        azure_post_json(&config, &threads, azure_line_thread(comment))?;
    }
    let vote = match verdict {
        Verdict::Approve => Some(10),
        Verdict::RequestChanges => Some(-5),
        Verdict::Comment => None,
    };
    if let Some(vote) = vote {
        let (me, _) = azure_viewer(&config)?;
        let path = format!(
            "/{}/_apis/git/repositories/{}/pullRequests/{number}/reviewers/{}?api-version={API_VERSION}",
            encode_segment(&project),
            encode_segment(&name),
            encode_segment(&me),
        );
        azure_send_json(&config, "PUT", &path, Some(json!({ "vote": vote })))?;
    }
    if !body.trim().is_empty() {
        azure_devops_work_item_comment_for(&config, repo, "pr", number, body)?;
    } else if vote.is_none() && comments.is_empty() {
        return Err("Write a comment before sending it.".into());
    }
    Ok(())
}

/// A thread on one line of a pull request's diff: the right side for new
/// lines, the left for deleted ones.
pub(crate) fn azure_line_thread(comment: &LineComment) -> Value {
    let first = comment.range_start().unwrap_or(&comment.end);
    let (start, end) = match comment.end.side {
        Side::New => ("rightFileStart", "rightFileEnd"),
        Side::Old => ("leftFileStart", "leftFileEnd"),
    };
    // A range crossing from deleted to added lines starts at the first new
    // line: one side holds the whole thread.
    let from = if first.side == comment.end.side {
        first.line
    } else {
        comment.end.line
    };
    let path = format!("/{}", comment.path.trim_start_matches('/'));
    let mut context = json!({ "filePath": path });
    context[start] = json!({ "line": from, "offset": 1 });
    context[end] = json!({ "line": comment.end.line, "offset": 1 });
    json!({
        "comments": [{ "parentCommentId": 0, "content": comment.body.trim(), "commentType": 1 }],
        "status": 1,
        "threadContext": context,
    })
}

/// Queues the pull request's failed build validations again.
pub fn azure_devops_rerun_failed(app: PathBuf, repo: &str, number: i64) -> Result<String, String> {
    let config = require_config(&app)?;
    validate_item("pr", number)?;
    let (project, name) = split_repo(repo)?;
    let pr = azure_get(
        &config,
        &format!(
            "/{}/_apis/git/repositories/{}/pullrequests/{number}?api-version={API_VERSION}",
            encode_segment(&project),
            encode_segment(&name)
        ),
    )?
    .value;
    let project_id = pr
        .pointer("/repository/project/id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            "Azure DevOps did not say which project this pull request is in".to_string()
        })?;
    let failed = failed_builds(&policy_evaluations(&config, &project, project_id, number)?);
    if failed.is_empty() {
        return Err("No failed builds to queue again".into());
    }
    for id in &failed {
        let path = format!(
            "/{}/_apis/policy/evaluations/{}?api-version={POLICY_API_VERSION}",
            encode_segment(&project),
            encode_segment(id)
        );
        azure_send_json(&config, "PATCH", &path, None)?;
    }
    Ok(format!(
        "Rerun requested for {} failed build{}.",
        failed.len(),
        if failed.len() == 1 { "" } else { "s" }
    ))
}

fn azure_send_json(
    config: &AzureDevOpsConfig,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<AzureResponse, String> {
    let url = format!("{}{}", config.url.trim_end_matches('/'), path);
    let request = azure_agent()
        .request(method, &url)
        .set("Authorization", &basic_auth(&config.token))
        .set("Accept", "application/json")
        .set("User-Agent", USER_AGENT);
    read_azure_response(match body {
        Some(body) => request
            .set("Content-Type", "application/json")
            .send_string(&body.to_string()),
        None => request.call(),
    })
}

fn azure_list_pr_reviews_for(
    config: &AzureDevOpsConfig,
    limit: u32,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let user_id = azure_current_user_id(config)?;
    let path = format!(
        "/_apis/git/pullrequests?searchCriteria.status=active&searchCriteria.reviewerId={}&$top={}&api-version={}",
        encode_segment(&user_id),
        limit.clamp(1, 100),
        API_VERSION
    );
    let response = azure_get(config, &path)?;
    let rows = response
        .value
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return pull requests".to_string())?;
    let mut items = Vec::new();
    for row in rows {
        let (project, repo_name) = pr_repo_parts(row);
        let repo = match (project, repo_name) {
            (Some(project), Some(repo_name)) => format!("{project}/{repo_name}"),
            _ => continue,
        };
        if let Some(mut item) = parse_pr(row, &repo, &config.url) {
            if item.attention_reason.is_empty() {
                item.attention_reason = "review_requested".into();
            }
            items.push(item);
        }
    }
    Ok(items)
}

// ---- Work items (Boards) ----

fn azure_list_wit_for(
    config: &AzureDevOpsConfig,
    project: Option<&str>,
    assigned_to_me: bool,
    state: &str,
    limit: u32,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let limit = limit.clamp(1, 100);
    let wiql = wit_wiql(project, assigned_to_me, state);
    let ids = azure_query_wit_ids(config, &wiql, limit)?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    azure_fetch_wit_batch(config, &ids)
}

fn wit_wiql(project: Option<&str>, assigned_to_me: bool, state: &str) -> String {
    let mut query = String::from("SELECT [System.Id] FROM WorkItems WHERE ");
    let mut clauses: Vec<String> = Vec::new();
    if let Some(project) = project.map(str::trim).filter(|p| !p.is_empty()) {
        clauses.push(format!(
            "[System.TeamProject] = '{}'",
            project.replace('\'', "''")
        ));
    }
    if assigned_to_me {
        clauses.push("[System.AssignedTo] = @me".into());
    }
    if !state.trim().eq_ignore_ascii_case("all") {
        clauses.push("[System.State] NOT IN ('Closed', 'Done', 'Removed')".into());
    }
    if clauses.is_empty() {
        clauses.push("[System.Id] > 0".into());
    }
    query.push_str(&clauses.join(" AND "));
    query.push_str(" ORDER BY [System.ChangedDate] DESC");
    query
}

fn azure_query_wit_ids(
    config: &AzureDevOpsConfig,
    wiql: &str,
    limit: u32,
) -> Result<Vec<i64>, String> {
    let path = format!("/_apis/wit/wiql?$top={limit}&api-version={API_VERSION}");
    let response = azure_post_json(config, &path, json!({ "query": wiql }))?;
    let relations = response
        .value
        .get("workItems")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return work items".to_string())?;
    Ok(relations
        .iter()
        .filter_map(|row| row.get("id").and_then(Value::as_i64))
        .filter(|id| *id > 0)
        .take(limit as usize)
        .collect())
}

fn azure_fetch_wit_batch(
    config: &AzureDevOpsConfig,
    ids: &[i64],
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let joined = ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let fields = [
        "System.Id",
        "System.Title",
        "System.State",
        "System.ChangedDate",
        "System.CreatedDate",
        "System.AssignedTo",
        "System.Tags",
        "System.TeamProject",
    ]
    .join(",");
    let path = format!(
        "/_apis/wit/workitems?ids={joined}&fields={fields}&$expand=relations&api-version={API_VERSION}"
    );
    let response = azure_get(config, &path)?;
    let rows = response
        .value
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return work items".to_string())?;
    let mut items: Vec<AzureDevOpsWorkItem> = rows.iter().filter_map(parse_wit).collect();
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(items)
}

// ---- Parsing ----

fn parse_pr_list(
    value: &Value,
    repo: &str,
    base_url: &str,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let rows = value
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return pull requests".to_string())?;
    Ok(rows
        .iter()
        .filter_map(|row| parse_pr(row, repo, base_url))
        .collect())
}

fn parse_pr(row: &Value, repo: &str, base_url: &str) -> Option<AzureDevOpsWorkItem> {
    let number = row.get("pullRequestId").and_then(Value::as_i64)?;
    if number <= 0 {
        return None;
    }
    let status = string_field(row, "status").unwrap_or_default();
    let (review_decision, reviews, requested_reviewers) = pr_reviews(row);
    Some(AzureDevOpsWorkItem {
        kind: "pr".into(),
        number,
        title: string_field(row, "title").unwrap_or_default(),
        // List responses often omit `_links.web` and even `remoteUrl`, so the
        // last resort builds the standard web URL from request-side data.
        url: pr_link(row)
            .or_else(|| pr_url_from_repository(row, number))
            .or_else(|| pr_url_from_parts(repo, base_url, number))
            .unwrap_or_default(),
        state: normalize_pr_state(&status),
        updated_at: string_field(row, "closedDate")
            .or_else(|| string_field(row, "creationDate"))
            .unwrap_or_default(),
        labels: parse_pr_labels(row),
        assignees: parse_pr_assignees(row),
        draft: row.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        repo: repo.into(),
        attention_reason: String::new(),
        author: row
            .get("createdBy")
            .and_then(|user| string_field(user, "displayName"))
            .unwrap_or_default(),
        review_decision,
        reviews,
        requested_reviewers,
        ..Default::default()
    })
}

fn parse_pr_details(value: &Value) -> Result<AzureDevOpsWorkItemDetails, String> {
    if !value.is_object() {
        return Err("Azure DevOps did not return that pull request".into());
    }
    let creator = value.get("createdBy");
    Ok(AzureDevOpsWorkItemDetails {
        body: string_field(value, "description").unwrap_or_default(),
        author: creator
            .and_then(|user| string_field(user, "displayName"))
            .unwrap_or_default(),
        author_avatar_url: creator
            .and_then(|user| string_field(user, "imageUrl"))
            .or_else(|| {
                creator.and_then(|user| {
                    user.get("_links")
                        .and_then(|links| links.get("avatar"))
                        .and_then(|avatar| string_field(avatar, "href"))
                })
            })
            .unwrap_or_default(),
        base_ref_name: short_ref(&string_field(value, "targetRefName").unwrap_or_default()),
        head_ref_name: short_ref(&string_field(value, "sourceRefName").unwrap_or_default()),
        review_decision: String::new(),
    })
}

/// A pull request's threads, oldest first: each one's first comment with
/// the rest as its replies, so a reply goes back into it. Deleted comments,
/// empty ones and notes Azure DevOps writes itself are left out.
fn parse_pr_thread(
    value: &Value,
    pr_url: &str,
    truncated: bool,
) -> Result<AzureDevOpsWorkItemThread, String> {
    let rows = value
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return comments".to_string())?;
    let mut comments: Vec<AzureDevOpsWorkItemComment> = Vec::new();
    for thread in rows {
        if thread
            .get("isDeleted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let thread_id = thread
            .get("id")
            .and_then(Value::as_i64)
            .map(|id| id.to_string())
            .unwrap_or_default();
        let status = string_field(thread, "status").unwrap_or_default();
        let resolved = matches!(
            status.to_ascii_lowercase().as_str(),
            "fixed" | "closed" | "resolved" | "wontfix" | "bydesign"
        );
        let context = thread.get("threadContext").filter(|c| !c.is_null());
        let path = context
            .and_then(|c| string_field(c, "filePath"))
            .map(|p| p.trim_start_matches('/').to_string())
            .unwrap_or_default();
        let line = context.and_then(|c| {
            ["rightFileStart", "leftFileStart"]
                .iter()
                .find_map(|side| c.get(*side)?.get("line")?.as_i64())
        });
        let Some(thread_comments) = thread.get("comments").and_then(Value::as_array) else {
            continue;
        };
        let mapped = thread_comments.iter().filter_map(|comment| {
            if comment
                .get("isDeleted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || string_field(comment, "commentType").as_deref() == Some("system")
            {
                return None;
            }
            let body = string_field(comment, "content").unwrap_or_default();
            if body.is_empty() {
                return None;
            }
            let author = comment.get("author");
            let comment_id = comment
                .get("id")
                .and_then(Value::as_i64)
                .map(|id| id.to_string())
                .unwrap_or_default();
            Some(AzureDevOpsWorkItemComment {
                id: if thread_id.is_empty() {
                    comment_id.clone()
                } else {
                    format!("{thread_id}-{comment_id}")
                },
                kind: if path.is_empty() {
                    "comment".into()
                } else {
                    "review_comment".into()
                },
                author: author
                    .and_then(|user| string_field(user, "displayName"))
                    .unwrap_or_default(),
                author_avatar_url: author
                    .and_then(|user| string_field(user, "imageUrl"))
                    .unwrap_or_default(),
                body,
                created_at: string_field(comment, "publishedDate")
                    .or_else(|| string_field(comment, "lastUpdatedDate"))
                    .unwrap_or_default(),
                url: pr_url.into(),
                state: String::new(),
                path: path.clone(),
                line,
                resolved,
                thread_id: thread_id.clone(),
                replies: Vec::new(),
                author_bot: false,
                resolvable: !thread_id.is_empty(),
            })
        });
        // Azure DevOps doesn't promise its order: the thread starts with
        // its oldest comment.
        let mut mapped: Vec<AzureDevOpsWorkItemComment> = mapped.collect();
        mapped.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        let mut mapped = mapped.into_iter();
        let Some(mut first) = mapped.next() else {
            continue;
        };
        first.replies = mapped.collect();
        comments.push(first);
    }
    comments.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(AzureDevOpsWorkItemThread {
        comments,
        truncated,
        review_decision: String::new(),
        base_ref_name: String::new(),
        head_ref_name: String::new(),
    })
}

fn parse_wit(row: &Value) -> Option<AzureDevOpsWorkItem> {
    let number = row.get("id").and_then(Value::as_i64)?;
    if number <= 0 {
        return None;
    }
    let fields = row.get("fields")?;
    let project = string_field(fields, "System.TeamProject").unwrap_or_default();
    Some(AzureDevOpsWorkItem {
        kind: "issue".into(),
        number,
        title: string_field(fields, "System.Title").unwrap_or_default(),
        url: row
            .get("_links")
            .and_then(|links| links.get("html"))
            .and_then(|html| string_field(html, "href"))
            .unwrap_or_default(),
        state: normalize_wit_state(&string_field(fields, "System.State").unwrap_or_default()),
        updated_at: string_field(fields, "System.ChangedDate")
            .or_else(|| string_field(fields, "System.CreatedDate"))
            .unwrap_or_default(),
        labels: parse_wit_labels(fields),
        assignees: parse_wit_assignees(fields),
        draft: false,
        repo: project,
        attention_reason: String::new(),
        ..Default::default()
    })
}

fn parse_wit_details(value: &Value) -> Result<AzureDevOpsWorkItemDetails, String> {
    if !value.is_object() {
        return Err("Azure DevOps did not return that work item".into());
    }
    let fields = value
        .get("fields")
        .ok_or_else(|| "Azure DevOps did not return that work item".to_string())?;
    let creator = fields.get("System.CreatedBy");
    Ok(AzureDevOpsWorkItemDetails {
        body: string_field(fields, "System.Description").unwrap_or_default(),
        author: creator
            .and_then(|user| string_field(user, "displayName"))
            .unwrap_or_default(),
        author_avatar_url: creator
            .and_then(|user| string_field(user, "imageUrl"))
            .unwrap_or_default(),
        base_ref_name: String::new(),
        head_ref_name: String::new(),
        review_decision: String::new(),
    })
}

fn parse_wit_thread(value: &Value, truncated: bool) -> Result<AzureDevOpsWorkItemThread, String> {
    let rows = value
        .get("comments")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return comments".to_string())?;
    let mut comments: Vec<AzureDevOpsWorkItemComment> = rows
        .iter()
        .filter_map(|row| {
            let id = row
                .get("commentId")
                .or_else(|| row.get("id"))
                .and_then(Value::as_i64)?;
            let body = string_field(row, "text").unwrap_or_default();
            if body.is_empty() {
                return None;
            }
            let author = row.get("createdBy");
            Some(AzureDevOpsWorkItemComment {
                id: id.to_string(),
                kind: "comment".into(),
                author: author
                    .and_then(|user| string_field(user, "displayName"))
                    .unwrap_or_default(),
                author_avatar_url: author
                    .and_then(|user| string_field(user, "imageUrl"))
                    .unwrap_or_default(),
                body,
                created_at: string_field(row, "createdDate").unwrap_or_default(),
                url: string_field(row, "url").unwrap_or_default(),
                state: String::new(),
                path: String::new(),
                line: None,
                resolved: false,
                thread_id: String::new(),
                replies: Vec::new(),
                author_bot: false,
                resolvable: false,
            })
        })
        .collect();
    comments.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let total = value
        .get("totalCount")
        .and_then(Value::as_i64)
        .unwrap_or(comments.len() as i64);
    Ok(AzureDevOpsWorkItemThread {
        truncated: truncated || total > comments.len() as i64,
        comments,
        review_decision: String::new(),
        base_ref_name: String::new(),
        head_ref_name: String::new(),
    })
}

struct IterationChange {
    path: String,
    change_type: String,
}

fn parse_iteration_change_list(value: &Value) -> Vec<IterationChange> {
    let entries = value
        .get("changeEntries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut changes = Vec::new();
    for entry in &entries {
        let item = entry.get("item");
        let path = item
            .and_then(|item| string_field(item, "path"))
            .unwrap_or_default();
        let is_folder = item
            .and_then(|item| item.get("isFolder"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if path.is_empty() || is_folder || path == "/" {
            continue;
        }
        let clean = path.trim_start_matches('/').to_string();
        if clean.is_empty() {
            continue;
        }
        changes.push(IterationChange {
            path: clean,
            change_type: string_field(entry, "changeType").unwrap_or_default(),
        });
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    changes.dedup_by(|a, b| a.path == b.path);
    changes
}

/// Outcome of rendering one changed file. `Skipped` keeps the file listed
/// without hunks; only `too_large` marks the whole diff truncated.
enum FileDiff {
    Ready {
        additions: i64,
        deletions: i64,
        block: String,
    },
    Unchanged,
    Skipped {
        too_large: bool,
    },
}

fn diff_iteration_file(
    config: &AzureDevOpsConfig,
    project: &str,
    repo_name: &str,
    change: &IterationChange,
    source_sha: &str,
    base_sha: &str,
) -> Result<FileDiff, String> {
    let change_type = change.change_type.to_ascii_lowercase();
    let need_base = change_type != "add";
    let need_target = change_type != "delete";
    if need_base && base_sha.trim().is_empty() {
        return Ok(FileDiff::Skipped { too_large: false });
    }
    if need_target && source_sha.trim().is_empty() {
        return Ok(FileDiff::Skipped { too_large: false });
    }
    let base = if need_base {
        azure_file_content(config, project, repo_name, &change.path, base_sha)?
    } else {
        None
    };
    let target = if need_target {
        azure_file_content(config, project, repo_name, &change.path, source_sha)?
    } else {
        None
    };
    if let (Some(FileContent::TooLarge), _) | (_, Some(FileContent::TooLarge)) = (&base, &target) {
        return Ok(FileDiff::Skipped { too_large: true });
    }
    if let (Some(FileContent::Binary), _) | (_, Some(FileContent::Binary)) = (&base, &target) {
        return Ok(FileDiff::Ready {
            additions: 0,
            deletions: 0,
            block: format!(
                "diff --git a/{0} b/{0}\nBinary files a/{0} and b/{0} differ\n",
                change.path
            ),
        });
    }
    let base_bytes = base.and_then(|content| content.into_text());
    let target_bytes = target.and_then(|content| content.into_text());
    if base_bytes.is_none() && target_bytes.is_none() {
        return Ok(FileDiff::Skipped { too_large: false });
    }
    if base_bytes == target_bytes {
        return Ok(FileDiff::Unchanged);
    }
    // Sides are already capped at MAX_DIFF_FILE_BYTES by azure_file_content.
    let is_add = base_bytes.is_none();
    let is_delete = target_bytes.is_none();
    let Some((additions, deletions, hunks)) =
        git_unified_hunks(base_bytes.as_deref(), target_bytes.as_deref())
    else {
        return Ok(FileDiff::Skipped { too_large: false });
    };
    let mut block = format!("diff --git a/{0} b/{0}\n", change.path);
    if is_add {
        block.push_str("new file mode 100644\n");
    }
    if is_delete {
        block.push_str("deleted file mode 100644\n");
    }
    if is_add {
        block.push_str(&format!("--- /dev/null\n+++ b/{}\n", change.path));
    } else if is_delete {
        block.push_str(&format!("--- a/{}\n+++ /dev/null\n", change.path));
    } else {
        block.push_str(&format!("--- a/{0}\n+++ b/{0}\n", change.path));
    }
    block.push_str(&hunks);
    Ok(FileDiff::Ready {
        additions,
        deletions,
        block,
    })
}

enum FileContent {
    Text(Vec<u8>),
    Binary,
    TooLarge,
}

impl FileContent {
    fn into_text(self) -> Option<Vec<u8>> {
        match self {
            FileContent::Text(bytes) => Some(bytes),
            FileContent::Binary | FileContent::TooLarge => None,
        }
    }
}

fn is_binary_bytes(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|byte| *byte == 0)
}

/// Fetch one file version via the Items API. `Ok(None)` means the path does
/// not exist at that commit (the other side of an add/delete).
fn azure_file_content(
    config: &AzureDevOpsConfig,
    project: &str,
    repo_name: &str,
    path: &str,
    sha: &str,
) -> Result<Option<FileContent>, String> {
    let encoded = path
        .split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/");
    let url_path = format!(
        "/{}/_apis/git/repositories/{}/items?path=/{}&versionDescriptor.version={}&versionDescriptor.versionType=commit&includeContent=true&api-version={}",
        encode_segment(project),
        encode_segment(repo_name),
        encoded,
        encode_segment(sha.trim()),
        API_VERSION
    );
    let (bytes, _) = match azure_get_bytes(config, &url_path) {
        Ok(content) => content,
        Err(error) if error == "Azure DevOps did not find that item" => return Ok(None),
        Err(error) => return Err(error),
    };
    if bytes.len() > MAX_DIFF_FILE_BYTES {
        // Oversized bodies stay listed without hunks; the caller marks the
        // diff truncated.
        return Ok(Some(FileContent::TooLarge));
    }
    Ok(Some(sniff_content(&bytes)))
}

fn sniff_content(bytes: &[u8]) -> FileContent {
    let trimmed_start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map(|start| &bytes[start..])
        .unwrap_or(&[]);
    if trimmed_start.starts_with(b"{") {
        if let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(bytes) {
            if map
                .get("isTruncated")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return FileContent::TooLarge;
            }
            if let Some(content) = map.get("content").and_then(Value::as_str) {
                let text = content.as_bytes();
                if is_binary_bytes(text) {
                    return FileContent::Binary;
                }
                return FileContent::Text(text.to_vec());
            }
        }
    }
    if is_binary_bytes(bytes) {
        return FileContent::Binary;
    }
    FileContent::Text(bytes.to_vec())
}

/// Creates a temporary directory with an unpredictable name and, on Unix,
/// owner-only permissions, so other local users on a shared machine cannot
/// guess or read the diff scratch files.
fn create_secure_tmp_dir() -> Option<PathBuf> {
    let dir =
        std::env::temp_dir().join(format!("gespan-ado-diff-{}", crate::accounts::request_id()));
    if fs::create_dir(&dir).is_err() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).is_err() {
            let _ = fs::remove_dir_all(&dir);
            return None;
        }
    }
    Some(dir)
}

/// Unified hunks between two file versions via the local `git` binary (the
/// provider already requires git for remote detection). Returns `None` when
/// the sides are identical or git is unavailable/fails, so callers degrade to
/// a hunk-less file entry instead of failing the whole diff.
fn git_unified_hunks(old: Option<&[u8]>, new: Option<&[u8]>) -> Option<(i64, i64, String)> {
    let dir = create_secure_tmp_dir()?;
    let cleanup = || {
        let _ = fs::remove_dir_all(&dir);
    };
    let old_path = dir.join("old");
    let new_path = dir.join("new");
    if let Some(bytes) = old {
        if fs::write(&old_path, bytes).is_err() {
            cleanup();
            return None;
        }
    }
    if let Some(bytes) = new {
        if fs::write(&new_path, bytes).is_err() {
            cleanup();
            return None;
        }
    }
    // Isolate the subprocess from user configuration: a global
    // `diff.external`, textconv driver, or attributes file could otherwise
    // replace the unified output this parser expects.
    let null_device = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let null_file: &Path = Path::new(null_device);
    let mut cmd = Command::new("git");
    crate::process::configure_child(&mut cmd);
    crate::scm::shell::apply_gui_env(&mut cmd);
    let output = cmd
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.safecrlf=false",
            "-c",
            "core.quotepath=false",
        ])
        .arg("-c")
        .arg(format!("core.attributesFile={null_device}"))
        .args([
            "diff",
            "--no-index",
            "--no-ext-diff",
            "--no-textconv",
            "--unified=3",
            "--no-color",
            "--",
        ])
        .arg(if old.is_some() {
            old_path.as_path()
        } else {
            null_file
        })
        .arg(if new.is_some() {
            new_path.as_path()
        } else {
            null_file
        })
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null_device)
        .current_dir(&dir)
        .output();
    cleanup();
    let output = output.ok()?;
    // Exit 0 means identical; 1 means differences; anything else is a failure.
    if output
        .status
        .code()
        .is_none_or(|code| code != 0 && code != 1)
    {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (additions, deletions, hunks, binary) = parse_git_diff_hunks(&stdout);
    if binary || !hunks.is_empty() {
        return Some((additions, deletions, hunks));
    }
    // Exit 1 without hunks only happens for mode-only changes; surface the
    // file without hunks rather than dropping it.
    if output.status.code() == Some(1) {
        return Some((0, 0, String::new()));
    }
    None
}

/// Split `git diff` output into counts plus the `@@` hunks (or the binary
/// marker). Header lines are regenerated by the caller with repo paths.
fn parse_git_diff_hunks(output: &str) -> (i64, i64, String, bool) {
    let mut additions = 0_i64;
    let mut deletions = 0_i64;
    let mut hunks = String::new();
    let mut in_hunks = false;
    for line in output.lines() {
        if line.starts_with("Binary files ") {
            return (0, 0, String::new(), true);
        }
        if line.starts_with("@@") {
            in_hunks = true;
        }
        if !in_hunks {
            continue;
        }
        hunks.push_str(line);
        hunks.push('\n');
        if line.starts_with('+') && !line.starts_with("+++") {
            additions += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
    }
    (additions, deletions, hunks, false)
}
fn parse_pr_labels(row: &Value) -> Vec<AzureDevOpsLabel> {
    row.get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|label| {
                    let name = string_field(label, "name")?;
                    if name.is_empty() {
                        return None;
                    }
                    Some(AzureDevOpsLabel {
                        name,
                        color: String::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_pr_assignees(row: &Value) -> Vec<AzureDevOpsAssignee> {
    let mut people: Vec<AzureDevOpsAssignee> = row
        .get("reviewers")
        .and_then(Value::as_array)
        .map(|reviewers| {
            reviewers
                .iter()
                .filter_map(|reviewer| {
                    let login = string_field(reviewer, "displayName")?;
                    if login.is_empty() {
                        return None;
                    }
                    Some(AzureDevOpsAssignee {
                        login,
                        avatar_url: string_field(reviewer, "imageUrl").unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(creator) = row.get("createdBy") {
        if let Some(login) = string_field(creator, "displayName").filter(|name| !name.is_empty()) {
            if !people.iter().any(|person| person.login == login) {
                people.push(AzureDevOpsAssignee {
                    login,
                    avatar_url: string_field(creator, "imageUrl").unwrap_or_default(),
                });
            }
        }
    }
    people
}

fn parse_wit_labels(fields: &Value) -> Vec<AzureDevOpsLabel> {
    string_field(fields, "System.Tags")
        .map(|tags| {
            tags.split(';')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(|tag| AzureDevOpsLabel {
                    name: tag.to_string(),
                    color: String::new(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_wit_assignees(fields: &Value) -> Vec<AzureDevOpsAssignee> {
    let assigned = fields.get("System.AssignedTo");
    assigned
        .and_then(|user| {
            let login = string_field(user, "displayName")?;
            if login.is_empty() {
                return None;
            }
            Some(vec![AzureDevOpsAssignee {
                login,
                avatar_url: string_field(user, "imageUrl").unwrap_or_default(),
            }])
        })
        .unwrap_or_default()
}

fn normalize_pr_state(state: &str) -> String {
    match state.trim().to_ascii_lowercase().as_str() {
        "active" => "open".into(),
        "completed" => "merged".into(),
        "abandoned" => "closed".into(),
        other => other.into(),
    }
}

fn normalize_wit_state(state: &str) -> String {
    match state.trim().to_ascii_lowercase().as_str() {
        "new" | "active" | "to do" | "doing" | "in progress" | "resolved" | "proposed"
        | "approved" | "committed" => "open".into(),
        "closed" | "done" | "removed" | "abandoned" | "cut" => "closed".into(),
        other => other.into(),
    }
}

fn pr_link(row: &Value) -> Option<String> {
    row.get("_links")
        .and_then(|links| links.get("web"))
        .and_then(|web| string_field(web, "href"))
        .filter(|href| !href.is_empty())
}

fn pr_url_from_repository(row: &Value, number: i64) -> Option<String> {
    let remote = row
        .get("repository")
        .and_then(|repo| string_field(repo, "remoteUrl"))
        .filter(|url| !url.is_empty())?;
    Some(format!(
        "{}/pullrequest/{number}",
        remote.trim_end_matches('/')
    ))
}

fn pr_url_from_parts(repo: &str, base_url: &str, number: i64) -> Option<String> {
    let (project, repo_name) = split_repo(repo).ok()?;
    let base = base_url.trim_end_matches('/').trim();
    if base.is_empty() {
        return None;
    }
    Some(format!(
        "{base}/{}/_git/{}/pullrequest/{number}",
        project.trim(),
        repo_name.trim()
    ))
}

fn pr_repo_parts(row: &Value) -> (Option<String>, Option<String>) {
    let repository = row.get("repository");
    let project = repository
        .and_then(|repo| repo.get("project"))
        .and_then(|project| string_field(project, "name"));
    let repo_name = repository.and_then(|repo| string_field(repo, "name"));
    (project, repo_name)
}

fn pr_web_url(config: &AzureDevOpsConfig, project: &str, repo: &str, number: i64) -> String {
    format!(
        "{}/{}/_git/{}/pullrequest/{}",
        config.url.trim_end_matches('/'),
        project.trim(),
        repo.trim(),
        number
    )
}

fn short_ref(value: &str) -> String {
    value
        .trim()
        .strip_prefix("refs/heads/")
        .unwrap_or(value.trim())
        .to_string()
}

fn latest_iteration_commits(value: &Value) -> Option<(i64, String, String)> {
    let rows = value.get("value").and_then(Value::as_array)?;
    let row = rows
        .iter()
        .filter(|row| row.get("id").and_then(Value::as_i64).unwrap_or(0) > 0)
        .max_by_key(|row| row.get("id").and_then(Value::as_i64).unwrap_or(0))?;
    let id = row.get("id").and_then(Value::as_i64)?;
    let commit_id = |key: &str| {
        row.get(key)
            .and_then(|commit| string_field(commit, "commitId"))
            .unwrap_or_default()
    };
    let source = commit_id("sourceRefCommit");
    if source.trim().is_empty() {
        return None;
    }
    let base = commit_id("commonRefCommit");
    let base = if base.trim().is_empty() {
        commit_id("targetRefCommit")
    } else {
        base
    };
    if base.trim().is_empty() {
        return None;
    }
    Some((id, source, base))
}

// ---- Validation ----

fn validate_kind(kind: &str) -> Result<(), String> {
    if kind == "issue" || kind == "pr" {
        Ok(())
    } else {
        Err("Unknown Azure DevOps task kind".into())
    }
}

fn validate_item(kind: &str, number: i64) -> Result<(), String> {
    validate_kind(kind)?;
    if number <= 0 {
        return Err("Invalid Azure DevOps item number".into());
    }
    Ok(())
}

/// Boards work items carry only the project name in `repo` (e.g. `"platform"`
/// instead of `"platform/web"`), so comment paths take the leading segment.
fn wit_project(repo: &str) -> Result<String, String> {
    let project = repo.split('/').next().unwrap_or("").trim().to_string();
    if project.is_empty() || project == "." || project == ".." {
        return Err("Invalid Azure DevOps project".into());
    }
    Ok(project)
}

fn split_repo(repo: &str) -> Result<(String, String), String> {
    let repo = validate_repo(repo)?;
    let (project, name) = repo
        .split_once('/')
        .ok_or_else(|| "Invalid Azure DevOps repository".to_string())?;
    Ok((project.to_string(), name.to_string()))
}

fn validate_repo(repo: &str) -> Result<String, String> {
    let repo = repo.trim();
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.trim().is_empty()
                || *part == "."
                || *part == ".."
                || part.contains(['?', '#', '\\'])
                || part.chars().any(char::is_control)
        })
    {
        return Err("Invalid Azure DevOps repository".into());
    }
    Ok(repo.to_string())
}

// ---- HTTP ----

struct AzureResponse {
    value: Value,
    truncated: bool,
}

fn basic_auth(token: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!(":{token}"));
    format!("Basic {encoded}")
}

fn azure_get(config: &AzureDevOpsConfig, path: &str) -> Result<AzureResponse, String> {
    let url = format!("{}{}", config.url.trim_end_matches('/'), path);
    let agent = azure_agent();
    read_azure_response(
        agent
            .get(&url)
            .set("Authorization", &basic_auth(&config.token))
            .set("Accept", "application/json")
            .set("User-Agent", USER_AGENT)
            .call(),
    )
}

fn azure_post_json(
    config: &AzureDevOpsConfig,
    path: &str,
    body: Value,
) -> Result<AzureResponse, String> {
    let url = format!("{}{}", config.url.trim_end_matches('/'), path);
    let payload = serde_json::to_string(&body).map_err(|error| error.to_string())?;
    let agent = azure_agent();
    read_azure_response(
        agent
            .post(&url)
            .set("Authorization", &basic_auth(&config.token))
            .set("Accept", "application/json")
            .set("Content-Type", "application/json")
            .set("User-Agent", USER_AGENT)
            .send_string(&payload),
    )
}

fn azure_current_user_id(config: &AzureDevOpsConfig) -> Result<String, String> {
    // connectionData is still under preview and rejects stable api-versions.
    let response = azure_get(
        config,
        &format!("/_apis/connectionData?api-version={CONNECTION_DATA_API_VERSION}"),
    )?;
    let id = response
        .value
        .get("authenticatedUser")
        .and_then(|user| string_field(user, "id"))
        .or_else(|| {
            response
                .value
                .get("authorizedUser")
                .and_then(|user| string_field(user, "id"))
        })
        .unwrap_or_default();
    if id.trim().is_empty() {
        return Err("Azure DevOps did not return the current user".into());
    }
    Ok(id)
}

fn azure_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(HTTP_TIMEOUT)
        .redirects(0)
        .build()
}

fn read_azure_response(
    result: Result<ureq::Response, ureq::Error>,
) -> Result<AzureResponse, String> {
    let (bytes, status, truncated) = read_azure_bytes(result, None)?;
    let body = String::from_utf8(bytes)
        .map_err(|_| "Azure DevOps returned an unreadable response".to_string())?;
    if !(200..300).contains(&status) {
        return Err(azure_http_error(status, &body));
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|_| "Azure DevOps returned invalid JSON".to_string())?;
    Ok(AzureResponse { value, truncated })
}

/// Raw-bytes variant for file content, which is not necessarily UTF-8 JSON.
fn azure_get_bytes(config: &AzureDevOpsConfig, path: &str) -> Result<(Vec<u8>, bool), String> {
    let url = format!("{}{}", config.url.trim_end_matches('/'), path);
    let agent = azure_agent();
    let (bytes, status, truncated) = read_azure_bytes(
        agent
            .get(&url)
            .set("Authorization", &basic_auth(&config.token))
            .set("Accept", "application/json")
            .set("User-Agent", USER_AGENT)
            .call(),
        // One byte past the cap is enough to flag the content as too large
        // without buffering an unbounded blob.
        Some(MAX_DIFF_FILE_BYTES),
    )?;
    if !(200..300).contains(&status) {
        let body = String::from_utf8_lossy(&bytes);
        return Err(azure_http_error(status, &body));
    }
    Ok((bytes, truncated))
}

fn read_azure_bytes(
    result: Result<ureq::Response, ureq::Error>,
    max_bytes: Option<usize>,
) -> Result<(Vec<u8>, u16, bool), String> {
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(401, _) | ureq::Error::Status(403, _)) => {
            return Err("Azure DevOps personal access token is invalid or lacks permission".into());
        }
        Err(ureq::Error::Status(404, _)) => {
            return Err("Azure DevOps did not find that item".into());
        }
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            return Err(azure_http_error(status, &body));
        }
        Err(_) => return Err("Could not reach Azure DevOps".into()),
    };
    let status = response.status();
    let truncated = response
        .header("x-ms-continuationtoken")
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let mut bytes = Vec::new();
    let read_result = if let Some(max_bytes) = max_bytes {
        response
            .into_reader()
            .take((max_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)
    } else {
        response.into_reader().read_to_end(&mut bytes)
    };
    read_result.map_err(|_| "Azure DevOps returned an unreadable response".to_string())?;
    Ok((bytes, status, truncated))
}

fn azure_http_error(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body).ok().and_then(|value| {
        value
            .get("message")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    message.unwrap_or_else(|| format!("Azure DevOps request failed ({status})"))
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
}

// ---- URLs / remotes ----

fn normalize_azure_devops_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim().trim_end_matches('/');
    if raw.is_empty() {
        return Err(
            "Enter your Azure DevOps organization URL (e.g. https://dev.azure.com/myorg)".into(),
        );
    }
    if raw.chars().any(char::is_whitespace) || raw.contains(['?', '#', '\\']) {
        return Err("Azure DevOps URL is invalid".into());
    }
    // Bare `myorg` becomes the hosted organization URL.
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else if raw.contains('.') || raw.contains('/') {
        format!("https://{raw}")
    } else {
        format!("https://dev.azure.com/{raw}")
    };
    // The PAT travels on every request as Basic auth, so cleartext HTTP is
    // never accepted, including for on-premises hosts.
    if !with_scheme.starts_with("https://") {
        return Err("Azure DevOps URL must use HTTPS".into());
    }
    let (_, rest) = with_scheme
        .split_once("://")
        .ok_or_else(|| "Azure DevOps URL is invalid".to_string())?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.contains('@') {
        return Err("Azure DevOps URL is invalid".into());
    }
    // Accept on-premises authorities such as `tfs.contoso.com:8080` or a
    // dotless `tfs` host; the port must be numeric when present.
    let (host, port) = authority.split_once(':').unwrap_or((authority, ""));
    if host.is_empty() || (!port.is_empty() && port.parse::<u16>().is_err()) {
        return Err("Azure DevOps URL is invalid".into());
    }
    let mut segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    // Drop API/git suffixes pasted from the browser address bar.
    if let Some(index) = segments.iter().position(|segment| {
        segment.eq_ignore_ascii_case("_apis") || segment.eq_ignore_ascii_case("_git")
    }) {
        segments.truncate(index);
    }
    let authority_lower = authority.to_ascii_lowercase();
    if authority_lower == "dev.azure.com" || authority_lower == "ssh.dev.azure.com" {
        let org = segments.first().ok_or_else(|| {
            "Azure DevOps URL must include your organization (e.g. https://dev.azure.com/myorg)"
                .to_string()
        })?;
        return Ok(format!("https://dev.azure.com/{}", org.trim()));
    }
    if let Some(org) = authority_lower.strip_suffix(".visualstudio.com") {
        let org = org.trim();
        if org.is_empty() || org.contains('.') {
            return Err("Azure DevOps URL is invalid".into());
        }
        return Ok(format!("https://dev.azure.com/{org}"));
    }
    if segments.is_empty() {
        return Err("Azure DevOps URL must include your organization or collection".into());
    }
    // On-premises (e.g. https://host/tfs/collection): keep host plus the
    // collection path so remotes match the same base.
    let kept = segments
        .iter()
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join("/");
    let scheme = with_scheme
        .split_once("://")
        .map(|(scheme, _)| scheme)
        .unwrap_or("https");
    Ok(format!("{}://{}/{}", scheme, authority, kept))
}

fn organization_from_url(url: &str) -> String {
    canonical_org_key(url).unwrap_or_default()
}

fn canonical_org_key(url: &str) -> Option<String> {
    let normalized = normalize_azure_devops_url(url).ok()?;
    Some(normalized.to_ascii_lowercase())
}

fn encode_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(*byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn azure_devops_repo_for(root: &Path, organization_url: &str) -> Result<String, String> {
    let mut cmd = Command::new("git");
    crate::process::configure_child(&mut cmd);
    crate::scm::shell::apply_gui_env(&mut cmd);
    let output = cmd
        .args(["config", "--get-regexp", r"^remote\..*\.url$"])
        .current_dir(root)
        .output()
        .map_err(|_| "Could not run git".to_string())?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err("Could not read git remotes".into());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut matches = Vec::new();
    for line in stdout.lines() {
        let Some((name, remote)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if let Some(repo) = project_repo_from_remote(remote.trim(), organization_url) {
            matches.push((name == "remote.origin.url", repo));
        }
    }
    matches
        .iter()
        .find(|(origin, _)| *origin)
        .or_else(|| matches.first())
        .map(|(_, repo)| repo.clone())
        .ok_or_else(|| "No Azure DevOps remote matches the configured organization".to_string())
}

fn project_repo_from_remote(remote: &str, organization_url: &str) -> Option<String> {
    let expected = canonical_org_key(organization_url)?;
    let (org_key, project, repo) = parse_azure_remote(remote)?;
    if org_key.to_ascii_lowercase() != expected {
        return None;
    }
    if !valid_repo_parts(&project, &repo) {
        return None;
    }
    Some(format!("{project}/{repo}"))
}

fn parse_azure_remote(remote: &str) -> Option<(String, String, String)> {
    let remote = remote.trim();
    if remote.is_empty() {
        return None;
    }
    // SSH: git@ssh.dev.azure.com:v3/{org}/{project}/{repo} (no scheme).
    if !remote.contains("://") && remote.contains('@') {
        if let Some((_, path)) = remote.split_once(':') {
            let path = path.strip_prefix("v3/").unwrap_or(path);
            let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
            if parts.len() >= 3 && remote.contains("dev.azure.com") {
                let org = parts[0].to_string();
                let project = percent_decode(parts[1]);
                let repo = percent_decode(parts[2..].join("/").trim_end_matches(".git"));
                // Repository names never contain `/`; extra segments belong to
                // on-prem collection paths, which HTTPS remotes already cover.
                let repo = repo.split('/').next_back().unwrap_or("").to_string();
                return Some((
                    format!("https://dev.azure.com/{}", org.to_ascii_lowercase()),
                    project,
                    repo,
                ));
            }
        }
    }
    // HTTPS (credentials stripped first).
    let without_scheme = remote.split_once("://")?.1;
    let after_auth = without_scheme.rsplit('@').next().unwrap_or(without_scheme);
    let (authority, path) = after_auth.split_once('/')?;
    let authority_lower = authority.to_ascii_lowercase();
    let segments: Vec<String> = path
        .split('/')
        .filter(|part| !part.is_empty())
        .map(percent_decode)
        .collect();
    if authority_lower == "dev.azure.com" {
        // {org}/{project}/_git/{repo}
        if segments.len() >= 4 && segments[2].eq_ignore_ascii_case("_git") {
            let org_key = format!("https://dev.azure.com/{}", segments[0].to_ascii_lowercase());
            let repo = segments[3..]
                .join("/")
                .trim_end_matches(".git")
                .trim()
                .to_string();
            return Some((org_key, segments[1].clone(), repo));
        }
        return None;
    }
    if let Some(org) = authority_lower.strip_suffix(".visualstudio.com") {
        // {project}/_git/{repo}
        if segments.len() >= 3 && segments[1].eq_ignore_ascii_case("_git") {
            let org_key = format!("https://dev.azure.com/{}", org.to_ascii_lowercase());
            let repo = segments[2..]
                .join("/")
                .trim_end_matches(".git")
                .trim()
                .to_string();
            return Some((org_key, segments[0].clone(), repo));
        }
        return None;
    }
    // On-premises or custom host: {collection...}/{project}/_git/{repo}
    let git_index = segments
        .iter()
        .position(|segment| segment.eq_ignore_ascii_case("_git"))?;
    if git_index < 2 || git_index + 1 >= segments.len() {
        return None;
    }
    let project = segments[git_index - 1].clone();
    let repo = segments[git_index + 1..]
        .join("/")
        .trim_end_matches(".git")
        .trim()
        .to_string();
    let org_path = segments[..git_index - 1].join("/");
    Some((
        format!("https://{authority_lower}/{org_path}").to_ascii_lowercase(),
        project,
        repo,
    ))
}

fn percent_decode(value: &str) -> String {
    let mut out: Vec<u8> = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                out.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out)
        .trim()
        .trim_end_matches(".git")
        .to_string()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn valid_repo_parts(project: &str, repo: &str) -> bool {
    for part in [project, repo] {
        if part.trim().is_empty()
            || part == "."
            || part == ".."
            || part.contains(['?', '#', '\\'])
            || part.chars().any(char::is_control)
        {
            return false;
        }
    }
    !project.contains('/') && !repo.contains('/')
}

// ---- Config storage ----

fn read_config(app: &Path) -> Result<Option<AzureDevOpsConfig>, String> {
    let Some(raw) = secrets::read(Tracker::AzureDevOps, app)? else {
        return Ok(None);
    };
    let mut config: AzureDevOpsConfig =
        serde_json::from_str(&raw).map_err(|_| "Azure DevOps settings are invalid".to_string())?;
    config.url = normalize_azure_devops_url(&config.url)?;
    config.token = config.token.trim().to_string();
    if config.token.is_empty() {
        Ok(None)
    } else {
        Ok(Some(config))
    }
}

fn require_config(app: &Path) -> Result<AzureDevOpsConfig, String> {
    read_config(app)?.ok_or_else(|| "Connect Azure DevOps in Settings".to_string())
}

fn write_config(app: &Path, config: &AzureDevOpsConfig) -> Result<(), String> {
    let value = serde_json::to_string(config).map_err(|error| error.to_string())?;
    secrets::write(Tracker::AzureDevOps, app, &value)
}

fn delete_config(app: &Path) -> Result<(), String> {
    secrets::delete(Tracker::AzureDevOps, app)
}

fn expand_home(input: &str) -> PathBuf {
    if input == "~" {
        return crate::scm::shell::dirs_home()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(input));
    }
    if let Some(rest) = input.strip_prefix("~/") {
        return crate::scm::shell::dirs_home()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(rest);
    }
    PathBuf::from(input)
}

#[cfg(test)]
mod review_tests {
    use super::*;

    #[test]
    fn votes_map_to_reviews_and_a_verdict() {
        let row = json!({"reviewers": [
            {"displayName": "Mira", "vote": 10},
            {"displayName": "Cubic", "vote": 5},
            {"displayName": "Jo", "vote": -5},
            {"displayName": "Al", "vote": -10},
            {"displayName": "Core team", "vote": 0, "isContainer": true},
            {"displayName": "Gone", "vote": 0, "hasDeclined": true},
            {"vote": 10}
        ]});
        let (decision, reviews, requested) = pr_reviews(&row);
        assert_eq!(decision, "changes_requested");
        let states: Vec<(&str, &str)> = reviews
            .iter()
            .map(|r| (r.login.as_str(), r.state.as_str()))
            .collect();
        assert_eq!(
            states,
            [
                ("Mira", "approved"),
                ("Cubic", "approved"),
                ("Jo", "changes_requested"),
                ("Al", "changes_requested")
            ]
        );
        assert_eq!(requested, ["Core team"]);
    }

    #[test]
    fn a_required_reviewer_who_has_not_voted_holds_an_approval() {
        let approved = json!({"reviewers": [{"displayName": "Mira", "vote": 10},
            {"displayName": "Jo", "vote": 0}]});
        assert_eq!(pr_reviews(&approved).0, "approved");
        let held = json!({"reviewers": [{"displayName": "Mira", "vote": 10},
            {"displayName": "Jo", "vote": 0, "isRequired": true}]});
        assert_eq!(pr_reviews(&held).0, "review_required");
        assert_eq!(
            pr_reviews(&json!({"reviewers": [{"displayName": "Jo", "vote": 0}]})).0,
            "review_required"
        );
        assert_eq!(pr_reviews(&json!({})).0, "");
    }

    #[test]
    fn list_rows_carry_their_author_and_reviews() {
        let row = json!({"pullRequestId": 4, "title": "T", "status": "active",
            "createdBy": {"displayName": "Vasanth"},
            "reviewers": [{"displayName": "Mira", "vote": -5}]});
        let item = parse_pr(&row, "proj/app", "https://dev.azure.com/org").unwrap();
        assert_eq!(item.author, "Vasanth");
        assert_eq!(item.review_decision, "changes_requested");
        assert_eq!(item.reviews.len(), 1);
    }

    #[test]
    fn build_and_status_policies_are_checks() {
        let evaluations = json!({"value": [
            {"evaluationId": "a", "status": "approved", "configuration": {
                "type": {"id": BUILD_POLICY, "displayName": "Build"},
                "settings": {"displayName": "CI"}}},
            {"evaluationId": "b", "status": "rejected", "configuration": {
                "type": {"id": BUILD_POLICY, "displayName": "Build"},
                "settings": {"displayName": "Windows tests"}}},
            {"evaluationId": "c", "status": "running", "configuration": {
                "type": {"id": STATUS_POLICY, "displayName": "Status"},
                "settings": {"statusName": "sonar"}}},
            {"evaluationId": "d", "status": "rejected", "configuration": {
                "type": {"id": STATUS_POLICY, "displayName": "Status"},
                "settings": {"statusName": "lint"}}},
            {"evaluationId": "e", "status": "notApplicable", "configuration": {
                "type": {"id": BUILD_POLICY, "displayName": "Build"}, "settings": {}}},
            {"evaluationId": "f", "status": "rejected", "configuration": {
                "type": {"id": "fa4e907d-c16b-4a4c-9dfa-4906e5d171dd",
                         "displayName": "Minimum number of reviewers"}}},
            {"evaluationId": "g", "status": "rejected", "configuration": {
                "isEnabled": false,
                "type": {"id": BUILD_POLICY, "displayName": "Build"}, "settings": {}}}
        ]});
        let summary = policy_checks(&evaluations);
        assert_eq!(
            (
                summary.total,
                summary.passed,
                summary.failed,
                summary.pending
            ),
            (4, 1, 2, 1)
        );
        assert_eq!(summary.failed_names, ["Windows tests", "lint"]);
        // Only a build validation can be queued again.
        assert_eq!(failed_builds(&evaluations), ["b"]);
        assert!(policy_checks(&json!({"value": []})).is_empty());
    }

    #[test]
    fn review_requests_drop_the_ones_you_voted_on() {
        let rows = json!({"value": [
            {"pullRequestId": 1, "status": "active", "title": "Asked",
             "repository": {"name": "app", "project": {"name": "proj"}},
             "reviewers": [{"id": "ME", "displayName": "Me", "vote": 0}]},
            {"pullRequestId": 2, "status": "active", "title": "Voted",
             "repository": {"name": "app", "project": {"name": "proj"}},
             "reviewers": [{"id": "me", "displayName": "Me", "vote": 10}]},
            {"pullRequestId": 3, "status": "active", "title": "Through a group",
             "repository": {"name": "lib", "project": {"name": "proj"}},
             "reviewers": [{"id": "team", "displayName": "Team", "vote": 0}]},
            {"pullRequestId": 4, "status": "active", "title": "No repo"}
        ]});
        let items = parse_review_requests(&rows, "me", "https://dev.azure.com/org").unwrap();
        let found: Vec<(i64, &str)> = items.iter().map(|i| (i.number, i.repo.as_str())).collect();
        assert_eq!(found, [(1, "proj/app"), (3, "proj/lib")]);
        assert!(items
            .iter()
            .all(|i| i.attention_reason == "review_requested"));
    }

    #[test]
    fn viewer_and_added_repos() {
        let viewer = parse_viewer(&json!({"authenticatedUser":
            {"id": "u1", "providerDisplayName": "Vasanth"}}))
        .unwrap();
        assert_eq!(viewer, ("u1".to_string(), "Vasanth".to_string()));
        assert!(parse_viewer(&json!({})).is_err());
        let url = "https://dev.azure.com/Contoso";
        assert_eq!(added_repo(url, "proj/app").unwrap(), "proj/app");
        assert_eq!(added_repo(url, "contoso/proj/app").unwrap(), "proj/app");
        assert!(added_repo(url, "fabrikam/proj/app")
            .unwrap_err()
            .contains("fabrikam"));
        assert!(added_repo(url, "app").is_err());
    }

    #[test]
    fn repo_search_filters_without_case_and_skips_disabled() {
        let repos = json!({"value": [
            {"name": "App", "project": {"name": "Proj"}},
            {"name": "app-old", "project": {"name": "Proj"}, "isDisabled": true},
            {"name": "lib", "project": {"name": "Proj"}}
        ]});
        assert_eq!(
            parse_repo_hits(&repos, "APP"),
            [("Proj/App".to_string(), "Proj project".to_string())]
        );
        assert_eq!(parse_repo_hits(&repos, "proj/").len(), 2);
    }
}

#[cfg(test)]
mod reply_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_pull_requests_threads_keep_their_replies_and_leave_system_notes_out() {
        let value = json!({"value": [
            {"id": 6, "comments": [
                {"id": 1, "commentType": "system", "content": "dana pushed 1 commit",
                 "publishedDate": "2026-11-20T09:00:00Z"}
            ]},
            {"id": 5, "status": "fixed",
             "threadContext": {"filePath": "/src/a.rs", "rightFileStart": {"line": 12}, "rightFileEnd": {"line": 14}},
             "comments": [
                {"id": 2, "content": "Done", "publishedDate": "2026-11-20T12:00:00Z",
                 "author": {"displayName": "Dana"}},
                {"id": 1, "content": "Index this?", "publishedDate": "2026-11-20T11:00:00Z",
                 "author": {"displayName": "Marko"}},
                {"id": 3, "content": "gone", "isDeleted": true}
             ]},
            {"id": 4, "status": "active", "comments": [
                {"id": 1, "content": "Looks good", "publishedDate": "2026-11-20T10:00:00Z",
                 "author": {"displayName": "Lee"}}
            ]}
        ]});
        let thread = parse_pr_thread(
            &value,
            "https://dev.azure.com/a/p/_git/r/pullrequest/7",
            false,
        )
        .unwrap();
        assert_eq!(
            thread.comments.len(),
            2,
            "one root per thread, the system one left out"
        );
        let general = &thread.comments[0];
        assert_eq!(
            (general.author.as_str(), general.thread_id.as_str()),
            ("Lee", "4")
        );
        assert!(general.resolvable && !general.resolved && general.path.is_empty());
        let review = &thread.comments[1];
        assert_eq!(review.thread_id, "5");
        assert_eq!((review.path.as_str(), review.line), ("src/a.rs", Some(12)));
        assert!(review.resolved);
        assert_eq!(review.replies.len(), 1, "the deleted comment is left out");
        assert_eq!(review.replies[0].body, "Done");
        assert_eq!(review.replies[0].id, "5-2");
    }

    #[test]
    fn a_reply_and_a_resolve_go_to_their_thread() {
        let (path, body) = reply_request("My Project", "web", 7, "5", "Thanks").unwrap();
        assert_eq!(
            path,
            format!("/My%20Project/_apis/git/repositories/web/pullrequests/7/threads/5/comments?api-version={API_VERSION}")
        );
        assert_eq!(
            body,
            json!({"content": "Thanks", "parentCommentId": 1, "commentType": 1})
        );
        let (path, body) = resolve_request("p", "web", 7, "5", true).unwrap();
        assert!(path.ends_with(&format!(
            "/pullrequests/7/threads/5?api-version={API_VERSION}"
        )));
        assert_eq!(body, json!({"status": "fixed"}));
        assert_eq!(
            resolve_request("p", "web", 7, "5", false).unwrap().1,
            json!({"status": "active"})
        );
        assert!(
            reply_request("p", "web", 7, "5-2", "x").is_err(),
            "a comment id isn't a thread"
        );
        assert!(reply_request("p", "web", 7, "", "x").is_err());
    }
}
