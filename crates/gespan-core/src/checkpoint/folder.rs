//! Checkpoints for folders without Git, ported from the manifest store in
//! src-tauri/src/checkpoint.rs. A structured edit snapshots its file before it
//! starts (`prepare`) and after it succeeds (`capture_edit`). Only files with
//! both snapshots are reviewed, and Undo restores the first one.
use super::*;

const MAX_SNAPSHOT_FILES: usize = 500;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    cwd: String,
    files: BTreeMap<String, SnapshotKind>,
    #[serde(default)]
    touched: BTreeSet<String>,
    /// Written by the desktop app for older Git manifests; kept on rewrite.
    #[serde(default)]
    tracked: BTreeSet<String>,
    /// Paths captured before a structured edit started. Only these are safe
    /// candidates for Undo.
    #[serde(default)]
    prepared: BTreeSet<String>,
    /// Worktree contents immediately after the session's latest edit.
    #[serde(default)]
    after: BTreeMap<String, SnapshotKind>,
    /// Stable line counts for the session-owned before/after pair.
    #[serde(default)]
    stats: BTreeMap<String, ChangeStats>,
    /// Paths whose contents changed between two edits by this session.
    #[serde(default)]
    diverged: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ChangeStats {
    status: String,
    additions: i64,
    deletions: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum SnapshotKind {
    Contents,
    Missing,
    Skipped,
}

impl Store {
    /// Start a folder's session recording; an existing one for it is kept.
    pub(super) fn ensure_folder(&self, session_id: &str, cwd: &str) -> Result<(), String> {
        let dir = self.session_dir(session_id);
        if let Some(manifest) = read_manifest(&dir)? {
            if same_cwd(&manifest.cwd, cwd) {
                return Ok(());
            }
            clear_manifest(&dir);
        }
        write_manifest(
            &dir,
            &Manifest {
                cwd: path_to_js(&project_root(cwd)?),
                ..Manifest::default()
            },
        )
    }

    /// A structured edit is about to write these files: snapshot them first.
    /// Git workspaces record whole turns instead, so this only serves folders.
    pub fn prepare(&self, session_id: &str, cwd: &str, paths: &[String]) -> Result<(), String> {
        validate_id(session_id, "session")?;
        if paths.is_empty() || self.matching_turn_review(session_id, cwd)?.is_some() {
            return Ok(());
        }
        let root = project_root(cwd)?;
        let dir = self.session_dir(session_id);
        let Some(mut manifest) = self.load_folder(session_id, cwd)? else {
            return Ok(());
        };
        let mut dirty = false;
        for path in paths {
            let Ok(relative) = folder_path(&root, path) else {
                continue;
            };
            // Keep the original pre-edit snapshot across later edits by this
            // session. The first tool-start event owns the safe undo boundary.
            if manifest.touched.contains(&relative) && manifest.prepared.contains(&relative) {
                if !after_matches_worktree(&dir, &root, &manifest, &relative)
                    && manifest.diverged.insert(relative)
                {
                    dirty = true;
                }
                continue;
            }
            if manifest.prepared.contains(&relative) {
                continue;
            }
            if manifest.touched.contains(&relative) {
                // A completion-only claim is untrusted; restart at this real
                // tool boundary.
                release_path(&mut manifest, &relative);
            }
            let before = snapshot_file(&dir.join("files"), &root, &relative)?;
            manifest.files.insert(relative.clone(), before);
            manifest.prepared.insert(relative);
            dirty = true;
        }
        if dirty {
            write_manifest(&dir, &manifest)?;
        }
        Ok(())
    }

    /// A structured edit succeeded: a turn notes its files, and a folder
    /// without Git snapshots their results.
    pub fn capture_edit(
        &self,
        session_id: &str,
        cwd: &str,
        paths: &[String],
    ) -> Result<(), String> {
        validate_id(session_id, "session")?;
        if paths.is_empty() || self.capture_turn_paths(session_id, cwd, paths)? {
            return Ok(());
        }
        let root = project_root(cwd)?;
        let dir = self.session_dir(session_id);
        let Some(mut manifest) = self.load_folder(session_id, cwd)? else {
            return Ok(());
        };
        let mut dirty = false;
        for path in paths {
            if manifest.touched.len() >= MAX_SNAPSHOT_FILES {
                break;
            }
            let Ok(relative) = folder_path(&root, path) else {
                continue;
            };
            manifest.touched.insert(relative.clone());
            if !manifest.files.contains_key(&relative) && !root.join(&relative).exists() {
                // A completion without a matching prepare event is retained
                // for review but is deliberately not undoable.
                let before = snapshot_file(&dir.join("files"), &root, &relative)?;
                manifest.files.insert(relative.clone(), before);
            }
            let after = snapshot_file(&dir.join("after"), &root, &relative)?;
            manifest.after.insert(relative.clone(), after);
            if let Some(stats) = session_stats(&dir, &manifest, &relative) {
                manifest.stats.insert(relative, stats);
            }
            dirty = true;
        }
        if dirty {
            write_manifest(&dir, &manifest)?;
        }
        Ok(())
    }

    pub(super) fn folder_status(
        &self,
        session_id: &str,
        cwd: &str,
    ) -> Result<Option<CheckpointStatus>, String> {
        let Some(manifest) = self.load_folder(session_id, cwd)? else {
            return Ok(None);
        };
        let root = project_root(cwd)?;
        let foreign = self.foreign_touched_paths(cwd, session_id);
        Ok(Some(manifest_status(
            &self.session_dir(session_id),
            &root,
            &manifest,
            &foreign,
        )))
    }

    pub(super) fn folder_file_diff(
        &self,
        session_id: &str,
        cwd: &str,
        relative: &str,
    ) -> Result<Option<crate::scm::FileDiff>, String> {
        let Some(manifest) = self.load_folder(session_id, cwd)? else {
            return Ok(None);
        };
        let root = project_root(cwd)?;
        let relative = resolve_turn_path(relative)?;
        if !manifest.touched.contains(&relative) || !manifest.prepared.contains(&relative) {
            return Err("This file was not changed by the session".into());
        }
        if manifest.diverged.contains(&relative) {
            return Err(
                "Exact lines are unavailable because the file changed between this session's edits"
                    .into(),
            );
        }
        let dir = self.session_dir(session_id);
        let before = manifest
            .files
            .get(&relative)
            .copied()
            .ok_or("Session baseline is unavailable")?;
        let after = manifest
            .after
            .get(&relative)
            .copied()
            .ok_or("Session result is unavailable")?;
        let original = read_snapshot(&dir.join("files"), &relative, before);
        let current = read_snapshot(&dir.join("after"), &relative, after);
        let too_large =
            matches!(original, FileState::Skipped) || matches!(current, FileState::Skipped);
        let binary = state_is_binary(&original) || state_is_binary(&current);
        let (original, current) = if binary || too_large {
            (String::new(), String::new())
        } else {
            (state_text(original), state_text(current))
        };
        let status = manifest
            .stats
            .get(&relative)
            .map_or_else(|| "modified".into(), |stats| stats.status.clone());
        Ok(Some(crate::scm::FileDiff {
            path: path_to_js(&root.join(&relative)),
            relative,
            status,
            original,
            current,
            binary,
            too_large,
        }))
    }

    pub(super) fn folder_undo(
        &self,
        session_id: &str,
        cwd: &str,
        relative: Option<&str>,
    ) -> Result<Option<CheckpointStatus>, String> {
        let Some(mut manifest) = self.load_folder(session_id, cwd)? else {
            return Ok(None);
        };
        let root = project_root(cwd)?;
        let dir = self.session_dir(session_id);
        let foreign = self.foreign_touched_paths(cwd, session_id);
        let changed = manifest_status(&dir, &root, &manifest, &foreign);
        if let Some(relative) = relative {
            let relative = resolve_turn_path(relative)?;
            let Some(file) = changed.files.iter().find(|file| file.relative == relative) else {
                return self.folder_status(session_id, cwd);
            };
            if !file.undoable {
                return Err(format!(
                    "Cannot safely undo {relative}: it changed outside this session"
                ));
            }
            restore_one(&dir, &root, &manifest, &relative)?;
            release_path(&mut manifest, &relative);
            write_manifest(&dir, &manifest)?;
            return self.folder_status(session_id, cwd);
        }
        if changed.files.iter().any(|file| !file.undoable) {
            return Err(
                "Cannot safely undo all: one or more files changed outside this session".into(),
            );
        }
        for file in &changed.files {
            restore_one(&dir, &root, &manifest, &file.relative)?;
        }
        clear_manifest(&dir);
        Ok(Some(CheckpointStatus::default()))
    }

    pub(super) fn folder_keep(
        &self,
        session_id: &str,
        cwd: &str,
        relative: Option<&str>,
    ) -> Result<Option<CheckpointStatus>, String> {
        let Some(mut manifest) = self.load_folder(session_id, cwd)? else {
            return Ok(None);
        };
        let dir = self.session_dir(session_id);
        let Some(relative) = relative else {
            clear_manifest(&dir);
            return Ok(Some(CheckpointStatus::default()));
        };
        let relative = resolve_turn_path(relative)?;
        release_path(&mut manifest, &relative);
        write_manifest(&dir, &manifest)?;
        self.folder_status(session_id, cwd)
    }

    fn load_folder(&self, session_id: &str, cwd: &str) -> Result<Option<Manifest>, String> {
        Ok(read_manifest(&self.session_dir(session_id))?
            .filter(|manifest| same_cwd(&manifest.cwd, cwd)))
    }

    /// Paths already claimed by another session in the same folder.
    fn foreign_touched_paths(&self, cwd: &str, except: &str) -> HashSet<String> {
        let mut paths = HashSet::new();
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return paths;
        };
        for entry in entries.flatten() {
            if entry.file_name() == except {
                continue;
            }
            let Ok(Some(manifest)) = read_manifest(&entry.path()) else {
                continue;
            };
            if same_cwd(&manifest.cwd, cwd) {
                paths.extend(manifest.touched.intersection(&manifest.prepared).cloned());
            }
        }
        paths
    }
}

