//! The saved Jira connection. The client lives in `provider_boards::jira`;
//! the Inbox's listing calls live in `gespan_core::inbox::jira`.
use gespan_tracker_secrets::{self as secrets, Tracker};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::provider_boards::jira::{normalize_jira_site, JiraConfig, JiraProvider};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraStatus {
    pub connected: bool,
    pub site: String,
    pub email: String,
}

pub fn jira_status(app: PathBuf) -> Result<JiraStatus, String> {
    let config = read_config(&app)?;
    Ok(status_for(config.as_ref()))
}

pub fn jira_set_config(
    app: PathBuf,
    site: String,
    email: String,
    token: String,
) -> Result<JiraStatus, String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        delete_config(&app)?;
        return Ok(status_for(None));
    }
    let provider = JiraProvider::new(&site, &email, &token)?;
    provider.verify()?;
    write_config(&app, provider.config())?;
    Ok(status_for(Some(provider.config())))
}

/// The connected Jira client, or `None` when Jira is not set up.
pub fn provider(app: &Path) -> Result<Option<JiraProvider>, String> {
    Ok(read_config(app)?.map(JiraProvider::from_config))
}

/// The connected Jira client, or the error asking to connect it.
pub fn require_provider(app: &Path) -> Result<JiraProvider, String> {
    require_config(app).map(JiraProvider::from_config)
}

fn status_for(config: Option<&JiraConfig>) -> JiraStatus {
    JiraStatus {
        connected: config.is_some(),
        site: config.map(|config| config.site.clone()).unwrap_or_default(),
        email: config
            .map(|config| config.email.clone())
            .unwrap_or_default(),
    }
}

// ---- Config storage ----

fn read_config(app: &Path) -> Result<Option<JiraConfig>, String> {
    let Some(raw) = secrets::read(Tracker::Jira, app)? else {
        return Ok(None);
    };
    let mut config: JiraConfig =
        serde_json::from_str(&raw).map_err(|_| "Jira settings are invalid".to_string())?;
    config.site = normalize_jira_site(&config.site)?;
    config.email = config.email.trim().to_string();
    config.token = config.token.trim().to_string();
    if config.token.is_empty() || config.email.is_empty() {
        Ok(None)
    } else {
        Ok(Some(config))
    }
}

fn require_config(app: &Path) -> Result<JiraConfig, String> {
    read_config(app)?.ok_or_else(|| "Connect Jira in Settings".to_string())
}

fn write_config(app: &Path, config: &JiraConfig) -> Result<(), String> {
    let value = serde_json::to_string(config).map_err(|error| error.to_string())?;
    secrets::write(Tracker::Jira, app, &value)
}

fn delete_config(app: &Path) -> Result<(), String> {
    secrets::delete(Tracker::Jira, app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_tracker_secrets::testing;

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
    fn bad_email_and_site_are_rejected_before_any_request() {
        testing::use_memory_keychain();
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().to_path_buf();
        let set = |site: &str, email: &str| {
            jira_set_config(app.clone(), site.into(), email.into(), "tok".into())
                .err()
                .unwrap()
        };
        assert_eq!(
            set("acme", "ada"),
            "Enter the email address of your Atlassian account"
        );
        assert_eq!(
            set("http://acme.atlassian.net", "ada@example.com"),
            "Jira site must use HTTPS"
        );
        assert_eq!(testing::stored(Tracker::Jira, dir.path()), None);
    }

    #[test]
    fn stored_config_is_read_and_empty_token_disconnects() {
        let dir = connected_dir();
        let app = dir.path().to_path_buf();
        let status = jira_status(app.clone()).unwrap();
        assert!(status.connected);
        assert_eq!(status.site, "https://acme.atlassian.net");
        assert_eq!(status.email, "ada@example.com");
        let status = jira_set_config(app.clone(), "".into(), "".into(), " ".into()).unwrap();
        assert!(!status.connected);
        assert_eq!(testing::stored(Tracker::Jira, dir.path()), None);
    }
}
