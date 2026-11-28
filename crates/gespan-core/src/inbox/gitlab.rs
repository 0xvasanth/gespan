//! GitLab Inbox adapter. The saved credential format is the one older Tauri builds wrote.
use gespan_tracker_secrets::{self as secrets, Tracker};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::review::{
    self, latest_reviews, CheckSummary, LineComment, Outcome, Review, Side, TtlCache, Verdict,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_GITLAB_URL: &str = "https://gitlab.com";
const DEFAULT_LIMIT: u32 = 40;
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
const USER_AGENT: &str = "Gespan";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitlabStatus {
    pub connected: bool,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct GitlabConfig {
    url: String,
    token: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabLabel {
    pub name: String,
    pub color: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabAssignee {
    pub login: String,
    pub avatar_url: String,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabWorkItem {
    pub kind: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub updated_at: String,
    pub labels: Vec<GitlabLabel>,
    pub assignees: Vec<GitlabAssignee>,
    pub draft: bool,
    pub repo: String,
    pub attention_reason: String,
    pub author: String,
    /// Merge requests (see [`super::Item`]): the review and check fields the
    /// Pull requests page groups by. Diff sizes stay 0: GitLab's lists don't carry them.
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
pub struct GitlabWorkItemDetails {
    pub body: String,
    pub author: String,
    pub author_avatar_url: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub review_decision: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabWorkItemComment {
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
    pub replies: Vec<GitlabWorkItemComment>,
    pub author_bot: bool,
    pub resolvable: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabWorkItemThread {
    pub comments: Vec<GitlabWorkItemComment>,
    pub truncated: bool,
    pub review_decision: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabMrFile {
    pub path: String,
    pub additions: i64,
    pub deletions: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabMrDiff {
    pub additions: i64,
    pub deletions: i64,
    pub files: Vec<GitlabMrFile>,
    pub patch: String,
    pub truncated: bool,
}

pub fn gitlab_status(app: PathBuf) -> Result<GitlabStatus, String> {
    let config = read_config(&app)?;
    Ok(GitlabStatus {
        connected: config.is_some(),
        url: config
            .map(|config| config.url)
            .unwrap_or_else(|| DEFAULT_GITLAB_URL.into()),
    })
}

pub fn gitlab_set_config(app: PathBuf, url: String, token: String) -> Result<GitlabStatus, String> {
    let url = normalize_gitlab_url(&url)?;
    let token = token.trim().to_string();
    if token.is_empty() {
        delete_config(&app)?;
        return Ok(GitlabStatus {
            connected: false,
            url,
        });
    }
    let config = GitlabConfig { url, token };
    let response = gitlab_get(&config, "/user")?;
    if response.value.get("id").and_then(Value::as_i64).is_none() {
        return Err("GitLab did not return the current user".into());
    }
    write_config(&app, &config)?;
    Ok(GitlabStatus {
        connected: true,
        url: config.url,
    })
}

pub fn gitlab_repo(app: PathBuf, cwd: String) -> Result<String, String> {
    let config = require_config(&app)?;
    gitlab_repo_for(&expand_home(&cwd), &config.url)
}

pub fn gitlab_list_work_items(
    app: PathBuf,
    cwd: String,
    kind: String,
    assigned_to_me: bool,
    state: String,
    limit: Option<u32>,
) -> Result<Vec<GitlabWorkItem>, String> {
    let config = require_config(&app)?;
    let repo = gitlab_repo_for(&expand_home(&cwd), &config.url)?;
    gitlab_list_work_items_for(
        &config,
        &repo,
        &kind,
        assigned_to_me,
        &state,
        limit.unwrap_or(DEFAULT_LIMIT),
    )
}

pub fn gitlab_list_todos(
    app: PathBuf,
    kind: String,
    limit: Option<u32>,
) -> Result<Vec<GitlabWorkItem>, String> {
    let config = require_config(&app)?;
    gitlab_list_todos_for(&config, &kind, limit.unwrap_or(DEFAULT_LIMIT))
}

pub fn gitlab_work_item_details(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
) -> Result<GitlabWorkItemDetails, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(&repo)?;
    gitlab_work_item_details_for(&config, &repo, &kind, number)
}

pub fn gitlab_work_item_thread(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
) -> Result<GitlabWorkItemThread, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(&repo)?;
    gitlab_work_item_thread_for(&config, &repo, &kind, number)
}

/// The access level `username` has on project `repo`, counting access
/// inherited from groups: 30 (Developer) and above can push. A user who is
/// not a member has none.
pub fn gitlab_member_access(app: PathBuf, repo: String, username: String) -> Result<i64, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(&repo)?;
    let path = format!(
        "/projects/{}/members/all?query={}&per_page=20",
        encode_path_component(&repo),
        encode_path_component(username.trim())
    );
    parse_member_access(&gitlab_get(&config, &path)?.value, &username)
}

fn parse_member_access(value: &Value, username: &str) -> Result<i64, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "GitLab did not return project members".to_string())?;
    Ok(rows
        .iter()
        .filter(|row| {
            string_field(row, "username")
                .is_some_and(|name| name.eq_ignore_ascii_case(username.trim()))
        })
        .filter_map(|row| row.get("access_level").and_then(Value::as_i64))
        .max()
        .unwrap_or(0))
}

/// The username of the account the saved token belongs to.
pub fn gitlab_current_user(app: PathBuf) -> Result<String, String> {
    let config = require_config(&app)?;
    parse_current_user(&gitlab_get(&config, "/user")?.value)
}

fn parse_current_user(value: &Value) -> Result<String, String> {
    string_field(value, "username")
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| "GitLab did not say which account is connected".to_string())
}

/// A comment on a merge request or issue: in the discussion `reply_to`
/// names, or a new one when it is empty.
pub fn gitlab_work_item_comment(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
    body: String,
    reply_to: String,
) -> Result<String, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(&repo)?;
    gitlab_work_item_comment_for(&config, &repo, &kind, number, &body, reply_to.trim())
}

/// Marks a discussion resolved, or open again.
pub fn gitlab_resolve_thread(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
    discussion: String,
    resolved: bool,
) -> Result<(), String> {
    let config = require_config(&app)?;
    let repo = validate_repo(&repo)?;
    validate_item(&kind, number)?;
    if discussion.trim().is_empty() {
        return Err("No thread to resolve".into());
    }
    gitlab_send(
        &config,
        "PUT",
        &resolve_path(&repo, &kind, number, discussion.trim(), resolved),
        None,
    )?;
    Ok(())
}

pub fn gitlab_mr_diff(app: PathBuf, repo: String, number: i64) -> Result<GitlabMrDiff, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(&repo)?;
    gitlab_mr_diff_for(&config, &repo, number)
}

/// The connected account's GitLab API, for the draft notes Gespan leaves on
/// a merge request itself.
pub struct GitlabApi {
    config: GitlabConfig,
}

impl GitlabApi {
    pub fn connect(app: &Path) -> Result<Self, String> {
        Ok(Self {
            config: require_config(app)?,
        })
    }
}

impl crate::review_draft::GitlabHttp for GitlabApi {
    fn get(&mut self, path: &str) -> Result<Value, String> {
        gitlab_get(&self.config, path).map(|r| r.value)
    }
    fn post_json(&mut self, path: &str, body: &Value) -> Result<Value, String> {
        gitlab_send(&self.config, "POST", path, Some(body)).map(|r| r.value)
    }
    fn put(&mut self, path: &str) -> Result<Value, String> {
        gitlab_send(&self.config, "PUT", path, None).map(|r| r.value)
    }
    fn mr_patch(&mut self, repo: &str, number: u64) -> Result<String, String> {
        let repo = validate_repo(repo)?;
        let number = i64::try_from(number).map_err(|e| e.to_string())?;
        gitlab_mr_diff_for(&self.config, &repo, number).map(|diff| diff.patch)
    }
}

/// The unified patch Gespan builds from a `/diffs` response.
#[cfg(test)]
pub(crate) fn patch_from_diffs(value: &Value) -> Result<String, String> {
    parse_mr_diff(value, false).map(|diff| diff.patch)
}

fn gitlab_list_work_items_for(
    config: &GitlabConfig,
    repo: &str,
    kind: &str,
    assigned_to_me: bool,
    state: &str,
    limit: u32,
) -> Result<Vec<GitlabWorkItem>, String> {
    validate_kind(kind)?;
    let resource = resource_for_kind(kind);
    let state = if state.trim().eq_ignore_ascii_case("all") {
        "all"
    } else {
        "opened"
    };
    let limit = limit.clamp(1, 100);
    let assigned = if assigned_to_me {
        "&scope=assigned_to_me"
    } else {
        "&scope=all"
    };
    let path = format!(
        "/projects/{}/{resource}?state={state}&order_by=updated_at&sort=desc&per_page={limit}&with_labels_details=true{assigned}",
        encode_path_component(repo)
    );
    let response = gitlab_get(config, &path)?;
    let mut items = parse_work_items(&response.value, kind, repo)?;
    if kind == "pr" {
        enrich_merge_requests(config, repo, &mut items);
    }
    Ok(items)
}

/// Open merge requests whose reviews and pipeline are looked up per list,
/// at most this many, with this many requests at once.
const MAX_ENRICHED: usize = 30;
const ENRICH_WORKERS: usize = 6;

/// Fills reviews, approvals and checks on the open merge requests of one
/// project's list. Each needs its own requests, so they run a few at once
/// and are kept for a minute; one that fails keeps its fields empty.
fn enrich_merge_requests(config: &GitlabConfig, repo: &str, items: &mut [GitlabWorkItem]) {
    let viewer = gitlab_viewer(config);
    for item in items.iter_mut() {
        item.viewer = viewer.clone();
    }
    let open: Vec<&mut GitlabWorkItem> = items
        .iter_mut()
        .filter(|item| item.state == "open")
        .take(MAX_ENRICHED)
        .collect();
    review::for_each_bounded(open, ENRICH_WORKERS, |item| {
        if let Ok(found) = cached_mr_review(config, repo, item.number, &item.updated_at) {
            found.apply(item);
        }
    });
}

/// The connected account's username, asked at most every few minutes;
/// empty when GitLab can't say.
fn gitlab_viewer(config: &GitlabConfig) -> String {
    static VIEWERS: TtlCache<String> =
        TtlCache::new(Duration::from_secs(300), Duration::from_secs(60));
    VIEWERS
        .get(&config.url, || {
            parse_current_user(&gitlab_get(config, "/user")?.value)
        })
        .unwrap_or_default()
}

/// What one merge request's reviewers, approvals and pipeline say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct MrReview {
    review_decision: String,
    reviews: Vec<Review>,
    /// `None` when the reviewers' states could not be read, so the list's
    /// reviewers stand.
    requested_reviewers: Option<Vec<String>>,
    check_summary: CheckSummary,
}

impl MrReview {
    fn apply(self, item: &mut GitlabWorkItem) {
        item.review_decision = self.review_decision;
        if let Some(requested) = self.requested_reviewers {
            item.requested_reviewers = requested;
        } else {
            item.requested_reviewers.retain(|login| {
                !self
                    .reviews
                    .iter()
                    .any(|review| review.login.eq_ignore_ascii_case(login))
            });
        }
        item.reviews = self.reviews;
        item.check_summary = self.check_summary;
    }
}

fn cached_mr_review(
    config: &GitlabConfig,
    repo: &str,
    number: i64,
    updated_at: &str,
) -> Result<MrReview, String> {
    static REVIEWS: TtlCache<MrReview> =
        TtlCache::new(Duration::from_secs(60), Duration::from_secs(60));
    let key = format!("{}|{repo}|{number}|{updated_at}", config.url);
    REVIEWS.get(&key, || fetch_mr_review(config, repo, number))
}

/// The merge request itself (its pipeline and merge status) must load; its
/// approvals, reviewers and jobs are each optional.
fn fetch_mr_review(config: &GitlabConfig, repo: &str, number: i64) -> Result<MrReview, String> {
    let base = item_path(repo, "pr", number);
    let mr = gitlab_get(config, &base)?.value;
    let approvals = gitlab_get(config, &format!("{base}/approvals")).ok();
    let reviewers = gitlab_get(config, &format!("{base}/reviewers")).ok();
    let jobs = mr
        .get("head_pipeline")
        .and_then(|pipeline| pipeline.get("id"))
        .and_then(Value::as_i64)
        .and_then(|id| {
            gitlab_get(
                config,
                &format!(
                    "/projects/{}/pipelines/{id}/jobs?per_page=100",
                    encode_path_component(repo)
                ),
            )
            .ok()
        });
    Ok(mr_review_from(
        &mr,
        approvals.as_ref().map(|r| &r.value),
        reviewers.as_ref().map(|r| &r.value),
        jobs.as_ref().map(|r| &r.value),
    ))
}

fn username(user: &Value) -> Option<String> {
    string_field(user, "username").filter(|name| !name.is_empty())
}

/// Maps a merge request, `/approvals`, `/reviewers` (GitLab 17's states,
/// `requested_changes` among them) and the head pipeline's `/jobs`.
fn mr_review_from(
    mr: &Value,
    approvals: Option<&Value>,
    reviewers: Option<&Value>,
    jobs: Option<&Value>,
) -> MrReview {
    let mut rows: Vec<(String, String, String)> = Vec::new();
    let mut requested: Option<Vec<String>> = None;
    if let Some(reviewers) = reviewers.and_then(Value::as_array) {
        let mut waiting = Vec::new();
        for reviewer in reviewers {
            let Some(login) = reviewer.get("user").and_then(username) else {
                continue;
            };
            let state = string_field(reviewer, "state").unwrap_or_default();
            match review::review_state(&state) {
                Some(state) => rows.push((login, state.into(), String::new())),
                None => waiting.push(login),
            }
        }
        requested = Some(waiting);
    }
    let approved_by: Vec<String> = approvals
        .and_then(|a| a.get("approved_by"))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("user").and_then(username))
                .collect()
        })
        .unwrap_or_default();
    // An approval stands over what the reviewer said before.
    rows.extend(
        approved_by
            .iter()
            .map(|login| (login.clone(), "approved".to_string(), "~".to_string())),
    );
    let mut reviews = latest_reviews(rows);
    for login in &approved_by {
        if let Some(review) = reviews
            .iter_mut()
            .find(|r| r.login.eq_ignore_ascii_case(login))
        {
            review.state = "approved".into();
        }
    }
    if let Some(waiting) = requested.as_mut() {
        waiting.retain(|login| !approved_by.iter().any(|a| a.eq_ignore_ascii_case(login)));
    }
    let merge_status = string_field(mr, "detailed_merge_status").unwrap_or_default();
    let approvals_left = approvals
        .and_then(|a| a.get("approvals_left"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let approved = approvals
        .and_then(|a| a.get("approved"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let review_decision = if merge_status == "requested_changes"
        || reviews.iter().any(|r| r.state == "changes_requested")
    {
        "changes_requested"
    } else if approved && approvals_left <= 0 && !approved_by.is_empty() {
        "approved"
    } else if merge_status == "not_approved"
        || approvals_left > 0
        || requested.as_ref().is_some_and(|r| !r.is_empty())
    {
        "review_required"
    } else {
        ""
    };
    MrReview {
        review_decision: review_decision.into(),
        reviews,
        requested_reviewers: requested,
        check_summary: mr_checks(mr, jobs),
    }
}

/// A job counts by its status; a failure allowed to fail passes. Without
/// the jobs, the head pipeline counts as one check.
fn mr_checks(mr: &Value, jobs: Option<&Value>) -> CheckSummary {
    if let Some(jobs) = jobs.and_then(Value::as_array) {
        return CheckSummary::tally(jobs.iter().map(|job| {
            let allowed = job
                .get("allow_failure")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let outcome =
                match gitlab_status_outcome(&string_field(job, "status").unwrap_or_default()) {
                    Outcome::Failed if allowed => Outcome::Passed,
                    outcome => outcome,
                };
            (string_field(job, "name").unwrap_or_default(), outcome)
        }));
    }
    match mr
        .get("head_pipeline")
        .and_then(|pipeline| string_field(pipeline, "status"))
    {
        Some(status) if !status.is_empty() => {
            CheckSummary::tally([("Pipeline".to_string(), gitlab_status_outcome(&status))])
        }
        _ => CheckSummary::default(),
    }
}

fn gitlab_status_outcome(status: &str) -> Outcome {
    match status.trim().to_ascii_lowercase().as_str() {
        "success" | "skipped" | "manual" => Outcome::Passed,
        "failed" | "canceled" | "cancelled" => Outcome::Failed,
        _ => Outcome::Pending,
    }
}

/// The open merge requests of a project added by hand.
pub fn gitlab_list_repo_prs(
    app: PathBuf,
    repo: &str,
    limit: u32,
) -> Result<Vec<GitlabWorkItem>, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(repo)?;
    gitlab_list_work_items_for(&config, &repo, "pr", false, "open", limit)
}

/// Open merge requests anywhere that ask for your review: one request.
pub fn gitlab_review_requests(app: PathBuf) -> Result<Vec<GitlabWorkItem>, String> {
    let config = require_config(&app)?;
    let viewer = gitlab_viewer(&config);
    if viewer.is_empty() {
        return Err("GitLab did not say which account is connected".into());
    }
    let path = format!(
        "/merge_requests?reviewer_username={}&state=opened&scope=all&order_by=updated_at&sort=desc&per_page=50",
        encode_path_component(&viewer)
    );
    let mut items = parse_review_requests(&gitlab_get(&config, &path)?.value)?;
    for item in &mut items {
        item.viewer = viewer.clone();
    }
    Ok(items)
}

/// Rows from `/merge_requests`, each naming its project in `references`.
fn parse_review_requests(value: &Value) -> Result<Vec<GitlabWorkItem>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "GitLab did not return merge requests".to_string())?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let full = row
                .get("references")
                .and_then(|r| string_field(r, "full"))?;
            let repo = full.rsplit_once('!').map(|(repo, _)| repo)?.to_string();
            if !valid_project_path(&repo) {
                return None;
            }
            let mut item = parse_work_item(row, "pr", &repo)?;
            item.attention_reason = "review_requested".into();
            Some(item)
        })
        .collect())
}

