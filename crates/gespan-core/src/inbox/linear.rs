//! Inbox entry points for Linear. The client lives in
//! `provider_boards::linear` and the saved credential in
//! `gespan_tracker_client::credentials::linear`; this file keeps the Inbox's
//! request shapes.
use std::path::PathBuf;

pub use gespan_tracker_client::credentials::linear::{
    linear_set_token, linear_status, LinearStatus,
};
use gespan_tracker_client::credentials::linear::{provider, require_provider};

pub use crate::provider_boards::linear::{
    LinearAssignee, LinearIssue, LinearIssueComment, LinearIssueDetails, LinearIssueThread,
    LinearLabel, LinearProvider, LinearTeam,
};

const DEFAULT_LIMIT: u32 = 40;

pub fn linear_list_teams(app: PathBuf) -> Result<Vec<LinearTeam>, String> {
    require_provider(&app)?.teams()
}

pub fn linear_list_issues(
    app: PathBuf,
    assigned_to_me: bool,
    state: String,
    team_ids: Vec<String>,
    limit: Option<u32>,
) -> Result<Vec<LinearIssue>, String> {
    let Some(provider) = provider(&app)? else {
        return Ok(Vec::new());
    };
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 100);
    provider.inbox_issues(assigned_to_me, &state, &team_ids, limit)
}

pub fn linear_issue_details(app: PathBuf, id: String) -> Result<LinearIssueDetails, String> {
    require_provider(&app)?.issue_details(&id)
}

pub fn linear_issue_thread(app: PathBuf, id: String) -> Result<LinearIssueThread, String> {
    require_provider(&app)?.issue_thread(&id)
}

pub fn linear_issue_comment(
    app: PathBuf,
    id: String,
    body: String,
    parent_id: String,
) -> Result<String, String> {
    require_provider(&app)?.post_comment(&id, &body, &parent_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_tracker_secrets::{self as secrets, testing, Tracker};

    fn connected_dir() -> tempfile::TempDir {
        testing::use_memory_keychain();
        let dir = tempfile::tempdir().unwrap();
        secrets::write(Tracker::Linear, dir.path(), "lin_api_test\n").unwrap();
        dir
    }

    #[test]
    fn comment_validation_happens_before_any_request() {
        let dir = connected_dir();
        let app = dir.path().to_path_buf();
        let comment = |id: &str, body: &str, parent: &str| {
            linear_issue_comment(app.clone(), id.into(), body.into(), parent.into()).unwrap_err()
        };
        assert_eq!(comment("bad id!", "Hi", ""), "Missing Linear issue");
        assert_eq!(comment("abc", "   ", ""), "Comment cannot be empty");
        assert_eq!(comment("abc", "Hi", "x y"), "Invalid Linear comment");
        assert_eq!(
            linear_issue_thread(app.clone(), " ".into()).unwrap_err(),
            "Missing Linear issue"
        );
        assert_eq!(
            linear_issue_details(app, "".into()).unwrap_err(),
            "Missing Linear issue"
        );
    }

    #[test]
    fn without_a_token_lists_are_empty_and_reads_ask_to_connect() {
        testing::use_memory_keychain();
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().to_path_buf();
        assert!(
            linear_list_issues(app.clone(), true, "open".into(), vec![], None)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            linear_list_teams(app).unwrap_err(),
            "Connect Linear in Settings"
        );
    }
}
