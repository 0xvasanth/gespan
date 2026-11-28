//! The contract every code host meets. A host adapter knows its host's REST
//! endpoints and JSON and nothing else: it sends through whatever
//! [`Transport`](super::Transport) it was given, so the same adapter works
//! over a CLI or a token.
use super::{error::Result, model::*};

pub trait PullRequestHost: Send + Sync {
    fn kind(&self) -> HostKind;
    fn capabilities(&self) -> Capabilities;

    /// The signed-in account's name on the host.
    fn current_user(&self) -> Result<String>;

    /// Repositories the account can see, most recently active first.
    fn repositories(&self) -> Result<Vec<Repository>>;
    fn pull_requests(&self, repo: &str, filter: StateFilter) -> Result<Vec<PullRequest>>;
    fn pull_request(&self, pr: &PrRef) -> Result<PullRequest>;
    /// Open pull requests anywhere that ask the account for a review.
    fn review_requests(&self) -> Result<Vec<PullRequest>>;

    /// Every conversation on the pull request: general comments and review
    /// threads on lines, each with its replies.
    fn threads(&self, pr: &PrRef) -> Result<Vec<Thread>>;
    /// A general comment on the pull request.
    fn add_comment(&self, pr: &PrRef, body: &str) -> Result<Comment>;
    /// A reply in an existing thread, by the id [`threads`](Self::threads)
    /// gave.
    fn reply(&self, pr: &PrRef, thread_id: &str, body: &str) -> Result<Comment>;
    /// New words for a comment, by the id [`threads`](Self::threads) gave.
    fn edit_comment(&self, pr: &PrRef, comment_id: &str, body: &str) -> Result<Comment>;
    /// Marks a review thread resolved, or open again.
    fn resolve_thread(&self, pr: &PrRef, thread_id: &str, resolved: bool) -> Result<()>;

    /// The pull request's commits, as the host lists them.
    fn commits(&self, pr: &PrRef) -> Result<Vec<Commit>>;
    fn files(&self, pr: &PrRef) -> Result<Vec<ChangedFile>>;
    fn reviews(&self, pr: &PrRef) -> Result<Vec<Review>>;
    /// Sends a review. The service has checked it against
    /// [`capabilities`](Self::capabilities) first.
    fn submit_review(&self, pr: &PrRef, review: &ReviewSubmission) -> Result<()>;
    fn checks(&self, pr: &PrRef) -> Result<Vec<Check>>;
}