/// Projects you are a member of whose name or path matches `query`.
pub fn gitlab_search_repos(app: PathBuf, query: &str) -> Result<Vec<(String, String)>, String> {
    let config = require_config(&app)?;
    let path = format!(
        "/projects?membership=true&simple=true&search={}&order_by=last_activity_at&per_page=20",
        encode_path_component(query.trim())
    );
    parse_repo_hits(&gitlab_get(&config, &path)?.value)
}

fn parse_repo_hits(value: &Value) -> Result<Vec<(String, String)>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "GitLab did not return projects".to_string())?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let repo =
                string_field(row, "path_with_namespace").filter(|r| valid_project_path(r))?;
            Some((repo, string_field(row, "description").unwrap_or_default()))
        })
        .collect())
}

/// Approves, requests changes or comments. GitLab's REST API has no
/// "Request changes", so that removes your approval, if any, and posts
/// `body` as a note: the request reads as the note's words.
pub fn gitlab_submit_review(
    app: PathBuf,
    repo: &str,
    number: i64,
    verdict: Verdict,
    body: &str,
    comments: &[LineComment],
) -> Result<(), String> {
    let config = require_config(&app)?;
    let repo = validate_repo(repo)?;
    validate_item("pr", number)?;
    let base = item_path(&repo, "pr", number);
    if !comments.is_empty() {
        // A diff note is a discussion positioned on the merge request's
        // current versions.
        let refs = gitlab_get(&config, &base)?.value["diff_refs"].clone();
        for comment in comments {
            let payload = gitlab_line_discussion(&refs, comment)?;
            gitlab_send(
                &config,
                "POST",
                &format!("{base}/discussions"),
                Some(&payload),
            )?;
        }
    }
    match verdict {
        Verdict::Approve => {
            gitlab_send(&config, "POST", &format!("{base}/approve"), None)?;
        }
        Verdict::RequestChanges => {
            // Not approved before is fine: GitLab answers 404 then.
            let _ = gitlab_send(&config, "POST", &format!("{base}/unapprove"), None);
        }
        Verdict::Comment => {}
    }
    if !body.trim().is_empty() {
        gitlab_work_item_comment_for(&config, &repo, "pr", number, body, "")?;
    } else if verdict != Verdict::Approve && comments.is_empty() {
        return Err("Write a comment before sending it.".into());
    }
    Ok(())
}

