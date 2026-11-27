//! Which repository a session works in, and which files it changed.
use crate::decisions::scope::{git_common_dir, strip_verbatim};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// The repository a folder belongs to: its canonical git common directory,
/// which a main checkout, its worktrees and their subfolders share. `None`
/// for a folder outside git or one that no longer exists.
pub fn repo_key(cwd: &Path) -> Option<PathBuf> {
    if !cwd.is_dir() {
        return None;
    }
    canonical(&git_common_dir(cwd)?)
}

/// The top of the checkout (main or worktree) a folder is in.
pub fn checkout_root(cwd: &Path) -> Option<PathBuf> {
    let top = crate::git::git_probe(cwd, &["rev-parse", "--show-toplevel"])?;
    let top = top.trim();
    (!top.is_empty()).then(|| PathBuf::from(top))
}

/// The branch checked out in `cwd`; `None` when detached or not a repository.
pub fn current_branch(cwd: &Path) -> Option<String> {
    let out = crate::git::git_probe(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let branch = out.trim();
    (!branch.is_empty()).then(|| branch.to_owned())
}

/// The files a session changed: the paths its own edits touched that are
/// still uncommitted in `checkout`, relative to it with `/` separators,
/// sorted. Another session's edits in the same checkout are not counted,
/// because only `edited` paths are kept.
pub fn session_files(edited: &[String], checkout: &Path) -> Vec<String> {
    let Some(root) = canonical(checkout) else {
        return Vec::new();
    };
    let wanted: BTreeSet<String> = edited
        .iter()
        .filter_map(|path| relative(path, &root))
        .collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let dirty = dirty_paths(&root);
    wanted.intersection(&dirty).cloned().collect()
}

fn canonical(path: &Path) -> Option<PathBuf> {
    let path = std::fs::canonicalize(path).ok()?;
    Some(PathBuf::from(strip_verbatim(&path.to_string_lossy())))
}

/// `path` relative to `root`, or `None` when it lies outside it. A deleted
/// file can't be canonicalized, so its parent is.
fn relative(path: &str, root: &Path) -> Option<String> {
    let path = Path::new(path);
    let relative = if path.is_absolute() {
        let resolved = canonical(path).or_else(|| {
            let parent = canonical(path.parent()?)?;
            Some(parent.join(path.file_name()?))
        })?;
        resolved.strip_prefix(root).ok()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    let parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty() && parts.iter().all(|p| p != "..")).then(|| parts.join("/"))
}

/// Every path `git status` reports as changed or untracked in `root`.
fn dirty_paths(root: &Path) -> BTreeSet<String> {
    let Some(out) = crate::git::git_probe(
        root,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--untracked-files=all",
            "-z",
        ],
    ) else {
        return BTreeSet::new();
    };
    let mut paths = BTreeSet::new();
    let mut entries = out.split('\0');
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let (status, path) = entry.split_at(3);
        paths.insert(path.to_owned());
        // With -z a rename or copy is followed by its old path.
        if status.starts_with(['R', 'C']) {
            entries.next();
        }
    }
    paths
}