fn manifest_status(
    dir: &Path,
    root: &Path,
    manifest: &Manifest,
    foreign: &HashSet<String>,
) -> CheckpointStatus {
    let mut files = Vec::new();
    for relative in &manifest.touched {
        // Without a tool-start snapshot there is no trustworthy session
        // boundary. Never guess from the shared folder.
        if !manifest.prepared.contains(relative) {
            continue;
        }
        let Some(before) = manifest.files.get(relative).copied() else {
            continue;
        };
        if let Some(after) = manifest.after.get(relative).copied() {
            if read_snapshot(&dir.join("files"), relative, before)
                == read_snapshot(&dir.join("after"), relative, after)
            {
                continue;
            }
        }
        if before == SnapshotKind::Skipped
            || read_worktree(root, relative) == read_snapshot(&dir.join("files"), relative, before)
        {
            continue;
        }
        let exact = !manifest.diverged.contains(relative);
        let undoable = exact
            && !foreign.contains(relative)
            && after_matches_worktree(dir, root, manifest, relative);
        let (status, additions, deletions) = match manifest.stats.get(relative) {
            Some(stats) if exact => (stats.status.clone(), stats.additions, stats.deletions),
            Some(stats) => (stats.status.clone(), 0, 0),
            None if root.join(relative).exists() => ("modified".into(), 0, 0),
            None => ("deleted".into(), 0, 0),
        };
        files.push(CheckpointFile {
            path: path_to_js(&root.join(relative)),
            relative: relative.clone(),
            status,
            additions,
            deletions,
            exact,
            undoable,
        });
    }
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    CheckpointStatus { files }
}