/// One end of a multi-line note: GitLab names a line by a code made of the
/// file's SHA-1 and the line's place in the old and new file.
fn gitlab_range_end(path: &str, line: &super::review::LineRef) -> Value {
    use sha1::Digest;
    let digest = sha1::Sha1::digest(path.as_bytes());
    let hash: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let kind = match (line.old_line, line.new_line) {
        (None, Some(_)) => Value::from("new"),
        (Some(_), None) => Value::from("old"),
        _ => Value::Null,
    };
    serde_json::json!({
        "line_code": format!("{hash}_{}_{}", line.old_pos, line.new_pos),
        "type": kind,
        "old_line": line.old_line,
        "new_line": line.new_line,
    })
}

/// A discussion on one line of a merge request's diff, positioned on its
/// `diff_refs`: a new line by its new number, a deleted one by its old.
pub(crate) fn gitlab_line_discussion(refs: &Value, comment: &LineComment) -> Result<Value, String> {
    let sha = |key: &str| {
        refs[key]
            .as_str()
            .filter(|sha| !sha.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                "GitLab did not say which versions the merge request compares".to_string()
            })
    };
    let mut position = serde_json::json!({
        "position_type": "text",
        "base_sha": sha("base_sha")?,
        "start_sha": sha("start_sha")?,
        "head_sha": sha("head_sha")?,
        "old_path": comment.path,
        "new_path": comment.path,
    });
    let end = &comment.end;
    // A context line is in both files and GitLab wants both numbers.
    match (end.old_line, end.new_line, end.side) {
        (Some(old), Some(new), _) => {
            position["old_line"] = old.into();
            position["new_line"] = new.into();
        }
        (_, _, Side::New) => position["new_line"] = end.line.into(),
        (_, _, Side::Old) => position["old_line"] = end.line.into(),
    }
    if let Some(start) = comment.range_start() {
        position["line_range"] = serde_json::json!({
            "start": gitlab_range_end(&comment.path, start),
            "end": gitlab_range_end(&comment.path, end),
        });
    }
    Ok(serde_json::json!({ "body": comment.body.trim(), "position": position }))
}

