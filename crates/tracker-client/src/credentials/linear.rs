//! The saved Linear connection. The client lives in `provider_boards::linear`;
//! the Inbox's listing calls live in `gespan_core::inbox::linear`.
use gespan_tracker_secrets::{self as secrets, Tracker};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::provider_boards::linear::LinearProvider;

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

/// The connected Linear client, or `None` when Linear is not set up.
pub fn provider(app: &Path) -> Result<Option<LinearProvider>, String> {
    Ok(read_token(app)?.map(|token| LinearProvider::new(&token)))
}

/// The connected Linear client, or the error asking to connect it.
pub fn require_provider(app: &Path) -> Result<LinearProvider, String> {
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
}
