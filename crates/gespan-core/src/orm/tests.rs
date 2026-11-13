use super::adopt::backup_path;
use super::{migrations, open, run, Kind};
use crate::store::my_tasks::LEGACY_SCHEMA;
use rusqlite::Connection;
use std::path::Path;

/// Every table and index, with the SQL SQLite keeps for it.
fn schema(path: &Path) -> Vec<(String, String, Option<String>)> {
    let conn = Connection::open(path).unwrap();
    let mut statement = conn
        .prepare(
            "SELECT type, name, sql FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .unwrap();
    let schema = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    schema
}

fn applied(path: &Path) -> Vec<(i64, String)> {
    let conn = Connection::open(path).unwrap();
    let mut statement = conn
        .prepare("SELECT id, name FROM __toasty_migrations ORDER BY id")
        .unwrap();
    let applied = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    applied
}

/// A file exactly as a build that shipped only `kind`'s first `count`
/// migrations left it: those migrations' SQL run, and recorded as applied
/// the way Toasty records them.
pub(crate) fn file_at_migration(kind: Kind, path: &Path, count: usize) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE __toasty_migrations (
           id INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL
         );",
    )
    .unwrap();
    for migration in &kind.migrations().migrations()[..count] {
        for statement in migration.sql().split("\n-- #[toasty::breakpoint]\n") {
            if !statement.trim().is_empty() {
                conn.execute_batch(statement).unwrap();
            }
        }
        conn.execute(
            "INSERT INTO __toasty_migrations (id, name, applied_at)
             VALUES (?1, ?2, datetime('now'))",
            rusqlite::params![migration.id() as i64, migration.name()],
        )
        .unwrap();
    }
}

/// A `gespan.db` as the hand-written store left it, with other tables and
/// rows in every column.
fn legacy_file(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(LEGACY_SCHEMA).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE worktree_removals (path TEXT PRIMARY KEY, sessions_json TEXT);
         INSERT INTO worktree_removals VALUES ('s1', 'Kept as it is');
         INSERT INTO task_decisions (id, project, statement, created_at, updated_at)
           VALUES (7, '/work/gespan', 'Recount at read time', 10, 20);
         INSERT INTO task_decisions VALUES
           (9, '/work/gespan', 'Archived one', 'because', 11, 21, 31);
         INSERT INTO my_tasks VALUES
           ('question:1:a', 'question', '/work/gespan', 'session:1', 'Fix fsid',
            'How?', '{\"items\":[]}', 100, 'answered', 'Recount', '[[\"Recount\"]]',
            120, 7),
           ('question:1:b', 'question', '/work/gespan', 'session:1', 'Fix fsid',
            'Still waiting?', '{}', 130, 'open', NULL, NULL, NULL, NULL);",
    )
    .unwrap();
}

fn rows(path: &Path, sql: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let conn = Connection::open(path).unwrap();
    let mut statement = conn.prepare(sql).unwrap();
    let width = statement.column_count();
    statement
        .query_map([], |row| (0..width).map(|i| row.get(i)).collect())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn adopting_keeps_every_row_of_the_hand_made_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    legacy_file(&path);
    let tasks = "SELECT key, kind, project, origin, source_label, headline, request_json,
        asked_at, status, answer, answer_json, closed_at, decision_id FROM my_tasks ORDER BY key";
    let decisions = "SELECT id, project, statement, why, created_at, updated_at, archived_at
        FROM task_decisions ORDER BY id";
    let before = (rows(&path, tasks), rows(&path, decisions));

    open(Kind::Gespan, &path).unwrap();

    assert_eq!((rows(&path, tasks), rows(&path, decisions)), before);
    assert_eq!(
        rows(&path, "SELECT path, sessions_json FROM worktree_removals"),
        [[
            rusqlite::types::Value::Text("s1".into()),
            rusqlite::types::Value::Text("Kept as it is".into())
        ]],
        "tables no model names are left alone"
    );
    // Adoption records the first migration; the later ones then apply.
    let mut every: Vec<(i64, String)> = Kind::Gespan
        .migrations()
        .migrations()
        .iter()
        .map(|m| (m.id() as i64, m.name().to_owned()))
        .collect();
    every.sort();
    assert_eq!(applied(&path), every);
}

