//! The saved Plane connection. The client lives in `provider_boards::plane`.
use gespan_tracker_secrets::{self as secrets, Tracker};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::provider_boards::plane::{PlaneConfig, PlaneProvider};
use crate::provider_boards::BoardProvider;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaneStatus {
    pub connected: bool,
    pub base_url: String,
    pub workspace: String,
}

pub fn plane_status(app: PathBuf) -> Result<PlaneStatus, String> {
    Ok(status_for(read_config(&app)?.as_ref()))
}

/// An empty key disconnects. Otherwise the key is verified before it is saved.
pub fn plane_set_config(
    app: PathBuf,
    base_url: String,
    workspace: String,
    key: String,
) -> Result<PlaneStatus, String> {
    let key = key.trim();
    if key.is_empty() {
        secrets::delete(Tracker::Plane, &app)?;
        return Ok(status_for(None));
    }
    let provider = PlaneProvider::new(&base_url, &workspace, key)?;
    provider.whoami().map_err(|error| error.to_string())?;
    let value = serde_json::to_string(provider.config()).map_err(|error| error.to_string())?;
    secrets::write(Tracker::Plane, &app, &value)?;
    Ok(status_for(Some(provider.config())))
}

/// The connected Plane client, or `None` when Plane is not set up.
pub fn provider(app: &Path) -> Result<Option<PlaneProvider>, String> {
    let Some(config) = read_config(app)? else {
        return Ok(None);
    };
    PlaneProvider::new(&config.base_url, &config.workspace, &config.api_key).map(Some)
}

fn status_for(config: Option<&PlaneConfig>) -> PlaneStatus {
    PlaneStatus {
        connected: config.is_some(),
        base_url: config.map(|c| c.base_url.clone()).unwrap_or_default(),
        workspace: config.map(|c| c.workspace.clone()).unwrap_or_default(),
    }
}

fn read_config(app: &Path) -> Result<Option<PlaneConfig>, String> {
    let Some(raw) = secrets::read(Tracker::Plane, app)? else {
        return Ok(None);
    };
    let config: PlaneConfig =
        serde_json::from_str(&raw).map_err(|_| "Plane settings are invalid".to_string())?;
    if config.api_key.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_tracker_secrets::testing;

    #[test]
    fn empty_key_disconnects_and_bad_input_is_rejected_before_any_request() {
        testing::use_memory_keychain();
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().to_path_buf();
        secrets::write(
            Tracker::Plane,
            dir.path(),
            r#"{"base_url":"https://api.plane.so","workspace":"acme","api_key":"k"}"#,
        )
        .unwrap();
        let status = plane_status(app.clone()).unwrap();
        assert!(status.connected);
        assert_eq!(status.workspace, "acme");
        assert!(provider(&app).unwrap().is_some());

        let err = plane_set_config(app.clone(), "ftp://x".into(), "acme".into(), "k".into());
        assert!(err.is_err());
        assert!(plane_status(app.clone()).unwrap().connected);

        let status = plane_set_config(app.clone(), String::new(), String::new(), "  ".into());
        assert!(!status.unwrap().connected);
        assert!(provider(&app).unwrap().is_none());
    }
}
