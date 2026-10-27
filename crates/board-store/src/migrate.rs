//! Schema migrations, one step per version:
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
//!   ticket.
//!
//! A step runs only when the stored version is below it, so a v3 database
//! moving to v4 does not copy the native extras again (which would bring back
//! labels or values removed since).
use crate::schema::{
    V2_TABLES, V3_COLUMNS, V3_TABLES, V4_TABLES, V5_COLUMNS, V5_TABLES, V6_COLUMNS, VERSION,
};
use rusqlite::{Connection, Transaction, TransactionBehavior};

type Step = fn(&Transaction<'_>) -> rusqlite::Result<()>;

/// Each step with the version it brings the database to, oldest first.
const STEPS: [(i64, Step); 5] = [(2, to_v2), (3, to_v3), (4, to_v4), (5, to_v5), (6, to_v6)];

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

/// Runs every step above the stored version, then records the current one.
/// It holds the write lock throughout and re-reads the version once it has
/// it, so when both apps open an old database at the same moment the second
/// one finds the work done; the copies use `INSERT OR IGNORE` for the same
/// reason.
pub fn run(conn: &Connection) -> rusqlite::Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let from = version(&tx)?;
    if from >= VERSION {
        return Ok(());
    }
    for (to, step) in STEPS {
        if from < to {
            step(&tx)?;
        }
    }
    tx.execute("DELETE FROM board_schema", [])?;
    tx.execute("INSERT INTO board_schema (version) VALUES (?1)", [VERSION])?;
    tx.commit()
}

fn to_v2(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(V2_TABLES)
}

fn to_v3(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(V3_TABLES)?;
    add_columns(tx, &V3_COLUMNS)?;
    copy_native_extras(tx)?;
    tx.execute_batch(
        "INSERT OR IGNORE INTO board_card_links (card_id, kind, ref, role, created_at)
         SELECT id, link_kind, link_id, 'primary', updated_at FROM board_cards
         WHERE link_kind IS NOT NULL AND link_id IS NOT NULL;",
    )
}

fn to_v4(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(V4_TABLES)
}

fn to_v5(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(V5_TABLES)?;
    add_columns(tx, &V5_COLUMNS)
}

fn to_v6(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    add_columns(tx, &V6_COLUMNS)
}

/// Adds each (table, column, declaration) the table does not have yet.
fn add_columns(tx: &Transaction<'_>, columns: &[(&str, &str, &str)]) -> rusqlite::Result<()> {
    for (table, column, declaration) in columns {
        if !column_exists(tx, table, column)? {
            tx.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {declaration}"
            ))?;
        }
    }
    Ok(())
}

fn copy_native_extras(tx: &Transaction<'_>) -> rusqlite::Result<()> {
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
