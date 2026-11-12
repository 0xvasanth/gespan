//! The tables of the shared `gespan.db` that `Store` keeps, as Toasty models.
//!
//! Gespan owns this file. Older Tauri builds made these tables by hand; a
//! file they left is adopted on open (see `orm::adopt`): [`legacy_upgrade`]
//! first brings each table up to the newest Tauri shape, then adoption
//! rebuilds it into the models' DDL. Every column any released Tauri schema
//! had is modelled here, so adoption drops no data.
//!
//! The store reads and writes these tables with SQL on the same Toasty
//! database (see `store::db`); the models define their schema and serve the
//! typed reads and writes that need no projection.
//!
//! Toasty's DDL has no DEFAULT, REFERENCES, CHECK, partial or descending
//! index. [`CONSTRAINTS`] restores what the Tauri schema had.
use rusqlite::Connection;

/// A conversation. Columns follow the Tauri layout; everything after
/// `blocks_json` is read through `sessions_cwd_cover_idx`.
#[derive(Debug, toasty::Model)]
#[table = "sessions"]
pub(crate) struct SessionRecord {
    #[key]
    id: String,
    cwd: String,
    harness: String,
    model: String,
    model_settings: String,
    runtime_mode: String,
    title: String,
    provider_session_id: Option<String>,
    blocks_json: String,
    created_at: i64,
    updated_at: i64,
    branch: Option<String>,
    context_used: Option<i64>,
    context_window: Option<i64>,
    archived: bool,
    worktree_cwd: Option<String>,
    has_user_message: bool,
    pinned: bool,
    linked_work_item_json: Option<String>,
    provider_account_id: Option<String>,
    worktree_removed: bool,
    is_draft: bool,
    automation_id: Option<String>,
    queued_messages_json: String,
    queue_status: Option<String>,
    sidebar_hidden: bool,
    /// Compatibility only: earlier Inbox Ask builds marked their chats here.
    inbox_ask: Option<String>,
}

/// Tauri's own migration ledger. Kept as it is so an adopted file still
/// tells which Tauri migrations ran; Gespan's are in `__toasty_migrations`.
#[derive(Debug, toasty::Model)]
#[table = "schema_migrations"]
pub(crate) struct TauriMigration {
    #[key]
    version: i64,
    applied_at: i64,
}

/// The desktop workspace layout: one row, id 1.
#[derive(Debug, toasty::Model)]
#[table = "workspace_snapshot"]
pub(crate) struct WorkspaceSnapshot {
    #[key]
    id: i64,
    snapshot_json: String,
    updated_at: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "in_flight_sessions"]
pub(crate) struct InFlightSession {
    #[key]
    session_id: String,
    cwd: String,
    sort_index: i64,
}

/// Notes and artifacts share this table, told apart by `content_kind`.
#[derive(Debug, toasty::Model)]
#[table = "notes"]
pub(crate) struct NoteRecord {
    #[key]
    pub(crate) id: String,
    #[unique]
    pub(crate) slug: String,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) tags_json: String,
    pub(crate) source_session_id: Option<String>,
    pub(crate) source_cwd: Option<String>,
    pub(crate) slug_pending: bool,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) content_kind: String,
    pub(crate) artifact_kind: Option<String>,
}

#[derive(Debug, toasty::Model)]
#[table = "session_reminders"]
pub(crate) struct SessionReminder {
    #[key]
    session_id: String,
    due_at: i64,
    fired_at: Option<i64>,
}

/// An indexed Pilot history: its blocks live in `pilot_blocks`.
#[derive(Debug, toasty::Model)]
#[table = "pilot_transcripts"]
pub(crate) struct PilotTranscript {
    #[key]
    session_id: String,
}

/// The block sequence each turn of an indexed Pilot starts at.
#[derive(Debug, toasty::Model)]
#[table = "pilot_turns"]
#[key(session_id, seq)]
pub(crate) struct PilotTurn {
    session_id: String,
    seq: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "pilot_blocks"]
