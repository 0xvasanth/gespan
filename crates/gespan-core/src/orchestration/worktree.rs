//! A worktree for each worker. It starts with the lead checkout's current
//! contents, and only the worker's own delta returns: every file is checked
//! byte for byte first, so anything the lead changed since is never
//! overwritten. A worktree nobody has reviewed is kept.
use super::overlaps;
use crate::checkpoint::{capture_workspace, git_command, run_git};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const REGULAR: &str = "100644";
const EXECUTABLE: &str = "100755";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub path: String,
    pub branch: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Integrated {
    pub files: Vec<String>,
    pub already_applied: usize,
}

/// One path's change between two snapshots.
#[derive(Debug)]
struct Entry {
    path: String,
    before_mode: String,
    after_mode: String,
    before: String,
    after: String,
}
impl Entry {
    fn deleted(&self) -> bool {
        self.after_mode == "000000"
    }
    fn added(&self) -> bool {
        self.before_mode == "000000"
    }
}

fn token(task: &str) -> String {
    let token: String = task
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(24)
        .collect::<String>()
        .to_lowercase();
    if token.is_empty() {
        "task".into()
    } else {
        token
    }
}
pub fn branch_name(task: &str) -> String {
    format!("gespan/orch-{}", token(task))
}
fn reference(task: &str, side: &str) -> String {
    format!("refs/gespan/orch/{}/{side}", token(task))
}
fn text(root: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(run_git(root, args)?)
        .map(|value| value.trim().to_owned())
        .map_err(|e| e.to_string())
}
fn head(root: &Path) -> Result<String, String> {
    text(root, &["rev-parse", "--verify", "HEAD"])
}
fn tree_of(root: &Path, commit: &str) -> Result<String, String> {
    text(root, &["rev-parse", &format!("{commit}^{{tree}}")])
}