/// Retries the head pipeline's failed jobs.
pub fn gitlab_rerun_failed(app: PathBuf, repo: &str, number: i64) -> Result<String, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(repo)?;
    validate_item("pr", number)?;
    let mr = gitlab_get(&config, &item_path(&repo, "pr", number))?.value;
    let pipeline = mr
        .get("head_pipeline")
        .and_then(|pipeline| pipeline.get("id"))
        .and_then(Value::as_i64)
        .ok_or_else(|| "This merge request has no pipeline to rerun".to_string())?;
    let project = encode_path_component(&repo);
    let failed = gitlab_get(
        &config,
        &format!("/projects/{project}/pipelines/{pipeline}/jobs?scope%5B%5D=failed&scope%5B%5D=canceled&per_page=100"),
    )
    .ok()
    .and_then(|jobs| jobs.value.as_array().map(Vec::len))
    .unwrap_or(0);
    if failed == 0 {
        return Err("Nothing failed in this merge request's pipeline".into());
    }
    gitlab_send(
        &config,
        "POST",
        &format!("/projects/{project}/pipelines/{pipeline}/retry"),
        None,
    )?;
    Ok(rerun_notice(failed, "job"))
}

fn rerun_notice(count: usize, what: &str) -> String {
    format!(
        "Rerun requested for {count} failed {what}{}.",
        if count == 1 { "" } else { "s" }
    )
}

fn gitlab_list_todos_for(
    config: &GitlabConfig,
    kind: &str,
    limit: u32,
) -> Result<Vec<GitlabWorkItem>, String> {
    validate_kind(kind)?;
    let target_type = if kind == "pr" {
        "MergeRequest"
    } else {
        "Issue"
    };
    let limit = limit.clamp(1, 100);
    let path = format!("/todos?state=pending&type={target_type}&per_page={limit}");
    let response = gitlab_get(config, &path)?;
    parse_todos(&response.value, kind)
}

fn gitlab_work_item_details_for(
    config: &GitlabConfig,
    repo: &str,
    kind: &str,
    number: i64,
) -> Result<GitlabWorkItemDetails, String> {
    validate_item(kind, number)?;
    let path = item_path(repo, kind, number);
    let response = gitlab_get(config, &path)?;
    parse_work_item_details(&response.value, kind)
}

fn gitlab_work_item_thread_for(
    config: &GitlabConfig,
    repo: &str,
    kind: &str,
    number: i64,
) -> Result<GitlabWorkItemThread, String> {
    validate_item(kind, number)?;
    let path = format!("{}/discussions?per_page=100", item_path(repo, kind, number));
    let response = gitlab_get(config, &path)?;
    parse_discussions(
        &response.value,
        &config.url,
        repo,
        kind,
        number,
        response.has_next_page,
    )
}

fn gitlab_work_item_comment_for(
    config: &GitlabConfig,
    repo: &str,
    kind: &str,
    number: i64,
    body: &str,
    reply_to: &str,
) -> Result<String, String> {
    validate_item(kind, number)?;
    let body = body.trim();
    if body.is_empty() {
        return Err("Comment cannot be empty".into());
    }
    let path = if reply_to.is_empty() {
        format!("{}/notes", item_path(repo, kind, number))
    } else {
        reply_path(repo, kind, number, reply_to)
    };
    let response = gitlab_post_form(config, &path, &[("body", body)])?;
    let id = response
        .value
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| "GitLab did not return a comment".to_string())?;
    Ok(note_url(&config.url, repo, kind, number, id))
}

fn gitlab_mr_diff_for(
    config: &GitlabConfig,
    repo: &str,
    number: i64,
) -> Result<GitlabMrDiff, String> {
    validate_item("pr", number)?;
    let path = format!(
        "/projects/{}/merge_requests/{number}/diffs?per_page=100",
        encode_path_component(repo)
    );
    match gitlab_get_request(config, &path).call() {
        // Older self-hosted GitLab versions expose /changes but not /diffs.
        // Match the HTTP status so permission and transport errors still surface.
        Err(ureq::Error::Status(404, _)) => {
            let path = format!("{}/changes", item_path(repo, "pr", number));
            let response = gitlab_get(config, &path)?;
            let changes = response
                .value
                .get("changes")
                .ok_or_else(|| "GitLab did not return merge request diffs".to_string())?;
            let overflow = response
                .value
                .get("overflow")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            parse_mr_diff(changes, response.has_next_page || overflow)
        }
        result => {
            let response = read_gitlab_response(result)?;
            parse_mr_diff(&response.value, response.has_next_page)
        }
    }
}

fn validate_kind(kind: &str) -> Result<(), String> {
    if kind == "issue" || kind == "pr" {
        Ok(())
    } else {
        Err("Unknown GitLab task kind".into())
    }
}

fn validate_item(kind: &str, number: i64) -> Result<(), String> {
    validate_kind(kind)?;
    if number <= 0 {
        return Err("Invalid GitLab item number".into());
    }
    Ok(())
}

fn resource_for_kind(kind: &str) -> &'static str {
    if kind == "pr" {
        "merge_requests"
    } else {
        "issues"
    }
}

