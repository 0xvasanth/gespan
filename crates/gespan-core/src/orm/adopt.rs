//! Before Toasty opens a file: the backup, and adoption of a file the
//! hand-written stores made.
//!
//! Adoption runs when a file has tables but no `__toasty_migrations`. In one
//! transaction with foreign keys off it runs the kind's legacy upgrade, then
//! rebuilds every table the first migration creates into exactly that DDL
//! (create under a temporary name, copy the columns both share, drop the old
//! table, rename), creates the first migration's indexes and records the
//! first migration as applied. Tables the first migration does not name are
//! left alone. Afterwards the file has the same shape as a fresh install.
use super::Kind;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use toasty::migration::MigrationFile;

/// Toasty's SQLite driver creates its tracking table with exactly this
/// statement; adoption uses the same text so the schema matches.
const TRACKING_TABLE: &str = "CREATE TABLE IF NOT EXISTS __toasty_migrations (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL
            )";

const BREAKPOINT: &str = "\n-- #[toasty::breakpoint]\n";

/// Where the copy taken before adoption or a migration goes.
pub(crate) fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak-pre-toasty");
    PathBuf::from(name)
}

/// Backs `path` up when adoption or a pending migration is about to change
/// it, then adopts it if it needs adopting.
pub(crate) fn prepare(kind: Kind, path: &Path) -> Result<(), String> {
    let existing = std::fs::metadata(path).is_ok_and(|meta| meta.len() > 0);
    if !existing {
        return Ok(());
    }
    let migrations = kind.migrations();
    let Some(first) = migrations.migrations().first().copied() else {
        return Ok(());
    };
    let error = |e: rusqlite::Error| format!("Cannot adopt {}: {e}", path.display());
    let mut conn = Connection::open(path).map_err(error)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(error)?;
    let state = State::read(&conn).map_err(error)?;
    let adopting = state.needs_adoption();
    let pending = migrations
        .migrations()
        .iter()
        .skip(usize::from(adopting))
        .any(|migration| !state.applied.contains(&migration.id()));
    if !adopting && !pending {
        return Ok(());
    }
    backup(&conn, path)?;
    if adopting {
        adopt(&mut conn, kind, &first).map_err(error)?;
    }
    Ok(())
}

struct State {
    tracked: bool,
    tables: HashSet<String>,
    applied: HashSet<u64>,
}

impl State {
    fn read(conn: &Connection) -> rusqlite::Result<Self> {
        let tables = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        let tracked = tables.contains("__toasty_migrations");
        let applied = if tracked {
            conn.prepare("SELECT id FROM __toasty_migrations")?
                .query_map([], |row| row.get::<_, i64>(0))?
                .map(|id| id.map(|id| id as u64))
                .collect::<rusqlite::Result<_>>()?
        } else {
            HashSet::new()
        };
        Ok(Self {
            tracked,
            tables,
            applied,
        })
    }

    fn needs_adoption(&self) -> bool {
        !self.tracked && !self.tables.is_empty()
    }
}

/// A consistent copy, WAL included, replacing any older backup.
fn backup(conn: &Connection, path: &Path) -> Result<(), String> {
    let target = backup_path(path);
    let mut partial = target.clone().into_os_string();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let fail = |e: &dyn std::fmt::Display| format!("Cannot back up {}: {e}", path.display());
    let _ = std::fs::remove_file(&partial);
    // The copy is created empty first, with the original's permissions, so a
    // private file (the settings sidecar is 0600) never has a readable
    // backup, even for a moment. VACUUM INTO accepts an empty target.
    create_private_like(path, &partial).map_err(|e| fail(&e))?;
    conn.execute("VACUUM INTO ?1", [partial.to_string_lossy()])
        .map_err(|e| fail(&e))?;
    std::fs::rename(&partial, &target).map_err(|e| fail(&e))
}

/// Creates `target` as a new empty file with `source`'s permissions.
fn create_private_like(source: &Path, target: &Path) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mode = std::fs::metadata(source)?.permissions().mode() & 0o777;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = source;
    options.open(target).map(drop)
}

