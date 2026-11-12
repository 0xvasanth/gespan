//! Small sidebar metadata can be gathered before the UI constructs chat entities.
use super::Store;
use crate::{agent::AccessMode, orchestration, session_groups};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct SessionSidebarMetadata {
    pub worktrees: HashMap<i64, Option<String>>,
    pub details: HashMap<i64, MobileSessionMetadata>,
    pub drafts: HashSet<i64>,
    pub sidebar_hidden: HashSet<i64>,
    pub started_by: HashMap<i64, i64>,
    pub orchestrations: HashMap<i64, orchestration::Summary>,
    pub groups: HashMap<String, session_groups::Groups>,
    pub reminders: Vec<super::Reminder>,
}

/// Scalar fields sent to paired phones, including sessions with no live view.
/// Model IDs use the native representation, as agent_settings does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MobileSessionMetadata {
    pub model: Option<String>,
    pub branch: Option<String>,
    pub worktree_cwd: Option<String>,
    pub worktree_removed: bool,
    pub access: AccessMode,
}

impl Store {
    /// One bounded metadata query per publication. Never decode transcript
    /// bodies or model controls, or load a Conversation just to list it.
    pub fn mobile_session_metadata(&self) -> Result<HashMap<i64, MobileSessionMetadata>, String> {
        // Bounded prefixes of each text column: Toasty 0.11 cannot project
        // `substr`.
        let query = "SELECT id,harness,substr(model,1,512),substr(branch,1,512),substr(worktree_cwd,1,4096),
             coalesce(worktree_removed,0),coalesce(runtime_mode,'supervised') FROM sessions";
        let mut result = HashMap::new();
        for row in self.sql()?.query(query, super::db::args![])? {
            let (id, provider, model, branch, worktree_cwd, worktree_removed, access) = (
                row.get::<String>(0)?,
                row.get::<String>(1)?,
                row.get::<Option<String>>(2)?,
                row.get::<Option<String>>(3)?,
                row.get::<Option<String>>(4)?,
                row.get::<bool>(5)?,
                row.get::<String>(6)?,
            );
            let id = self.shared_register(&id)?;
            result.insert(
                id,
                MobileSessionMetadata {
                    model: model
                        .as_deref()
                        .and_then(|model| super::native_model(model, &provider)),
                    branch,
                    worktree_cwd,
                    worktree_removed,
                    access: AccessMode::from_id(&access),
                },
            );
        }
        Ok(result)
    }

    /// Reads no transcript bodies or full agent settings. Call this on a loader
    /// thread, before handing the snapshot to the interactive shell.
    pub fn session_sidebar_metadata(&self, projects: &[String]) -> SessionSidebarMetadata {
        SessionSidebarMetadata {
            worktrees: self.session_worktrees().unwrap_or_default(),
            details: self.mobile_session_metadata().unwrap_or_default(),
            drafts: self.draft_sessions().unwrap_or_default(),
            sidebar_hidden: self.shared_sidebar_hidden().unwrap_or_default(),
            started_by: self.started_links().unwrap_or_default(),
            orchestrations: self.orchestration_summaries().unwrap_or_default(),
            groups: projects
                .iter()
                .filter_map(|project| {
                    self.groups(project)
                        .ok()
                        .map(|groups| (project.clone(), groups))
                })
                .collect(),
            reminders: self.reminders().unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn mobile_metadata_keeps_saved_branches_worktrees_models_and_access_without_decoding_history() {
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let session = store.create("claude", "/project").unwrap();
        let id = store.persisted_session_id(session.id).unwrap();
        store.conn().execute("UPDATE sessions SET model='claude:opus-5-5[effort=high]',branch='feature/mobile',worktree_cwd='/project-worktrees/mobile',runtime_mode='full-access',model_settings='broken JSON',blocks_json='broken JSON' WHERE id=?1", [&id]).unwrap();
        let metadata = store
            .mobile_session_metadata()
            .unwrap()
            .remove(&session.id)
            .unwrap();
        assert_eq!(metadata.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(metadata.branch.as_deref(), Some("feature/mobile"));
        assert_eq!(
            metadata.worktree_cwd.as_deref(),
            Some("/project-worktrees/mobile")
        );
        assert_eq!(metadata.access, AccessMode::FullAccess);
        assert!(!metadata.worktree_removed);
        // Scalar updates need not change updated_at to appear on the phone.
        store.conn().execute("UPDATE sessions SET model='claude:sonnet',branch='main',worktree_removed=1,runtime_mode='read-only' WHERE id=?1", [&id]).unwrap();
        let next = store
            .mobile_session_metadata()
            .unwrap()
            .remove(&session.id)
            .unwrap();
        assert_eq!(next.model.as_deref(), Some("sonnet"));
        assert_eq!(next.branch.as_deref(), Some("main"));
        assert!(next.worktree_removed);
        assert_eq!(next.access, AccessMode::ReadOnly);
    }

    #[test]
    fn sidebar_snapshot_keeps_old_history_without_reading_transcripts() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let first = store.create("codex", "/work/first").unwrap();
        let second = store.create("claude", "/work/second").unwrap();
        // A malformed history must not be decoded while gathering sidebar state.
        let original = store.persisted_session_id(first.id).unwrap();
        store
            .conn()
            .execute(
                "UPDATE sessions SET blocks_json='not JSON' WHERE id=?1",
                [&original],
            )
            .unwrap();
        let snapshot = store.session_sidebar_metadata(&["/work/first".into()]);
        assert!(snapshot.worktrees.contains_key(&first.id));
        assert!(snapshot.worktrees.contains_key(&second.id));
        assert!(snapshot.details.contains_key(&first.id));
        assert!(snapshot.details.contains_key(&second.id));
        assert!(snapshot.groups.contains_key("/work/first"));
        assert!(!snapshot.groups.contains_key("/work/second"));
    }
}