#[test]
fn an_adopted_file_has_the_same_schema_as_a_fresh_install() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = dir.path().join("fresh.db");
    let adopted = dir.path().join("adopted.db");
    open(Kind::Gespan, &fresh).unwrap();
    legacy_file(&adopted);

    open(Kind::Gespan, &adopted).unwrap();

    let without_other_tables = |path: &Path| {
        let mut schema = schema(path);
        schema.retain(|(_, name, _)| name != "worktree_removals");
        schema
    };
    assert_eq!(without_other_tables(&adopted), schema(&fresh));
}

#[test]
fn adopted_data_still_reads_through_the_models() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    legacy_file(&path);

    let store = crate::store::Store::open(&path).unwrap();

    let past = store.past_tasks().unwrap();
    assert_eq!(past.len(), 1);
    assert_eq!(past[0].answer.as_deref(), Some("Recount"));
    assert_eq!(past[0].decision, Some(7));
    let [decision] = &store.decisions().unwrap()[..] else {
        panic!("the archived decision stays off the list");
    };
    assert_eq!((decision.id, decision.sources.len()), (7, 1));
    // The adopted id sequence continues past the copied rows.
    let id = store.save_decision("/p", "Next", "", &[], 40).unwrap();
    assert!(id > 9, "{id}");
}

#[test]
fn reopening_applies_nothing_and_leaves_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    legacy_file(&path);
    open(Kind::Gespan, &path).unwrap();
    let schema_before = schema(&path);
    let applied_before = applied(&path);
    std::fs::remove_file(backup_path(&path)).unwrap();

    open(Kind::Gespan, &path).unwrap();

    assert_eq!(schema(&path), schema_before);
    assert_eq!(applied(&path), applied_before);
    assert!(
        !backup_path(&path).exists(),
        "nothing to apply, so no backup either"
    );
}

#[test]
fn a_pending_migration_is_applied_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    // A file from before the latest migration.
    let count = Kind::Gespan.migrations().migrations().len();
    let last = *Kind::Gespan.migrations().migrations().last().unwrap();
    file_at_migration(Kind::Gespan, &path, count - 1);

    let database = open(Kind::Gespan, &path).unwrap();
    open(Kind::Gespan, &path).unwrap();

    let ids: Vec<i64> = applied(&path).into_iter().map(|(id, _)| id).collect();
    assert_eq!(
        ids.iter().filter(|id| **id == last.id() as i64).count(),
        1,
        "applied once, then skipped"
    );
    let report = run(Kind::Gespan.migrations().apply(&database.db())).unwrap();
    assert_eq!((report.applied(), report.skipped()), (0, ids.len()));
}

#[test]
fn a_backup_is_written_before_adoption_changes_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    legacy_file(&path);

    open(Kind::Gespan, &path).unwrap();

    let backup = backup_path(&path);
    assert_eq!(
        backup.file_name().unwrap().to_str(),
        Some("gespan.db.bak-pre-toasty")
    );
    // The backup is the file before adoption: hand-made tables, no tracking.
    let names: Vec<String> = schema(&backup).into_iter().map(|(_, n, _)| n).collect();
    assert!(names.contains(&"my_tasks".to_owned()), "{names:?}");
    assert!(!names.contains(&"__toasty_migrations".to_owned()));
    let conn = Connection::open(&backup).unwrap();
    let kept: i64 = conn
        .query_row("SELECT count(*) FROM my_tasks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(kept, 2);
}

#[test]
fn a_new_file_gets_no_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("gespan.db");
    open(Kind::Gespan, &path).unwrap();
    assert!(path.exists());
    assert!(!backup_path(&path).exists());
}