/// The tables and indexes the first migration creates, each with the one
/// statement SQLite keeps for it, in creation order.
///
/// The migration runs on an empty in-memory database and its objects are
/// read back from `sqlite_master`, so adoption does not depend on how Toasty
/// groups statements into steps: an index written into the same step as its
/// table is still an index of its own here, created after the table is
/// rebuilt instead of on the old table that is about to be dropped.
struct FirstSchema {
    tables: Vec<(String, String)>,
    indexes: Vec<(String, String)>,
}

fn first_schema(first: &MigrationFile) -> rusqlite::Result<FirstSchema> {
    let scratch = Connection::open_in_memory()?;
    for step in first.sql().split(BREAKPOINT) {
        scratch.execute_batch(step)?;
    }
    let objects = |kind: &str| -> rusqlite::Result<Vec<(String, String)>> {
        scratch
            .prepare(
                "SELECT name, sql FROM sqlite_master
                 WHERE type = ?1 AND name NOT LIKE 'sqlite_%' AND sql IS NOT NULL
                 ORDER BY rowid",
            )?
            .query_map([kind], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect()
    };
    let other: Option<String> = scratch
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type NOT IN ('table', 'index') LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(sql) = other {
        return Err(unsupported(&sql));
    }
    Ok(FirstSchema {
        tables: objects("table")?,
        indexes: objects("index")?,
    })
}

fn unsupported(sql: &str) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(
        format!(
            "adoption only handles CREATE TABLE and CREATE INDEX in the first migration: {sql}"
        )
        .into(),
    )
}

