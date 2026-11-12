//! The legacy upgrade: the hand-made schema's own versions, which gespan-core
//! runs once, while it adopts an old `gespan.db` into Toasty
//! (`Kind::legacy_upgrade`), before it rebuilds each table into the first
//! migration's shape. Nothing here runs on a database Toasty already owns.
//!
//! One step per version of the old `board_schema` table:
//! - v2: the tables both apps created before `board_schema` existed.
//! - v3: the native app's `board_native_*` extras become the shared v3
//!   tables, and each card's single `link_kind`/`link_id` becomes a row in
//!   `board_card_links`. The old tables and columns stay as they are, so a
//!   build from before v3 still opens the database (its later writes there
//!   are not carried over).
//! - v4: the `board_views` table for saved views.
//! - v5: board icon, colour, default view, display and priority settings;
//!   `board_templates`; `board_session_settings`.
//! - v6: `board_cards.title_from_ticket`, set when a card's title came from a
//!   ticket; existing cards titled with their ticket's key or snapshot title
//!   are marked.
//! - v7: `board_cards.body_from_ticket`, set when a card's description came
//!   from a tracker. No earlier build copied one, so existing cards stay
//!   unmarked.
//! - v8: `board_settings.run_in_worktree` and `run_worktree_choice`. A board
//!   from before v8 keeps running in place until the switch is turned on.
//!
//! A step runs only when the stored version is below it, so a v3 database
//! moving to v4 does not copy the native extras again (which would bring back
//! labels or values removed since). [`drop_legacy_tables`] then removes the
//! version table and the native tables, which a fresh install never has.
use crate::schema::{
    V2_TABLES, V3_COLUMNS, V3_TABLES, V4_TABLES, V5_COLUMNS, V5_TABLES, V6_COLUMNS, V7_COLUMNS,
    V8_COLUMNS, VERSION,
};
use rusqlite::Connection;

type Step = fn(&Connection) -> rusqlite::Result<()>;

/// Each step with the version it brings the database to, oldest first.
const STEPS: [(i64, Step); 7] = [
    (2, to_v2),
    (3, to_v3),
    (4, to_v4),
    (5, to_v5),
    (6, to_v6),
    (7, to_v7),
    (8, to_v8),
];

/// The tables the native app kept its extras in before v3, and the version
/// table. A fresh install has none of them.
const LEGACY_TABLES: [&str; 7] = [
    "board_schema",
    "board_native_card_labels",
    "board_native_labels",
    "board_native_meta",
    "board_native_projects",
    "board_native_subtasks",
    "board_native_comments",
];

/// The stored board schema version; 0 when none was ever written.
pub fn version(conn: &Connection) -> rusqlite::Result<i64> {
    if !table_exists(conn, "board_schema")? {
        return Ok(0);
    }
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM board_schema",
        [],
        |row| row.get(0),
    )
}

/// Brings a hand-made board schema up to version 8, the last one, and
/// records it. The caller holds the write transaction (adoption runs this in
/// its own, with foreign keys off). A file without a `boards` table has no
/// board to keep and is left alone: adoption creates the tables new.
pub fn legacy_upgrade(conn: &Connection) -> rusqlite::Result<()> {
    if !table_exists(conn, "boards")? {
        return Ok(());
    }
    let from = version(conn)?;
    if from >= VERSION {
        return Ok(());
    }
    for (to, step) in STEPS {
        if from < to {
            step(conn)?;
        }
    }
    conn.execute("DELETE FROM board_schema", [])?;
    conn.execute("INSERT INTO board_schema (version) VALUES (?1)", [VERSION])?;
    Ok(())
}

/// Drops the version table and the pre-v3 native tables, once
/// [`legacy_upgrade`] has copied what they held. Adoption runs it last, so
/// an adopted file has the same tables as a fresh install.
pub fn drop_legacy_tables(conn: &Connection) -> rusqlite::Result<()> {
    for table in LEGACY_TABLES {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS {table}"))?;
    }
    Ok(())
}

fn to_v2(tx: &Connection) -> rusqlite::Result<()> {
    tx.execute_batch(V2_TABLES)
}

fn to_v3(tx: &Connection) -> rusqlite::Result<()> {
    tx.execute_batch(V3_TABLES)?;
    add_columns(tx, &V3_COLUMNS)?;
    copy_native_extras(tx)?;
    tx.execute_batch(
        "INSERT OR IGNORE INTO board_card_links (card_id, kind, ref, role, created_at)
         SELECT id, link_kind, link_id, 'primary', updated_at FROM board_cards
         WHERE link_kind IS NOT NULL AND link_id IS NOT NULL;",
    )
}

