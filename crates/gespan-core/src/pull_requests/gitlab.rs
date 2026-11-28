//! GitLab's REST API (v4), the same through `glab api` or a token. A merge
//! request is a pull request here; its `iid` is the number.
//!
//! Thread ids are discussion ids and comment ids are note ids, as GitLab
//! gives them.
use super::{
    error::{HostError, Result},
    host::PullRequestHost,
    model::*,
    rest::{array, encode, first_line, number, paged, some_text, text},
    transport::{ApiRequest, Method, Transport},
};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};

pub struct GitLab {
    transport: Box<dyn Transport>,
}

impl GitLab {
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

fn project(repo: &str) -> Result<String> {
    if repo.trim().is_empty() || !repo.contains('/') {
        return Err(HostError::Invalid(format!(
            "\"{repo}\" isn't a GitLab project path"
        )));
    }
    Ok(format!("projects/{}", encode(repo)))
}

fn mr_path(pr: &PrRef) -> Result<String> {
    Ok(format!(
        "{}/merge_requests/{}",
        project(&pr.repo)?,
        pr.number
    ))
}

fn merge_request(value: &Value, repo: &str) -> PullRequest {
    let repo = some_text(&value["references"]["full"])
        .and_then(|full| full.rsplit_once('!').map(|(path, _)| path.to_owned()))
        .unwrap_or_else(|| repo.to_owned());
    PullRequest {
        repo,
        number: value["iid"].as_u64().unwrap_or_default(),
        title: text(&value["title"]),
        body: text(&value["description"]),
        state: match value["state"].as_str().unwrap_or_default() {
            "merged" => PrState::Merged,
            "closed" | "locked" => PrState::Closed,
            _ => PrState::Open,
        },
        draft: value["draft"].as_bool().unwrap_or(false)
            || value["work_in_progress"].as_bool().unwrap_or(false),
        author: text(&value["author"]["username"]),
        source_branch: text(&value["source_branch"]),
        target_branch: text(&value["target_branch"]),
        url: text(&value["web_url"]),
        created_at: text(&value["created_at"]),
        updated_at: text(&value["updated_at"]),
        head_sha: some_text(&value["sha"]),
        reviewers: array(value["reviewers"].clone())
            .iter()
            .filter_map(|r| some_text(&r["username"]))
            .collect(),
    }
}

fn note(value: &Value) -> Comment {
    Comment {
        id: value["id"].to_string(),
        author: text(&value["author"]["username"]),
        body: text(&value["body"]),
        created_at: text(&value["created_at"]),
        updated_at: text(&value["updated_at"]),
        system: value["system"].as_bool().unwrap_or(false),
    }
}

/// Where a note sits, from its `position`.
fn position(value: &Value) -> Option<LineLocation> {
    if value.is_null() {
        return None;
    }
    let (side, line) = match number(&value["new_line"]) {
        Some(line) => (Side::New, line),
        None => (Side::Old, number(&value["old_line"])?),
    };
    let start = &value["line_range"]["start"];
    let start_line = match side {
        Side::New => number(&start["new_line"]),
        Side::Old => number(&start["old_line"]),
    }
    .filter(|start| *start != line);
    Some(LineLocation {
        path: some_text(&value["new_path"]).unwrap_or_else(|| text(&value["old_path"])),
        side,
        line,
        start_line,
    })
}

/// One line of a file's diff: its kind and its old and new positions, as
/// GitLab counts them. An added line keeps the old position of the next old
/// line, a deleted one the new position of the next new line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiffLine {
    pub kind: char,
    pub old: u32,
    pub new: u32,
}

/// The line on `side` numbered `line` in a unified diff.
pub(crate) fn find_line(diff: &str, side: Side, line: u32) -> Option<DiffLine> {
    let (mut old, mut new) = (0u32, 0u32);
    for row in diff.lines() {
        if let Some(header) = row.strip_prefix("@@ ") {
            let mut parts = header.split_whitespace();
            let start = |part: Option<&str>, sign: char| {
                part.and_then(|p| p.strip_prefix(sign))
                    .and_then(|p| p.split(',').next())
                    .and_then(|n| n.parse::<u32>().ok())
            };
            old = start(parts.next(), '-')?;
            new = start(parts.next(), '+')?;
            continue;
        }
        let kind = row.chars().next().unwrap_or(' ');
        let here = DiffLine { kind, old, new };
        match kind {
            '+' => {
                if side == Side::New && new == line {
                    return Some(here);
                }
                new += 1;
            }
            '-' => {
                if side == Side::Old && old == line {
                    return Some(here);
                }
                old += 1;
            }
            '\\' => {}
            _ => {
                let hit = match side {
                    Side::New => new == line,
                    Side::Old => old == line,
                };
                if hit {
                    return Some(here);
                }
                old += 1;
                new += 1;
            }
        }
    }
    None
}

