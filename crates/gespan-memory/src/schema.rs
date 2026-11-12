use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::Error;

/// Each step brings the database from the previous version to `version`.
/// Steps are append-only: a released step is never edited.
const STEPS: &[(i64, &str)] = &[(1, V1), (2, V2)];

pub(crate) const VERSION: i64 = 2;

const V1: &str = "CREATE TABLE entries (
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
    CREATE INDEX flags_entry_open ON flags(entry, dismissed);";

/// Keyword search: an external-content FTS5 index over `entries`, kept in
/// step by triggers. The `rebuild` at the end indexes rows that already exist
/// when a v1 database is upgraded.
const V2: &str = "CREATE VIRTUAL TABLE entries_fts USING fts5(
        title, text, why, content='entries', content_rowid='id'
    );
    CREATE TRIGGER entries_fts_ai AFTER INSERT ON entries BEGIN
        INSERT INTO entries_fts(rowid, title, text, why)
        VALUES (new.id, new.title, new.text, new.why);
    END;
    CREATE TRIGGER entries_fts_ad AFTER DELETE ON entries BEGIN
        INSERT INTO entries_fts(entries_fts, rowid, title, text, why)
        VALUES ('delete', old.id, old.title, old.text, old.why);
    END;
    CREATE TRIGGER entries_fts_au AFTER UPDATE OF title, text, why ON entries BEGIN
        INSERT INTO entries_fts(entries_fts, rowid, title, text, why)
        VALUES ('delete', old.id, old.title, old.text, old.why);
        INSERT INTO entries_fts(rowid, title, text, why)
        VALUES (new.id, new.title, new.text, new.why);
    END;
    INSERT INTO entries_fts(entries_fts) VALUES ('rebuild');";

pub(crate) fn migrate(conn: &Connection) -> Result<(), Error> {
    // IMMEDIATE takes the write lock up front, so two openers of a fresh file
    // serialise, and the DDL plus the version rows commit together or not at all.
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    apply(&tx, VERSION)?;
    tx.commit()?;
    Ok(())
}

/// Run every step above the stored version, up to `target`.
fn apply(tx: &Transaction<'_>, target: i64) -> Result<(), Error> {
    tx.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")?;
    let found: Option<i64> =
        tx.query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))?;
    let current = found.unwrap_or(0);
    if current > VERSION {
        return Err(Error::NewerSchema {
            found: current,
            supported: VERSION,
        });
    }
    for (version, ddl) in STEPS {
        if current < *version && *version <= target {
            tx.execute_batch(ddl)?;
            // One row holds the version, however many steps ran.
            tx.execute("DELETE FROM schema_version", [])?;
            tx.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [version],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_database_upgrades_and_existing_rows_become_searchable() {
        let conn = Connection::open_in_memory().unwrap();
        // A database exactly as a v1 build left it, holding one entry.
        {
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).unwrap();
            apply(&tx, 1).unwrap();
            tx.commit().unwrap();
        }
        conn.execute(
            "INSERT INTO entries (kind, scope, status, title, text, why, created_ms) \
             VALUES ('decision', 'global', 'active', 'Old row', 'mentions axolotl', 'y', 1)",
            [],
        )
        .unwrap();
        let v: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 1);
        assert!(conn.prepare("SELECT 1 FROM entries_fts").is_err());

        migrate(&conn).unwrap();

        let v: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, VERSION);
        let hit = |q: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM entries_fts WHERE entries_fts MATCH ?1",
                [q],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            hit("axolotl"),
            1,
            "pre-existing row is indexed by the upgrade"
        );

        // Triggers are live for new writes and updates after the upgrade.
        conn.execute(
            "INSERT INTO entries (kind, scope, status, title, text, why, created_ms) \
             VALUES ('decision', 'global', 'active', 'New row', 'mentions narwhal', 'y', 2)",
            [],
        )
        .unwrap();
        assert_eq!(hit("narwhal"), 1);
        conn.execute(
            "UPDATE entries SET text = 'mentions walrus' WHERE title = 'New row'",
            [],
        )
        .unwrap();
        assert_eq!(hit("narwhal"), 0);
        assert_eq!(hit("walrus"), 1);
        conn.execute("DELETE FROM entries WHERE title = 'New row'", [])
            .unwrap();
        assert_eq!(hit("walrus"), 0);

        // Migrating again is a no-op.
        migrate(&conn).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn newer_schema_is_refused() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (99)", [])
            .unwrap();
        assert!(matches!(
            migrate(&conn),
            Err(Error::NewerSchema { found: 99, .. })
        ));
    }
}
