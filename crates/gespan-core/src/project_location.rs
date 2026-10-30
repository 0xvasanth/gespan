//! Projects follow a renamed folder, ported from src-tauri/src/fs.rs
//! (`resolve_project_location`) and projectLocation.ts.
//!
//! A rename keeps the folder's filesystem identity, so each project's identity
//! is remembered while its folder exists. When the saved folder is gone, only
//! its former siblings are compared, which keeps the search bounded instead of
//! watching or scanning the disk. Identities use the desktop app's format and
//! its remembered ones are read too, so either app can find a renamed folder.
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::files::path_key;
use crate::scm::expand_home;
use crate::store::Store;

/// The setting holding remembered identities, named like the desktop app's
/// localStorage key and in the same shape: `{pathKey: {path, identity}}`.
pub const SETTING: &str = "gespan.projectLocations";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectLocation {
    pub path: String,
    pub identity: String,
}

/// A project whose folder was found under a new name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    pub from: String,
    pub to: String,
}

/// The folder at `path`, or the sibling that has `identity` when `path` no
/// longer exists. Without a saved identity a missing folder is never guessed,
/// and two siblings with the same identity (a symlink beside the folder) are
/// ambiguous, so neither is chosen.
pub fn resolve(path: &str, identity: Option<&str>) -> Result<Option<ProjectLocation>, String> {
    let expanded = expand_home(Path::new(path));
    if expanded.is_dir() {
        return Ok(Some(ProjectLocation {
            path: path.into(),
            identity: directory_identity(&expanded)?,
        }));
    }
    let Some(identity) = identity.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let Some(parent) = expanded.parent() else {
        return Ok(None);
    };
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let candidates = entries.flatten().filter_map(|entry| {
        let candidate = entry.path();
        if !candidate.is_dir() {
            return None;
        }
        let found = directory_identity(&candidate).ok()?;
        Some((entry.file_name().to_string_lossy().into_owned(), found))
    });
    Ok(
        unique_match(candidates, identity).map(|name| ProjectLocation {
            path: sibling(path, &name),
            identity: identity.into(),
        }),
    )
}