#[test]
fn connections_get_gespans_pragmas() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gespan.db");
    let database = open(Kind::Gespan, &path).unwrap();
    let rows = database
        .run(|mut db| async move {
            toasty::sql::query(
                "SELECT (SELECT timeout FROM pragma_busy_timeout),
                        (SELECT foreign_keys FROM pragma_foreign_keys),
                        (SELECT journal_mode FROM pragma_journal_mode)",
            )
            .exec(&mut db)
            .await
        })
        .unwrap();
    assert_eq!(
        format!("{rows:?}"),
        format!(
            "{:?}",
            vec![toasty::stmt::Value::record_from_vec(vec![
                5000_i64.into(),
                1_i64.into(),
                "wal".into()
            ])]
        )
    );
}

#[test]
fn a_failed_transaction_rolls_back() {
    let database = open(Kind::Gespan, Path::new(":memory:")).unwrap();
    let insert = "INSERT INTO task_decisions (project, statement, why, created_at, updated_at)
        VALUES ('/p', 'Gone', '', 1, 1)";
    let result: Result<(), String> = database.transaction(|tx| {
        Box::pin(async move {
            toasty::sql::statement(insert).exec(tx).await?;
            toasty::sql::statement("INSERT INTO no_such_table VALUES (1)")
                .exec(tx)
                .await?;
            Ok(())
        })
    });
    assert!(result.is_err());
    let count = database
        .run(|mut db| async move {
            toasty::sql::query("SELECT count(*) FROM task_decisions")
                .exec(&mut db)
                .await
        })
        .unwrap();
    assert_eq!(
        format!("{count:?}"),
        format!(
            "{:?}",
            vec![toasty::stmt::Value::record_from_vec(vec![0_i64.into()])]
        )
    );
}

#[test]
fn run_works_inside_another_tokio_runtime() {
    let outside = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let store = crate::store::Store::open(Path::new(":memory:")).unwrap();
    let decisions = outside.block_on(async {
        let id = store.save_decision("/p", "Inside", "", &[], 1).unwrap();
        assert_eq!(run(async { 40 + 2 }), 42);
        (id, store.decisions().unwrap().len())
    });
    assert_eq!(decisions.1, 1);
    let multi = tokio::runtime::Runtime::new().unwrap();
    let answer = multi.block_on(async {
        tokio::spawn(async { run(async { "from a worker" }) })
            .await
            .unwrap()
    });
    assert_eq!(answer, "from a worker");
}

/// Fails when a model changed with no migration for it: run
/// `cargo run -p gespan-core --bin toasty-migrate -- <kind> <name>`.
#[test]
fn models_match_the_last_migration() {
    for kind in Kind::ALL {
        let pending = migrations::pending_sql(kind).unwrap();
        assert!(
            pending.is_none(),
            "The {} models changed with no migration. Run \
             `cargo run -p gespan-core --bin toasty-migrate -- {} <name>`. Pending:\n{}",
            kind.name(),
            kind.name(),
            pending.unwrap_or_default()
        );
        let history = migrations::history(kind).unwrap();
        let embedded: Vec<_> = kind
            .migrations()
            .migrations()
            .iter()
            .map(|m| (m.id(), m.name().to_owned()))
            .collect();
        let on_disk: Vec<_> = history
            .entries()
            .iter()
            .map(|e| (e.id, e.name.clone()))
            .collect();
        assert_eq!(embedded, on_disk, "{} migrations are embedded", kind.name());
    }
}

#[test]
fn every_kind_opens_in_memory() {
    for kind in Kind::ALL {
        let database = open(kind, Path::new(":memory:")).unwrap();
        assert_eq!(database.kind(), kind);
        assert_eq!(Kind::from_name(kind.name()), Some(kind));
    }
}

/// The `sessions` table the first Tauri release made, and its Pilot tables.
const DESKTOP_SCHEMA_V1: &str = include_str!("../../tests/fixtures/desktop/schema-v1.sql");
const PILOT_TABLES: &str = include_str!("../../tests/fixtures/desktop/pilot-tables.sql");
const BOARD_V2: &str = include_str!("../../tests/fixtures/board-v2.sql");

