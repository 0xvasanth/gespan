//! Azure DevOps Repos' REST API (7.1), over HTTPS with a personal access
//! token. The transport's base URL is the organization
//! (`https://dev.azure.com/acme`); a repository is `project/repository`.
//!
//! Thread ids are Azure's thread ids; a comment id is `thread:comment`,
//! because a comment is only addressable through its thread.
use super::{
    error::{HostError, Result},
    host::PullRequestHost,
    model::*,
    rest::{array, encode, first_line, id, number, some_text, text, with_query, MAX_PAGES},
    transport::{ApiRequest, Method, Transport},
};
use serde_json::{json, Value};

const VERSION: &str = "api-version=7.1";
/// Pull requests asked for a page at a time.
const PAGE: usize = 100;

pub struct AzureDevOps {
    transport: Box<dyn Transport>,
    /// `https://dev.azure.com/acme`, for the web links the API leaves out.
    organization_url: String,
}

impl AzureDevOps {
    pub fn new(transport: Box<dyn Transport>, organization_url: &str) -> Self {
        Self {
            transport,
            organization_url: organization_url.trim_end_matches('/').to_owned(),
        }
    }
    fn get(&self, path: &str) -> Result<Value> {
        self.transport
            .send(&ApiRequest::get(with_query(path, VERSION)))?
            .json()
    }
    fn send(&self, method: Method, path: &str, body: Value) -> Result<Value> {
        self.transport
            .send(&ApiRequest::with_body(
                method,
                with_query(path, VERSION),
                body,
            ))?
            .json()
    }
    /// The signed-in account: its id (for votes and reviewer searches) and
    /// its name.
    fn me(&self) -> Result<(String, String)> {
        let data = self
            .transport
            .send(&ApiRequest::get("_apis/connectionData"))?
            .json::<Value>()?;
        let user = &data["authenticatedUser"];
        let id = text(&user["id"]);
        if id.is_empty() {
            return Err(HostError::Unauthorized("no signed-in user".into()));
        }
        let name = some_text(&user["properties"]["Account"]["$value"])
            .unwrap_or_else(|| text(&user["providerDisplayName"]));
        Ok((id, name))
    }
    /// Pull request list pages, `$top` at a time, until one comes back short.
    fn pull_request_pages(&self, path: &str) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for page in 0..MAX_PAGES as usize {
            let rows = array(
                self.get(&with_query(
                    path,
                    &format!("$top={PAGE}&$skip={}", page * PAGE),
                ))?["value"]
                    .clone(),
            );
            let full = rows.len() == PAGE;
            out.extend(rows);
            if !full {
                break;
            }
        }
        Ok(out)
    }
    fn web_repo(&self, repo: &str) -> Result<String> {
        let (project, name) = split_repo(repo)?;
        Ok(format!(
            "{}/{}/_git/{}",
            self.organization_url,
            encode(project),
            encode(name)
        ))
    }
    fn pull_request_from(&self, value: &Value, fallback_repo: &str) -> PullRequest {
        let repo = match (
            some_text(&value["repository"]["project"]["name"]),
            some_text(&value["repository"]["name"]),
        ) {
            (Some(project), Some(name)) => format!("{project}/{name}"),
            _ => fallback_repo.to_owned(),
        };
        let number = value["pullRequestId"].as_u64().unwrap_or_default();
        let url = self
            .web_repo(&repo)
            .map(|base| format!("{base}/pullrequest/{number}"))
            .unwrap_or_default();
        PullRequest {
            number,
            title: text(&value["title"]),
            body: text(&value["description"]),
            state: match value["status"].as_str().unwrap_or_default() {
                "completed" => PrState::Merged,
                "abandoned" => PrState::Closed,
                _ => PrState::Open,
            },
            draft: value["isDraft"].as_bool().unwrap_or(false),
            author: some_text(&value["createdBy"]["uniqueName"])
                .unwrap_or_else(|| text(&value["createdBy"]["displayName"])),
            source_branch: branch(&value["sourceRefName"]),
            target_branch: branch(&value["targetRefName"]),
            url,
            created_at: text(&value["creationDate"]),
            // Azure DevOps has no "updated"; closing is the last change it dates.
            updated_at: some_text(&value["closedDate"])
                .unwrap_or_else(|| text(&value["creationDate"])),
            head_sha: some_text(&value["lastMergeSourceCommit"]["commitId"]),
            reviewers: array(value["reviewers"].clone())
                .iter()
                .filter_map(|r| {
                    some_text(&r["uniqueName"]).or_else(|| some_text(&r["displayName"]))
                })
                .collect(),
            repo,
        }
    }
}