/// The one candidate name with `identity`; none when several share it.
fn unique_match(
    candidates: impl IntoIterator<Item = (String, String)>,
    identity: &str,
) -> Option<String> {
    let mut found = None;
    for (name, candidate) in candidates {
        if candidate != identity {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(name);
    }
    found
}

/// `path` with its last component replaced, keeping its separators so the
/// new path reads like the saved one on every platform.
fn sibling(path: &str, name: &str) -> String {
    let trimmed = path.trim_end_matches(|c| c == '/' || (cfg!(windows) && c == '\\'));
    let cut = trimmed
        .rfind(|c| c == '/' || (cfg!(windows) && c == '\\'))
        .map_or(0, |ix| ix + 1);
    format!("{}{name}", &trimmed[..cut])
}

#[cfg(unix)]
fn directory_identity(path: &Path) -> Result<String, String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    Ok(format!("unix:{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn directory_identity(path: &Path) -> Result<String, String> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    };

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|error| error.to_string())?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    let succeeded = unsafe {
        GetFileInformationByHandle(file.as_raw_handle() as _, std::ptr::addr_of_mut!(info))
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let file_index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Ok(format!(
        "windows:{}:{file_index}",
        info.dwVolumeSerialNumber
    ))
}

/// Projects that live on this machine; remote folders keep their host's path.
fn is_local(path: &str) -> bool {
    let key = path_key(path);
    !(path.is_empty()
        || key == "/"
        || key == "~"
        || (key.len() == 2 && key.ends_with(':'))
        || crate::remote::parse_remote_path(path).is_some())
}

fn read(store: &Store) -> BTreeMap<String, ProjectLocation> {
    let parse = |raw: &str| {
        serde_json::from_str::<BTreeMap<String, serde_json::Value>>(raw)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(key, value)| {
                let location: ProjectLocation = serde_json::from_value(value).ok()?;
                (!location.path.is_empty() && !location.identity.is_empty())
                    .then_some((key, location))
            })
            .collect::<BTreeMap<_, _>>()
    };
    let mut locations = store
        .desktop_preferences()
        .and_then(|preferences| preferences.get(SETTING))
        .map(parse)
        .unwrap_or_default();
    if let Ok(Some(own)) = store.setting(SETTING) {
        locations.extend(parse(&own));
    }
    locations
}

/// Remembers the identity of each project folder that exists and finds the
/// ones that were renamed. A found project's sessions move with it; projects
/// that are missing with no match are left as they are.
pub fn follow_moved_projects(store: &Store, projects: &[String]) -> Vec<Moved> {
    let mut locations = read(store);
    let before = locations.clone();
    let mut moved = Vec::new();
    for project in projects.iter().filter(|path| is_local(path)) {
        let key = path_key(project);
        let saved = locations
            .get(&key)
            .map(|location| location.identity.clone());
        let location = match resolve(project, saved.as_deref()) {
            Ok(Some(location)) => location,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(%error, project = %project, "cannot read a project folder's identity");
                continue;
            }
        };
        let renamed = path_key(&location.path) != key;
        if renamed {
            if let Err(error) = store.rebase_project(project, &location.path) {
                tracing::warn!(%error, "cannot move sessions to a renamed project folder");
                continue;
            }
            locations.remove(&key);
            moved.push(Moved {
                from: project.clone(),
                to: location.path.clone(),
            });
        }
        locations.insert(path_key(&location.path), location);
    }
    if locations != before {
        let saved = serde_json::to_string(&locations).unwrap_or_default();
        if let Err(error) = store.save_setting(SETTING, &saved) {
            tracing::warn!(%error, "cannot remember project folder identities");
        }
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::path::PathBuf;

    fn text(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn desktop_session(store: &Store, id: &str, cwd: &str) {
        store
            .conn()
            .execute(
                "INSERT INTO sessions(id,cwd,harness,model,runtime_mode,title,blocks_json,
                 created_at,updated_at,has_user_message)
                 VALUES(?1,?2,'codex','codex:model-one','auto-accept-edits','Saved chat',
                 '[{\"id\":\"prompt\",\"role\":\"user\",\"text\":\"Hello\"}]',100,200,1)",
                rusqlite::params![id, cwd],
            )
            .unwrap();
    }

    fn cwd_of(store: &Store, id: &str) -> String {
        store
            .conn()
            .query_row("SELECT cwd FROM sessions WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn project(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn a_renamed_folder_is_found_by_its_identity() {
        let parent = tempfile::tempdir().unwrap();
        let original = project(parent.path(), "gespan");
        let renamed = parent.path().join("gespan-personal");
        project(parent.path(), "unrelated");

        let first = resolve(&text(&original), None).unwrap().unwrap();
        assert_eq!(first.path, text(&original));

        std::fs::rename(&original, &renamed).unwrap();
        let resolved = resolve(&text(&original), Some(&first.identity))
            .unwrap()
            .unwrap();
        assert_eq!(resolved.path, text(&renamed));
        assert_eq!(resolved.identity, first.identity);
    }

    #[test]
    fn a_missing_folder_is_not_guessed_without_a_saved_identity() {
        let parent = tempfile::tempdir().unwrap();
        project(parent.path(), "some-project");
        let missing = parent.path().join("old-name");
        assert_eq!(resolve(&text(&missing), None).unwrap(), None);
        assert_eq!(resolve(&text(&missing), Some("")).unwrap(), None);
    }

    #[test]
    fn a_missing_parent_or_an_unknown_identity_finds_nothing() {
        let parent = tempfile::tempdir().unwrap();
        let gone = parent.path().join("gone").join("project");
        assert_eq!(resolve(&text(&gone), Some("unix:1:2")).unwrap(), None);
        let original = project(parent.path(), "project");
        let identity = resolve(&text(&original), None).unwrap().unwrap().identity;
        // Created first so it cannot reuse the removed folder's inode.
        project(parent.path(), "project-2");
        std::fs::remove_dir(&original).unwrap();
        assert_eq!(resolve(&text(&original), Some(&identity)).unwrap(), None);
    }

    #[test]
    fn several_siblings_with_the_identity_are_ambiguous() {
        let named = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(name, id)| (name.to_string(), id.to_string()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            unique_match(named(&[("a", "x:1"), ("b", "x:2")]), "x:2"),
            Some("b".into())
        );
        assert_eq!(
            unique_match(named(&[("a", "x:2"), ("link", "x:2")]), "x:2"),
            None
        );
        assert_eq!(unique_match(named(&[("a", "x:1")]), "x:2"), None);
    }

    #[test]
    fn the_new_path_keeps_the_saved_paths_separators() {
        assert_eq!(sibling("/work/gespan", "renamed"), "/work/renamed");
        assert_eq!(sibling("/work/gespan/", "renamed"), "/work/renamed");
        assert_eq!(sibling("C:/work/gespan", "renamed"), "C:/work/renamed");
    }

    #[test]
    fn a_renamed_project_takes_its_sessions_and_they_load_in_the_desktop_shape() {
        let parent = tempfile::tempdir().unwrap();
        let original = project(parent.path(), "gespan");
        let renamed = parent.path().join("gespan-personal");
        let other = project(parent.path(), "other");
        let store = Store::open_gespan(&parent.path().join("gespan.db")).unwrap();
        desktop_session(&store, "moved-session", &text(&original));
        desktop_session(&store, "other-session", &text(&other));
        let projects = vec![text(&original), text(&other)];

        // The first run only remembers both folders.
        assert!(follow_moved_projects(&store, &projects).is_empty());
        let remembered: serde_json::Value =
            serde_json::from_str(&store.setting(SETTING).unwrap().unwrap()).unwrap();
        assert_eq!(
            remembered[path_key(&text(&original))]["path"],
            text(&original)
        );

        std::fs::rename(&original, &renamed).unwrap();
        assert_eq!(
            follow_moved_projects(&store, &projects),
            vec![Moved {
                from: text(&original),
                to: text(&renamed),
            }]
        );
        assert_eq!(cwd_of(&store, "moved-session"), text(&renamed));
        assert_eq!(cwd_of(&store, "other-session"), text(&other));

        // The row is unchanged apart from its folder, and still loads.
        let (harness, model, title, blocks): (String, String, String, String) = store
            .conn()
            .query_row(
                "SELECT harness,model,title,blocks_json FROM sessions WHERE id='moved-session'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (harness.as_str(), model.as_str(), title.as_str()),
            ("codex", "codex:model-one", "Saved chat")
        );
        assert_eq!(blocks, r#"[{"id":"prompt","role":"user","text":"Hello"}]"#);
        let session = store
            .sessions()
            .unwrap()
            .into_iter()
            .find(|s| store.persisted_session_id(s.id).unwrap() == "moved-session")
            .unwrap();
        assert_eq!(session.cwd, text(&renamed));
        assert_eq!(store.load(session.id).unwrap().rows[0].role, "user");

        // The identity now lives under the new path only.
        let remembered: serde_json::Value =
            serde_json::from_str(&store.setting(SETTING).unwrap().unwrap()).unwrap();
        assert!(remembered.get(path_key(&text(&original))).is_none());
        assert_eq!(
            remembered[path_key(&text(&renamed))]["path"],
            text(&renamed)
        );
    }

    #[test]
    fn an_existing_or_unmatched_project_changes_nothing() {
        let parent = tempfile::tempdir().unwrap();
        let kept = project(parent.path(), "kept");
        let lost = parent.path().join("lost");
        let store = Store::open_gespan(&parent.path().join("gespan.db")).unwrap();
        desktop_session(&store, "kept-session", &text(&kept));
        desktop_session(&store, "lost-session", &text(&lost));
        let projects = vec![text(&kept), text(&lost)];

        assert!(follow_moved_projects(&store, &projects).is_empty());
        // A folder that appears under another name without a remembered
        // identity is not claimed.
        project(parent.path(), "lost-renamed");
        assert!(follow_moved_projects(&store, &projects).is_empty());
        assert_eq!(cwd_of(&store, "kept-session"), text(&kept));
        assert_eq!(cwd_of(&store, "lost-session"), text(&lost));
    }

    #[test]
    fn local_sessions_move_with_a_renamed_project() {
        let parent = tempfile::tempdir().unwrap();
        let original = project(parent.path(), "gespan");
        let renamed = parent.path().join("renamed");
        let store = Store::open(&parent.path().join("native.db")).unwrap();
        let session = store.create("codex", &text(&original)).unwrap();
        let projects = vec![text(&original)];
        follow_moved_projects(&store, &projects);
        std::fs::rename(&original, &renamed).unwrap();
        assert_eq!(follow_moved_projects(&store, &projects).len(), 1);
        let cwd = store
            .sessions()
            .unwrap()
            .into_iter()
            .find(|s| s.id == session.id)
            .unwrap()
            .cwd;
        assert_eq!(cwd, text(&renamed));
    }

    #[test]
    fn remote_and_root_folders_are_skipped() {
        assert!(!is_local(""));
        assert!(!is_local("/"));
        assert!(!is_local("~"));
        assert!(!is_local("C:"));
        assert!(!is_local(&crate::remote::remote_path(
            "env",
            "/work/gespan"
        )));
        assert!(is_local("/work/gespan"));
    }
}
