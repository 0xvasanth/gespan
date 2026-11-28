//! GitHub's REST API (v3), the same through `gh api` or a token.
//!
//! Comment ids carry their kind, because GitHub keeps general comments
//! (`issue:<id>`) and line comments (`review:<id>`) in separate APIs. A line
//! thread's id is its first comment's.
use super::{
    error::{HostError, Result},
    host::PullRequestHost,
    model::*,
    rest::{array, first_line, number, paged, some_text, text, with_query},
    transport::{ApiRequest, Method, Transport},
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub struct GitHub {
    transport: Box<dyn Transport>,
}

impl GitHub {
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self { transport }
    }
    fn get(&self, path: &str) -> Result<Value> {
        self.transport.send(&ApiRequest::get(path))?.json()
    }
    fn send(&self, method: Method, path: &str, body: Value) -> Result<Value> {
        self.transport
            .send(&ApiRequest::with_body(method, path, body))?
            .json()
    }
}

fn split_repo(repo: &str) -> Result<(&str, &str)> {
    repo.split_once('/')
        .filter(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'))
        .ok_or_else(|| HostError::Invalid(format!("\"{repo}\" isn't an owner/name repository")))
}

fn pull_request(value: &Value, repo: &str) -> PullRequest {
    let state = if !value["merged_at"].is_null() || value["merged"] == json!(true) {
        PrState::Merged
    } else if value["state"] == "closed" {
        PrState::Closed
    } else {
        PrState::Open
    };
    let repo = some_text(&value["base"]["repo"]["full_name"])
        .or_else(|| {
            // Search results name the repository only by its API URL.
            value["repository_url"]
                .as_str()
                .and_then(|url| url.split("/repos/").nth(1))
                .map(str::to_owned)
        })
        .unwrap_or_else(|| repo.to_owned());
    PullRequest {
        repo,
        number: value["number"].as_u64().unwrap_or_default(),
        title: text(&value["title"]),
        body: text(&value["body"]),
        state,
        draft: value["draft"].as_bool().unwrap_or(false),
        author: text(&value["user"]["login"]),
        source_branch: text(&value["head"]["ref"]),
        target_branch: text(&value["base"]["ref"]),
        url: text(&value["html_url"]),
        created_at: text(&value["created_at"]),
        updated_at: text(&value["updated_at"]),
        head_sha: some_text(&value["head"]["sha"]),
        reviewers: array(value["requested_reviewers"].clone())
            .iter()
            .filter_map(|r| some_text(&r["login"]))
            .collect(),
    }
}

fn comment(value: &Value, kind: &str) -> Comment {
    Comment {
        id: format!("{kind}:{}", value["id"]),
        author: text(&value["user"]["login"]),
        body: text(&value["body"]),
        created_at: text(&value["created_at"]),
        updated_at: text(&value["updated_at"]),
        system: false,
    }
}

fn side(value: &Value) -> Side {
    if value == "LEFT" {
        Side::Old
    } else {
        Side::New
    }
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Old => "LEFT",
        Side::New => "RIGHT",
    }
}

/// `issue:12` or `review:34`: which API a comment lives in, and its number.
fn comment_id(id: &str) -> Result<(&str, u64)> {
    let (kind, number) = id
        .split_once(':')
        .ok_or_else(|| HostError::Invalid(format!("\"{id}\" isn't a GitHub comment id")))?;
    let number = number
        .parse()
        .map_err(|_| HostError::Invalid(format!("\"{id}\" isn't a GitHub comment id")))?;
    match kind {
        "issue" | "review" => Ok((kind, number)),
        _ => Err(HostError::Invalid(format!(
            "\"{id}\" isn't a GitHub comment id"
        ))),
    }
}

fn check_status(status: &str, conclusion: &str) -> CheckStatus {
    if status != "completed" && !status.is_empty() {
        return CheckStatus::Pending;
    }
    match conclusion {
        "success" | "neutral" => CheckStatus::Success,
        "skipped" => CheckStatus::Skipped,
        "cancelled" | "stale" => CheckStatus::Cancelled,
        "" => CheckStatus::Pending,
        _ => CheckStatus::Failure,
    }
}