#[key(session_id, seq)]
#[unique(session_id, block_id)]
#[index(name = "pilot_blocks_user", session_id, is_user, is_draft)]
pub(crate) struct PilotBlock {
    session_id: String,
    seq: i64,
    block_id: String,
    block_json: String,
    search_text: String,
    is_user: bool,
    is_draft: bool,
}

#[derive(Debug, toasty::Model)]
#[table = "orchestration_runs"]
pub(crate) struct OrchestrationRun {
    #[key]
    lead_id: String,
    state: String,
}

#[derive(Debug, toasty::Model)]
#[table = "orchestration_sidebar"]
pub(crate) struct OrchestrationSidebar {
    #[key]
    lead_id: String,
    summary: String,
}

/// A session an orchestration lead started; the sidebar lists it under the lead.
#[derive(Debug, toasty::Model)]
#[table = "orchestration_workers"]
pub(crate) struct OrchestrationWorker {
    #[key]
    session_id: String,
    lead_id: String,
}

/// One native setting in `gespan.db.gpui.sqlite3`, the Settings database.
#[derive(Debug, toasty::Model)]
#[table = "settings"]
pub(crate) struct Setting {
    #[key]
    pub(crate) key: String,
    pub(crate) value: String,
}

/// Run on every open, after the migrations. What the Tauri schema declared
/// and Toasty's DDL cannot: descending and partial indexes, the
/// `ON DELETE CASCADE` foreign keys, and the CHECK rules, as triggers.
pub(crate) const CONSTRAINTS: &[&str] = &[
    // The sidebar projection, kept off the table (and off `blocks_json`).
    // The session list names it with INDEXED BY.
    "CREATE INDEX IF NOT EXISTS sessions_cwd_cover_idx
       ON sessions (cwd, has_user_message, updated_at DESC, id, harness,
                    model, runtime_mode, title, provider_session_id,
                    created_at, branch, archived, pinned,
                    linked_work_item_json, worktree_cwd, worktree_removed,
                    is_draft, automation_id, sidebar_hidden)",
    "CREATE INDEX IF NOT EXISTS sessions_legacy_inbox ON sessions (id) WHERE inbox_ask IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS notes_updated_idx ON notes (updated_at DESC, id)",
    "CREATE INDEX IF NOT EXISTS session_reminders_pending ON session_reminders (due_at)
       WHERE fired_at IS NULL",
    // session_reminders.due_at: CHECK (due_at > 0).
    "CREATE TRIGGER IF NOT EXISTS session_reminders_due_insert BEFORE INSERT ON session_reminders
       WHEN NEW.due_at <= 0
       BEGIN SELECT RAISE(ABORT, 'CHECK constraint failed: due_at > 0'); END",
    "CREATE TRIGGER IF NOT EXISTS session_reminders_due_update
       BEFORE UPDATE OF due_at ON session_reminders WHEN NEW.due_at <= 0
       BEGIN SELECT RAISE(ABORT, 'CHECK constraint failed: due_at > 0'); END",
    // workspace_snapshot.id: CHECK (id = 1).
    "CREATE TRIGGER IF NOT EXISTS workspace_snapshot_one_insert BEFORE INSERT ON workspace_snapshot
       WHEN NEW.id IS NOT 1
       BEGIN SELECT RAISE(ABORT, 'CHECK constraint failed: id = 1'); END",
    "CREATE TRIGGER IF NOT EXISTS workspace_snapshot_one_update
       BEFORE UPDATE OF id ON workspace_snapshot WHEN NEW.id IS NOT 1
       BEGIN SELECT RAISE(ABORT, 'CHECK constraint failed: id = 1'); END",
    // session_reminders and pilot_transcripts REFERENCES sessions(id).
    "CREATE TRIGGER IF NOT EXISTS session_reminders_session_insert
       BEFORE INSERT ON session_reminders
       WHEN NOT EXISTS (SELECT 1 FROM sessions WHERE id = NEW.session_id)
       BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END",
    "CREATE TRIGGER IF NOT EXISTS session_reminders_session_update
       BEFORE UPDATE OF session_id ON session_reminders
       WHEN NOT EXISTS (SELECT 1 FROM sessions WHERE id = NEW.session_id)
       BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END",
    "CREATE TRIGGER IF NOT EXISTS pilot_transcripts_session_insert
       BEFORE INSERT ON pilot_transcripts
       WHEN NOT EXISTS (SELECT 1 FROM sessions WHERE id = NEW.session_id)
       BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END",
    // pilot_turns and pilot_blocks REFERENCES pilot_transcripts(session_id).
    "CREATE TRIGGER IF NOT EXISTS pilot_turns_transcript_insert BEFORE INSERT ON pilot_turns
       WHEN NOT EXISTS (SELECT 1 FROM pilot_transcripts WHERE session_id = NEW.session_id)
       BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END",
    "CREATE TRIGGER IF NOT EXISTS pilot_blocks_transcript_insert BEFORE INSERT ON pilot_blocks
       WHEN NOT EXISTS (SELECT 1 FROM pilot_transcripts WHERE session_id = NEW.session_id)
       BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END",
    // ON DELETE CASCADE.
    "CREATE TRIGGER IF NOT EXISTS sessions_delete_cascade AFTER DELETE ON sessions
       BEGIN
         DELETE FROM session_reminders WHERE session_id = OLD.id;
         DELETE FROM pilot_transcripts WHERE session_id = OLD.id;
       END",
    "CREATE TRIGGER IF NOT EXISTS pilot_transcripts_delete_cascade
       AFTER DELETE ON pilot_transcripts
       BEGIN
         DELETE FROM pilot_turns WHERE session_id = OLD.session_id;
         DELETE FROM pilot_blocks WHERE session_id = OLD.session_id;
       END",
];