/// The rest of a `gespan.db` as Tauri and the hand-written native stores
/// left it, with rows in every table. `notes` is the first notes shape
/// (before `content_kind`), so the legacy upgrade has columns to add.
const GESPAN_TABLES: &str = "
INSERT INTO sessions VALUES
  ('s-1', '/work', 'codex', 'codex:m', '{}', 'supervised', 'First', 'thread',
   '[{\"id\":\"u\",\"role\":\"user\",\"text\":\"hi\"}]', 1, 2),
  ('s-2', '/work', 'claude', 'claude:m', '{}', 'auto-accept-edits', 'Worker', NULL, '[]', 3, 4);
INSERT INTO pilot_transcripts VALUES ('s-1');
INSERT INTO pilot_turns VALUES ('s-1', 0);
INSERT INTO pilot_blocks VALUES ('s-1', 0, 'b0', '{}', 'hi', 1, 0);
CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);
INSERT INTO schema_migrations VALUES (1, 0), (19, 0);
CREATE TABLE workspace_snapshot(id INTEGER PRIMARY KEY CHECK(id=1),
 snapshot_json TEXT NOT NULL, updated_at INTEGER NOT NULL);
INSERT INTO workspace_snapshot VALUES (1, '{\"tabs\":[]}', 5);
CREATE TABLE in_flight_sessions(session_id TEXT PRIMARY KEY,cwd TEXT NOT NULL,sort_index INTEGER NOT NULL);
INSERT INTO in_flight_sessions VALUES ('s-1', '/work', 0);
CREATE TABLE notes (
  id TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE, title TEXT NOT NULL,
  body TEXT NOT NULL DEFAULT '', tags_json TEXT NOT NULL DEFAULT '[]',
  source_session_id TEXT, source_cwd TEXT, slug_pending INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE INDEX notes_updated_idx ON notes (updated_at DESC, id);
INSERT INTO notes VALUES ('n1', 'first-note', 'Title', 'Body', '[\"a\"]', 's-1', '/work', 0, 10, 11);
CREATE TABLE session_reminders(session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
 due_at INTEGER NOT NULL CHECK(due_at>0),fired_at INTEGER);
CREATE INDEX session_reminders_pending ON session_reminders(due_at) WHERE fired_at IS NULL;
INSERT INTO session_reminders VALUES ('s-1', 100, NULL);
CREATE TABLE orchestration_runs(lead_id TEXT PRIMARY KEY, state TEXT NOT NULL);
CREATE TABLE orchestration_sidebar(lead_id TEXT PRIMARY KEY, summary TEXT NOT NULL);
CREATE TABLE orchestration_workers(session_id TEXT PRIMARY KEY, lead_id TEXT NOT NULL);
INSERT INTO orchestration_runs VALUES ('s-1', '{}');
INSERT INTO orchestration_sidebar VALUES ('s-1', 'Leading one worker');
INSERT INTO orchestration_workers VALUES ('s-2', 's-1');
INSERT INTO automations VALUES ('a1', '{}', 1, 100, 5);
INSERT INTO automation_runs VALUES ('r1', 'a1', 6, '{}');
INSERT INTO review_drafts VALUES ('r1', 'a1', '/repo', 'Review #7', '{}', 'waiting', 7, 7);
INSERT INTO flows VALUES ('f1', '{}', 1);
INSERT INTO flow_runs VALUES ('fr1', 'f1', 'running', 2, '{}');
INSERT INTO flow_visits VALUES ('v1', 'fr1', 0, 'start', 'done', '{}'), ('v2', 'fr1', 1, 'next', 'running', '{}');
INSERT INTO task_decisions VALUES (7, '/work', 'Recount at read time', 'because', 10, 20, NULL);
INSERT INTO my_tasks VALUES
  ('question:1:a', 'question', '/work', 'session:1', 'Fix fsid', 'How?', '{}', 100,
   'answered', 'Recount', '[[\"Recount\"]]', 120, 7);
";

/// `browser.db` as `gespan_browser::audit` made it before Toasty.
const BROWSER_V1: &str = "
CREATE TABLE browser_audit (
  id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, session_id TEXT NOT NULL,
  request_id TEXT NOT NULL, global_epoch TEXT NOT NULL, session_epoch TEXT NOT NULL,
  action TEXT NOT NULL, tab_id TEXT, url TEXT, params TEXT NOT NULL, outcome TEXT NOT NULL,
  reason TEXT, duration_ms INTEGER NOT NULL, commands_sent INTEGER NOT NULL DEFAULT 0);
CREATE INDEX browser_audit_session_ts ON browser_audit (session_id, ts DESC);
CREATE INDEX browser_audit_ts ON browser_audit (ts);
CREATE TABLE browser_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, kind TEXT NOT NULL,
  actor TEXT NOT NULL, session_id TEXT, detail TEXT);