fn item_path(repo: &str, kind: &str, number: i64) -> String {
    format!(
        "/projects/{}/{}/{number}",
        encode_path_component(repo),
        resource_for_kind(kind)
    )
}

fn parse_work_items(value: &Value, kind: &str, repo: &str) -> Result<Vec<GitlabWorkItem>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "GitLab did not return work items".to_string())?;
    Ok(rows
        .iter()
        .filter_map(|row| parse_work_item(row, kind, repo))
        .collect())
}

fn parse_todos(value: &Value, kind: &str) -> Result<Vec<GitlabWorkItem>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "GitLab did not return to-do items".to_string())?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let repo = row
                .get("project")
                .and_then(|project| string_field(project, "path_with_namespace"))?;
            if !valid_project_path(&repo) {
                return None;
            }
            let target = row.get("target")?;
            let mut item = parse_work_item(target, kind, &repo)?;
            if item.url.is_empty() {
                item.url = string_field(row, "target_url").unwrap_or_default();
            }
            if item.updated_at.is_empty() {
                item.updated_at = string_field(row, "created_at").unwrap_or_default();
            }
            item.attention_reason = string_field(row, "action_name").unwrap_or_default();
            Some(item)
        })
        .collect())
}

fn parse_work_item(row: &Value, kind: &str, repo: &str) -> Option<GitlabWorkItem> {
    let number = row.get("iid").and_then(Value::as_i64)?;
    if number <= 0 {
        return None;
    }
    let title = string_field(row, "title").unwrap_or_default();
    let title_lower = title.to_ascii_lowercase();
    let draft = kind == "pr"
        && (row.get("draft").and_then(Value::as_bool).unwrap_or(false)
            || row
                .get("work_in_progress")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            || title_lower.starts_with("draft:")
            || title_lower.starts_with("wip:"));
    Some(GitlabWorkItem {
        kind: kind.into(),
        number,
        title,
        url: string_field(row, "web_url").unwrap_or_default(),
        state: normalize_state(&string_field(row, "state").unwrap_or_default()),
        updated_at: string_field(row, "updated_at").unwrap_or_default(),
        labels: parse_labels(row),
        assignees: parse_assignees(row),
        draft,
        repo: repo.into(),
        attention_reason: String::new(),
        author: row.get("author").and_then(username).unwrap_or_default(),
        requested_reviewers: if kind == "pr" {
            row.get("reviewers")
                .and_then(Value::as_array)
                .map(|people| people.iter().filter_map(username).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        },
        ..Default::default()
    })
}

fn validate_repo(repo: &str) -> Result<String, String> {
    let repo = repo.trim();
    if valid_project_path(repo) {
        Ok(repo.to_string())
    } else {
        Err("Invalid GitLab project".into())
    }
}

fn parse_work_item_details(value: &Value, kind: &str) -> Result<GitlabWorkItemDetails, String> {
    if !value.is_object() {
        return Err("GitLab did not return that item".into());
    }
    let author = value.get("author");
    Ok(GitlabWorkItemDetails {
        body: string_field(value, "description").unwrap_or_default(),
        author: author
            .and_then(|author| string_field(author, "username"))
            .or_else(|| author.and_then(|author| string_field(author, "name")))
            .unwrap_or_default(),
        author_avatar_url: author
            .and_then(|author| string_field(author, "avatar_url"))
            .unwrap_or_default(),
        base_ref_name: if kind == "pr" {
            string_field(value, "target_branch").unwrap_or_default()
        } else {
            String::new()
        },
        head_ref_name: if kind == "pr" {
            string_field(value, "source_branch").unwrap_or_default()
        } else {
            String::new()
        },
        review_decision: String::new(),
    })
}

/// A merge request's or issue's discussions, oldest first: each one's first
/// note with the rest as its replies, so a reply goes back into it. Notes
/// GitLab writes itself ("added 2 commits") are left out.
fn parse_discussions(
    value: &Value,
    base_url: &str,
    repo: &str,
    kind: &str,
    number: i64,
    has_next_page: bool,
) -> Result<GitlabWorkItemThread, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "GitLab did not return comments".to_string())?;
    let note = |row: &Value, discussion: &str| {
        let id = row.get("id").and_then(Value::as_i64)?;
        let author = row.get("author");
        let position = row.get("position");
        Some(GitlabWorkItemComment {
            id: id.to_string(),
            kind: if position.is_some_and(|p| !p.is_null()) {
                "review_comment".into()
            } else {
                "comment".into()
            },
            author: author
                .and_then(|author| string_field(author, "username"))
                .or_else(|| author.and_then(|author| string_field(author, "name")))
                .unwrap_or_default(),
            author_avatar_url: author
                .and_then(|author| string_field(author, "avatar_url"))
                .unwrap_or_default(),
            body: string_field(row, "body").unwrap_or_default(),
            created_at: string_field(row, "created_at").unwrap_or_default(),
            url: note_url(base_url, repo, kind, number, id),
            state: String::new(),
            path: position
                .and_then(|p| string_field(p, "new_path").or_else(|| string_field(p, "old_path")))
                .unwrap_or_default(),
            line: position.and_then(|p| {
                p.get("new_line")
                    .and_then(Value::as_i64)
                    .or_else(|| p.get("old_line").and_then(Value::as_i64))
            }),
            resolved: row
                .get("resolved")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            thread_id: discussion.to_string(),
            replies: Vec::new(),
            author_bot: author
                .and_then(|author| author.get("bot"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            resolvable: row
                .get("resolvable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    };
    let mut comments = Vec::new();
    for discussion in rows {
        let id = string_field(discussion, "id").unwrap_or_default();
        let notes: Vec<GitlabWorkItemComment> = discussion
            .get("notes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|row| !row.get("system").and_then(Value::as_bool).unwrap_or(false))
            .filter_map(|row| note(row, &id))
            .collect();
        let mut notes = notes.into_iter();
        let Some(mut first) = notes.next() else {
            continue;
        };
        first.replies = notes.collect();
        // A thread is resolved only once every resolvable note in it is.
        first.resolved = first.resolvable
            && first.resolved
            && first.replies.iter().all(|r| !r.resolvable || r.resolved);
        comments.push(first);
    }
    Ok(GitlabWorkItemThread {
        comments,
        truncated: has_next_page,
        review_decision: String::new(),
        base_ref_name: String::new(),
        head_ref_name: String::new(),
    })
}

/// Where a reply to a discussion is posted.
pub(super) fn reply_path(repo: &str, kind: &str, number: i64, discussion: &str) -> String {
    format!(
        "{}/discussions/{}/notes",
        item_path(repo, kind, number),
        encode_path_component(discussion)
    )
}

/// Where a discussion is marked resolved, or open again.
pub(super) fn resolve_path(
    repo: &str,
    kind: &str,
    number: i64,
    discussion: &str,
    resolved: bool,
) -> String {
    format!(
        "{}/discussions/{}?resolved={resolved}",
        item_path(repo, kind, number),
        encode_path_component(discussion)
    )
}

fn parse_mr_diff(value: &Value, has_next_page: bool) -> Result<GitlabMrDiff, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "GitLab did not return merge request diffs".to_string())?;
    let mut files = Vec::new();
    let mut patch = String::new();
    let mut total_additions = 0;
    let mut total_deletions = 0;
    let mut truncated = has_next_page;

    for row in rows {
        let old_path = string_field(row, "old_path").unwrap_or_default();
        let new_path = string_field(row, "new_path").unwrap_or_else(|| old_path.clone());
        if new_path.is_empty() && old_path.is_empty() {
            continue;
        }
        let diff = string_field_preserve(row, "diff").unwrap_or_default();
        let (additions, deletions) = diff_counts(&diff);
        total_additions += additions;
        total_deletions += deletions;
        files.push(GitlabMrFile {
            path: if new_path.is_empty() {
                old_path.clone()
            } else {
                new_path.clone()
            },
            additions,
            deletions,
        });
        truncated |= row
            .get("too_large")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || row
                .get("collapsed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        if diff.is_empty() || patch.len() >= MAX_DIFF_BYTES {
            continue;
        }
        let block = gitlab_diff_block(row, &old_path, &new_path, &diff);
        if patch.len() + block.len() > MAX_DIFF_BYTES {
            truncated = true;
            continue;
        }
        patch.push_str(&block);
    }

    Ok(GitlabMrDiff {
        additions: total_additions,
        deletions: total_deletions,
        files,
        patch,
        truncated,
    })
}

fn gitlab_diff_block(row: &Value, old_path: &str, new_path: &str, diff: &str) -> String {
    let old = if old_path.is_empty() {
        new_path
    } else {
        old_path
    };
    let new = if new_path.is_empty() {
        old_path
    } else {
        new_path
    };
    let mut block = format!("diff --git a/{old} b/{new}\n");
    if row
        .get("new_file")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        block.push_str("new file mode 100644\n");
    }
    if row
        .get("deleted_file")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        block.push_str("deleted file mode 100644\n");
    }
    if row
        .get("renamed_file")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        block.push_str(&format!("rename from {old}\nrename to {new}\n"));
    }
    let old_header = if row
        .get("new_file")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "/dev/null".to_string()
    } else {
        format!("a/{old}")
    };
    let new_header = if row
        .get("deleted_file")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "/dev/null".to_string()
    } else {
        format!("b/{new}")
    };
    block.push_str(&format!("--- {old_header}\n+++ {new_header}\n"));
    block.push_str(diff);
    if !diff.ends_with('\n') {
        block.push('\n');
    }
    block
}

