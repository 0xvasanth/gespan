//! v2 → v3: the native app's `board_native_*` extras become the shared v3
//! tables, and each card's single `link_kind`/`link_id` becomes a row in
//! `board_card_links`. The old tables and columns stay as they are, so a build
//! from before v3 still opens the database (its later writes there are not
//! carried over).
use crate::schema::{V2_TABLES, V3_COLUMNS, V3_TABLES, VERSION};
use rusqlite::{Connection, Transaction, TransactionBehavior};

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

/// Creates and fills the v3 tables, then records version 3. It holds the
/// write lock throughout and re-reads the version once it has it, so when both
/// apps open an old database at the same moment the second one finds the work
/// done; the copies use `INSERT OR IGNORE` for the same reason.
pub fn run(conn: &Connection) -> rusqlite::Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if version(&tx)? >= VERSION {
        return Ok(());
    }
    tx.execute_batch(V2_TABLES)?;
    tx.execute_batch(V3_TABLES)?;
    for (table, column, declaration) in V3_COLUMNS {
        if !column_exists(&tx, table, column)? {
            tx.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {declaration}"
            ))?;
        }
    }
    copy_native_extras(&tx)?;
    tx.execute_batch(
        "INSERT OR IGNORE INTO board_card_links (card_id, kind, ref, role, created_at)
         SELECT id, link_kind, link_id, 'primary', updated_at FROM board_cards
         WHERE link_kind IS NOT NULL AND link_id IS NOT NULL;",
    )?;
    tx.execute("DELETE FROM board_schema", [])?;
    tx.execute("INSERT INTO board_schema (version) VALUES (?1)", [VERSION])?;
    tx.commit()
}

fn copy_native_extras(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    // Rows whose card or label is gone (a delete made with foreign keys off)
    // are skipped rather than failing the whole migration.
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
                 WHERE {live_card}"
            ),
        ),
        (
            "board_native_comments",
            format!(
                "INSERT OR IGNORE INTO board_comments (id, card_id, body, author, created_at)
                 SELECT id, card_id, body, 'user', created_at FROM board_native_comments
                 WHERE {live_card}"
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