fn session_stats(dir: &Path, manifest: &Manifest, relative: &str) -> Option<ChangeStats> {
    let before = manifest.files.get(relative).copied()?;
    let after = manifest.after.get(relative).copied()?;
    if before == SnapshotKind::Skipped || after == SnapshotKind::Skipped {
        return None;
    }
    let (additions, deletions) = diff_numstat(
        &blob_path(&dir.join("files"), relative).ok()?,
        &blob_path(&dir.join("after"), relative).ok()?,
    )?;
    let status = match (before, after) {
        (SnapshotKind::Missing, SnapshotKind::Missing) => "modified",
        (SnapshotKind::Missing, _) => "added",
        (_, SnapshotKind::Missing) => "deleted",
        _ => "modified",
    };
    Some(ChangeStats {
        status: status.into(),
        additions,
        deletions,
    })
}

/// `git diff --no-index` counts lines between two snapshot files; it needs no
/// repository.
fn diff_numstat(before: &Path, after: &Path) -> Option<(i64, i64)> {
    let parent = before.parent()?;
    let mut command = git_command(
        parent,
        &["diff", "--no-index", "--no-ext-diff", "--numstat", "--"],
    );
    let output = command.arg(before).arg(after).output().ok()?;
    // --no-index exits 1 when the files differ.
    if !output.status.success() && output.status.code() != Some(1) {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut fields = text.lines().next()?.split('\t');
    let additions = fields.next()?.parse().ok()?;
    let deletions = fields.next()?.parse().ok()?;
    Some((additions, deletions))
}

fn after_matches_worktree(dir: &Path, root: &Path, manifest: &Manifest, relative: &str) -> bool {
    manifest.after.get(relative).is_some_and(|kind| {
        read_worktree(root, relative) == read_snapshot(&dir.join("after"), relative, *kind)
    })
}

fn release_path(manifest: &mut Manifest, relative: &str) {
    manifest.files.remove(relative);
    manifest.touched.remove(relative);
    manifest.tracked.remove(relative);
    manifest.prepared.remove(relative);
    manifest.after.remove(relative);
    manifest.stats.remove(relative);
    manifest.diverged.remove(relative);
}

fn restore_one(dir: &Path, root: &Path, manifest: &Manifest, relative: &str) -> Result<(), String> {
    if path_contains_symlink(root, relative) {
        return Err(format!("Cannot restore through symbolic link {relative}"));
    }
    let path = root.join(relative);
    match manifest.files.get(relative) {
        Some(SnapshotKind::Contents) => {
            let blob = blob_path(&dir.join("files"), relative)?;
            let bytes = std::fs::read(&blob).map_err(|e| e.to_string())?;
            if path.is_dir() {
                return Err(format!("{relative} is a directory"));
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            set_file_mode(&path, file_mode(&blob))
        }
        Some(SnapshotKind::Missing) => match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
            _ => Ok(()),
        },
        Some(SnapshotKind::Skipped) => Ok(()),
        None => Err(format!("Missing original snapshot for {relative}")),
    }
}

fn snapshot_file(blob_root: &Path, root: &Path, relative: &str) -> Result<SnapshotKind, String> {
    let abs = root.join(relative);
    let blob = blob_path(blob_root, relative)?;
    let meta = match std::fs::symlink_metadata(&abs) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // An empty blob lets numstat count a created or deleted file.
            if let Some(parent) = blob.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(blob, []).map_err(|e| e.to_string())?;
            return Ok(SnapshotKind::Missing);
        }
        Err(error) => return Err(error.to_string()),
    };
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > MAX_TEXT_FILE_BYTES {
        return Ok(SnapshotKind::Skipped);
    }
    let bytes = std::fs::read(&abs).map_err(|e| e.to_string())?;
    if let Some(parent) = blob.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&blob, bytes).map_err(|e| e.to_string())?;
    set_file_mode(&blob, file_mode(&abs))?;
    Ok(SnapshotKind::Contents)
}