fn diff_counts(diff: &str) -> (i64, i64) {
    let mut additions = 0;
    let mut deletions = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            additions += 1;
        } else if line.starts_with('-') {
            deletions += 1;
        }
    }
    (additions, deletions)
}

fn parse_labels(row: &Value) -> Vec<GitlabLabel> {
    row.get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|label| {
                    if let Some(name) = label.as_str() {
                        return Some(GitlabLabel {
                            name: name.trim().to_string(),
                            color: String::new(),
                        });
                    }
                    let name = string_field(label, "name")?;
                    Some(GitlabLabel {
                        name,
                        color: string_field(label, "color")
                            .unwrap_or_default()
                            .trim_start_matches('#')
                            .to_string(),
                    })
                })
                .filter(|label| !label.name.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_assignees(row: &Value) -> Vec<GitlabAssignee> {
    let mut people: Vec<&Value> = row
        .get("assignees")
        .and_then(Value::as_array)
        .map(|people| people.iter().collect())
        .unwrap_or_default();
    if people.is_empty() {
        if let Some(assignee) = row.get("assignee").filter(|value| value.is_object()) {
            people.push(assignee);
        }
    }
    people
        .into_iter()
        .filter_map(|person| {
            let login =
                string_field(person, "username").or_else(|| string_field(person, "name"))?;
            if login.is_empty() {
                return None;
            }
            Some(GitlabAssignee {
                login,
                avatar_url: string_field(person, "avatar_url").unwrap_or_default(),
            })
        })
        .collect()
}

fn normalize_state(state: &str) -> String {
    match state.trim().to_ascii_lowercase().as_str() {
        "opened" | "reopened" => "open".into(),
        other => other.into(),
    }
}

fn note_url(base_url: &str, repo: &str, kind: &str, number: i64, id: i64) -> String {
    let item = if kind == "pr" {
        "merge_requests"
    } else {
        "issues"
    };
    format!(
        "{}/{repo}/-/{item}/{number}#note_{id}",
        base_url.trim_end_matches('/')
    )
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
}

fn string_field_preserve(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

struct GitlabResponse {
    value: Value,
    has_next_page: bool,
}

fn gitlab_get(config: &GitlabConfig, path: &str) -> Result<GitlabResponse, String> {
    read_gitlab_response(gitlab_get_request(config, path).call())
}

fn gitlab_get_request(config: &GitlabConfig, path: &str) -> ureq::Request {
    let url = format!("{}/api/v4{}", config.url.trim_end_matches('/'), path);
    let agent = gitlab_agent();
    agent
        .get(&url)
        .set("PRIVATE-TOKEN", &config.token)
        .set("Accept", "application/json")
        .set("User-Agent", USER_AGENT)
}

fn gitlab_post_form(
    config: &GitlabConfig,
    path: &str,
    fields: &[(&str, &str)],
) -> Result<GitlabResponse, String> {
    let url = format!("{}/api/v4{}", config.url.trim_end_matches('/'), path);
    let agent = gitlab_agent();
    read_gitlab_response(
        agent
            .post(&url)
            .set("PRIVATE-TOKEN", &config.token)
            .set("Accept", "application/json")
            .set("User-Agent", USER_AGENT)
            .send_form(fields),
    )
}

fn gitlab_send(
    config: &GitlabConfig,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<GitlabResponse, String> {
    let url = format!("{}/api/v4{}", config.url.trim_end_matches('/'), path);
    let request = gitlab_agent()
        .request(method, &url)
        .set("PRIVATE-TOKEN", &config.token)
        .set("Accept", "application/json")
        .set("User-Agent", USER_AGENT);
    read_gitlab_response(match body {
        Some(body) => request
            .set("Content-Type", "application/json")
            .send_string(&body.to_string()),
        None => request.call(),
    })
}

fn gitlab_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(HTTP_TIMEOUT)
        .redirects(0)
        .build()
}

fn read_gitlab_response(
    result: Result<ureq::Response, ureq::Error>,
) -> Result<GitlabResponse, String> {
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => {
            return Err("GitLab access token is invalid or lacks permission".into());
        }
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            return Err(gitlab_http_error(status, &body));
        }
        Err(_) => return Err("Could not reach GitLab".into()),
    };
    let status = response.status();
    let has_next_page = response
        .header("X-Next-Page")
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let body = response
        .into_string()
        .map_err(|_| "GitLab returned an unreadable response".to_string())?;
    if !(200..300).contains(&status) {
        return Err(gitlab_http_error(status, &body));
    }
    let value =
        serde_json::from_str(&body).map_err(|_| "GitLab returned invalid JSON".to_string())?;
    Ok(GitlabResponse {
        value,
        has_next_page,
    })
}