/// The leading SQL identifier in `text` and the text after it.
fn identifier(text: &str) -> Option<(String, &str)> {
    if let Some(quoted) = text.strip_prefix('"') {
        let mut name = String::new();
        let mut chars = quoted.char_indices();
        while let Some((at, c)) = chars.next() {
            if c != '"' {
                name.push(c);
            } else if quoted[at + 1..].starts_with('"') {
                name.push('"');
                chars.next();
            } else {
                return Some((name, &quoted[at + 1..]));
            }
        }
        None
    } else {
        let end = text.find(|c: char| c.is_whitespace() || c == '(')?;
        (end > 0).then(|| (text[..end].to_owned(), &text[end..]))
    }
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn columns(conn: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    conn.prepare("SELECT name FROM pragma_table_info(?1)")?
        .query_map([table], |row| row.get(0))?
        .collect()
}

fn adopt(conn: &mut Connection, kind: Kind, first: &MigrationFile) -> rusqlite::Result<()> {
    let first_schema = first_schema(first)?;
    conn.execute_batch("PRAGMA foreign_keys=OFF;")?;
    let result = (|| {
        let tx = rusqlite::Transaction::new(conn, rusqlite::TransactionBehavior::Immediate)?;
        // Another process may have adopted the file while this one waited.
        if !State::read(&tx)?.needs_adoption() {
            return Ok(());
        }
        kind.legacy_upgrade(&tx)?;
        for (name, sql) in &first_schema.tables {
            let exists = tx
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [name],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                tx.execute_batch(sql)?;
                continue;
            }
            let staging = format!("__toasty_new_{name}");
            let rest = sql
                .strip_prefix("CREATE TABLE ")
                .and_then(identifier)
                .map(|(_, rest)| rest)
                .ok_or_else(|| unsupported(sql))?;
            tx.execute_batch(&format!("CREATE TABLE {}{rest}", quote(&staging)))?;
            let old: HashSet<String> = columns(&tx, name)?.into_iter().collect();
            let shared = columns(&tx, &staging)?
                .into_iter()
                .filter(|column| old.contains(column))
                .map(|column| quote(&column))
                .collect::<Vec<_>>()
                .join(", ");
            if !shared.is_empty() {
                tx.execute_batch(&format!(
                    "INSERT INTO {} ({shared}) SELECT {shared} FROM {}",
                    quote(&staging),
                    quote(name)
                ))?;
            }
            // Dropping the old table drops its indexes, including any that
            // share a name with one of the first migration's.
            tx.execute_batch(&format!(
                "DROP TABLE {}; ALTER TABLE {} RENAME TO {};",
                quote(name),
                quote(&staging),
                quote(name)
            ))?;
        }
        for (name, sql) in &first_schema.indexes {
            // A table the first migration does not name may still hold an
            // old index of this name; that is a clash to report, not skip.
            tx.execute_batch(sql).map_err(|e| {
                rusqlite::Error::ToSqlConversionFailure(
                    format!("adoption cannot create index {name}: {e}").into(),
                )
            })?;
        }
        tx.execute_batch(TRACKING_TABLE)?;
        tx.execute(
            "INSERT INTO __toasty_migrations (id, name, applied_at) VALUES (?1, ?2, datetime('now'))",
            params![first.id() as i64, first.name()],
        )?;
        tx.commit()
    })();
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    result
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn a_backup_keeps_the_original_files_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private.sqlite3");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1);")
            .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        super::backup(&conn, &path).unwrap();
        let backup = backup_path(&path);
        let mode = std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the backup is as private as the original");
        let copy = Connection::open(&backup).unwrap();
        let rows: i64 = copy
            .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    use super::*;

    /// A first migration that writes a table and its unique index into one
    /// step, as Toasty may, then a second table with its index in steps of
    /// their own.
    const FIRST: MigrationFile = MigrationFile::new(
        41,
        "0000_init.sql",
        "CREATE TABLE \"cards\" (\n    \"id\" TEXT NOT NULL,\n    \"board\" TEXT NOT NULL,\n    \"number\" BIGINT NOT NULL,\n    PRIMARY KEY (\"id\")\n);\nCREATE UNIQUE INDEX \"cards_number_idx\" ON \"cards\" (\"board\", \"number\");\n-- #[toasty::breakpoint]\nCREATE TABLE \"labels\" (\n    \"id\" TEXT NOT NULL,\n    PRIMARY KEY (\"id\")\n);\n-- #[toasty::breakpoint]\nCREATE INDEX \"labels_id_idx\" ON \"labels\" (\"id\");",
    );

    fn indexes(conn: &Connection) -> Vec<(String, String)> {
        conn.prepare(
            "SELECT name, sql FROM sqlite_master WHERE type = 'index' AND sql IS NOT NULL
             ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    #[test]
    fn an_index_in_its_tables_step_is_split_out() {
        let schema = first_schema(&FIRST).unwrap();
        let names = |objects: &[(String, String)]| -> Vec<String> {
            objects.iter().map(|(name, _)| name.clone()).collect()
        };
        assert_eq!(names(&schema.tables), ["cards", "labels"]);
        assert_eq!(
            names(&schema.indexes),
            ["cards_number_idx", "labels_id_idx"]
        );
        assert!(schema.tables[0].1.starts_with("CREATE TABLE \"cards\""));
        assert!(
            !schema.tables[0].1.contains("INDEX"),
            "{}",
            schema.tables[0].1
        );
    }

    #[test]
    fn adoption_keeps_a_unique_index_written_with_its_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        // The old table had a plain index of the same name and no labels.
        conn.execute_batch(
            "CREATE TABLE cards (id TEXT PRIMARY KEY, board TEXT NOT NULL, number INTEGER NOT NULL);
             CREATE INDEX cards_number_idx ON cards (board, number);
             INSERT INTO cards VALUES ('k1', 'b1', 1), ('k2', 'b1', 2);",
        )
        .unwrap();
        let fresh = Connection::open_in_memory().unwrap();
        fresh
            .execute_batch(&FIRST.sql().replace(BREAKPOINT, "\n"))
            .unwrap();

        adopt(&mut conn, Kind::Settings, &FIRST).unwrap();

        assert_eq!(indexes(&conn), indexes(&fresh));
        let kept: i64 = conn
            .query_row("SELECT count(*) FROM cards", [], |row| row.get(0))
            .unwrap();
        assert_eq!(kept, 2);
        let duplicate = conn.execute("INSERT INTO cards VALUES ('k3', 'b1', 2)", []);
        assert!(
            duplicate.is_err(),
            "the unique index holds on the adopted table"
        );
    }

    #[test]
    fn a_first_migration_with_other_statements_is_refused() {
        let first = MigrationFile::new(
            42,
            "0000_init.sql",
            "CREATE TABLE \"t\" (\"id\" TEXT NOT NULL, PRIMARY KEY (\"id\"));\n-- #[toasty::breakpoint]\nCREATE VIEW \"v\" AS SELECT id FROM t;",
        );
        assert!(first_schema(&first).is_err());
    }
}
