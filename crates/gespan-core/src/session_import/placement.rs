//! Where an on-disk session belongs in Gespan: a project, one of its linked
//! worktrees, or nowhere. Also whether Gespan already holds it.
use std::collections::HashSet;
use std::path::Path;

use crate::files::path_key;
use crate::provider::Kind;

/// Where an imported session goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    /// The session ran in a project's own folder.
    Project {
        project: String,
    },
    /// The session ran in a linked worktree of a project.
    Worktree {
        project: String,
        worktree: String,
    },
    Skip(Skip),
}

/// Why a session is not imported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skip {
    /// The folder is neither a project nor a linked worktree of one.
    NoProject,
    /// The folder no longer exists.
    FolderMissing,
    /// Gespan already holds a session with this provider id.
    AlreadyImported,
    /// The session file could not be read.
    Unreadable(String),
}

/// Comparison key that survives a trailing slash or a symlinked spelling.
fn key(path: &str) -> String {
    let real = std::fs::canonicalize(path)
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned));
    path_key(real.as_deref().unwrap_or(path))
}

/// Places a session that ran in `cwd`. `root` maps a linked worktree to its
/// main checkout, as `git::linked_worktree_root` does. The project itself wins
/// over the worktree rule; a plain subfolder of a project is skipped.
pub fn place(cwd: &str, projects: &[String], root: impl Fn(&str) -> Option<String>) -> Placement {
    if !Path::new(cwd).is_dir() {
        return Placement::Skip(Skip::FolderMissing);
    }
    let folder = key(cwd);
    if let Some(project) = projects.iter().find(|p| key(p) == folder) {
        return Placement::Project {
            project: project.clone(),
        };
    }
    let main = root(cwd).map(|main| key(&main));
    match main.and_then(|main| projects.iter().find(|p| key(p) == main)) {
        Some(project) => Placement::Worktree {
            project: project.clone(),
            worktree: cwd.to_string(),
        },
        None => Placement::Skip(Skip::NoProject),
    }
}

/// Whether Gespan holds a session of `kind` with this provider id. `held`
/// is [`crate::store::Store::provider_session_ids`]: live, archived and
/// hidden sessions alike.
pub fn already_imported(held: &HashSet<(String, String)>, kind: Kind, provider_id: &str) -> bool {
    held.contains(&(kind.id().to_owned(), provider_id.to_owned()))
}
