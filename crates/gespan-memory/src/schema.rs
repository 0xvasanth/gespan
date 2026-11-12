use rusqlite::Connection;

use crate::Error;

pub(crate) const VERSION: i64 = 1;

pub(crate) fn migrate(conn: &Connection) -> Result<(), Error> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")?;
    let found: Option<i64> =
        conn.query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))?;
    match found {
        Some(v) if v > VERSION => {
            return Err(Error::NewerSchema {
                found: v,
                supported: VERSION,
            })
        }
        Some(_) => return Ok(()),
        None => {}
    }
    conn.execute_batch(
        "CREATE TABLE entries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            scope TEXT NOT NULL,
            status TEXT NOT NULL,
            title TEXT NOT NULL,
            text TEXT NOT NULL,
            why TEXT NOT NULL,
            user_answer TEXT,
            session INTEGER,
            question_ref TEXT,
            user_turn TEXT,
            import_src TEXT,
            pinned INTEGER NOT NULL DEFAULT 0,
            supersedes INTEGER REFERENCES entries(id),
            replaced_by INTEGER REFERENCES entries(id),
            created_ms INTEGER NOT NULL,
            decided_ms INTEGER
        );
        CREATE INDEX entries_scope_status ON entries(scope, status);
        CREATE INDEX entries_replaced_by ON entries(replaced_by);
        CREATE TABLE flags (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            entry INTEGER NOT NULL REFERENCES entries(id),
            session INTEGER NOT NULL,
            reason TEXT NOT NULL,
            created_ms INTEGER NOT NULL,
            dismissed INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX flags_entry_open ON flags(entry, dismissed);",
    )?;
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        [VERSION],
    )?;
    Ok(())
}