/// Every path that differs, read from `git diff-tree --raw`.
fn changed(root: &Path, before: &str, after: &str) -> Result<Vec<Entry>, String> {
    let raw = run_git(
        root,
        &[
            "diff-tree",
            "-r",
            "-z",
            "--no-renames",
            "--raw",
            before,
            after,
        ],
    )?;
    let raw = String::from_utf8(raw).map_err(|e| e.to_string())?;
    let mut fields = raw.split('\0');
    let mut entries = Vec::new();
    while let Some(header) = fields.next().filter(|field| !field.is_empty()) {
        let path = fields.next().ok_or("Malformed Git diff")?;
        let parts: Vec<_> = header.trim_start_matches(':').split(' ').collect();
        let [before_mode, after_mode, before, after, _] = parts[..] else {
            return Err("Malformed Git diff".into());
        };
        entries.push(Entry {
            path: path.to_owned(),
            before_mode: before_mode.to_owned(),
            after_mode: after_mode.to_owned(),
            before: before.to_owned(),
            after: after.to_owned(),
        });
    }
    Ok(entries)
}
fn inside(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(format!("Refused path {relative}"));
        }
        path.push(part);
    }
    Ok(path)
}
/// A link anywhere on the way would write outside the checkout.
fn through_link(root: &Path, relative: &str) -> bool {
    let mut current = root.to_path_buf();
    for part in relative.split('/') {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
            Err(_) => return true,
        }
    }
    false
}
fn is_plain(mode: &str) -> bool {
    matches!(mode, REGULAR | EXECUTABLE | "000000")
}
/// The blob `root` holds at `relative`, as Git would store it.
fn file_blob(root: &Path, relative: &str) -> Result<Option<String>, String> {
    let path = inside(root, relative)?;
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() => text(root, &["hash-object", "--", relative]).map(Some),
        Ok(_) => Err(format!("{relative} is not a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}
#[cfg(unix)]
fn set_mode(path: &Path, mode: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if mode == EXECUTABLE { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())
}
#[cfg(not(unix))]
fn set_mode(_: &Path, _: &str) -> Result<(), String> {
    Ok(())
}
/// Writes one entry's result under `root`. The blob goes through the target's
/// own filters, so a line-ending setting can't make it look changed.
fn write_entry(root: &Path, entry: &Entry) -> Result<(), String> {
    let path = inside(root, &entry.path)?;
    if entry.deleted() {
        return match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
            _ => Ok(()),
        };
    }
    let bytes = {
        let mut command = git_command(
            root,
            &[
                "cat-file",
                "--filters",
                &format!("--path={}", entry.path),
                &entry.after,
            ],
        );
        crate::checkpoint::checked_output(&mut command)?
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    set_mode(&path, &entry.after_mode)
}

/// Makes the task's checkout, or finds it again after a crash between Git and
/// saved state. `None` means this project can't have one, so the worker shares
/// the lead's: a folder without Git, or a repository with no commits yet.
pub fn prepare(lead: &Path, task: &str) -> Result<Option<Workspace>, String> {
    if crate::files::remote::is_remote(lead) || head(lead).is_err() {
        return Ok(None);
    }
    let branch = branch_name(task);
    let existing = crate::git::snapshot(lead)?
        .worktrees
        .into_iter()
        .find(|tree| tree.branch.as_deref() == Some(branch.as_str()));
    if let Some(tree) = existing {
        let matches = text(lead, &["rev-parse", "--verify", &reference(task, "base")]).is_ok()
            && head(Path::new(&tree.path)) == head(lead);
        return if matches {
            Ok(Some(Workspace {
                path: tree.path,
                branch,
            }))
        } else {
            Err(format!(
                "The recovered orchestration worktree for {branch} is incomplete or has unexpected changes. It was kept for manual review."
            ))
        };
    }
    let known = text(
        lead,
        &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
    )
    .is_ok();
    let tree = crate::git::create_project_worktree(lead, &branch, Some("HEAD"), known)?;
    let workspace = Workspace {
        path: tree.path,
        branch,
    };
    if let Err(error) = seed(lead, &workspace, task) {
        let _ = remove(lead, &workspace, task);
        return Err(error);
    }
    Ok(Some(workspace))
}
fn seed(lead: &Path, workspace: &Workspace, task: &str) -> Result<(), String> {
    let worker = Path::new(&workspace.path);
    let seed = capture_workspace(lead, &reference(task, "seed"))?;
    for entry in changed(lead, &head(lead)?, &seed)? {
        if !is_plain(&entry.after_mode)
            || through_link(lead, &entry.path)
            || through_link(worker, &entry.path)
        {
            return Err(format!(
                "Cannot seed orchestration worktree: {} is not a regular file",
                entry.path
            ));
        }
        write_entry(worker, &entry)?;
    }
    capture_workspace(worker, &reference(task, "base")).map(|_| ())
}

/// True only when the checkout still holds exactly what it started with.
pub fn unchanged(workspace: &Workspace, task: &str) -> Result<bool, String> {
    let worker = Path::new(&workspace.path);
    let base = text(worker, &["rev-parse", "--verify", &reference(task, "base")])?;
    let after = capture_workspace(worker, &reference(task, "after"))?;
    Ok(tree_of(worker, &base)? == tree_of(worker, &after)?)
}

/// Applies the worker's delta to the lead checkout. A retry may find some of it
/// already there, from an earlier attempt: both states are safe, so this is
/// idempotent. Everything is checked before the first write.
pub fn integrate(
    lead: &Path,
    workspace: &Workspace,
    task: &str,
    files: &[String],
) -> Result<Integrated, String> {
    let worker = Path::new(&workspace.path);
    if head(worker)? != head(lead)? {
        return Err(
            "The worker or lead branch moved while this task was running. The worker worktree was kept for manual review."
                .into(),
        );
    }
    let base = text(worker, &["rev-parse", "--verify", &reference(task, "base")])?;
    let after = capture_workspace(worker, &reference(task, "after"))?;
    let entries = changed(worker, &base, &after)?;
    let mut already_applied = 0;
    for entry in &entries {
        let path = &entry.path;
        if !overlaps(std::slice::from_ref(path), files) {
            return Err(format!(
                "Cannot integrate {path}: it is outside this task's files ({}). The worker worktree was kept. Retry with corrected files only if the write is genuinely required, otherwise send a correction.",
                files.join(", ")
            ));
        }
        if !is_plain(&entry.before_mode) || !is_plain(&entry.after_mode) {
            return Err(format!(
                "Cannot integrate {path}: it is not a regular file. The worker worktree was kept."
            ));
        }
        if through_link(lead, path) {
            return Err(format!(
                "Cannot integrate {path}: the target path contains a symbolic link. The worker worktree was kept."
            ));
        }
        let target = file_blob(lead, path).map_err(|error| {
            format!("Cannot integrate {path}: {error}. The worker worktree was kept.")
        })?;
        let matches = |blob: &str, absent: bool| match &target {
            Some(target) => !absent && target == blob,
            None => absent,
        };
        if matches(&entry.after, entry.deleted()) {
            already_applied += 1;
        } else if !matches(&entry.before, entry.added()) {
            return Err(format!(
                "Cannot integrate {path}: the lead checkout changed since this worker started. The worker worktree was kept."
            ));
        }
    }
    for entry in &entries {
        write_entry(lead, entry)?;
    }
    Ok(Integrated {
        files: entries.into_iter().map(|entry| entry.path).collect(),
        already_applied,
    })
}

/// Removes the checkout, its temporary branch and its snapshots.
pub fn remove(lead: &Path, workspace: &Workspace, task: &str) -> Result<(), String> {
    if Path::new(&workspace.path).exists() {
        run_git(
            lead,
            &["worktree", "remove", "--force", "--", &workspace.path],
        )?;
    }
    let _ = run_git(lead, &["worktree", "prune"]);
    if crate::git::is_orchestration_branch(&workspace.branch)
        && text(
            lead,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{}", workspace.branch),
            ],
        )
        .is_ok()
    {
        run_git(lead, &["branch", "-D", &workspace.branch])?;
    }
    for side in ["seed", "base", "after"] {
        let _ = run_git(lead, &["update-ref", "-d", &reference(task, side)]);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