CREATE INDEX browser_events_ts ON browser_events (ts);
CREATE TABLE browser_leases (
  tab_id TEXT PRIMARY KEY, run_id TEXT NOT NULL, target_id TEXT NOT NULL,
  session_id TEXT NOT NULL, opened_by TEXT, url TEXT NOT NULL, created_at INTEGER NOT NULL,
  closed_at INTEGER);
CREATE TABLE browser_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
INSERT INTO browser_audit VALUES
  (1, 10, 's1', 'q1', 'g', 'e', 'navigate', 't1', 'https://example.com', '{}', 'ok', NULL, 12, 2);
INSERT INTO browser_events VALUES (1, 11, 'tab_adopted', 'user', 's1', NULL);
INSERT INTO browser_leases VALUES ('t1', 'run', 'target', 's1', 'agent', 'https://example.com', 9, NULL);
INSERT INTO browser_settings VALUES ('settings', '{\"mode\":\"managed\"}');
";

/// `graph.db` as schema v1 of `gespan_graph` made it before Toasty.
fn graph_v1() -> String {
    use gespan_graph::{EdgeKind, NodeKind};
    let quoted = |names: Vec<&str>| {
        names
            .into_iter()
            .map(|n| format!("'{n}'"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let nodes = quoted(NodeKind::ALL.into_iter().map(NodeKind::as_str).collect());
    let edges = quoted(EdgeKind::ALL.into_iter().map(EdgeKind::as_str).collect());
    let (node, edge) = (NodeKind::ALL[0].as_str(), EdgeKind::ALL[0].as_str());
    format!(
        "CREATE TABLE graph_schema (version INTEGER NOT NULL);
INSERT INTO graph_schema VALUES (1);
CREATE TABLE graph_nodes (
  id INTEGER PRIMARY KEY, kind TEXT NOT NULL CHECK (kind IN ({nodes})), key TEXT NOT NULL,
  label TEXT NOT NULL, props_json TEXT NOT NULL DEFAULT '{{}}', created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL, last_seen_run INTEGER, UNIQUE (kind, key));
CREATE TABLE graph_edges (
  id INTEGER PRIMARY KEY,
  src INTEGER NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
  dst INTEGER NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ({edges})), props_json TEXT NOT NULL DEFAULT '{{}}',
  valid_from INTEGER NOT NULL,
  valid_from_basis TEXT NOT NULL CHECK (valid_from_basis IN ('native', 'owner_created', 'observed_by')),
  valid_to INTEGER, source TEXT NOT NULL, scope TEXT NOT NULL, created_at INTEGER NOT NULL,
  last_seen_run INTEGER,
  CHECK (valid_to IS NULL OR valid_to >= valid_from), CHECK (src <> dst));
CREATE UNIQUE INDEX graph_edges_open_uq ON graph_edges (kind, src, dst, source, scope) WHERE valid_to IS NULL;
CREATE INDEX graph_edges_scope_open ON graph_edges (source, scope) WHERE valid_to IS NULL;
CREATE INDEX graph_edges_scope ON graph_edges (scope);
CREATE INDEX graph_edges_out_open ON graph_edges (src, valid_from DESC, dst) WHERE valid_to IS NULL;
CREATE INDEX graph_edges_in_open ON graph_edges (dst, valid_from DESC, src) WHERE valid_to IS NULL;
CREATE INDEX graph_edges_out_all ON graph_edges (src, valid_from DESC, dst, valid_to);
CREATE INDEX graph_edges_in_all ON graph_edges (dst, valid_from DESC, src, valid_to);
CREATE TABLE graph_scopes (source TEXT NOT NULL, scope TEXT NOT NULL,
  observed_updated_at INTEGER NOT NULL, observed_at INTEGER NOT NULL, PRIMARY KEY (source, scope));
CREATE TABLE graph_run_scopes (run INTEGER NOT NULL, source TEXT NOT NULL, scope TEXT NOT NULL,
  close_at INTEGER NOT NULL, PRIMARY KEY (run, source, scope));
CREATE TABLE graph_runs (id INTEGER PRIMARY KEY, started_at INTEGER NOT NULL, finished_at INTEGER,
  observed_at INTEGER NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'cancelled', 'failed')),
  report_json TEXT);
INSERT INTO graph_nodes VALUES (1, '{node}', 'a', 'A', '{{}}', 1, 1, 1), (2, '{node}', 'b', 'B', '{{}}', 1, 1, NULL);
INSERT INTO graph_edges VALUES (1, 1, 2, '{edge}', '{{}}', 5, 'native', NULL, 'github', 'acme', 5, 1);
INSERT INTO graph_scopes VALUES ('github', 'acme', 4, 5);
INSERT INTO graph_run_scopes VALUES (1, 'github', 'acme', 6);
INSERT INTO graph_runs VALUES (1, 4, 6, 5, 'completed', '{{}}');
"
    )
}

