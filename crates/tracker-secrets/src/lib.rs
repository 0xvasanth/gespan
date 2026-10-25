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

const KEYCHAIN_HINT: &str = "Check that the keychain is unlocked and Gespan may use it; on Linux, a Secret Service such as GNOME Keyring or KWallet must be running.";

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

/// The apps build the data directory in different ways (Tauri's path API,
/// Native's environment lookups or a `--gespan-db` argument), so the account
/// uses the resolved directory: two spellings of one directory share an entry.
pub fn account(tracker: Tracker, data_dir: &Path) -> String {
    let resolved = fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    format!("{}:{}", tracker.key(), resolved.display())
}

/// The saved secret. An old file is moved into the keychain on first read; if
/// the keychain cannot be read or refuses the write, the file stays and its
/// value is still returned, so a locked keychain never disconnects a tracker
/// that has not migrated yet, or loses its secret.
pub fn read(tracker: Tracker, data_dir: &Path) -> Result<Option<String>, String> {
    let entry = entry(tracker, data_dir)?;
    match entry.get_password() {
        Ok(value) => {
            discard_legacy_file(tracker, data_dir);
            Ok(Some(value))
        }
        Err(keyring::Error::NoEntry) => {
            let Some(value) = read_legacy_file(tracker, data_dir)? else {
                return Ok(None);
            };
            if entry.set_password(&value).is_ok() {
                discard_legacy_file(tracker, data_dir);
            }
            Ok(Some(value))
        }
        // A locked or unavailable keychain: an unmigrated file still serves,
        // and stays until the keychain can take it. Without one, the tracker
        // reads as not connected; most people with no usable keychain never
        // connected it, and an error here would fail every Inbox refresh.
        Err(_) => read_legacy_file(tracker, data_dir),
    }
}

pub fn write(tracker: Tracker, data_dir: &Path, value: &str) -> Result<(), String> {
    entry(tracker, data_dir)?.set_password(value).map_err(|_| {
        format!(
            "Could not save the {} credentials to the keychain. {KEYCHAIN_HINT}",
            tracker.label()
        )
    })?;
    discard_legacy_file(tracker, data_dir);
    Ok(())
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

/// Once the keychain holds the secret, the old file is only a leftover: a
/// failed removal is tried again on the next read rather than failing this one.
fn discard_legacy_file(tracker: Tracker, data_dir: &Path) {
    let _ = remove_legacy_file(tracker, data_dir);
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