fn split_repo(repo: &str) -> Result<(&str, &str)> {
    repo.split_once('/')
        .filter(|(project, name)| !project.is_empty() && !name.is_empty() && !name.contains('/'))
        .ok_or_else(|| {
            HostError::Invalid(format!(
                "\"{repo}\" isn't a project/repository on Azure DevOps"
            ))
        })
}

/// `{project}/_apis/git/repositories/{repo}/{rest}`.
fn repo_api(repo: &str, rest: &str) -> Result<String> {
    let (project, name) = split_repo(repo)?;
    Ok(format!(
        "{}/_apis/git/repositories/{}/{rest}",
        encode(project),
        encode(name)
    ))
}

fn pr_api(pr: &PrRef, rest: &str) -> Result<String> {
    repo_api(
        &pr.repo,
        &if rest.is_empty() {
            format!("pullRequests/{}", pr.number)
        } else {
            format!("pullRequests/{}/{rest}", pr.number)
        },
    )
}

fn branch(value: &Value) -> String {
    let name = text(value);
    name.strip_prefix("refs/heads/").unwrap_or(&name).to_owned()
}

fn comment(value: &Value, thread: &str) -> Comment {
    Comment {
        id: format!("{thread}:{}", id(&value["id"])),
        author: some_text(&value["author"]["uniqueName"])
            .unwrap_or_else(|| text(&value["author"]["displayName"])),
        body: text(&value["content"]),
        created_at: text(&value["publishedDate"]),
        updated_at: some_text(&value["lastUpdatedDate"])
            .unwrap_or_else(|| text(&value["publishedDate"])),
        system: value["commentType"] == "system",
    }
}

/// A thread's place in a file: the right side is the new file, the left the
/// old.
fn thread_location(context: &Value) -> Option<LineLocation> {
    let path = some_text(&context["filePath"])?;
    let path = path.trim_start_matches('/').to_owned();
    let (side, start, end) = if !context["rightFileEnd"].is_null() {
        (
            Side::New,
            &context["rightFileStart"],
            &context["rightFileEnd"],
        )
    } else {
        (
            Side::Old,
            &context["leftFileStart"],
            &context["leftFileEnd"],
        )
    };
    let line = number(&end["line"])?;
    Some(LineLocation {
        path,
        side,
        line,
        start_line: number(&start["line"]).filter(|start| *start != line),
    })
}

fn split_comment_id(comment_id: &str) -> Result<(&str, &str)> {
    comment_id
        .split_once(':')
        .filter(|(thread, comment)| !thread.is_empty() && !comment.is_empty())
        .ok_or_else(|| {
            HostError::Invalid(format!("\"{comment_id}\" isn't an Azure DevOps comment id"))
        })
}