/// The board tables at version 7 (no worktree settings yet), from the v2
/// fixture brought up by the hand-made schema and then turned back.
const BOARD_V8_TO_V7: &str = "
INSERT INTO board_settings (board_id, write_back) VALUES ('b1', 0);
ALTER TABLE board_settings DROP COLUMN run_in_worktree;
ALTER TABLE board_settings DROP COLUMN run_worktree_choice;
UPDATE board_schema SET version = 7;";

/// Writes a pre-Toasty file's tables and rows.
type Build = Box<dyn Fn(&Connection)>;

/// One pre-Toasty file per case: the kind, a label, and how to build it.
fn legacy_files() -> Vec<(Kind, &'static str, Build)> {
    let gespan = |board_v7: bool| -> Build {
        Box::new(move |conn: &Connection| {
            conn.execute_batch(DESKTOP_SCHEMA_V1).unwrap();
            conn.execute_batch(PILOT_TABLES).unwrap();
            conn.execute_batch(BOARD_V2).unwrap();
            if board_v7 {
                gespan_board_store::migrate::legacy_upgrade(conn).unwrap();
                conn.execute_batch(BOARD_V8_TO_V7).unwrap();
            }
            conn.execute_batch(crate::automations::LEGACY_SCHEMA)
                .unwrap();
            conn.execute_batch(crate::flow::store::LEGACY_SCHEMA)
                .unwrap();
            conn.execute_batch(LEGACY_SCHEMA).unwrap();
            conn.execute_batch(GESPAN_TABLES).unwrap();
        })
    };
    vec![
        (Kind::Gespan, "gespan.db with board v2", gespan(false)),
        (Kind::Gespan, "gespan.db with board v7", gespan(true)),
        (
            Kind::Settings,
            "settings",
            Box::new(|conn: &Connection| {
                conn.execute_batch(
                    "CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     INSERT INTO settings VALUES ('theme', '\"dark\"'), ('font', '13');",
                )
                .unwrap()
            }),
        ),
        (
            Kind::Counters,
            "counters",
            Box::new(|conn: &Connection| {
                conn.execute_batch(crate::diagnostics::metrics::LEGACY_SCHEMA)
                    .unwrap();
                conn.execute_batch(
                    "INSERT INTO diagnostic_counters VALUES
                       ('2026-11-01', 'store.open', 'io', 3, '0.1.2'),
                       ('2026-11-02', 'store.open', 'busy', 1, '0.1.3');",
                )
                .unwrap()
            }),
        ),
        (
            Kind::Browser,
            "browser",
            Box::new(|conn: &Connection| conn.execute_batch(BROWSER_V1).unwrap()),
        ),
        (
            Kind::Graph,
            "graph",
            Box::new(|conn: &Connection| conn.execute_batch(&graph_v1()).unwrap()),
        ),
    ]
}

