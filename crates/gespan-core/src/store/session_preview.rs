//! Bounded hover-card metadata. Never read transcripts or discover agent state.
use super::db::args;
use super::Store;

#[derive(Clone, Debug, Default)]
pub struct SessionPreviewData {
    pub model: Option<String>,
    pub worktree_cwd: Option<String>,
}

impl Store {
    pub fn session_preview(&self, id: i64) -> Result<SessionPreviewData, String> {
        // Read just these two scalar columns, never blocks_json/model_settings,
        // and at most a bounded prefix of each: Toasty 0.11 cannot project
        // `substr`.
        self.sql()?
            .optional(
                "SELECT substr(model,1,512),substr(worktree_cwd,1,4096) FROM sessions WHERE id=?1",
                args![self.persisted_session_id(id)?],
            )?
            .map(|row| {
                Ok(SessionPreviewData {
                    model: row.get(0)?,
                    worktree_cwd: row.get(1)?,
                })
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn desktop_preview_never_decodes_blocks_or_model_controls() {
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let original = store.persisted_session_id(session.id).unwrap();
        store.conn().execute("UPDATE sessions SET model='codex:gpt-6-sol',model_settings='broken JSON',blocks_json='broken JSON',worktree_cwd='/linked' WHERE id=?1", [original]).unwrap();
        let preview = store.session_preview(session.id).unwrap();
        assert_eq!(preview.model.as_deref(), Some("codex:gpt-6-sol"));
        assert_eq!(preview.worktree_cwd.as_deref(), Some("/linked"));
    }

    #[test]
    fn preview_reads_only_bounded_settings_without_history() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let original = store.persisted_session_id(session.id).unwrap();
        // The preview must still work when full history cannot be read.
        store
            .conn()
            .execute(
                "UPDATE sessions SET model=?2,worktree_cwd='/worktree',blocks_json='broken JSON' WHERE id=?1",
                [original, "m".repeat(1000)],
            )
            .unwrap();
        let preview = store.session_preview(session.id).unwrap();
        assert_eq!(preview.model.unwrap().len(), 512);
        assert_eq!(preview.worktree_cwd.as_deref(), Some("/worktree"));
    }
}
