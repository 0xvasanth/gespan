use crate::inbox::{Checks, Details, Diff, Filters, Item, LineComment, Status, Thread, Verdict};
use std::path::Path;

/// An open pull request with its head commit's checks.
pub type OpenPullRequest = (Item, Checks);
/// A merged pull request, when it merged and who merged it.
pub type MergedPullRequest = (Item, String, String);

/// One git host's pull requests and issues. Rows come back with their
/// `provider` set; a call a host can't make fails with a sentence the app can
/// show.
pub trait PullRequestHost {
    /// `github`, `gitlab` or `azuredevops`.
    fn provider(&self) -> &'static str;

    /// Whether the host is set up and signed in.
    fn status(&self) -> Result<Status, String>;
    /// Saves `url` and `token` for the host after checking them.
    fn connect(&self, url: String, token: String) -> Result<Status, String>;
    /// The signed-in account's login, as seen from `project`.
    fn current_user(&self, project: &Path) -> Result<String, String>;
    /// What `author` may do in `item`'s repository: `admin`, `maintain`,
    /// `write`, `read` or `none`.
    fn permission(&self, item: &Item, author: &str) -> Result<String, String>;

    /// The repositories `project`'s checkout points at, its own first.
    fn repositories(&self, project: &Path) -> Result<Vec<String>, String>;
    /// Repositories the account can reach whose name matches `query`, with
    /// their descriptions.
    fn search_repos(&self, query: &str) -> Result<Vec<(String, String)>, String>;

    /// `kind` (`issue` or `pr`) rows of `repo`, as `project` sees them.
    fn work_items(
        &self,
        project: &Path,
        repo: &str,
        kind: &str,
        filters: &Filters,
        state: &str,
        limit: u32,
    ) -> Result<Vec<Item>, String>;
    /// `kind` rows assigned to you anywhere, or `None` when the host lists
    /// them per repository instead.
    fn assigned(&self, kind: &str, limit: u32) -> Option<Result<Vec<Item>, String>>;
    /// The open pull requests of a repository no project points at.
    fn repo_pull_requests(&self, repo: &str) -> Result<Vec<Item>, String>;
    /// Open pull requests anywhere that ask for your review.
    fn review_requests(&self) -> Result<Vec<Item>, String>;

    /// The body, branches and review state.
    fn details(&self, item: &Item) -> Result<Details, String>;
    /// Comments, reviews, review threads and commits.
    fn thread(&self, item: &Item) -> Result<Thread, String>;
    /// Posts `body`, as a reply to `reply_to` when it isn't empty, and
    /// returns the new comment's URL.
    fn comment(&self, item: &Item, body: String, reply_to: String) -> Result<String, String>;
    /// Marks a review thread resolved, or open again.
    fn resolve_thread(&self, item: &Item, thread_id: &str, resolved: bool) -> Result<(), String>;

    /// The pull request's files and patch; `full_context` asks for whole
    /// files where the host can.
    fn diff(&self, item: &Item, full_context: bool) -> Result<Diff, String>;
    /// The head commit's checks.
    fn checks(&self, item: &Item) -> Result<Checks, String> {
        let _ = item;
        Err("Checks are not available for this host".into())
    }
    /// Merges, closes, reopens or changes a pull request's draft state.
    fn action(&self, item: &Item, action: &str) -> Result<Item, String> {
        let _ = (item, action);
        Err("This host can't do that from Gespan yet".into())
    }
    /// Sends a review: a verdict, a body, and comments on lines.
    fn submit_review(
        &self,
        item: &Item,
        verdict: Verdict,
        body: &str,
        comments: &[LineComment],
    ) -> Result<(), String>;
    /// Reruns a pull request's failed pipeline jobs, and says what started.
    fn rerun_failed(&self, item: &Item) -> Result<String, String> {
        let _ = item;
        Err("Rerun GitHub checks with their workflow runs".into())
    }

    /// Open pull requests of `repo` with their checks, for automations.
    fn open_pull_requests(
        &self,
        project: &Path,
        repo: &str,
        review_requested: bool,
        limit: u32,
    ) -> Result<Vec<OpenPullRequest>, String> {
        let _ = (project, repo, review_requested, limit);
        Err("This host's pull requests can't start automations yet".into())
    }
    /// Recently merged pull requests of `repo`, for automations.
    fn merged_pull_requests(
        &self,
        project: &Path,
        repo: &str,
        limit: u32,
    ) -> Result<Vec<MergedPullRequest>, String> {
        let _ = (project, repo, limit);
        Err("This host's pull requests can't start automations yet".into())
    }
}
