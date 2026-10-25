//! Tracker secrets (Linear, Jira, GitLab, Azure DevOps, Plane) in the OS
//! keychain, shared by the Tauri app and Gespan Native so both read the same
//! entries.
//!
//! One entry per tracker and data directory: service [`SERVICE`], account
//! `<tracker>:<data dir>`. The value is exactly what the old `0600` file held,
//! so callers keep their parsers. The first read moves an old file into the
//! keychain and deletes it.
use std::{fs, io::ErrorKind, path::Path};

#[cfg(feature = "testing")]
pub mod testing;

pub const SERVICE: &str = "dev.gespan.trackers";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tracker {
    Linear,
    Jira,
    Gitlab,
    AzureDevOps,
    Plane,
}

impl Tracker {
    pub fn key(self) -> &'static str {
        match self {
            Tracker::Linear => "linear",
            Tracker::Jira => "jira",
            Tracker::Gitlab => "gitlab",
            Tracker::AzureDevOps => "azuredevops",
            Tracker::Plane => "plane",
        }
    }

    /// The `0600` file this tracker used before the keychain, if any.
    pub fn legacy_file(self) -> Option<&'static str> {
        match self {
            Tracker::Linear => Some("linear-token"),
            Tracker::Jira => Some("jira-config.json"),
            Tracker::Gitlab => Some("gitlab-config.json"),
            Tracker::AzureDevOps => Some("azure-devops-config.json"),
            Tracker::Plane => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tracker::Linear => "Linear",
            Tracker::Jira => "Jira",
            Tracker::Gitlab => "GitLab",
            Tracker::AzureDevOps => "Azure DevOps",
            Tracker::Plane => "Plane",
        }
    }
}

pub fn account(tracker: Tracker, data_dir: &Path) -> String {
    format!("{}:{}", tracker.key(), data_dir.display())
}

/// The saved secret. An old file is moved into the keychain on first read; if
/// the keychain refuses it, the file stays and its value is still returned, so
/// a locked keychain never disconnects a tracker or loses its secret.
pub fn read(tracker: Tracker, data_dir: &Path) -> Result<Option<String>, String> {
    let entry = entry(tracker, data_dir)?;
    match entry.get_password() {
        Ok(value) => {
            remove_legacy_file(tracker, data_dir)?;
            Ok(Some(value))
        }
        Err(keyring::Error::NoEntry) => {
            let Some(value) = read_legacy_file(tracker, data_dir)? else {
                return Ok(None);
            };
            if entry.set_password(&value).is_ok() {
                remove_legacy_file(tracker, data_dir)?;
            }
            Ok(Some(value))
        }
        // Keychain errors can quote what they were handed; report only which
        // tracker failed.
        Err(_) => Err(format!(
            "Could not read the {} credentials from the keychain",
            tracker.label()
        )),
    }
}

pub fn write(tracker: Tracker, data_dir: &Path, value: &str) -> Result<(), String> {
    entry(tracker, data_dir)?.set_password(value).map_err(|_| {
        format!(
            "Could not save the {} credentials to the keychain",
            tracker.label()
        )
    })?;
    remove_legacy_file(tracker, data_dir)
}

pub fn delete(tracker: Tracker, data_dir: &Path) -> Result<(), String> {
    match entry(tracker, data_dir)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(_) => {
            return Err(format!(
                "Could not remove the {} credentials from the keychain",
                tracker.label()
            ))
        }
    }
    remove_legacy_file(tracker, data_dir)
}

fn entry(tracker: Tracker, data_dir: &Path) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &account(tracker, data_dir))
        .map_err(|_| format!("Could not open the keychain for {}", tracker.label()))
}

fn read_legacy_file(tracker: Tracker, data_dir: &Path) -> Result<Option<String>, String> {
    let Some(name) = tracker.legacy_file() else {
        return Ok(None);
    };
    match fs::read_to_string(data_dir.join(name)) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn remove_legacy_file(tracker: Tracker, data_dir: &Path) -> Result<(), String> {
    let Some(name) = tracker.legacy_file() else {
        return Ok(());
    };
    match fs::remove_file(data_dir.join(name)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}