fn to_v4(tx: &Connection) -> rusqlite::Result<()> {
    tx.execute_batch(V4_TABLES)
}

fn to_v5(tx: &Connection) -> rusqlite::Result<()> {
    tx.execute_batch(V5_TABLES)?;
    add_columns(tx, &V5_COLUMNS)
}

fn to_v6(tx: &Connection) -> rusqlite::Result<()> {
    add_columns(tx, &V6_COLUMNS)?;
    mark_ticket_titles(tx)
}

fn to_v7(tx: &Connection) -> rusqlite::Result<()> {
    add_columns(tx, &V7_COLUMNS)
}

fn to_v8(tx: &Connection) -> rusqlite::Result<()> {
    add_columns(tx, &V8_COLUMNS)
}

/// Marks the cards made from a ticket before v6 as best it can tell: a card
/// whose title is its ticket's key or its snapshot title (ignoring case and
/// surrounding space). The mark then outlives a later ticket rename. Only
/// ever sets the mark, so running it again changes nothing.
fn mark_ticket_titles(tx: &Connection) -> rusqlite::Result<()> {
    let links: Vec<(String, String, String)> = {
        let mut statement = tx.prepare(
            "SELECT c.id, c.title, l.ref FROM board_cards c
             JOIN board_card_links l ON l.card_id = c.id
             WHERE l.role = 'ticket' AND l.kind IN ('jira', 'linear', 'plane')",
        )?;
        let rows = statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (card_id, title, reference) in links {
        let Ok(ticket) = serde_json::from_str::<serde_json::Value>(&reference) else {
            continue;
        };
        let wanted = title.trim().to_lowercase();
        let copied = ["key", "title"].iter().any(|field| {
            ticket
                .get(field)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| !wanted.is_empty() && text.trim().to_lowercase() == wanted)
        });
        if copied {
            tx.execute(
                "UPDATE board_cards SET title_from_ticket = 1 WHERE id = ?1",
                [&card_id],
            )?;
        }
    }
    Ok(())
}

/// Adds each (table, column, declaration) the table does not have yet.
fn add_columns(tx: &Connection, columns: &[(&str, &str, &str)]) -> rusqlite::Result<()> {
    for (table, column, declaration) in columns {
        if !column_exists(tx, table, column)? {
            tx.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {declaration}"
            ))?;
        }
    }
    Ok(())
}

fn copy_native_extras(tx: &Connection) -> rusqlite::Result<()> {
    // Rows whose card or label is gone (a delete made with foreign keys off)
    // are skipped rather than failing the whole migration. Subtasks and
    // comments are listed by position or time, then rowid, so they are
    // copied in the old rowid order to keep ties in the order they were made.
    let live_card = "card_id IN (SELECT id FROM board_cards)";
    let copies = [
        (
            "board_native_meta",
            format!(
                "INSERT OR IGNORE INTO board_card_meta (card_id, priority, due_at)
                 SELECT card_id, priority, due_at FROM board_native_meta WHERE {live_card}"
            ),
        ),
        (
            "board_native_projects",
            format!(
                "INSERT OR IGNORE INTO board_card_projects (card_id, project)
                 SELECT card_id, project FROM board_native_projects WHERE {live_card}"
            ),
        ),
        (
            "board_native_labels",
            "INSERT OR IGNORE INTO board_labels (id, board_id, name, color, position)
             SELECT id, board_id, name, color,
                    ROW_NUMBER() OVER (PARTITION BY board_id ORDER BY lower(name), id) - 1
             FROM board_native_labels WHERE board_id IN (SELECT id FROM boards)"
                .to_string(),
        ),
        (
            "board_native_card_labels",
            format!(
                "INSERT OR IGNORE INTO board_card_labels (card_id, label_id)
                 SELECT card_id, label_id FROM board_native_card_labels
                 WHERE {live_card} AND label_id IN (SELECT id FROM board_labels)"
            ),
        ),
        (
            "board_native_subtasks",
            format!(
                "INSERT OR IGNORE INTO board_subtasks (id, card_id, title, done, position)
                 SELECT id, card_id, title, done, position FROM board_native_subtasks
                 WHERE {live_card} ORDER BY rowid"
            ),
        ),
        (
            "board_native_comments",
            format!(
                "INSERT OR IGNORE INTO board_comments (id, card_id, body, author, created_at)
                 SELECT id, card_id, body, 'user', created_at FROM board_native_comments
                 WHERE {live_card} ORDER BY rowid"
            ),
        ),
    ];
    for (table, sql) in copies {
        if table_exists(tx, table)? {
            tx.execute_batch(&sql)?;
        }
    }
    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        [table, column],
        |row| row.get(0),
    )
}