fn gitlab_http_error(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body).ok().and_then(|value| {
        value
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| value.get("error").and_then(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    message.unwrap_or_else(|| format!("GitLab request failed ({status})"))
}

fn normalize_gitlab_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let raw = if raw.is_empty() {
        DEFAULT_GITLAB_URL
    } else {
        raw
    };
    if raw.contains("://") && !raw.starts_with("https://") && !raw.starts_with("http://") {
        return Err("GitLab URL must use HTTP or HTTPS".into());
    }
    let value = if raw.starts_with("https://") || raw.starts_with("http://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let (_, rest) = value
        .split_once("://")
        .ok_or_else(|| "GitLab URL must use HTTP or HTTPS".to_string())?;
    let path = rest.split_once('/').map(|(_, path)| path).unwrap_or("");
    if rest.is_empty()
        || rest.starts_with('/')
        || rest.contains('@')
        || rest.contains('?')
        || rest.contains('#')
        || rest.contains('\\')
        || rest.chars().any(char::is_whitespace)
        || path.split('/').any(|segment| {
            matches!(
                segment.to_ascii_lowercase().as_str(),
                "." | ".." | "%2e" | "%2e%2e" | "%2e." | ".%2e"
            )
        })
    {
        return Err("GitLab URL is invalid".into());
    }
    let normalized = value.trim_end_matches('/');
    let normalized = normalized.strip_suffix("/api/v4").unwrap_or(normalized);
    Ok(normalized.trim_end_matches('/').to_string())
}

fn encode_path_component(value: &str) -> String {
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

fn gitlab_repo_for(root: &Path, gitlab_url: &str) -> Result<String, String> {
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
        if let Some(repo) = project_from_remote(remote.trim(), gitlab_url) {
            matches.push((name == "remote.origin.url", repo));
        }
    }
    matches
        .iter()
        .find(|(origin, _)| *origin)
        .or_else(|| matches.first())
        .map(|(_, repo)| repo.clone())
        .ok_or_else(|| "No GitLab remote matches the configured host".to_string())
}

fn project_from_remote(remote: &str, gitlab_url: &str) -> Option<String> {
    let configured = configured_remote(gitlab_url)?;
    let (authority, mut path) = remote_authority_path(remote)?;
    if host_without_port(&authority) != host_without_port(&configured.authority) {
        return None;
    }
    path = path.trim_matches('/').to_string();
    let prefix = configured.path.trim_matches('/');
    if !prefix.is_empty() {
        path = path.strip_prefix(&format!("{prefix}/"))?.to_string();
    }
    if let Some(stripped) = path.strip_suffix(".git") {
        path = stripped.to_string();
    }
    if !valid_project_path(&path) {
        return None;
    }
    Some(path)
}

struct ConfiguredRemote {
    authority: String,
    path: String,
}

fn configured_remote(url: &str) -> Option<ConfiguredRemote> {
    let normalized = normalize_gitlab_url(url).ok()?;
    let (_, rest) = normalized.split_once("://")?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    Some(ConfiguredRemote {
        authority: authority.to_ascii_lowercase(),
        path: path.to_string(),
    })
}

fn remote_authority_path(remote: &str) -> Option<(String, String)> {
    let remote = remote.trim();
    if let Some((_, rest)) = remote.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let authority = authority.rsplit('@').next()?.to_ascii_lowercase();
        return Some((authority, path.to_string()));
    }
    let (user_host, path) = remote.split_once(':')?;
    let authority = user_host.rsplit('@').next()?.to_ascii_lowercase();
    Some((authority, path.to_string()))
}

fn host_without_port(authority: &str) -> String {
    let authority = authority.trim().to_ascii_lowercase();
    if authority.starts_with('[') {
        return authority
            .split(']')
            .next()
            .map(|host| format!("{host}]"))
            .unwrap_or(authority);
    }
    authority
        .split(':')
        .next()
        .unwrap_or(&authority)
        .to_string()
}

fn valid_project_path(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && !part.chars().any(char::is_whitespace)
                && !part.contains(['?', '#', '\\'])
        })
}

fn read_config(app: &Path) -> Result<Option<GitlabConfig>, String> {
    let Some(raw) = secrets::read(Tracker::Gitlab, app)? else {
        return Ok(None);
    };
    let mut config: GitlabConfig =
        serde_json::from_str(&raw).map_err(|_| "GitLab settings are invalid".to_string())?;
    config.url = normalize_gitlab_url(&config.url)?;
    config.token = config.token.trim().to_string();
    if config.token.is_empty() {
        Ok(None)
    } else {
        Ok(Some(config))
    }
}

fn require_config(app: &Path) -> Result<GitlabConfig, String> {
    read_config(app)?.ok_or_else(|| "Connect GitLab in Settings".to_string())
}

fn write_config(app: &Path, config: &GitlabConfig) -> Result<(), String> {
    let value = serde_json::to_string(config).map_err(|error| error.to_string())?;
    secrets::write(Tracker::Gitlab, app, &value)
}

