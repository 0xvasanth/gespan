//! GitHub through `gh`. A project on another machine is asked on that
//! machine, with its account, through the remote host's `git_github_*`
//! commands; everything else runs the local `gh`.
use super::{MergedPullRequest, OpenPullRequest, PullRequestHost};
use crate::files::remote as remote_workspace;
use crate::inbox::{
    self, github as gh, review::TtlCache, Checks, Details, Diff, Filters, Item, LineComment,
    Status, Thread, Verdict,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The `gh --search` that lists only pull requests asking for your review.
pub const REVIEW_REQUESTED_SEARCH: &str = "review-requested:@me";

pub(super) struct GitHub {
    /// Where `gh` runs when a call names no project.
    cwd: PathBuf,
}

impl GitHub {
    pub(super) fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }

    /// The project an item belongs to, or the default folder.
    fn project<'a>(&'a self, item: &'a Item) -> &'a Path {
        if item.project_path.is_empty() {
            &self.cwd
        } else {
            Path::new(&item.project_path)
        }
    }
}

/// Runs a GitHub operation with the project's own `gh`: a remote project's
/// checkout and account are on its machine.
pub(crate) fn call<T: DeserializeOwned, L: Serialize>(
    project: &Path,
    command: &str,
    mut args: Value,
    local: impl FnOnce() -> Result<L, String>,
) -> Result<T, String> {
    if remote_workspace::is_remote(project) {
        args["cwd"] = json!(project);
        return remote_workspace::command(command, args);
    }
    inbox::convert(local()?)
}

/// The login of the `gh` account on a remote project's machine, cached per
/// machine for a few minutes; empty when its host can't say.
fn remote_login(project: &Path) -> String {
    static LOGINS: TtlCache<String> =
        TtlCache::new(Duration::from_secs(300), Duration::from_secs(120));
    LOGINS
        .get(inbox::github_account(project), || {
            remote_workspace::command::<String>("git_github_login", json!({"cwd": project}))
        })
        .unwrap_or_default()
}

/// A link in a remote session, read by its machine's account. A machine
/// that can't answer leaves it to the local `gh`, which still reads public
/// links.
fn remote_link<T: DeserializeOwned>(
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

/// What `gh` searches for `kind` under `filters`, or `None` when nothing of
/// that kind can match: review requests are never issues.
pub(crate) fn search_for(filters: &Filters, kind: &str) -> Option<&'static str> {
    match (filters.review_requested, kind) {
        (true, "pr") => Some(REVIEW_REQUESTED_SEARCH),
        (true, _) => None,
        (false, _) => Some(""),
    }
}

/// `project` on this machine, or why the call can't reach it.
fn local(project: &Path) -> Result<&Path, String> {
    if remote_workspace::is_remote(project) {
        return Err(format!(
            "{} is on another machine: only its comments are watched",
            project.display()
        ));
    }
    Ok(project)
}

fn with_provider(mut items: Vec<Item>) -> Vec<Item> {
    for item in &mut items {
        item.provider = "github".into();
    }
    items
}

