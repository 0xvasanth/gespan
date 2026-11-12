//! Session storage. Desktop launches use Gespan's existing schema directly.
mod app_access;
#[cfg(test)]
mod conn_tests;
pub(crate) mod db;
mod gespan_import;
mod in_flight;
mod inbox_ask;
mod managed_sessions;
pub(crate) mod my_tasks;
mod notes;
mod pilots;
mod provider_ids;
mod remote;
mod remote_legacy;
pub(crate) mod schema;
pub use remote::{RemoteProject, RemoteSession, RemoteUpload};
pub(crate) use shared::handle as desktop_session_handle;
mod session_metadata;
pub use session_metadata::{MobileSessionMetadata, SessionSidebarMetadata};
mod session_preview;
pub use session_preview::SessionPreviewData;
mod edit;
pub(crate) mod shared;
/// Desktop blocks as native rows, for tests outside the store.
#[cfg(test)]
pub(crate) use shared::blocks::project as project_desktop_blocks;
mod side_threads;
use crate::transcript::{Row, Transcript};
pub use edit::EditSnapshot;
pub use gespan_import::gespan_database;
pub use gespan_import::native_model;
pub use shared::{HistoryPage, PAGE_TURNS};
pub use side_threads::DesktopSideThread;
use std::path::Path;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Session {
    pub id: i64,
    pub title: String,
    pub provider: String,
    pub cwd: String,
    pub provider_id: Option<String>,
    pub has_messages: bool,
    /// Unix milliseconds; zero means an older session has no recorded time.
    pub updated_at: u64,
    /// Unix milliseconds the session started, where the store records it;
    /// zero otherwise, and the id orders sessions instead.
    #[serde(default)]
    pub created_at: u64,
    pub archived: bool,
    pub pinned: bool,
    pub work_item: Option<crate::session_groups::WorkItem>,
}

/// A session's pending or delivered reminder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub session_id: i64,
    /// Unix milliseconds.
    pub due_at: u64,
    pub fired_at: Option<u64>,
}

pub struct Store {
    path: std::path::PathBuf,
    shared: shared::Shared,
    /// The database through Toasty (`orm::Kind::Gespan`). A read-only store
    /// opens it read-only: SQLite refuses its writes.
    orm: crate::orm::Database,
    /// A read-only store on a file Gespan has not adopted yet reads an
    /// adopted copy kept here, so the file itself is never changed.
    _snapshot: Option<tempfile::TempDir>,
}

