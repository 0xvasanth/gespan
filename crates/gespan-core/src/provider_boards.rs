//! External boards (Linear, Jira, and later Plane) behind one `BoardProvider`
//! trait, so the board sync engine and the Inbox share a single client per
//! tracker. Calls block on HTTP: run them on a worker, never on the UI thread.
mod model;

pub use model::*;

/// One tracker connection, built from already-loaded credentials. Storing
/// credentials is the caller's job.
pub trait BoardProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;

    /// The account the credentials belong to.
    fn whoami(&self) -> ProviderResult<Account>;

    /// Proves the credentials work. Same as `whoami` unless a provider needs more.
    fn connect(&self) -> ProviderResult<Account> {
        self.whoami()
    }

    fn list_containers(&self) -> ProviderResult<Vec<Container>>;
    fn list_states(&self, container_id: &str) -> ProviderResult<Vec<WorkflowState>>;
    fn list_labels(&self, container_id: &str) -> ProviderResult<Vec<BoardLabel>>;
    fn list_members(&self, container_id: &str) -> ProviderResult<Vec<Member>>;

    fn list_issues(
        &self,
        filter: &IssueFilter,
        cursor: Option<&str>,
    ) -> ProviderResult<Page<RemoteIssue>>;
    fn get_issue(&self, id: &str) -> ProviderResult<RemoteIssue>;

    fn create_issue(&self, container_id: &str, draft: &IssueDraft) -> ProviderResult<RemoteIssue>;
    fn update_issue(&self, id: &str, patch: &IssuePatch) -> ProviderResult<RemoteIssue>;
    fn move_issue(&self, id: &str, target: &MoveTarget) -> ProviderResult<RemoteIssue>;
    fn comment(
        &self,
        issue_id: &str,
        body: &str,
        reply_to: Option<&str>,
    ) -> ProviderResult<CommentRef>;

    /// Issues in `container_id` changed after `cursor` (everything when `None`),
    /// oldest first.
    fn sync_since(
        &self,
        container_id: &str,
        cursor: Option<&SyncCursor>,
    ) -> ProviderResult<SyncBatch>;

    /// Turns a verified webhook delivery into hints to refetch.
    fn parse_webhook(
        &self,
        headers: &[(String, String)],
        body: &[u8],
    ) -> ProviderResult<Vec<ChangeHint>>;
}

#[cfg(test)]
pub(crate) mod fake;