impl PullRequestHost for GitHub {
    fn provider(&self) -> &'static str {
        "github"
    }

    fn status(&self) -> Result<Status, String> {
        let installed = crate::scm::resolve_gui_binary("gh").is_some();
        let authenticated = installed
            && inbox::run_gh(
                &inbox::gh_root(&self.cwd),
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

    /// `gh` keeps its own sign-in: there is nothing to save.
    fn connect(&self, _url: String, _token: String) -> Result<Status, String> {
        self.status()
    }

    fn current_user(&self, project: &Path) -> Result<String, String> {
        gh::git_github_login_for(&inbox::gh_root(local(project)?))
    }

    fn permission(&self, item: &Item, author: &str) -> Result<String, String> {
        let project = local(Path::new(&item.project_path))?;
        gh::git_github_permission_for(&inbox::gh_root(project), &item.repo, author)
    }

    fn repositories(&self, project: &Path) -> Result<Vec<String>, String> {
        call(project, "git_github_repositories", json!({}), || {
            gh::git_github_repositories_for(project)
        })
    }

    fn search_repos(&self, query: &str) -> Result<Vec<(String, String)>, String> {
        gh::git_github_search_repos_for(&inbox::gh_root(&self.cwd), query)
    }

    fn work_items(
        &self,
        project: &Path,
        repo: &str,
        kind: &str,
        filters: &Filters,
        state: &str,
        limit: u32,
    ) -> Result<Vec<Item>, String> {
        let Some(search) = search_for(filters, kind) else {
            return Ok(vec![]);
        };
        let assigned_to_me = filters.assigned_to_me;
        let mut items: Vec<Item> = call(
            project,
            "git_github_work_items",
            json!({
                "repo": repo,
                "kind": kind,
                "assignedToMe": assigned_to_me,
                "state": state,
                "search": search,
                "limit": limit,
            }),
            || {
                gh::git_github_work_items_for(
                    project,
                    repo,
                    kind,
                    assigned_to_me,
                    state,
                    search,
                    limit,
                )
            },
        )?;
        // Another machine signs in with its own account; a host that can't
        // say which leaves `viewer` empty rather than failing the list.
        if kind == "pr"
            && remote_workspace::is_remote(project)
            && items.iter().any(|i| i.viewer.is_empty())
        {
            let viewer = remote_login(project);
            for item in items.iter_mut().filter(|i| i.viewer.is_empty()) {
                item.viewer = viewer.clone();
            }
        }
        Ok(items)
    }

    /// GitHub lists what's assigned to you per repository.
    fn assigned(&self, _kind: &str, _limit: u32) -> Option<Result<Vec<Item>, String>> {
        None
    }

    fn repo_pull_requests(&self, repo: &str) -> Result<Vec<Item>, String> {
        let rows = gh::git_github_work_items_for(
            &inbox::gh_root(&self.cwd),
            repo,
            "pr",
            false,
            "open",
            "",
            40,
        )?;
        inbox::convert(rows).map(with_provider)
    }

    fn review_requests(&self) -> Result<Vec<Item>, String> {
        gh::git_github_review_requests_for(&inbox::gh_root(&self.cwd))
            .and_then(inbox::convert)
            .map(with_provider)
    }

    fn details(&self, item: &Item) -> Result<Details, String> {
        let project = self.project(item);
        call(
            project,
            "git_github_work_item_details",
            json!({"repo": item.repo, "kind": item.kind, "number": item.number}),
            || gh::git_github_work_item_details_for(project, &item.repo, &item.kind, item.number),
        )
    }

    fn thread(&self, item: &Item) -> Result<Thread, String> {
        let project = self.project(item);
        call(
            project,
            "git_github_work_item_thread",
            json!({"repo": item.repo, "kind": item.kind, "number": item.number}),
            || gh::git_github_work_item_thread_for(project, &item.repo, &item.kind, item.number),
        )
    }

    fn comment(&self, item: &Item, body: String, reply_to: String) -> Result<String, String> {
        let project = self.project(item);
        call(
            project,
            "git_github_work_item_comment",
            json!({
                "repo": item.repo,
                "kind": item.kind,
                "number": item.number,
                "body": body,
                "inReplyTo": reply_to,
            }),
            || {
                gh::git_github_work_item_comment_for(
                    project,
                    &item.repo,
                    &item.kind,
                    item.number,
                    &body,
                    &reply_to,
                )
            },
        )
    }

    fn resolve_thread(&self, item: &Item, thread_id: &str, resolved: bool) -> Result<(), String> {
        if item.kind != "pr" {
            return Err("This host has no resolving".into());
        }
        let project = self.project(item);
        if remote_workspace::is_remote(project) {
            return Err("Resolve this thread on GitHub: the project is on another machine".into());
        }
        gh::git_github_resolve_thread_for(&inbox::gh_root(project), thread_id, resolved)
    }

    fn diff(&self, item: &Item, full_context: bool) -> Result<Diff, String> {
        let project = self.project(item);
        call(
            project,
            "git_github_pr_diff",
            json!({"repo": item.repo, "number": item.number, "fullContext": full_context}),
            || gh::git_github_pr_diff_for(project, &item.repo, item.number, full_context),
        )
    }

    fn checks(&self, item: &Item) -> Result<Checks, String> {
        let project = self.project(item);
        call(
            project,
            "git_github_pr_checks",
            json!({"repo": item.repo, "number": item.number}),
            || gh::git_github_pr_checks_for(project, &item.repo, item.number),
        )
    }

    fn action(&self, item: &Item, action: &str) -> Result<Item, String> {
        let project = self.project(item);
        let mut updated: Item = call(
            project,
            "git_github_pr_action",
            json!({"repo": item.repo, "number": item.number, "action": action}),
            || gh::git_github_pr_action_for(project, &item.repo, item.number, action),
        )?;
        updated.provider = item.provider.clone();
        updated.project_path = item.project_path.clone();
        Ok(updated)
    }

    fn submit_review(
        &self,
        item: &Item,
        verdict: Verdict,
        body: &str,
        comments: &[LineComment],
    ) -> Result<(), String> {
        let project = self.project(item);
        let root = inbox::gh_root(project);
        call::<Value, _>(
            project,
            "git_github_pr_review",
            json!({
                "repo": item.repo,
                "number": item.number,
                "verdict": verdict.as_str(),
                "body": body,
                "comments": comments,
            }),
            || {
                gh::git_github_pr_review_for(
                    &root,
                    &item.repo,
                    item.number,
                    verdict,
                    body,
                    comments,
                )
                .map(|()| Value::Null)
            },
        )
        .map(drop)
    }

    fn linked_item(&self, cwd: &Path, repo: &str, kind: &str, number: i64) -> Result<Item, String> {
        let mut item: Item = match remote_link(cwd, "git_github_work_item", repo, kind, number) {
            Some(item) => item,
            None => inbox::convert(gh::git_github_work_item_for(
                &inbox::gh_root(cwd),
                repo,
                kind,
                number,
            )?)?,
        };
        item.provider = "github".into();
        Ok(item)
    }

    fn linked_details(
        &self,
        cwd: &Path,
        repo: &str,
        kind: &str,
        number: i64,
    ) -> Result<Details, String> {
        match remote_link(cwd, "git_github_work_item_details", repo, kind, number) {
            Some(details) => Ok(details),
            None => inbox::convert(gh::git_github_work_item_details_for(
                &inbox::gh_root(cwd),
                repo,
                kind,
                number,
            )?),
        }
    }

    fn open_pull_requests(
        &self,
        project: &Path,
        repo: &str,
        review_requested: bool,
        limit: u32,
    ) -> Result<Vec<OpenPullRequest>, String> {
        let root = inbox::gh_root(local(project)?);
        let search = if review_requested {
            REVIEW_REQUESTED_SEARCH
        } else {
            ""
        };
        gh::git_github_pr_states_for(&root, repo, "open", search, limit)?
            .into_iter()
            .map(|pr| {
                Ok((
                    inbox::convert(pr.item)?,
                    Checks {
                        head_oid: pr.head_oid,
                        checks: inbox::convert(pr.checks)?,
                    },
                ))
            })
            .collect()
    }

    fn merged_pull_requests(
        &self,
        project: &Path,
        repo: &str,
        limit: u32,
    ) -> Result<Vec<MergedPullRequest>, String> {
        let root = inbox::gh_root(local(project)?);
        gh::git_github_pr_states_for(&root, repo, "merged", "", limit)?
            .into_iter()
            .map(|pr| Ok((inbox::convert(pr.item)?, pr.merged_at, pr.merged_by)))
            .collect()
    }
}

/// Issues assigned to you in `repo`, for automations.
pub(crate) fn assigned_issues(project: &Path, repo: &str, limit: u32) -> Result<Vec<Item>, String> {
    let root = inbox::gh_root(local(project)?);
    inbox::convert(gh::git_github_work_items_for(
        &root, repo, "issue", true, "open", "", limit,
    )?)
}