fn tables(conn: &Connection) -> Vec<String> {
    conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table'
         AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .unwrap()
    .query_map([], |row| row.get(0))
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// Every row of `table`, `columns` only, in a stable order.
fn table_rows(
    conn: &Connection,
    table: &str,
    columns: &[String],
) -> rusqlite::Result<Vec<Vec<rusqlite::types::Value>>> {
    let list = columns
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = conn.prepare(&format!("SELECT {list} FROM \"{table}\" ORDER BY {list}"))?;
    let rows = statement
        .query_map([], |row| (0..columns.len()).map(|i| row.get(i)).collect())?
        .collect();
    rows
}

#[test]
fn every_kind_adopts_to_the_fresh_schema() {
    for (kind, label, build) in legacy_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        build(&Connection::open(&path).unwrap());
        // Every table the fixture made has rows, except the board tables the
        // hand-made upgrade created, still empty, on the way to v7.
        let made = tables(&Connection::open(&path).unwrap());
        // What adoption must keep: the file after the kind's own legacy
        // upgrade, which is where the hand-made stores would have left it.
        let expected = dir.path().join("expected.db");
        std::fs::copy(&path, &expected).unwrap();
        let upgraded = Connection::open(&expected).unwrap();
        kind.legacy_upgrade(&upgraded).unwrap();
        let mut kept = Vec::new();
        for table in tables(&upgraded) {
            let columns: Vec<String> = upgraded
                .prepare("SELECT name FROM pragma_table_info(?1)")
                .unwrap()
                .query_map([&table], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let rows = table_rows(&upgraded, &table, &columns).unwrap();
            if made.contains(&table) && !(label.ends_with("v7") && table.starts_with("board")) {
                assert!(!rows.is_empty(), "{label}: the fixture fills {table}");
            }
            kept.push((table, columns, rows));
        }
        drop(upgraded);
        let fresh = dir.path().join("fresh.db");
        open(kind, &fresh).unwrap();

        open(kind, &path).unwrap();

        let conn = Connection::open(&path).unwrap();
        for (table, columns, rows) in &kept {
            let after = table_rows(&conn, table, columns)
                .unwrap_or_else(|e| panic!("{label}: {table} lost a column: {e}"));
            assert_eq!(&after, rows, "{label}: every row of {table} is kept");
        }
        assert_eq!(
            schema(&path),
            schema(&fresh),
            "{label}: tables, columns, indexes and triggers match a fresh install"
        );
        assert_eq!(applied(&path), applied(&fresh), "{label}");
        let backup = backup_path(&path);
        assert_eq!(
            tables(&Connection::open(&backup).unwrap()),
            made,
            "{label}: the backup is the file before adoption"
        );
        std::fs::remove_file(&backup).unwrap();
        let schema_before = schema(&path);
        open(kind, &path).unwrap();
        assert_eq!(
            schema(&path),
            schema_before,
            "{label}: reopening changes nothing"
        );
        assert_eq!(applied(&path), applied(&fresh), "{label}");
        assert!(!backup.exists(), "{label}: nothing to apply on reopen");
    }
}

/// Every index a fresh install has, unique ones included, is in an adopted
/// file too, even when the old table had an index of the same name or its
/// own unique constraint.
#[test]
fn an_adopted_file_has_every_index_of_a_fresh_install() {
    let indexes = |path: &Path| -> Vec<(String, String, Option<String>)> {
        let conn = Connection::open(path).unwrap();
        let mut statement = conn
            .prepare(
                "SELECT name, tbl_name, sql FROM sqlite_master
                 WHERE type = 'index' ORDER BY name",
            )
            .unwrap();
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        rows
    };
    for (kind, label, build) in legacy_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        build(&Connection::open(&path).unwrap());
        let fresh = dir.path().join("fresh.db");
        open(kind, &fresh).unwrap();

        open(kind, &path).unwrap();

        let fresh_indexes = indexes(&fresh);
        assert_eq!(indexes(&path), fresh_indexes, "{label}");
        let unique = fresh_indexes
            .iter()
            .filter(|(_, _, sql)| sql.as_deref().is_some_and(|s| s.contains("UNIQUE")))
            .count();
        if kind == Kind::Gespan {
            assert!(unique >= 3, "{label}: the unique indexes are checked too");
        }
    }
}