impl PullRequestHost for GitHub {
    fn kind(&self) -> HostKind {
        HostKind::GitHub
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            line_ranges: true,
            request_changes: true,
            file_patches: true,
            edit_comments: true,
            // Resolving is GraphQL-only on GitHub.
            resolve_threads: false,
        }
    }

    fn current_user(&self) -> Result<String> {
        Ok(text(&self.get("user")?["login"]))
    }

    fn repositories(&self) -> Result<Vec<Repository>> {
        Ok(paged(&*self.transport, "user/repos?sort=updated", array)?
            .iter()
            .map(|repo| Repository {
                full_name: text(&repo["full_name"]),
                url: text(&repo["html_url"]),
                default_branch: some_text(&repo["default_branch"]),
            })
            .collect())
    }

    fn pull_requests(&self, repo: &str, filter: StateFilter) -> Result<Vec<PullRequest>> {
        let (owner, name) = split_repo(repo)?;
        let state = match filter {
            StateFilter::Open => "open",
            StateFilter::Closed | StateFilter::Merged => "closed",
            StateFilter::All => "all",
        };
        let rows = paged(
            &*self.transport,
            &format!("repos/{owner}/{name}/pulls?state={state}"),
            array,
        )?;
        Ok(rows
            .iter()
            .map(|row| pull_request(row, repo))
            .filter(|pr| match filter {
                StateFilter::Closed => pr.state == PrState::Closed,
                StateFilter::Merged => pr.state == PrState::Merged,
                _ => true,
            })
            .collect())
    }

    fn pull_request(&self, pr: &PrRef) -> Result<PullRequest> {
        let (owner, name) = split_repo(&pr.repo)?;
        Ok(pull_request(
            &self.get(&format!("repos/{owner}/{name}/pulls/{}", pr.number))?,
            &pr.repo,
        ))
    }

    fn review_requests(&self) -> Result<Vec<PullRequest>> {
        let rows = paged(
            &*self.transport,
            "search/issues?q=is%3Apr+is%3Aopen+review-requested%3A%40me",
            |page| array(page["items"].clone()),
        )?;
        Ok(rows.iter().map(|row| pull_request(row, "")).collect())
    }

    fn threads(&self, pr: &PrRef) -> Result<Vec<Thread>> {
        let (owner, name) = split_repo(&pr.repo)?;
        let general = paged(
            &*self.transport,
            &format!("repos/{owner}/{name}/issues/{}/comments", pr.number),
            array,
        )?;
        let mut threads: Vec<Thread> = general
            .iter()
            .map(|row| {
                let comment = comment(row, "issue");
                Thread {
                    id: comment.id.clone(),
                    location: None,
                    resolved: None,
                    comments: vec![comment],
                }
            })
            .collect();
        let line_comments = paged(
            &*self.transport,
            &format!("repos/{owner}/{name}/pulls/{}/comments", pr.number),
            array,
        )?;
        // Replies point at the thread's first comment.
        let mut by_root: BTreeMap<u64, Thread> = BTreeMap::new();
        let mut order = Vec::new();
        for row in &line_comments {
            let id = row["id"].as_u64().unwrap_or_default();
            let root = row["in_reply_to_id"].as_u64().unwrap_or(id);
            let thread = by_root.entry(root).or_insert_with(|| {
                order.push(root);
                Thread {
                    id: format!("review:{root}"),
                    location: None,
                    resolved: None,
                    comments: Vec::new(),
                }
            });
            if root == id {
                let line = number(&row["line"]).or_else(|| number(&row["original_line"]));
                thread.location = line.map(|line| LineLocation {
                    path: text(&row["path"]),
                    side: side(&row["side"]),
                    line,
                    start_line: number(&row["start_line"])
                        .or_else(|| number(&row["original_start_line"])),
                });
            }
            thread.comments.push(comment(row, "review"));
        }
        threads.extend(order.into_iter().filter_map(|root| by_root.remove(&root)));
        Ok(threads)
    }

    fn add_comment(&self, pr: &PrRef, body: &str) -> Result<Comment> {
        let (owner, name) = split_repo(&pr.repo)?;
        let row = self.send(
            Method::Post,
            &format!("repos/{owner}/{name}/issues/{}/comments", pr.number),
            json!({ "body": body }),
        )?;
        Ok(comment(&row, "issue"))
    }

    fn reply(&self, pr: &PrRef, thread_id: &str, body: &str) -> Result<Comment> {
        let (owner, name) = split_repo(&pr.repo)?;
        match comment_id(thread_id)? {
            // General comments aren't threaded on GitHub: a reply is a new one.
            ("issue", _) => self.add_comment(pr, body),
            (_, root) => {
                let row = self.send(
                    Method::Post,
                    &format!(
                        "repos/{owner}/{name}/pulls/{}/comments/{root}/replies",
                        pr.number
                    ),
                    json!({ "body": body }),
                )?;
                Ok(comment(&row, "review"))
            }
        }
    }

    fn edit_comment(&self, pr: &PrRef, comment_id_text: &str, body: &str) -> Result<Comment> {
        let (owner, name) = split_repo(&pr.repo)?;
        let (kind, id) = comment_id(comment_id_text)?;
        let path = match kind {
            "issue" => format!("repos/{owner}/{name}/issues/comments/{id}"),
            _ => format!("repos/{owner}/{name}/pulls/comments/{id}"),
        };
        let row = self.send(Method::Patch, &path, json!({ "body": body }))?;
        Ok(comment(&row, kind))
    }

    fn resolve_thread(&self, _: &PrRef, _: &str, _: bool) -> Result<()> {
        Err(HostError::Unsupported("resolving threads"))
    }

    fn commits(&self, pr: &PrRef) -> Result<Vec<Commit>> {
        let (owner, name) = split_repo(&pr.repo)?;
        let rows = paged(
            &*self.transport,
            &format!("repos/{owner}/{name}/pulls/{}/commits", pr.number),
            array,
        )?;
        Ok(rows
            .iter()
            .map(|row| {
                let message = text(&row["commit"]["message"]);
                Commit {
                    sha: text(&row["sha"]),
                    title: first_line(&message),
                    author: some_text(&row["author"]["login"])
                        .unwrap_or_else(|| text(&row["commit"]["author"]["name"])),
                    authored_at: text(&row["commit"]["author"]["date"]),
                    url: text(&row["html_url"]),
                    message,
                }
            })
            .collect())
    }

    fn files(&self, pr: &PrRef) -> Result<Vec<ChangedFile>> {
        let (owner, name) = split_repo(&pr.repo)?;
        let rows = paged(
            &*self.transport,
            &format!("repos/{owner}/{name}/pulls/{}/files", pr.number),
            array,
        )?;
        Ok(rows
            .iter()
            .map(|row| ChangedFile {
                path: text(&row["filename"]),
                previous_path: some_text(&row["previous_filename"]),
                status: match row["status"].as_str().unwrap_or_default() {
                    "added" => FileStatus::Added,
                    "removed" => FileStatus::Deleted,
                    "renamed" => FileStatus::Renamed,
                    _ => FileStatus::Modified,
                },
                additions: number(&row["additions"]),
                deletions: number(&row["deletions"]),
                patch: some_text(&row["patch"]),
            })
            .collect())
    }

    fn reviews(&self, pr: &PrRef) -> Result<Vec<Review>> {
        let (owner, name) = split_repo(&pr.repo)?;
        let rows = paged(
            &*self.transport,
            &format!("repos/{owner}/{name}/pulls/{}/reviews", pr.number),
            array,
        )?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                let state = match row["state"].as_str().unwrap_or_default() {
                    "APPROVED" => ReviewState::Approved,
                    "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
                    "COMMENTED" => ReviewState::Commented,
                    "DISMISSED" => ReviewState::Dismissed,
                    // A pending review is the viewer's own unsent draft.
                    _ => return None,
                };
                Some(Review {
                    author: text(&row["user"]["login"]),
                    state,
                    body: text(&row["body"]),
                    submitted_at: some_text(&row["submitted_at"]),
                })
            })
            .collect())
    }

    fn submit_review(&self, pr: &PrRef, review: &ReviewSubmission) -> Result<()> {
        let (owner, name) = split_repo(&pr.repo)?;
        let comments: Vec<Value> = review
            .comments
            .iter()
            .map(|draft| {
                let location = &draft.location;
                let mut comment = json!({
                    "path": location.path,
                    "body": draft.body,
                    "line": location.line,
                    "side": side_name(location.side),
                });
                if location.is_range() {
                    comment["start_line"] = json!(location.start_line);
                    comment["start_side"] = json!(side_name(location.side));
                }
                comment
            })
            .collect();
        let event = match review.verdict {
            Verdict::Comment => "COMMENT",
            Verdict::Approve => "APPROVE",
            Verdict::RequestChanges => "REQUEST_CHANGES",
        };
        let mut body = json!({ "event": event, "comments": comments });
        if !review.body.trim().is_empty() {
            body["body"] = json!(review.body);
        }
        self.send(
            Method::Post,
            &format!("repos/{owner}/{name}/pulls/{}/reviews", pr.number),
            body,
        )?;
        Ok(())
    }

    fn checks(&self, pr: &PrRef) -> Result<Vec<Check>> {
        let (owner, name) = split_repo(&pr.repo)?;
        let head = self
            .pull_request(pr)?
            .head_sha
            .ok_or_else(|| HostError::Parse("the pull request has no head commit".into()))?;
        let runs = paged(
            &*self.transport,
            &with_query(
                &format!("repos/{owner}/{name}/commits/{head}/check-runs"),
                "filter=latest",
            ),
            |page| array(page["check_runs"].clone()),
        )?;
        let mut checks: Vec<Check> = runs
            .iter()
            .map(|run| Check {
                name: text(&run["name"]),
                status: check_status(
                    run["status"].as_str().unwrap_or_default(),
                    run["conclusion"].as_str().unwrap_or_default(),
                ),
                url: some_text(&run["html_url"]),
            })
            .collect();
        // Older CI reports commit statuses instead of check runs.
        let statuses = self.get(&format!("repos/{owner}/{name}/commits/{head}/status"))?;
        checks.extend(
            array(statuses["statuses"].clone())
                .iter()
                .map(|status| Check {
                    name: text(&status["context"]),
                    status: match status["state"].as_str().unwrap_or_default() {
                        "success" => CheckStatus::Success,
                        "pending" => CheckStatus::Pending,
                        _ => CheckStatus::Failure,
                    },
                    url: some_text(&status["target_url"]),
                }),
        );
        Ok(checks)
    }
}