impl Store {
    /// This store's database through Toasty (`orm::Kind::Gespan`).
    pub(crate) fn orm(&self) -> Result<&crate::orm::Database, String> {
        Ok(&self.orm)
    }
    /// Raw SQL on this store's database, for what Toasty's typed API cannot
    /// express (see `store::db`).
    pub(crate) fn sql(&self) -> Result<db::Sql<'_>, String> {
        Ok(db::Sql(&self.orm))
    }
    /// A write transaction over this store's tables.
    pub(crate) fn begin(&self) -> Result<db::SqlTx<'_>, String> {
        self.sql()?.begin()
    }
    pub fn reset_provider_id_if(&self, id: i64, expected: Option<&str>) -> Result<(), String> {
        self.shared_reset_provider_id_if(id, expected)
    }
    pub fn provider_accounts(&self) -> Result<crate::accounts::Book, String> {
        self.shared_accounts()
    }
    pub fn save_provider_accounts(&self, book: &crate::accounts::Book) -> Result<(), String> {
        self.shared_save_accounts(book)
    }
    /// A store for tests and fixtures: the shared schema, like
    /// [`Self::open_gespan`].
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_gespan(path)
    }
    /// The selected desktop or isolated-development database.
    pub fn database_path(&self) -> &Path {
        &self.path
    }
    pub fn data_dir(&self) -> Option<&Path> {
        (self.path != Path::new(":memory:"))
            .then(|| self.path.parent())
            .flatten()
    }
    pub fn setting(&self, key: &str) -> Result<Option<String>, String> {
        self.shared_setting(key)
    }
    pub fn save_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.shared_save_setting(key, value)
    }
    pub fn ui_state(&self) -> Result<Option<String>, String> {
        self.shared_ui_state()
    }
    pub fn agent_settings(&self, id: i64) -> Result<Option<crate::agent::Settings>, String> {
        let remote = id < 0 || self.remote_session(id)?.is_some();
        if remote {
            if let Some(raw) = self.setting(&format!("remote-agent:{id}"))? {
                return serde_json::from_str(&raw)
                    .map(Some)
                    .map_err(|e| format!("Cannot read remote settings: {e}"));
            }
            if id < 0 {
                return Ok(None);
            }
        }
        let mut settings = self.shared_agent(id)?;
        if remote {
            if let Some(settings) = settings.as_mut() {
                if settings.worktree_cwd.is_none() {
                    settings.worktree_cwd = self.legacy_remote_worktree(id)?;
                }
            }
        }
        Ok(settings)
    }
    /// Read workspace membership without loading every session's full agent settings.
    pub fn session_worktrees(
        &self,
    ) -> Result<std::collections::HashMap<i64, Option<String>>, String> {
        self.shared_worktrees()
    }
    pub fn context_usage(&self, id: i64) -> Result<Option<crate::context::Usage>, String> {
        self.shared_context(id)
    }
    pub fn save_context_usage(&self, id: i64, usage: &crate::context::Usage) -> Result<(), String> {
        self.shared_patch(
            id,
            &[
                ("context_used", serde_json::json!(usage.used)),
                ("context_window", serde_json::json!(usage.window)),
            ],
        )
    }
    pub fn save_note(&self, note: &crate::response::Note) -> Result<(), String> {
        self.save_response_note(note)
    }
    pub fn notes(&self) -> Result<Vec<crate::response::Note>, String> {
        Ok(self
            .list_notes()?
            .into_iter()
            .map(|note| {
                let source_session = self.note_source_session(&note).unwrap_or(0);
                crate::response::Note {
                    id: note
                        .id
                        .strip_prefix("gpui-note-")
                        .and_then(|id| id.parse().ok())
                        .unwrap_or_else(|| shared::handle(&note.id) as u64),
                    title: note.title,
                    body: note.body,
                    tags: note.tags,
                    source_session,
                    source_cwd: note.source_cwd.unwrap_or_default(),
                }
            })
            .collect())
    }
    pub fn save_agent_settings(
        &self,
        id: i64,
        settings: &crate::agent::Settings,
    ) -> Result<(), String> {
        if id < 0 || self.remote_session(id)?.is_some() {
            return self.save_setting(
                &format!("remote-agent:{id}"),
                &serde_json::to_string(settings).map_err(|e| e.to_string())?,
            );
        }
        self.shared_save_agent(id, settings)
    }
    pub fn configure_agent(
        &self,
        id: i64,
        provider: &str,
        settings: &crate::agent::Settings,
    ) -> Result<(), String> {
        if self
            .agent_settings(id)?
            .is_some_and(|saved| saved.revision > settings.revision)
        {
            return Ok(());
        }
        self.change_provider(id, provider)?;
        self.save_agent_settings(id, settings)
    }
    /// A new session that starts with another conversation's `rows`, as a fork
    /// or a handoff does. Remote hosts own their transcripts, so a remote
    /// session starts empty and relies on its settings' handoff recap.
    pub fn create_from(
        &mut self,
        provider: &str,
        cwd: &str,
        title: &str,
        settings: &crate::agent::Settings,
        rows: &[Row],
    ) -> Result<Session, String> {
        let mut summary = self.create(provider, cwd)?;
        summary.title = title.into();
        self.title(summary.id, title)?;
        self.configure_agent(summary.id, provider, settings)?;
        if !rows.is_empty() && self.remote_session(summary.id)?.is_none() {
            self.write_rows(summary.id, 0, rows)?;
            summary.has_messages = true;
        }
        Ok(summary)
    }
    pub fn change_provider(&self, id: i64, provider: &str) -> Result<(), String> {
        if let Some(mut remote) = self.remote_session(id)? {
            if remote.summary.provider != provider {
                // The host fixes the harness at creation, as React's remote view does.
                if remote.session_id.is_some() {
                    return Err("Start a new remote conversation to change provider".into());
                }
                remote.summary.provider = provider.into();
                self.save_remote_session(&remote)?;
            }
            return Ok(());
        }
        self.shared_change_provider(id, provider)
    }
    pub fn save_ui_state(&self, state: &str) -> Result<(), String> {
        self.shared_save_setting("workspace", state)
    }
    fn local_sessions(&self) -> Result<Vec<Session>, String> {
        self.shared_sessions()
    }
    /// Each session another one started through the app CLI, and its caller.
    pub fn started_links(&self) -> Result<std::collections::HashMap<i64, i64>, String> {
        let mut links = self.shared_started_links()?;
        let saved: std::collections::HashMap<_, _> = self
            .settings_with_prefix("started-by:")?
            .into_iter()
            .map(|(child, value)| Ok((child, value.trim().parse().map_err(|_| "Invalid caller")?)))
            .collect::<Result<_, String>>()?;
        links.extend(saved);
        Ok(links)
    }
    pub fn save_started_link(&self, child: i64, caller: i64) -> Result<(), String> {
        self.save_setting(&format!("started-by:{child}"), &caller.to_string())
    }
    /// Every lead's last orchestration summary.
    pub fn orchestration_summaries(
        &self,
    ) -> Result<std::collections::HashMap<i64, crate::orchestration::Summary>, String> {
        Ok(self
            .settings_with_prefix("orchestration:")?
            .into_iter()
            .filter_map(|(lead, value)| Some((lead, serde_json::from_str(&value).ok()?)))
            .collect())
    }
    pub fn save_orchestration_summary(
        &self,
        lead: i64,
        summary: &crate::orchestration::Summary,
    ) -> Result<(), String> {
        let value = serde_json::to_string(summary).map_err(|e| e.to_string())?;
        self.save_setting(&format!("orchestration:{lead}"), &value)
    }
    /// Every saved run, by lead. A run that ended saves as empty.
    pub fn orchestration_runs(&self) -> Result<Vec<(i64, crate::orchestration::run::Run)>, String> {
        Ok(self
            .settings_with_prefix("orchestration-run:")?
            .into_iter()
            .filter_map(|(lead, value)| Some((lead, serde_json::from_str(&value).ok()?)))
            .collect())
    }
    pub fn save_orchestration_run(&self, lead: i64, state: &str) -> Result<(), String> {
        self.save_setting(&format!("orchestration-run:{lead}"), state)
    }
    /// `prefix<session id>` settings, by session id.
    fn settings_with_prefix(&self, prefix: &str) -> Result<Vec<(i64, String)>, String> {
        self.shared_settings_with_prefix(prefix)
    }
    /// Sessions whose latest message is saved as a draft and not yet sent.
    pub fn draft_sessions(&self) -> Result<std::collections::HashSet<i64>, String> {
        self.shared_drafts()
    }
    fn local_create(&self, provider: &str, cwd: &str) -> Result<Session, String> {
        self.shared_create(provider, cwd)
    }
    /// A WAL reader for background search, independent of the writer's mutex.
    /// In-memory fixture stores must continue using their original connection.
    pub fn search_reader(&self) -> Result<Option<Self>, String> {
        if self.path == Path::new(":memory:") {
            return Ok(None);
        }
        self.shared_search_reader().map(Some)
    }
    /// Stream saved message bodies without loading conversations into the UI.
    /// Rejoin native row fragments so phrase matches span storage boundaries.
    pub fn search_sessions(
        &self,
        query: &str,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(Vec<crate::search::Hit>, bool), String> {
        self.shared_search(query, cancel)
    }
    /// The cursor for the turns before `load`'s rows, when a Pilot's history
    /// is paged rather than loaded whole.
    pub fn history_before(&self, session: i64) -> Result<Option<i64>, String> {
        if self.remote_session(session)?.is_some() {
            return Ok(None);
        }
        self.shared_history_before(session)
    }
    /// The page of turns before `before`, or with `all` every older turn.
    pub fn load_before(&self, session: i64, before: i64, all: bool) -> Result<HistoryPage, String> {
        self.shared_load_before(session, before, all)
    }
    pub fn load(&self, session: i64) -> Result<Transcript, String> {
        if let Some(remote) = self.remote_session(session)? {
            let Some(id) = remote.session_id.as_deref() else {
                return Ok(Transcript::default());
            };
            let client = crate::remote::default_connections()?;
            let machine = remote.machine(&client)?;
            let mut snapshot = client.load_session(&machine.id, id, None)?;
            crate::remote::attachments::qualify_snapshot(
                &mut snapshot,
                &remote.project.environment_id,
                id,
            );
            return Self::project_remote_history(&snapshot).map(|(history, _)| history);
        }
        self.shared_load(session)
    }
    /// Drops a conversation's transcript rows from `start` on.
    pub fn trim_rows(&mut self, session: i64, start: usize) -> Result<(), String> {
        if self.remote_session(session)?.is_some() {
            return Err("Remote transcripts are owned by the host".into());
        }
        self.shared_trim_rows(session, start)
    }
    pub fn write_rows(&mut self, session: i64, start: usize, rows: &[Row]) -> Result<(), String> {
        self.write_rows_with_message(session, start, rows, None)
    }
    pub fn write_rows_with_message(
        &mut self,
        session: i64,
        start: usize,
        rows: &[Row],
        message_id: Option<&str>,
    ) -> Result<(), String> {
        if self.remote_session(session)?.is_some() {
            return Err("Remote transcripts are owned by the host".into());
        }
        self.shared_write_rows(session, start, rows, message_id)
    }
    /// Archiving preserves the transcript, provider context and configuration,
    /// and ends the session's tool access.
    pub fn set_archived(&self, id: i64, archived: bool) -> Result<(), String> {
        self.write_archived(id, archived)?;
        if archived {
            crate::tool_servers::revoke(id);
            crate::browser::session_deleted(id);
        }
        Ok(())
    }
    fn write_archived(&self, id: i64, archived: bool) -> Result<(), String> {
        if self.remote_session(id)?.is_some() {
            return self.patch_remote_session(id, serde_json::json!({"archived": archived}));
        }
        self.shared_patch(id, &[("archived", serde_json::json!(archived))])
    }
    pub fn set_pinned(&self, ids: &[i64], pinned: bool) -> Result<(), String> {
        let any_remote = ids
            .iter()
            .map(|id| self.remote_session(*id).map(|remote| remote.is_some()))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .any(|remote| remote);
        if any_remote {
            for id in ids {
                if self.remote_session(*id)?.is_some() {
                    self.patch_remote_session(*id, serde_json::json!({"pinned": pinned}))?;
                } else {
                    self.set_pinned(&[*id], pinned)?;
                }
            }
            return Ok(());
        }
        self.shared_pins(ids, pinned)
    }
    /// The work item one session is linked to, read on its own instead of
    /// listing every session ([`Self::sessions`]); for callers that poll a
    /// single session. Reads the same place [`Self::set_work_item`] writes.
    /// An error when the session doesn't exist.
    pub fn work_item(&self, id: i64) -> Result<Option<crate::session_groups::WorkItem>, String> {
        if let Some(remote) = self.remote_session(id)? {
            return Ok(remote.summary.work_item);
        }
        self.shared_work_item(id)
    }
    pub fn set_work_item(
        &self,
        id: i64,
        item: Option<&crate::session_groups::WorkItem>,
    ) -> Result<(), String> {
        if self.remote_session(id)?.is_some() {
            return self.patch_remote_session(id, serde_json::json!({"linkedWorkItem": item}));
        }
        self.shared_patch(
            id,
            &[(
                "linked_work_item_json",
                item.map(shared::encode)
                    .transpose()?
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null),
            )],
        )
    }
    /// A project's folders and folded groups.
    pub fn groups(&self, cwd: &str) -> Result<crate::session_groups::Groups, String> {
        self.shared_groups(cwd)
    }
    pub fn save_groups(
        &self,
        cwd: &str,
        groups: &crate::session_groups::Groups,
    ) -> Result<(), String> {
        self.shared_save_setting(&format!("groups:{cwd}"), &shared::encode(groups)?)
    }
    pub fn reminders(&self) -> Result<Vec<Reminder>, String> {
        self.shared_reminders()
    }
    /// Schedules (or reschedules, re-arming) each session's reminder.
    pub fn set_reminders(&mut self, ids: &[i64], due_at: u64) -> Result<(), String> {
        self.shared_set_reminders(ids, due_at)
    }
    /// Re-arms only the reminder occurrence the user snoozed. A stale popup
    /// must not overwrite a newer schedule created in another window.
    pub fn snooze_reminder_if(
        &self,
        id: i64,
        expected_due: u64,
        new_due: u64,
    ) -> Result<bool, String> {
        self.shared_snooze_reminder_if(id, expected_due, new_due)
    }
    /// Clears the sessions' reminders; with `expected`, only that one, so a
    /// stale delivery never clears a newer reminder.
    pub fn clear_reminders(&self, ids: &[i64], expected: Option<u64>) -> Result<(), String> {
        self.shared_clear_reminders(ids, expected)
    }
    /// Marks due, undelivered reminders delivered and returns them, once.
    pub fn take_due_reminders(&mut self, now: u64) -> Result<Vec<Reminder>, String> {
        let due: Vec<_> = self
            .reminders()?
            .into_iter()
            .filter(|r| r.due_at <= now && r.fired_at.is_none())
            .collect();
        self.shared_fire_reminders(&due, now)
    }
    /// Deletes the conversation, its settings and reminder for good.
    /// Removes the session and ends its tool access.
    pub fn delete(&mut self, id: i64) -> Result<(), String> {
        self.remove_session(id)?;
        crate::tool_servers::revoke(id);
        crate::browser::session_deleted(id);
        Ok(())
    }
    fn remove_session(&mut self, id: i64) -> Result<(), String> {
        if self.remote_session(id)?.is_some() {
            return self.delete_remote_session(id);
        }
        self.shared_delete(id)
    }
    pub fn provider_id(&self, id: i64, provider_id: &str) -> Result<(), String> {
        self.shared_patch(
            id,
            &[("provider_session_id", serde_json::json!(provider_id))],
        )
    }
    /// Keep conversation history while detaching a removed working copy.
    pub fn detach_worktree(
        &self,
        path: &str,
        main: &str,
    ) -> Result<Vec<(Session, crate::agent::Settings)>, String> {
        self.shared_detach_worktree(path, main)
    }
    /// Moves the sessions saved in `from` to `to`, as the desktop app's
    /// `session_rebase_project` does: only the folder changes.
    pub fn rebase_project(&self, from: &str, to: &str) -> Result<(), String> {
        if from.trim().is_empty() || to.trim().is_empty() {
            return Err("project paths are required".into());
        }
        self.shared_rebase_project(from, to)
    }
    /// Lists the sessions saved in the linked worktree `worktree` under its
    /// main checkout `root`, each still running in the worktree. Returns the
    /// sessions it moved.
    pub fn fold_worktree_sessions(&self, worktree: &str, root: &str) -> Result<Vec<i64>, String> {
        let ids: Vec<i64> = self
            .sessions()?
            .into_iter()
            .filter(|session| session.id >= 0 && session.cwd == worktree)
            .map(|session| session.id)
            .collect();
        if ids.is_empty() {
            return Ok(ids);
        }
        for id in &ids {
            let mut settings = self.agent_settings(*id)?.unwrap_or_default();
            if settings.worktree_cwd.is_none() {
                settings.worktree_cwd = Some(worktree.into());
                self.save_agent_settings(*id, &settings)?;
            }
        }
        self.rebase_project(worktree, root)?;
        Ok(ids)
    }
    pub fn reset_provider_id(&self, id: i64) -> Result<(), String> {
        self.shared_patch(id, &[("provider_session_id", serde_json::Value::Null)])
    }
    pub fn title(&self, id: i64, title: &str) -> Result<(), String> {
        if self.remote_session(id)?.is_some() {
            return self.patch_remote_session(id, serde_json::json!({"title": title}));
        }
        self.shared_patch(id, &[("title", serde_json::json!(title))])
    }
    /// A late generated title cannot replace a manual rename or recreate a deleted session.
    pub fn replace_title(&self, id: i64, expected: &str, title: &str) -> Result<bool, String> {
        self.shared_replace_title(id, expected, title)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sessions_from_the_gespan_store_carry_their_start_time() {
        let store = super::Store::open_gespan(std::path::Path::new(":memory:")).unwrap();
        let before = crate::time::now_ms();
        let first = store.create("claude", "/work/app").unwrap();
        let second = store.create("claude", "/work/app").unwrap();
        assert!(
            first.created_at >= before,
            "a new session knows when it started"
        );
        let listed = store.sessions().unwrap();
        let started = |id| listed.iter().find(|s| s.id == id).unwrap().created_at;
        assert_eq!(started(first.id), first.created_at, "and it reads back");
        assert!(started(second.id) >= started(first.id));
    }
    #[test]
    fn a_worktrees_sessions_fold_under_its_root_and_keep_running_there() {
        let store = super::Store::open(std::path::Path::new(":memory:")).unwrap();
        let tree = store.create("claude", "/work/app-worktrees/fix").unwrap();
        let nested = store.create("codex", "/work/app-worktrees/fix").unwrap();
        // A session that already picked another worktree keeps it.
        let picked = crate::agent::Settings {
            worktree_cwd: Some("/work/app-worktrees/other".into()),
            model: Some("opus".into()),
            ..Default::default()
        };
        store.save_agent_settings(nested.id, &picked).unwrap();
        let root = store.create("claude", "/work/app").unwrap();
        let elsewhere = store
            .create("claude", "/work/app-worktrees/fix/sub")
            .unwrap();
        let mut moved = store
            .fold_worktree_sessions("/work/app-worktrees/fix", "/work/app")
            .unwrap();
        moved.sort();
        let mut expected = vec![tree.id, nested.id];
        expected.sort();
        assert_eq!(moved, expected);
        let cwd = |id| {
            store
                .sessions()
                .unwrap()
                .into_iter()
                .find(|s| s.id == id)
                .unwrap()
                .cwd
        };
        assert_eq!(cwd(tree.id), "/work/app");
        assert_eq!(cwd(nested.id), "/work/app");
        assert_eq!(cwd(root.id), "/work/app");
        assert_eq!(cwd(elsewhere.id), "/work/app-worktrees/fix/sub");
        let worktree = |id| {
            store
                .agent_settings(id)
                .unwrap()
                .unwrap_or_default()
                .worktree_cwd
        };
        assert_eq!(
            worktree(tree.id).as_deref(),
            Some("/work/app-worktrees/fix")
        );
        assert_eq!(
            worktree(nested.id).as_deref(),
            Some("/work/app-worktrees/other")
        );
        assert_eq!(
            store
                .agent_settings(nested.id)
                .unwrap()
                .unwrap()
                .model
                .as_deref(),
            Some("opus")
        );
        assert_eq!(worktree(root.id), None);
        // Nothing left to move the second time.
        assert!(store
            .fold_worktree_sessions("/work/app-worktrees/fix", "/work/app")
            .unwrap()
            .is_empty());
    }
    #[test]
    fn create_from_seeds_a_new_session_with_rows_and_settings() {
        let mut store = super::Store::open(std::path::Path::new(":memory:")).unwrap();
        let rows = vec![
            super::Row {
                role: "user".into(),
                text: "Plan the footer".into(),
                first: true,
                sent_at: Some(7),
            },
            super::Row {
                role: "assistant".into(),
                text: "Here is the plan.".into(),
                first: true,
                sent_at: None,
            },
        ];
        let settings = crate::agent::Settings {
            model: Some("opus".into()),
            handoff: Some(crate::agent::Handoff {
                from: "codex".into(),
                brief: "## Session so far".into(),
            }),
            ..Default::default()
        };
        let summary = store
            .create_from("claude", "/work/gespan", "Fork of Footer", &settings, &rows)
            .unwrap();
        assert_eq!(summary.title, "Fork of Footer");
        assert!(summary.has_messages);
        assert_eq!(summary.provider_id, None);
        assert_eq!(store.load(summary.id).unwrap().rows, rows);
        let saved = store.agent_settings(summary.id).unwrap().unwrap();
        assert_eq!(saved.model.as_deref(), Some("opus"));
        assert_eq!(saved.handoff.unwrap().from, "codex");
        let listed = store.sessions().unwrap();
        let listed = listed.iter().find(|s| s.id == summary.id).unwrap();
        assert!(listed.has_messages);
        assert_eq!(listed.title, "Fork of Footer");
    }
    #[test]
    fn create_from_no_rows_is_an_empty_session() {
        let mut store = super::Store::open(std::path::Path::new(":memory:")).unwrap();
        let summary = store
            .create_from(
                "claude",
                "/work/gespan",
                "Handoff",
                &Default::default(),
                &[],
            )
            .unwrap();
        assert!(!summary.has_messages);
        assert!(store.load(summary.id).unwrap().rows.is_empty());
    }
    #[test]
    fn removed_worktree_keeps_history_and_requires_reattachment() {
        let mut store = super::Store::open(std::path::Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project-worktrees/feature").unwrap();
        let settings = crate::agent::Settings {
            worktree_cwd: Some("/project-worktrees/feature".into()),
            ..Default::default()
        };
        store.save_agent_settings(session.id, &settings).unwrap();
        store
            .provider_id(session.id, "old-provider-thread")
            .unwrap();
        store
            .write_rows(
                session.id,
                0,
                &[crate::transcript::Row {
                    role: "user".into(),
                    text: "Keep my conversation".into(),
                    first: true,
                    sent_at: None,
                }],
            )
            .unwrap();
        let detached = store
            .detach_worktree("/project-worktrees/feature", "/project")
            .unwrap();
        assert_eq!(detached.len(), 1);
        assert_eq!(detached[0].0.cwd, "/project");
        assert!(detached[0].0.provider_id.is_none());
        assert!(detached[0].1.worktree_removed);
        assert_eq!(
            store.load(session.id).unwrap().rows[0].text,
            "Keep my conversation"
        );
        assert!(
            store
                .agent_settings(session.id)
                .unwrap()
                .unwrap()
                .worktree_removed
        );
    }

    #[test]
    fn workspace_membership_reads_main_and_worktree_sessions_without_loading_history() {
        let store = super::Store::open(std::path::Path::new(":memory:")).unwrap();
        let main = store.create("codex", "/project").unwrap();
        let feature = store.create("claude", "/project").unwrap();
        let corrupt = store.create("codex", "/project").unwrap();
        store
            .save_agent_settings(
                feature.id,
                &crate::agent::Settings {
                    worktree_cwd: Some("/project-feature".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .save_setting(&format!("agent:{}", corrupt.id), "invalid json")
            .unwrap();
        let scopes = store.session_worktrees().unwrap();
        assert_eq!(scopes.len(), 3);
        assert_eq!(scopes[&main.id], None);
        assert_eq!(scopes[&feature.id].as_deref(), Some("/project-feature"));
        assert_eq!(scopes[&corrupt.id], None);
    }

    use super::*;
    #[test]
    fn archive_and_activity_time_survive_restart_without_changing_history() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("native.db");
        let mut store = Store::open(&path).unwrap();
        let session = store.create("codex", "/project").unwrap();
        assert!(session.updated_at > 0);
        let original = store.persisted_session_id(session.id).unwrap();
        store
            .conn()
            .execute("UPDATE sessions SET updated_at=1 WHERE id=?1", [&original])
            .unwrap();
        let rows = vec![Row {
            role: "user".into(),
            text: "Keep this conversation".into(),
            first: true,
            sent_at: None,
        }];
        store.write_rows(session.id, 0, &rows).unwrap();
        store.provider_id(session.id, "resume-this-thread").unwrap();
        let updated_at = store.sessions().unwrap()[0].updated_at;
        assert!(updated_at > 1);
        store.set_archived(session.id, true).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        let saved = &store.sessions().unwrap()[0];
        assert!(saved.archived);
        assert_eq!(
            saved.updated_at, updated_at,
            "archiving does not make an old session look new"
        );
        assert_eq!(saved.provider_id.as_deref(), Some("resume-this-thread"));
        assert_eq!(store.load(session.id).unwrap().rows, rows);
        store.set_archived(session.id, false).unwrap();
        assert!(!store.sessions().unwrap()[0].archived);
    }
    #[test]
    fn archiving_and_deleting_end_a_sessions_tool_access() {
        use crate::tool_servers::{is_open, Access, Mount};
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(&directory.path().join("sessions.db")).unwrap();
        let open = |id: i64| {
            let mount = Mount {
                session: id,
                ..Default::default()
            };
            assert!(Access::open(&mount).is_some() && is_open(id));
        };
        let archived = store.create("claude", "/work").unwrap().id;
        open(archived);
        store.set_archived(archived, false).unwrap();
        assert!(is_open(archived), "unarchiving changes nothing");
        store.set_archived(archived, true).unwrap();
        assert!(!is_open(archived));
        let deleted = store.create("claude", "/work").unwrap().id;
        open(deleted);
        store.delete(deleted).unwrap();
        assert!(!is_open(deleted));
    }
    #[test]
    fn a_saved_run_survives_restart_and_goes_with_its_lead() {
        use crate::orchestration::{
            run::{Run, RunStatus},
            Plan,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.sqlite3");
        let store = Store::open(&path).unwrap();
        let lead = store.create("codex", "/p").unwrap().id;
        let other = store.create("codex", "/p").unwrap().id;
        let run = Run::new(
            Plan {
                title: "Work".into(),
                summary: String::new(),
                tasks: Vec::new(),
                max_workers: 2,
            },
            "/p".into(),
            "codex".into(),
            Vec::new(),
        );
        let state = serde_json::to_string(&run).unwrap();
        store.save_orchestration_run(lead, &state).unwrap();
        store.save_orchestration_run(other, "").unwrap();
        drop(store);
        let mut store = Store::open(&path).unwrap();
        let saved = store.orchestration_runs().unwrap();
        assert_eq!(saved.len(), 1, "an ended run loads as nothing");
        assert_eq!(saved[0].0, lead);
        assert_eq!(saved[0].1.status, RunStatus::Active);
        assert_eq!(saved[0].1.max_workers, 2);
        store.delete(lead).unwrap();
        assert!(store.orchestration_runs().unwrap().is_empty());
    }
    #[test]
    fn started_links_and_orchestration_summaries_survive_restart_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.sqlite3");
        let store = Store::open(&path).unwrap();
        let lead = store.create("codex", "/p").unwrap().id;
        let child = store.create("codex", "/p").unwrap().id;
        store.save_started_link(child, lead).unwrap();
        let summary = crate::orchestration::Summary {
            tasks: vec![crate::orchestration::SummaryTask {
                session: Some(child),
                title: "Tests".into(),
                provider: "codex".into(),
                model: None,
                status: "running".into(),
            }],
            live: true,
            paused: false,
        };
        store.save_orchestration_summary(lead, &summary).unwrap();
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert_eq!(store.started_links().unwrap()[&child], lead);
        assert_eq!(store.orchestration_summaries().unwrap()[&lead], summary);
        assert_eq!(summary.tasks[0].label(&summary, false), "Working");
        store.delete(child).unwrap();
        store.delete(lead).unwrap();
        assert!(store.started_links().unwrap().is_empty());
        assert!(store.orchestration_summaries().unwrap().is_empty());
    }
    #[test]
    fn pins_links_groups_and_reminders_survive_restart_and_delete_clears_them() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("native.db");
        let mut store = Store::open(&path).unwrap();
        let keep = store.create("codex", "/project").unwrap();
        let gone = store.create("claude", "/project").unwrap();
        store.set_pinned(&[keep.id], true).unwrap();
        let item = crate::session_groups::WorkItem::Github(
            crate::session_groups::parse_github_work_item("https://github.com/acme/app/pull/9")
                .unwrap(),
        );
        store.set_work_item(keep.id, Some(&item)).unwrap();
        let mut groups = crate::session_groups::Groups::default();
        groups.create_folder(&[keep.id, gone.id], "f".into());
        groups.pinned_collapsed = true;
        store.save_groups("/project", &groups).unwrap();
        store.set_reminders(&[keep.id, gone.id], 100).unwrap();
        store.set_reminders(&[keep.id], 200).unwrap();
        drop(store);
        let mut store = Store::open(&path).unwrap();
        let saved = store
            .sessions()
            .unwrap()
            .into_iter()
            .find(|s| s.id == keep.id)
            .unwrap();
        assert!(saved.pinned);
        assert_eq!(saved.work_item, Some(item));
        assert_eq!(store.groups("/project").unwrap(), groups);
        assert_eq!(store.groups("/other").unwrap(), Default::default());
        let due = store.take_due_reminders(150).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].session_id, gone.id);
        assert!(
            store.take_due_reminders(150).unwrap().is_empty(),
            "delivered once"
        );
        store.clear_reminders(&[keep.id], Some(100)).unwrap();
        assert_eq!(
            store.reminders().unwrap().len(),
            2,
            "a stale time clears nothing"
        );
        store.delete(gone.id).unwrap();
        assert_eq!(store.reminders().unwrap().len(), 1);
        assert!(store.sessions().unwrap().iter().all(|s| s.id != gone.id));
        assert!(
            store.load(gone.id).map_or(true, |t| t.rows.is_empty()),
            "nothing of a deleted conversation loads"
        );
        store.clear_reminders(&[keep.id], None).unwrap();
        assert!(store.reminders().unwrap().is_empty());
        store.set_work_item(keep.id, None).unwrap();
        store.set_pinned(&[keep.id], false).unwrap();
        let saved = &store.sessions().unwrap()[0];
        assert!(!saved.pinned && saved.work_item.is_none());
    }
    fn ticket(key: &str) -> crate::session_groups::WorkItem {
        crate::session_groups::WorkItem::Ticket(Box::new(crate::tickets::TicketRef {
            tracker: crate::tickets::Tracker::Linear,
            key: key.into(),
            url: None,
            title: None,
            status: None,
            status_group: None,
            fetched_at: None,
        }))
    }
    #[test]
    fn work_item_reads_one_session_like_the_session_list_does() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&tmp.path().join("native.db")).unwrap();
        let linked = store.create("codex", "/project").unwrap();
        let plain = store.create("codex", "/project").unwrap();
        store
            .set_work_item(linked.id, Some(&ticket("ENG-1")))
            .unwrap();
        assert_eq!(store.work_item(linked.id).unwrap(), Some(ticket("ENG-1")));
        assert_eq!(store.work_item(plain.id).unwrap(), None);
        // A relink and an unlink show on the next read.
        store
            .set_work_item(linked.id, Some(&ticket("ENG-2")))
            .unwrap();
        assert_eq!(store.work_item(linked.id).unwrap(), Some(ticket("ENG-2")));
        store.set_work_item(linked.id, None).unwrap();
        assert_eq!(store.work_item(linked.id).unwrap(), None);
        // Unreadable JSON reads as no link, as in the list.
        let original = store.persisted_session_id(plain.id).unwrap();
        store
            .conn()
            .execute(
                "UPDATE sessions SET linked_work_item_json='{broken' WHERE id=?1",
                [&original],
            )
            .unwrap();
        assert_eq!(store.work_item(plain.id).unwrap(), None);
        // A session that doesn't exist is an error, not "no link".
        store.delete(plain.id).unwrap();
        assert!(store.work_item(plain.id).is_err());

        // A remote session reads from its saved binding.
        let project = store
            .remember_remote_project(
                "env",
                &serde_json::json!({"id":"project","cwd":"/host/repo"}),
            )
            .unwrap();
        let remote = store.create("codex", &project.key).unwrap();
        assert!(remote.id < 0);
        assert_eq!(store.work_item(remote.id).unwrap(), None);
        store
            .set_work_item(remote.id, Some(&ticket("ENG-3")))
            .unwrap();
        assert_eq!(store.work_item(remote.id).unwrap(), Some(ticket("ENG-3")));
        let listed = store
            .sessions()
            .unwrap()
            .into_iter()
            .find(|s| s.id == remote.id)
            .unwrap();
        assert_eq!(listed.work_item, Some(ticket("ENG-3")));
    }
    #[test]
    fn send_times_survive_streaming_restart_and_queued_delivery() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("native.db");
        let mut store = Store::open(&path).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let sent_at = Some(1_700_000_000_000);
        let mut transcript = Transcript::default();
        transcript.begin("user");
        transcript.rows[0].sent_at = sent_at;
        transcript.append(&"A long queued prompt. ".repeat(300));
        assert!(transcript.rows.len() > 1);
        assert!(transcript.rows[1..].iter().all(|r| r.sent_at.is_none()));
        let answer = transcript.begin("assistant");
        transcript.append("Starting");
        store.write_rows(session.id, 0, &transcript.rows).unwrap();
        let answer_time = transcript.rows[answer].sent_at;
        transcript.append(" and finished");
        store
            .write_rows(session.id, answer, &transcript.rows[answer..])
            .unwrap();
        let mut roster = crate::pilot::Roster::default();
        let pilot = roster.create(Vec::new()).id.clone();
        roster
            .get_mut(&pilot)
            .unwrap()
            .outbox
            .push(crate::pilot::Message {
                id: "queued".into(),
                text: "Tomorrow".into(),
                sent_at,
                internal: false,
                error: None,
                attachments: Vec::new(),
                report: Vec::new(),
            });
        roster.save(&store).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        let saved = store.load(session.id).unwrap();
        assert_eq!(saved.rows, transcript.rows);
        assert_eq!(saved.rows[answer].sent_at, answer_time);
        assert_eq!(
            crate::pilot::Roster::load(&store).unwrap().pilots[0].outbox[0].sent_at,
            sent_at
        );
        let legacy: Row =
            serde_json::from_str(r#"{"role":"user","text":"Old","first":true}"#).unwrap();
        assert_eq!(legacy.sent_at, None);
    }
    #[test]
    fn delayed_configuration_cannot_restore_an_old_model_or_provider() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/tmp").unwrap();
        let old = crate::agent::Settings {
            revision: 1,
            model: Some("first-model".into()),
            ..Default::default()
        };
        let current = crate::agent::Settings {
            revision: 2,
            model: Some("selected-model".into()),
            effort: Some("high".into()),
            ..Default::default()
        };
        store
            .configure_agent(session.id, "claude", &current)
            .unwrap();
        store.provider_id(session.id, "new-thread").unwrap();
        store.configure_agent(session.id, "codex", &old).unwrap();
        let saved = store.agent_settings(session.id).unwrap().unwrap();
        assert_eq!(saved.model, current.model);
        assert_eq!(saved.effort, current.effort);
        let session = store.sessions().unwrap().remove(0);
        assert_eq!(session.provider, "claude");
        assert_eq!(session.provider_id.as_deref(), Some("new-thread"));
    }
    #[test]
    fn wal_restart_preserves_partial_output_and_provider_context() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("native.db");
        let mut store = Store::open(&path).unwrap();
        let s = store.create("codex", "/tmp").unwrap();
        assert!(!s.has_messages);
        assert!(!store.sessions().unwrap()[0].has_messages);
        let mut t = Transcript::default();
        t.begin("user");
        t.append("hello");
        t.begin("assistant");
        t.append("partial 🙂");
        store.write_rows(s.id, 0, &t.rows).unwrap();
        store.provider_id(s.id, "thread-1").unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(store.sessions().unwrap()[0].has_messages);
        assert_eq!(store.load(s.id).unwrap().rows, t.rows);
        assert_eq!(
            store.sessions().unwrap()[0].provider_id.as_deref(),
            Some("thread-1")
        );
    }
}

#[cfg(test)]
mod trim_tests {
    use super::*;

    fn rows(pairs: &[(&str, &str)]) -> Vec<Row> {
        let mut transcript = Transcript::default();
        for (role, text) in pairs {
            transcript.begin(role);
            transcript.append(text);
        }
        transcript.rows
    }
    #[test]
    fn trimming_drops_only_the_tail_in_both_stores() {
        for shared in [false, true] {
            let mut store = if shared {
                Store::open_gespan(Path::new(":memory:"))
            } else {
                Store::open(Path::new(":memory:"))
            }
            .unwrap();
            let session = store.create("codex", "/project").unwrap().id;
            let saved = rows(&[
                ("user", "Earlier"),
                ("assistant", "Kept"),
                ("user", "Fix the login"),
                ("error", "Signed out of Codex."),
            ]);
            store.write_rows(session, 0, &saved).unwrap();
            store.trim_rows(session, 3).unwrap();
            assert_eq!(
                store.load(session).unwrap().rows,
                saved[..3],
                "shared: {shared}"
            );
            // Trimming past the end changes nothing.
            store.trim_rows(session, 10).unwrap();
            assert_eq!(store.load(session).unwrap().rows.len(), 3);
        }
    }
}