impl PullRequestHost for AzureDevOps {
    fn kind(&self) -> HostKind {
        HostKind::AzureDevOps
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            line_ranges: true,
            request_changes: true,
            // Changes come as paths only; the patch text needs a diff of its own.
            file_patches: false,
            edit_comments: true,
            resolve_threads: true,
        }
    }

    fn current_user(&self) -> Result<String> {
        Ok(self.me()?.1)
    }

    fn repositories(&self) -> Result<Vec<Repository>> {
        Ok(array(self.get("_apis/git/repositories")?["value"].clone())
            .iter()
            .map(|row| Repository {
                full_name: format!("{}/{}", text(&row["project"]["name"]), text(&row["name"])),
                url: text(&row["webUrl"]),
                default_branch: some_text(&row["defaultBranch"]).map(|b| branch(&json!(b))),
            })
            .collect())
    }

    fn pull_requests(&self, repo: &str, filter: StateFilter) -> Result<Vec<PullRequest>> {
        let status = match filter {
            StateFilter::Open => "active",
            StateFilter::Closed => "abandoned",
            StateFilter::Merged => "completed",
            StateFilter::All => "all",
        };
        Ok(self
            .pull_request_pages(&repo_api(
                repo,
                &format!("pullrequests?searchCriteria.status={status}"),
            )?)?
            .iter()
            .map(|row| self.pull_request_from(row, repo))
            .collect())
    }

    fn pull_request(&self, pr: &PrRef) -> Result<PullRequest> {
        Ok(self.pull_request_from(&self.get(&pr_api(pr, "")?)?, &pr.repo))
    }

    fn review_requests(&self) -> Result<Vec<PullRequest>> {
        let (me, _) = self.me()?;
        let projects = array(self.get("_apis/projects")?["value"].clone());
        let mut out = Vec::new();
        for project in &projects {
            let name = text(&project["name"]);
            let rows = self.pull_request_pages(&format!(
                "{}/_apis/git/pullrequests?searchCriteria.reviewerId={}&searchCriteria.status=active",
                encode(&name),
                encode(&me)
            ))?;
            out.extend(rows.iter().map(|row| self.pull_request_from(row, "")));
        }
        Ok(out)
    }

    fn threads(&self, pr: &PrRef) -> Result<Vec<Thread>> {
        let rows = array(self.get(&pr_api(pr, "threads")?)?["value"].clone());
        Ok(rows
            .iter()
            .filter(|row| row["isDeleted"] != true)
            .map(|row| {
                let thread_id = id(&row["id"]);
                let status = row["status"].as_str().unwrap_or_default();
                Thread {
                    location: thread_location(&row["threadContext"]),
                    resolved: match status {
                        "" | "unknown" => None,
                        "active" | "pending" => Some(false),
                        _ => Some(true),
                    },
                    comments: array(row["comments"].clone())
                        .iter()
                        .filter(|c| c["isDeleted"] != true)
                        .map(|c| comment(c, &thread_id))
                        .collect(),
                    id: thread_id,
                }
            })
            .filter(|thread| !thread.comments.is_empty())
            .collect())
    }

    fn add_comment(&self, pr: &PrRef, body: &str) -> Result<Comment> {
        let thread = self.send(
            Method::Post,
            &pr_api(pr, "threads")?,
            json!({
                "comments": [{ "parentCommentId": 0, "content": body, "commentType": 1 }],
                "status": 1,
            }),
        )?;
        let first = array(thread["comments"].clone())
            .into_iter()
            .next()
            .unwrap_or(Value::Null);
        Ok(comment(&first, &id(&thread["id"])))
    }

    fn reply(&self, pr: &PrRef, thread_id: &str, body: &str) -> Result<Comment> {
        let row = self.send(
            Method::Post,
            &pr_api(pr, &format!("threads/{}/comments", encode(thread_id)))?,
            json!({ "content": body, "parentCommentId": 1, "commentType": 1 }),
        )?;
        Ok(comment(&row, thread_id))
    }

    fn edit_comment(&self, pr: &PrRef, comment_id: &str, body: &str) -> Result<Comment> {
        let (thread, comment_number) = split_comment_id(comment_id)?;
        let row = self.send(
            Method::Patch,
            &pr_api(
                pr,
                &format!(
                    "threads/{}/comments/{}",
                    encode(thread),
                    encode(comment_number)
                ),
            )?,
            json!({ "content": body }),
        )?;
        Ok(comment(&row, thread))
    }

    fn resolve_thread(&self, pr: &PrRef, thread_id: &str, resolved: bool) -> Result<()> {
        self.send(
            Method::Patch,
            &pr_api(pr, &format!("threads/{}", encode(thread_id)))?,
            json!({ "status": if resolved { "fixed" } else { "active" } }),
        )?;
        Ok(())
    }

    fn commits(&self, pr: &PrRef) -> Result<Vec<Commit>> {
        let base = self.web_repo(&pr.repo)?;
        Ok(array(self.get(&pr_api(pr, "commits")?)?["value"].clone())
            .iter()
            .map(|row| {
                let message = text(&row["comment"]);
                let sha = text(&row["commitId"]);
                Commit {
                    title: first_line(&message),
                    message,
                    author: text(&row["author"]["name"]),
                    authored_at: text(&row["author"]["date"]),
                    url: format!("{base}/commit/{sha}"),
                    sha,
                }
            })
            .collect())
    }

    fn files(&self, pr: &PrRef) -> Result<Vec<ChangedFile>> {
        let iterations = array(self.get(&pr_api(pr, "iterations")?)?["value"].clone());
        let Some(latest) = iterations.iter().filter_map(|i| i["id"].as_u64()).max() else {
            return Ok(Vec::new());
        };
        let changes = self.get(&pr_api(pr, &format!("iterations/{latest}/changes"))?)?;
        Ok(array(changes["changeEntries"].clone())
            .iter()
            .filter(|entry| entry["item"]["gitObjectType"] != "tree")
            .map(|entry| {
                let kind = entry["changeType"].as_str().unwrap_or_default();
                let status = if kind.contains("rename") {
                    FileStatus::Renamed
                } else if kind.contains("add") {
                    FileStatus::Added
                } else if kind.contains("delete") {
                    FileStatus::Deleted
                } else {
                    FileStatus::Modified
                };
                ChangedFile {
                    path: text(&entry["item"]["path"])
                        .trim_start_matches('/')
                        .to_owned(),
                    previous_path: some_text(&entry["originalPath"])
                        .map(|p| p.trim_start_matches('/').to_owned()),
                    status,
                    additions: None,
                    deletions: None,
                    patch: None,
                }
            })
            .collect())
    }

    fn reviews(&self, pr: &PrRef) -> Result<Vec<Review>> {
        let value = self.get(&pr_api(pr, "")?)?;
        Ok(array(value["reviewers"].clone())
            .iter()
            .filter_map(|row| {
                let state = match row["vote"].as_i64().unwrap_or_default() {
                    vote if vote >= 5 => ReviewState::Approved,
                    vote if vote <= -5 => ReviewState::ChangesRequested,
                    _ => return None,
                };
                Some(Review {
                    author: some_text(&row["uniqueName"])
                        .unwrap_or_else(|| text(&row["displayName"])),
                    state,
                    body: String::new(),
                    submitted_at: None,
                })
            })
            .collect())
    }

    fn submit_review(&self, pr: &PrRef, review: &ReviewSubmission) -> Result<()> {
        let threads = pr_api(pr, "threads")?;
        for draft in &review.comments {
            let location = &draft.location;
            let start = location.start_line.unwrap_or(location.line);
            let point = |line: u32| json!({ "line": line, "offset": 1 });
            let mut context = json!({ "filePath": format!("/{}", location.path) });
            let (from, to) = match location.side {
                Side::New => ("rightFileStart", "rightFileEnd"),
                Side::Old => ("leftFileStart", "leftFileEnd"),
            };
            context[from] = point(start);
            context[to] = point(location.line);
            self.send(
                Method::Post,
                &threads,
                json!({
                    "comments": [{ "parentCommentId": 0, "content": draft.body, "commentType": 1 }],
                    "status": 1,
                    "threadContext": context,
                }),
            )?;
        }
        if !review.body.trim().is_empty() {
            self.add_comment(pr, &review.body)?;
        }
        let vote = match review.verdict {
            Verdict::Comment => return Ok(()),
            Verdict::Approve => 10,
            // "Waiting for author": changes wanted, not a rejection.
            Verdict::RequestChanges => -5,
        };
        let (me, _) = self.me()?;
        self.send(
            Method::Put,
            &pr_api(pr, &format!("reviewers/{}", encode(&me)))?,
            json!({ "vote": vote }),
        )?;
        Ok(())
    }

    fn checks(&self, pr: &PrRef) -> Result<Vec<Check>> {
        Ok(array(self.get(&pr_api(pr, "statuses")?)?["value"].clone())
            .iter()
            .map(|row| {
                let genre = text(&row["context"]["genre"]);
                let name = text(&row["context"]["name"]);
                Check {
                    name: if genre.is_empty() {
                        name
                    } else {
                        format!("{genre}/{name}")
                    },
                    status: match row["state"].as_str().unwrap_or_default() {
                        "succeeded" => CheckStatus::Success,
                        "failed" | "error" => CheckStatus::Failure,
                        "notApplicable" => CheckStatus::Skipped,
                        _ => CheckStatus::Pending,
                    },
                    url: some_text(&row["targetUrl"]),
                }
            })
            .collect())
    }
}
