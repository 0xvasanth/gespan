//! The working copy a project's window is focused on. Worktrees are picked in
//! the composer; the sidebar lists every worktree's sessions.

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct WorktreeFocus {
    pub path: String,
    pub branch: Option<String>,
}