fn read_snapshot(blob_root: &Path, relative: &str, kind: SnapshotKind) -> FileState {
    match kind {
        SnapshotKind::Missing => FileState::Missing,
        SnapshotKind::Skipped => FileState::Skipped,
        SnapshotKind::Contents => match blob_path(blob_root, relative)
            .ok()
            .and_then(|path| std::fs::read(path).ok())
        {
            Some(bytes) => FileState::Contents(bytes),
            None => FileState::Missing,
        },
    }
}

fn read_worktree(root: &Path, relative: &str) -> FileState {
    let abs = root.join(relative);
    match std::fs::symlink_metadata(&abs) {
        Err(_) => FileState::Missing,
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => FileState::Skipped,
        Ok(meta) if meta.len() > MAX_TEXT_FILE_BYTES => FileState::Skipped,
        Ok(_) => std::fs::read(&abs).map_or(FileState::Missing, FileState::Contents),
    }
}

fn blob_path(blob_root: &Path, relative: &str) -> Result<PathBuf, String> {
    Ok(blob_root.join(resolve_turn_path(relative)?))
}

/// A tool's path, relative to the folder, as the manifest names it.
fn folder_path(root: &Path, path: &str) -> Result<String, String> {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    resolve_edit_path(root, &canonical, path.trim())
}

fn same_cwd(saved: &str, cwd: &str) -> bool {
    let canonical =
        |path: &str| project_root(path).and_then(|p| p.canonicalize().map_err(|e| e.to_string()));
    matches!((canonical(saved), canonical(cwd)), (Ok(a), Ok(b)) if a == b)
}

#[cfg(unix)]
fn set_file_mode(path: &Path, mode: Option<u32>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    match mode {
        Some(mode) => std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[cfg(not(unix))]
fn set_file_mode(_path: &Path, _mode: Option<u32>) -> Result<(), String> {
    Ok(())
}

fn read_manifest(dir: &Path) -> Result<Option<Manifest>, String> {
    match std::fs::read(dir.join("manifest.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<(), String> {
    write_json(&dir.join("manifest.json"), manifest)
}

/// Drop the folder recording, leaving any Git turn history beside it.
fn clear_manifest(dir: &Path) {
    let _ = std::fs::remove_file(dir.join("manifest.json"));
    let _ = std::fs::remove_dir_all(dir.join("files"));
    let _ = std::fs::remove_dir_all(dir.join("after"));
}
