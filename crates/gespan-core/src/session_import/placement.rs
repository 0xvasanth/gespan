//! Where an on-disk session belongs in Gespan: a project, one of its linked
//! worktrees, or nowhere. Also whether Gespan already holds it.
use std::collections::{HashMap, HashSet};
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
    /// Gespan could not save the session.
    Failed(String),
}

/// Comparison key that survives a trailing slash or a symlinked spelling.
fn key(path: &str) -> String {
    let real = std::fs::canonicalize(path)
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned));
    path_key(real.as_deref().unwrap_or(path))
}

/// Places a session that ran in `cwd`. `root` maps a linked worktree to its
/// main checkout, as `git::linked_worktree_root` does (or, for many folders,
/// `git::WorktreeRoots`). The project itself wins
/// over the worktree rule; a plain subfolder of a project is skipped.
///
/// For many sessions, use a [`Placer`]: it reads each project's key once and
/// each folder once.
pub fn place(cwd: &str, projects: &[String], root: impl Fn(&str) -> Option<String>) -> Placement {
    Placer::new(projects, root).place(cwd)
}

/// Places many sessions against one set of projects. Sessions share a few
/// hundred folders, so each folder is placed once, and asking git about
/// worktrees, the slow part, happens once per folder.
pub struct Placer<R> {
    /// Each project and its comparison key.
    projects: Vec<(String, String)>,
    root: R,
    placed: HashMap<String, Placement>,
}

impl<R: FnMut(&str) -> Option<String>> Placer<R> {
    pub fn new(projects: &[String], root: R) -> Self {
        Self {
            projects: projects.iter().map(|p| (p.clone(), key(p))).collect(),
            root,
            placed: HashMap::new(),
        }
    }

    /// Where a session that ran in `cwd` goes; see [`place`].
    pub fn place(&mut self, cwd: &str) -> Placement {
        if let Some(placed) = self.placed.get(cwd) {
            return placed.clone();
        }
        let placed = self.place_folder(cwd);
        self.placed.insert(cwd.to_owned(), placed.clone());
        placed
    }

    fn project_keyed(&self, folder: &str) -> Option<&String> {
        self.projects
            .iter()
            .find(|(_, key)| key == folder)
            .map(|(project, _)| project)
    }

    fn place_folder(&mut self, cwd: &str) -> Placement {
        if !Path::new(cwd).is_dir() {
            return Placement::Skip(Skip::FolderMissing);
        }
        if let Some(project) = self.project_keyed(&key(cwd)) {
            return Placement::Project {
                project: project.clone(),
            };
        }
        let main = (self.root)(cwd).map(|main| key(&main));
        match main.and_then(|main| self.project_keyed(&main)) {
            Some(project) => Placement::Worktree {
                project: project.clone(),
                worktree: cwd.to_string(),
            },
            None => Placement::Skip(Skip::NoProject),
        }
    }
}

/// Whether Gespan holds a session of `kind` with this provider id. `held`
/// is [`crate::store::Store::provider_session_ids`]: live, archived and
/// hidden sessions alike.
pub fn already_imported(held: &HashSet<(String, String)>, kind: Kind, provider_id: &str) -> bool {
    held.contains(&(kind.id().to_owned(), provider_id.to_owned()))
}