#[test]
fn every_migration_splits_into_statements_on_every_platform() {
    // Toasty splits a migration on "\n-- #[toasty::breakpoint]\n". A file
    // checked out with CRLF (Git's default on Windows) does not split, runs
    // as one statement and creates only its first table. .gitattributes
    // keeps migrations LF; this fails if that ever stops holding.
    for kind in Kind::ALL {
        for migration in kind.migrations().migrations() {
            assert!(
                !migration.sql().contains('\r'),
                "{} {} has CRLF line endings",
                kind.name(),
                migration.name()
            );
            let statements = migration
                .sql()
                .split("\n-- #[toasty::breakpoint]\n")
                .filter(|sql| !sql.trim().is_empty())
                .count();
            let creates = migration.sql().matches("CREATE TABLE").count();
            assert!(
                statements >= creates,
                "{} {} splits into {statements} statements for {creates} tables",
                kind.name(),
                migration.name()
            );
        }
    }
}

#[test]
fn a_read_only_database_reads_through_the_models_and_refuses_writes() {
    use crate::store::schema::Setting;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.sqlite3");
    let database = open(Kind::Settings, &path).unwrap();
    database
        .run(|mut db| async move {
            Setting::create()
                .key("kept")
                .value("1")
                .exec(&mut db)
                .await
                .map(drop)
        })
        .unwrap();
    drop(database);
    let before = super::settled_bytes(&path);

    let reader = super::open_read_only(Kind::Settings, &path).unwrap();
    assert!(reader.is_current().unwrap());
    let rows = reader
        .run(|mut db| async move { Setting::all().exec(&mut db).await })
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| (row.key.as_str(), row.value.as_str()))
            .collect::<Vec<_>>(),
        [("kept", "1")]
    );
    let write = reader.run(|mut db| async move {
        Setting::create()
            .key("new")
            .value("2")
            .exec(&mut db)
            .await
            .map(drop)
    });
    assert!(
        write
            .as_ref()
            .is_err_and(|error| error.contains("readonly")),
        "{write:?}"
    );
    drop(reader);
    assert_eq!(super::settled_bytes(&path), before, "the file is unchanged");
}

#[test]
fn a_file_gespan_never_migrated_is_not_current() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.sqlite3");
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL)")
        .unwrap();
    let reader = super::open_read_only(Kind::Settings, &path).unwrap();
    assert!(!reader.is_current().unwrap(), "no migration ledger");
    drop(reader);

    // A ledger missing the newest migration is not current either.
    drop(open(Kind::Settings, &path).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM __toasty_migrations", [])
        .unwrap();
    let reader = super::open_read_only(Kind::Settings, &path).unwrap();
    assert!(!reader.is_current().unwrap(), "an incomplete ledger");
}

#[test]
fn a_read_only_open_of_a_missing_file_fails_without_creating_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.sqlite3");
    assert!(super::open_read_only(Kind::Settings, &path).is_err());
    assert!(!path.exists());
}
