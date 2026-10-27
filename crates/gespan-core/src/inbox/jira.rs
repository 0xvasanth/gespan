//! Inbox entry points for Jira. The client lives in
//! `provider_boards::jira` and the saved credential in
//! `gespan_tracker_client::credentials::jira`; this file keeps the Inbox's
//! request shapes.
use std::path::PathBuf;

pub use gespan_tracker_client::credentials::jira::{jira_set_config, jira_status, JiraStatus};
use gespan_tracker_client::credentials::jira::{provider, require_provider};

pub use crate::provider_boards::jira::{
    JiraAssignee, JiraIssue, JiraIssueComment, JiraIssueDetails, JiraIssueThread, JiraLabel,
    JiraProject, JiraProvider,
};

const DEFAULT_LIMIT: u32 = 40;

pub fn jira_list_projects(app: PathBuf) -> Result<Vec<JiraProject>, String> {
    require_provider(&app)?.projects()
}

pub fn jira_list_issues(
    app: PathBuf,
    assigned_to_me: bool,
    state: String,
    project_ids: Vec<String>,
    limit: Option<u32>,
) -> Result<Vec<JiraIssue>, String> {
    let Some(provider) = provider(&app)? else {
        return Ok(Vec::new());
    };
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 100);
    provider.inbox_issues(assigned_to_me, &state, &project_ids, limit)
}

pub fn jira_issue_details(app: PathBuf, key: String) -> Result<JiraIssueDetails, String> {
    require_provider(&app)?.issue_details(&key)
}

pub fn jira_issue_thread(app: PathBuf, key: String) -> Result<JiraIssueThread, String> {
    require_provider(&app)?.issue_thread(&key)
}

pub fn jira_issue_comment(app: PathBuf, key: String, body: String) -> Result<String, String> {
    require_provider(&app)?
        .post_comment(&key, &body)
        .map(|comment| comment.url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_tracker_secrets::{self as secrets, testing, Tracker};

    fn connected_dir() -> tempfile::TempDir {
        testing::use_memory_keychain();
        let dir = tempfile::tempdir().unwrap();
        secrets::write(
            Tracker::Jira,
            dir.path(),
            r#"{"site":"acme","email":"ada@example.com","token":"tok"}"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn malformed_issue_key_is_rejected_before_any_request() {
        let dir = connected_dir();
        let app = dir.path().to_path_buf();
        assert_eq!(
            jira_issue_details(app.clone(), "ENG 1".into())
                .err()
                .unwrap(),
            "Missing Jira issue"
        );
        assert_eq!(
            jira_issue_thread(app.clone(), "".into()).err().unwrap(),
            "Missing Jira issue"
        );
        assert_eq!(
            jira_issue_comment(app, "ENG-1".into(), "  ".into()).unwrap_err(),
            "Comment cannot be empty"
        );
    }

    #[test]
    fn without_config_lists_are_empty_and_reads_ask_to_connect() {
        testing::use_memory_keychain();
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().to_path_buf();
        assert!(
            jira_list_issues(app.clone(), true, "open".into(), vec![], None)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            jira_list_projects(app).err().unwrap(),
            "Connect Jira in Settings"
        );
    }
}
