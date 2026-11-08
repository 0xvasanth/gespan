//! Schema migrations, one step per version (board-store's pattern):
//! - v1: the graph tables and indexes (`schema::ddl`).
use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::schema::{ddl, VERSION};

type Step = fn(&Transaction<'_>) -> rusqlite::Result<()>;

/// Each step with the version it brings the database to, oldest first.
const STEPS: [(i64, Step); 1] = [(1, to_v1)];

/// The stored graph schema version; 0 when none was ever written.
pub fn version(conn: &Connection) -> rusqlite::Result<i64> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'graph_schema')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(0);
    }
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM graph_schema",
        [],
        |row| row.get(0),
    )
}

/// Runs every step above the stored version, then records the current one.
/// It holds the write lock throughout and re-reads the version once it has
/// it, so two processes opening a fresh file at once don't both create it.
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
    tx.execute("DELETE FROM graph_schema", [])?;
    tx.execute("INSERT INTO graph_schema (version) VALUES (?1)", [VERSION])?;
    tx.commit()
}

fn to_v1(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(&ddl())
}