fn has_table(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
}

/// Adds `column` to `table` when an older build left it out. `then` runs
/// once, right after, to fill the new column from the row.
fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    declaration: &str,
    then: Option<&str>,
) -> rusqlite::Result<()> {
    let present: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        [table, column],
        |row| row.get(0),
    )?;
    if !present {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {declaration}"
        ))?;
        if let Some(fill) = then {
            conn.execute_batch(fill)?;
        }
    }
    Ok(())
}

/// Adoption's legacy step for the tables above: brings a file a Tauri build
/// (or a hand-written Gespan build) left up to the newest hand-made shape,
/// so the rebuild copies every value and no NOT NULL column is left empty.
/// Tables the file lacks need nothing; adoption creates them.
pub(crate) fn legacy_upgrade(conn: &Connection) -> rusqlite::Result<()> {
    if has_table(conn, "sessions")? {
        // Tauri's `ensure_session_column` list, in the order it added them.
        // The fills are what the store read in place of a missing column.
        for (column, declaration, fill) in [
            ("branch", "TEXT", None),
            ("context_used", "INTEGER", None),
            ("context_window", "INTEGER", None),
            ("archived", "INTEGER NOT NULL DEFAULT 0", None),
            ("worktree_cwd", "TEXT", None),
            (
                "has_user_message",
                "INTEGER NOT NULL DEFAULT 0",
                Some("UPDATE sessions SET has_user_message = CASE WHEN blocks_json != '[]' THEN 1 ELSE 0 END"),
            ),
            ("pinned", "INTEGER NOT NULL DEFAULT 0", None),
            ("linked_work_item_json", "TEXT", None),
            ("provider_account_id", "TEXT", None),
            ("worktree_removed", "INTEGER NOT NULL DEFAULT 0", None),
            ("is_draft", "INTEGER NOT NULL DEFAULT 0", None),
            ("automation_id", "TEXT", None),
            ("queued_messages_json", "TEXT NOT NULL DEFAULT '[]'", None),
            ("queue_status", "TEXT", None),
            ("sidebar_hidden", "INTEGER NOT NULL DEFAULT 0", None),
            ("inbox_ask", "TEXT", None),
        ] {
            ensure_column(conn, "sessions", column, declaration, fill)?;
        }
    }
    if has_table(conn, "notes")? {
        // Existing slugs may already be referenced, so every existing row
        // starts established (slug_pending 0) whatever its slug looks like.
        for (column, declaration) in [
            ("tags_json", "TEXT NOT NULL DEFAULT '[]'"),
            ("slug_pending", "INTEGER NOT NULL DEFAULT 0"),
            ("content_kind", "TEXT NOT NULL DEFAULT 'note'"),
            ("artifact_kind", "TEXT"),
        ] {
            ensure_column(conn, "notes", column, declaration, None)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::orm::{open, Kind};
    use std::path::Path;

    fn exec(database: &crate::orm::Database, sql: &'static str) -> Result<u64, String> {
        database.run(|mut db| async move { toasty::sql::statement(sql).exec(&mut db).await })
    }

    fn count(database: &crate::orm::Database, sql: &'static str) -> i64 {
        let rows = database
            .run(|mut db| async move { toasty::sql::query(sql).exec(&mut db).await })
            .unwrap();
        match &rows[0] {
            toasty::stmt::Value::Record(record) => match record.fields[0] {
                toasty::stmt::Value::I64(n) => n,
                ref other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    fn session(database: &crate::orm::Database, id: &'static str) {
        let sql: &'static str = match id {
            "a" => "INSERT INTO sessions VALUES('a','/p','codex','codex:m','{}','supervised','A',NULL,'[]',1,1,NULL,NULL,NULL,0,NULL,0,0,NULL,NULL,0,0,NULL,'[]',NULL,0,NULL)",
            _ => "INSERT INTO sessions VALUES('b','/p','codex','codex:m','{}','supervised','B',NULL,'[]',1,1,NULL,NULL,NULL,0,NULL,0,0,NULL,NULL,0,0,NULL,'[]',NULL,0,NULL)",
        };
        exec(database, sql).unwrap();
    }

    #[test]
    fn deleting_a_session_cascades_to_its_reminder_and_pilot_history() {
        let database = open(Kind::Gespan, Path::new(":memory:")).unwrap();
        session(&database, "a");
        session(&database, "b");
        for sql in [
            "INSERT INTO session_reminders VALUES('a',10,NULL)",
            "INSERT INTO session_reminders VALUES('b',10,NULL)",
            "INSERT INTO pilot_transcripts VALUES('a')",
            "INSERT INTO pilot_turns VALUES('a',1)",
            "INSERT INTO pilot_blocks VALUES('a',1,'x','{}','',1,0)",
        ] {
            exec(&database, sql).unwrap();
        }
        exec(&database, "DELETE FROM sessions WHERE id='a'").unwrap();
        assert_eq!(
            count(&database, "SELECT count(*) FROM session_reminders"),
            1
        );
        for table in [
            "SELECT count(*) FROM pilot_transcripts",
            "SELECT count(*) FROM pilot_turns",
            "SELECT count(*) FROM pilot_blocks",
        ] {
            assert_eq!(count(&database, table), 0, "{table}");
        }
    }

    #[test]
    fn rows_must_point_at_an_existing_parent() {
        let database = open(Kind::Gespan, Path::new(":memory:")).unwrap();
        for sql in [
            "INSERT INTO session_reminders VALUES('missing',10,NULL)",
            "INSERT INTO pilot_transcripts VALUES('missing')",
            "INSERT INTO pilot_turns VALUES('missing',1)",
            "INSERT INTO pilot_blocks VALUES('missing',1,'x','{}','',1,0)",
        ] {
            let error = exec(&database, sql).unwrap_err();
            assert!(error.contains("FOREIGN KEY"), "{sql}: {error}");
        }
        session(&database, "a");
        exec(
            &database,
            "INSERT INTO session_reminders VALUES('a',10,NULL)",
        )
        .unwrap();
        let error = exec(&database, "UPDATE session_reminders SET session_id='gone'").unwrap_err();
        assert!(error.contains("FOREIGN KEY"), "{error}");
    }

    #[test]
    fn a_reminder_is_due_at_a_positive_time() {
        let database = open(Kind::Gespan, Path::new(":memory:")).unwrap();
        session(&database, "a");
        for sql in [
            "INSERT INTO session_reminders VALUES('a',0,NULL)",
            "INSERT INTO session_reminders VALUES('a',-5,NULL)",
        ] {
            let error = exec(&database, sql).unwrap_err();
            assert!(error.contains("due_at > 0"), "{error}");
        }
        exec(
            &database,
            "INSERT INTO session_reminders VALUES('a',1,NULL)",
        )
        .unwrap();
        let error = exec(&database, "UPDATE session_reminders SET due_at=0").unwrap_err();
        assert!(error.contains("due_at > 0"), "{error}");
    }

    #[test]
    fn the_workspace_snapshot_has_one_row() {
        let database = open(Kind::Gespan, Path::new(":memory:")).unwrap();
        let error = exec(&database, "INSERT INTO workspace_snapshot VALUES(2,'{}',1)").unwrap_err();
        assert!(error.contains("id = 1"), "{error}");
        exec(&database, "INSERT INTO workspace_snapshot VALUES(1,'{}',1)").unwrap();
        let error = exec(&database, "UPDATE workspace_snapshot SET id=2").unwrap_err();
        assert!(error.contains("id = 1"), "{error}");
    }

    #[test]
    fn partial_and_descending_indexes_exist_and_are_used() {
        let database = open(Kind::Gespan, Path::new(":memory:")).unwrap();
        for (name, partial) in [
            ("sessions_cwd_cover_idx", false),
            ("sessions_legacy_inbox", true),
            ("notes_updated_idx", false),
            ("session_reminders_pending", true),
            ("pilot_blocks_user", false),
        ] {
            let rows = database
                .run(|mut db| async move {
                    toasty::sql::query("SELECT partial FROM pragma_index_list(?1) WHERE name=?2")
                        .bind(match name {
                            "sessions_cwd_cover_idx" | "sessions_legacy_inbox" => "sessions",
                            "notes_updated_idx" => "notes",
                            "session_reminders_pending" => "session_reminders",
                            _ => "pilot_blocks",
                        })
                        .bind(name)
                        .exec(&mut db)
                        .await
                })
                .unwrap();
            assert_eq!(
                format!("{rows:?}"),
                format!(
                    "{:?}",
                    vec![toasty::stmt::Value::record_from_vec(vec![i64::from(
                        partial
                    )
                    .into()])]
                ),
                "{name}"
            );
        }
        // The pending-reminder scan reads the partial index.
        let plan = database
            .run(|mut db| async move {
                toasty::sql::query(
                    "EXPLAIN QUERY PLAN SELECT session_id FROM session_reminders
                     WHERE fired_at IS NULL AND due_at <= 5",
                )
                .exec(&mut db)
                .await
            })
            .unwrap();
        assert!(
            format!("{plan:?}").contains("session_reminders_pending"),
            "{plan:?}"
        );
        // The descending column survives in the cover index.
        let sql = count(
            &database,
            "SELECT count(*) FROM sqlite_master WHERE name='sessions_cwd_cover_idx'
             AND sql LIKE '%updated_at DESC%'",
        );
        assert_eq!(sql, 1);
    }

    #[test]
    fn constraints_survive_reopening_an_adopted_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        drop(open(Kind::Gespan, &path).unwrap());
        let database = open(Kind::Gespan, &path).unwrap();
        let triggers = count(
            &database,
            "SELECT count(*) FROM sqlite_master WHERE type='trigger'",
        );
        // Every Gespan trigger: this module's and the other stores' in the file.
        assert_eq!(
            triggers as usize,
            Kind::Gespan
                .constraints()
                .iter()
                .filter(|sql| sql.contains("CREATE TRIGGER"))
                .count()
        );
    }
}