/// GitLab's code for a line: the path's SHA-1, then its old and new
/// positions.
pub(crate) fn line_code(path: &str, line: DiffLine) -> String {
    let digest = Sha1::digest(path.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{hex}_{}_{}", line.old, line.new)
}

/// The old and new line numbers a position names for `line`: an unchanged
/// line has both, an added one only the new, a deleted one only the old.
fn line_numbers(line: DiffLine, target: &mut Value) {
    match line.kind {
        '+' => target["new_line"] = json!(line.new),
        '-' => target["old_line"] = json!(line.old),
        _ => {
            target["old_line"] = json!(line.old);
            target["new_line"] = json!(line.new);
        }
    }
}

fn job_status(status: &str) -> CheckStatus {
    match status {
        "success" => CheckStatus::Success,
        "failed" => CheckStatus::Failure,
        "canceled" | "canceling" => CheckStatus::Cancelled,
        "skipped" | "manual" => CheckStatus::Skipped,
        _ => CheckStatus::Pending,
    }
}

impl PullRequestHost for GitLab {
    fn kind(&self) -> HostKind {
        HostKind::GitLab
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            line_ranges: true,
            // GitLab can approve or not; "request changes" has no REST call.
            request_changes: false,
            file_patches: true,
            edit_comments: true,
            resolve_threads: true,
        }
    }

    fn current_user(&self) -> Result<String> {
        Ok(text(&self.get("user")?["username"]))
    }

    fn repositories(&self) -> Result<Vec<Repository>> {
        Ok(paged(
            &*self.transport,
            "projects?membership=true&simple=true&order_by=last_activity_at",
            array,
        )?
        .iter()
        .map(|row| Repository {
            full_name: text(&row["path_with_namespace"]),
            url: text(&row["web_url"]),
            default_branch: some_text(&row["default_branch"]),
        })
        .collect())
    }

    fn pull_requests(&self, repo: &str, filter: StateFilter) -> Result<Vec<PullRequest>> {
        let state = match filter {
            StateFilter::Open => "opened",
            StateFilter::Closed => "closed",
            StateFilter::Merged => "merged",
            StateFilter::All => "all",
        };
        Ok(paged(
            &*self.transport,
            &format!("{}/merge_requests?state={state}", project(repo)?),
            array,
        )?
        .iter()
        .map(|row| merge_request(row, repo))
        .collect())
    }

    fn pull_request(&self, pr: &PrRef) -> Result<PullRequest> {
        Ok(merge_request(&self.get(&mr_path(pr)?)?, &pr.repo))
    }

    fn review_requests(&self) -> Result<Vec<PullRequest>> {
        let me = self.current_user()?;
        Ok(paged(
            &*self.transport,
            &format!(
                "merge_requests?scope=all&state=opened&reviewer_username={}",
                encode(&me)
            ),
            array,
        )?
        .iter()
        .map(|row| merge_request(row, ""))
        .collect())
    }

    fn threads(&self, pr: &PrRef) -> Result<Vec<Thread>> {
        let rows = paged(
            &*self.transport,
            &format!("{}/discussions", mr_path(pr)?),
            array,
        )?;
        Ok(rows
            .iter()
            .map(|row| {
                let notes = array(row["notes"].clone());
                let resolvable: Vec<&Value> = notes
                    .iter()
                    .filter(|n| n["resolvable"].as_bool().unwrap_or(false))
                    .collect();
                Thread {
                    id: text(&row["id"]),
                    location: notes.first().and_then(|first| position(&first["position"])),
                    resolved: (!resolvable.is_empty()).then(|| {
                        resolvable
                            .iter()
                            .all(|n| n["resolved"].as_bool().unwrap_or(false))
                    }),
                    comments: notes.iter().map(note).collect(),
                }
            })
            .collect())
    }

    fn add_comment(&self, pr: &PrRef, body: &str) -> Result<Comment> {
        Ok(note(&self.send(
            Method::Post,
            &format!("{}/notes", mr_path(pr)?),
            json!({ "body": body }),
        )?))
    }

    fn reply(&self, pr: &PrRef, thread_id: &str, body: &str) -> Result<Comment> {
        Ok(note(&self.send(
            Method::Post,
            &format!("{}/discussions/{}/notes", mr_path(pr)?, encode(thread_id)),
            json!({ "body": body }),
        )?))
    }

    fn edit_comment(&self, pr: &PrRef, comment_id: &str, body: &str) -> Result<Comment> {
        Ok(note(&self.send(
            Method::Put,
            &format!("{}/notes/{}", mr_path(pr)?, encode(comment_id)),
            json!({ "body": body }),
        )?))
    }

    fn resolve_thread(&self, pr: &PrRef, thread_id: &str, resolved: bool) -> Result<()> {
        self.send(
            Method::Put,
            &format!("{}/discussions/{}", mr_path(pr)?, encode(thread_id)),
            json!({ "resolved": resolved }),
        )?;
        Ok(())
    }

    fn commits(&self, pr: &PrRef) -> Result<Vec<Commit>> {
        Ok(paged(
            &*self.transport,
            &format!("{}/commits", mr_path(pr)?),
            array,
        )?
        .iter()
        .map(|row| {
            let message = text(&row["message"]);
            Commit {
                sha: text(&row["id"]),
                title: some_text(&row["title"]).unwrap_or_else(|| first_line(&message)),
                message,
                author: text(&row["author_name"]),
                authored_at: text(&row["authored_date"]),
                url: text(&row["web_url"]),
            }
        })
        .collect())
    }

    fn files(&self, pr: &PrRef) -> Result<Vec<ChangedFile>> {
        Ok(
            paged(&*self.transport, &format!("{}/diffs", mr_path(pr)?), array)?
                .iter()
                .map(|row| {
                    let status = if row["new_file"] == true {
                        FileStatus::Added
                    } else if row["deleted_file"] == true {
                        FileStatus::Deleted
                    } else if row["renamed_file"] == true {
                        FileStatus::Renamed
                    } else {
                        FileStatus::Modified
                    };
                    let patch = some_text(&row["diff"]);
                    let count = |sign: char| {
                        patch.as_ref().map(|diff| {
                            diff.lines()
                                .filter(|l| l.starts_with(sign) && !l.starts_with("@@"))
                                .count() as u32
                        })
                    };
                    ChangedFile {
                        path: text(&row["new_path"]),
                        previous_path: (status == FileStatus::Renamed)
                            .then(|| text(&row["old_path"])),
                        status,
                        additions: count('+'),
                        deletions: count('-'),
                        patch,
                    }
                })
                .collect(),
        )
    }

    fn reviews(&self, pr: &PrRef) -> Result<Vec<Review>> {
        let approvals = self.get(&format!("{}/approvals", mr_path(pr)?))?;
        Ok(array(approvals["approved_by"].clone())
            .iter()
            .map(|row| Review {
                author: text(&row["user"]["username"]),
                state: ReviewState::Approved,
                body: String::new(),
                submitted_at: None,
            })
            .collect())
    }

    fn submit_review(&self, pr: &PrRef, review: &ReviewSubmission) -> Result<()> {
        let path = mr_path(pr)?;
        if !review.comments.is_empty() {
            let mr = self.get(&path)?;
            let refs = &mr["diff_refs"];
            let diffs = paged(&*self.transport, &format!("{path}/diffs"), array)?;
            for draft in &review.comments {
                let location = &draft.location;
                let file = diffs
                    .iter()
                    .find(|f| {
                        f["new_path"] == location.path.as_str()
                            || f["old_path"] == location.path.as_str()
                    })
                    .ok_or_else(|| {
                        HostError::Invalid(format!(
                            "{} isn't changed in this merge request",
                            location.path
                        ))
                    })?;
                let diff = text(&file["diff"]);
                let find = |line: u32| {
                    find_line(&diff, location.side, line).ok_or_else(|| {
                        HostError::Invalid(format!(
                            "line {line} of {} isn't in the diff",
                            location.path
                        ))
                    })
                };
                let end = find(location.line)?;
                let mut position = json!({
                    "position_type": "text",
                    "base_sha": refs["base_sha"],
                    "start_sha": refs["start_sha"],
                    "head_sha": refs["head_sha"],
                    "old_path": file["old_path"],
                    "new_path": file["new_path"],
                });
                line_numbers(end, &mut position);
                if location.is_range() {
                    let start = find(location.start_line.unwrap_or(location.line))?;
                    let kind = match location.side {
                        Side::New => "new",
                        Side::Old => "old",
                    };
                    let point = |line: DiffLine| {
                        let mut point = json!({
                            "line_code": line_code(&text(&file["new_path"]), line),
                            "type": kind,
                        });
                        line_numbers(line, &mut point);
                        point
                    };
                    position["line_range"] = json!({ "start": point(start), "end": point(end) });
                }
                self.send(
                    Method::Post,
                    &format!("{path}/discussions"),
                    json!({ "body": draft.body, "position": position }),
                )?;
            }
        }
        if !review.body.trim().is_empty() {
            self.send(
                Method::Post,
                &format!("{path}/notes"),
                json!({ "body": review.body }),
            )?;
        }
        match review.verdict {
            Verdict::Approve => {
                self.send(Method::Post, &format!("{path}/approve"), json!({}))?;
            }
            Verdict::RequestChanges => {
                return Err(HostError::Unsupported("requesting changes"));
            }
            Verdict::Comment => {}
        }
        Ok(())
    }

    fn checks(&self, pr: &PrRef) -> Result<Vec<Check>> {
        let pipelines = self.get(&format!("{}/pipelines", mr_path(pr)?))?;
        // The newest pipeline is listed first.
        let Some(latest) = pipelines.as_array().and_then(|p| p.first()) else {
            return Ok(Vec::new());
        };
        let jobs = paged(
            &*self.transport,
            &format!("{}/pipelines/{}/jobs", project(&pr.repo)?, latest["id"]),
            array,
        )?;
        Ok(jobs
            .iter()
            .map(|job| Check {
                name: text(&job["name"]),
                status: job_status(job["status"].as_str().unwrap_or_default()),
                url: some_text(&job["web_url"]),
            })
            .collect())
    }
}