fn delete_config(app: &Path) -> Result<(), String> {
    secrets::delete(Tracker::Gitlab, app)
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
mod member_access_tests {
    use super::*;

    #[test]
    fn a_members_access_is_their_highest_level_and_a_stranger_has_none() {
        // The shape `/projects/:id/members/all?query=` returns: a search can
        // match more than the user asked for.
        let rows = serde_json::json!([
            {"id": 1, "username": "alice", "access_level": 30},
            {"id": 1, "username": "alice", "access_level": 40},
            {"id": 2, "username": "alice-bot", "access_level": 50}
        ]);
        assert_eq!(parse_member_access(&rows, "Alice").unwrap(), 40);
        assert_eq!(parse_member_access(&rows, "mallory").unwrap(), 0);
        assert!(parse_member_access(&serde_json::json!({}), "alice").is_err());
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn approvals_and_reviewer_states_make_the_verdict() {
        let mr = json!({"detailed_merge_status": "mergeable",
            "head_pipeline": {"id": 7, "status": "success"}});
        let approvals = json!({"approved": true, "approvals_left": 0,
            "approved_by": [{"user": {"username": "mira"}}]});
        let reviewers = json!([
            {"user": {"username": "mira"}, "state": "reviewed"},
            {"user": {"username": "jo"}, "state": "unreviewed"},
            {"user": {"username": ""}, "state": "approved"}
        ]);
        let review = mr_review_from(&mr, Some(&approvals), Some(&reviewers), None);
        assert_eq!(review.review_decision, "approved");
        assert_eq!(
            review.reviews,
            [Review {
                login: "mira".into(),
                state: "approved".into()
            }]
        );
        assert_eq!(review.requested_reviewers, Some(vec!["jo".to_string()]));
        // Without jobs, the head pipeline is one check.
        assert_eq!(
            (review.check_summary.total, review.check_summary.passed),
            (1, 1)
        );
    }

    #[test]
    fn a_request_for_changes_wins_over_approvals() {
        let mr = json!({"detailed_merge_status": "requested_changes"});
        let reviewers = json!([{"user": {"username": "mira"}, "state": "requested_changes"}]);
        let approvals = json!({"approved": true, "approvals_left": 0,
            "approved_by": [{"user": {"username": "cubic"}}]});
        let review = mr_review_from(&mr, Some(&approvals), Some(&reviewers), None);
        assert_eq!(review.review_decision, "changes_requested");
        let states: Vec<(&str, &str)> = review
            .reviews
            .iter()
            .map(|r| (r.login.as_str(), r.state.as_str()))
            .collect();
        assert_eq!(
            states,
            [("mira", "changes_requested"), ("cubic", "approved")]
        );
    }

    #[test]
    fn free_tier_approved_without_approvers_still_needs_review() {
        // GitLab Free says `approved` when no rule needs anyone.
        let approvals = json!({"approved": true, "approvals_left": 0, "approved_by": []});
        let none = mr_review_from(&json!({}), Some(&approvals), None, None);
        assert_eq!(none.review_decision, "");
        assert_eq!(none.requested_reviewers, None);
        let required = mr_review_from(
            &json!({"detailed_merge_status": "not_approved"}),
            Some(&json!({"approved": false, "approvals_left": 1, "approved_by": []})),
            None,
            None,
        );
        assert_eq!(required.review_decision, "review_required");
        assert!(required.check_summary.is_empty());
    }

    #[test]
    fn jobs_count_with_allowed_failures_passing() {
        let jobs = json!([
            {"name": "lint", "status": "success"},
            {"name": "windows", "status": "failed"},
            {"name": "flaky", "status": "failed", "allow_failure": true},
            {"name": "deploy", "status": "manual"},
            {"name": "e2e", "status": "running"},
            {"name": "macos", "status": "canceled"}
        ]);
        let summary = mr_checks(&json!({"head_pipeline": {"status": "failed"}}), Some(&jobs));
        assert_eq!(
            (
                summary.total,
                summary.passed,
                summary.failed,
                summary.pending
            ),
            (6, 3, 2, 1)
        );
        assert_eq!(summary.failed_names, ["windows", "macos"]);
    }

    #[test]
    fn list_rows_carry_author_and_reviewers_and_reviews_override_them() {
        let row = json!({"iid": 3, "title": "Fix", "state": "opened",
            "author": {"username": "vasanth"},
            "reviewers": [{"username": "mira"}, {"username": "jo"}]});
        let mut item = parse_work_item(&row, "pr", "grp/app").unwrap();
        assert_eq!(item.author, "vasanth");
        assert_eq!(item.requested_reviewers, ["mira", "jo"]);
        // Without reviewer states, whoever reviewed is no longer waited on.
        MrReview {
            reviews: vec![Review {
                login: "Mira".into(),
                state: "approved".into(),
            }],
            ..Default::default()
        }
        .apply(&mut item);
        assert_eq!(item.requested_reviewers, ["jo"]);
        let issue = parse_work_item(&row, "issue", "grp/app").unwrap();
        assert!(issue.requested_reviewers.is_empty());
    }

    #[test]
    fn review_requests_name_their_project_from_references() {
        let rows = json!([
            {"iid": 12, "title": "A", "state": "opened",
             "references": {"full": "grp/sub/app!12"}},
            {"iid": 13, "title": "B", "state": "opened"},
            {"iid": 14, "title": "C", "state": "opened",
             "references": {"full": "../x!14"}}
        ]);
        let items = parse_review_requests(&rows).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].repo, "grp/sub/app");
        assert_eq!(items[0].attention_reason, "review_requested");
        assert!(parse_review_requests(&json!({})).is_err());
    }

    #[test]
    fn repo_hits_and_rerun_notices() {
        let hits = parse_repo_hits(&json!([
            {"path_with_namespace": "grp/app", "description": "The app"},
            {"path_with_namespace": "grp/lib", "description": null},
            {"name": "no path"}
        ]))
        .unwrap();
        assert_eq!(
            hits,
            [
                ("grp/app".to_string(), "The app".to_string()),
                ("grp/lib".to_string(), String::new())
            ]
        );
        assert_eq!(rerun_notice(1, "job"), "Rerun requested for 1 failed job.");
        assert_eq!(rerun_notice(2, "job"), "Rerun requested for 2 failed jobs.");
    }
}

#[cfg(test)]
mod reply_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discussions_become_threads_with_their_replies_and_leave_system_notes_out() {
        let value = json!([
            {"id": "d-system", "individual_note": true, "notes": [
                {"id": 1, "system": true, "body": "added 1 commit", "author": {"username": "dana"}}
            ]},
            {"id": "d-general", "individual_note": true, "notes": [
                {"id": 2, "body": "Looks good", "created_at": "2026-11-20T10:00:00Z",
                 "author": {"username": "lee", "avatar_url": "https://gitlab.com/lee.png"}}
            ]},
            {"id": "d-review", "individual_note": false, "notes": [
                {"id": 3, "body": "Index this?", "created_at": "2026-11-20T11:00:00Z",
                 "author": {"username": "marko"}, "resolvable": true, "resolved": true,
                 "position": {"new_path": "src/a.rs", "new_line": 14}},
                {"id": 4, "body": "Done", "created_at": "2026-11-20T12:00:00Z",
                 "author": {"username": "renovate-bot", "bot": true}, "resolvable": true, "resolved": false}
            ]}
        ]);
        let thread =
            parse_discussions(&value, "https://gitlab.com", "acme/web", "pr", 7, false).unwrap();
        assert_eq!(thread.comments.len(), 2, "the system note is left out");
        let general = &thread.comments[0];
        assert_eq!(
            general.thread_id, "d-general",
            "even a lone note takes replies"
        );
        assert!(!general.resolvable && general.path.is_empty());
        let review = &thread.comments[1];
        assert_eq!(review.thread_id, "d-review");
        assert_eq!((review.path.as_str(), review.line), ("src/a.rs", Some(14)));
        assert!(review.resolvable);
        assert!(!review.resolved, "one open note keeps the thread open");
        assert_eq!(review.replies.len(), 1);
        assert!(review.replies[0].author_bot);
        assert_eq!(
            review.url,
            "https://gitlab.com/acme/web/-/merge_requests/7#note_3"
        );
    }

    #[test]
    fn replies_and_resolving_go_to_the_discussion() {
        assert_eq!(
            reply_path("group/sub/web", "pr", 7, "abc123"),
            "/projects/group%2Fsub%2Fweb/merge_requests/7/discussions/abc123/notes"
        );
        assert_eq!(
            resolve_path("acme/web", "pr", 7, "abc123", true),
            "/projects/acme%2Fweb/merge_requests/7/discussions/abc123?resolved=true"
        );
        assert_eq!(
            resolve_path("acme/web", "issue", 3, "abc123", false),
            "/projects/acme%2Fweb/issues/3/discussions/abc123?resolved=false"
        );
    }
}
