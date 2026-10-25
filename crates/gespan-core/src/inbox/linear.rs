//! Inbox entry points for Linear. The client lives in `provider_boards::linear`;
//! this file keeps the Inbox's saved credential and request shapes.
use gespan_tracker_secrets::{self as secrets, Tracker};
use std::path::{Path, PathBuf};

use serde::Serialize;

pub use crate::provider_boards::linear::{
    LinearAssignee, LinearIssue, LinearIssueComment, LinearIssueDetails, LinearIssueThread,
    LinearLabel, LinearProvider, LinearTeam,
};

const DEFAULT_LIMIT: u32 = 40;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearStatus {
    pub connected: bool,
}

pub fn linear_status(app: PathBuf) -> Result<LinearStatus, String> {
    Ok(LinearStatus {
        connected: read_token(&app)?.is_some(),
    })
}

pub fn linear_set_token(app: PathBuf, token: String) -> Result<LinearStatus, String> {
    let trimmed = token.trim().to_string();
    if trimmed.is_empty() {
        delete_token(&app)?;
        return Ok(LinearStatus { connected: false });
    }
    LinearProvider::new(&trimmed).verify()?;
    write_token(&app, &trimmed)?;
    Ok(LinearStatus { connected: true })
}

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
    let Some(token) = read_token(&app)? else {
        return Ok(Vec::new());
    };
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 100);
    LinearProvider::new(&token).inbox_issues(assigned_to_me, &state, &team_ids, limit)
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

/// The connected Linear client, or `None` when Linear is not set up.
pub(crate) fn provider(app: &Path) -> Result<Option<LinearProvider>, String> {
    Ok(read_token(app)?.map(|token| LinearProvider::new(&token)))
}

fn require_provider(app: &Path) -> Result<LinearProvider, String> {
    require_token(app).map(|token| LinearProvider::new(&token))
}

fn read_token(app: &Path) -> Result<Option<String>, String> {
    let Some(raw) = secrets::read(Tracker::Linear, app)? else {
        return Ok(None);
    };
    let token = raw.trim().to_string();
    if token.is_empty() {
        Ok(None)
    } else {
        Ok(Some(token))
    }
}

fn require_token(app: &Path) -> Result<String, String> {
    read_token(app)?.ok_or_else(|| "Connect Linear in Settings".to_string())
}

fn write_token(app: &Path, token: &str) -> Result<(), String> {
    secrets::write(Tracker::Linear, app, token)
}

fn delete_token(app: &Path) -> Result<(), String> {
    secrets::delete(Tracker::Linear, app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_tracker_secrets::testing;

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
    fn empty_token_disconnects() {
        let dir = connected_dir();
        let app = dir.path().to_path_buf();
        assert!(linear_status(app.clone()).unwrap().connected);
        assert!(
            !linear_set_token(app.clone(), "  ".into())
                .unwrap()
                .connected
        );
        assert_eq!(testing::stored(Tracker::Linear, dir.path()), None);
        assert!(!linear_status(app).unwrap().connected);
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
