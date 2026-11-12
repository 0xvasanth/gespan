//! Toasty's SQLite driver with Gespan's connection settings.
use async_trait::async_trait;
use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;
use toasty_core::driver::operation::{Operation, RawSql, RawSqlRet};
use toasty_core::driver::{Capability, ConnectContext, Connection, Driver as ToastyDriver};
use toasty_core::schema::db::Migration;
use toasty_core::schema::diff;
use toasty_driver_sqlite::Sqlite;

/// Opens every connection with a five second busy timeout, foreign keys on
/// and (for writable files) write-ahead logging, as the hand-written stores
/// did. One connection does all the work.
#[derive(Debug)]
pub(crate) struct Driver {
    sqlite: Sqlite,
    /// Whether to switch the file to write-ahead logging: a writable file.
    wal: bool,
}

impl Driver {
    pub(crate) fn new(path: &Path) -> Self {
        if super::is_memory(path) {
            Self {
                sqlite: Sqlite::in_memory(),
                wal: false,
            }
        } else {
            Self {
                sqlite: Sqlite::open(path),
                wal: true,
            }
        }
    }

    /// A driver that opens `path` read-only: SQLite refuses every write, and
    /// the file's journal mode is left as it is.
    pub(crate) fn read_only(path: &Path) -> Self {
        Self {
            sqlite: Sqlite::open(read_only_uri(path)),
            wal: false,
        }
    }
}

/// `path` as a read-only SQLite URI. The driver opens with URI filenames on,
/// so `mode=ro` makes the connection read-only.
fn read_only_uri(path: &Path) -> String {
    let mut text = String::new();
    for c in path.to_string_lossy().chars() {
        match c {
            '\\' if cfg!(windows) => text.push('/'),
            '%' => text.push_str("%25"),
            '?' => text.push_str("%3f"),
            '#' => text.push_str("%23"),
            c => text.push(c),
        }
    }
    let prefix = if text.starts_with('/') {
        "file://"
    } else if text.as_bytes().get(1) == Some(&b':') {
        // A Windows drive path, `C:/...`.
        "file:///"
    } else {
        "file:"
    };
    format!("{prefix}{text}?mode=ro")
}

async fn pragma(
    connection: &mut dyn Connection,
    schema: &Arc<toasty_core::Schema>,
    sql: &str,
) -> toasty_core::Result<()> {
    connection
        .exec(
            schema,
            Operation::RawSql(RawSql {
                sql: sql.into(),
                params: Vec::new(),
                // Some pragmas answer with a row; `None` would refuse that.
                ret: RawSqlRet::Infer,
            }),
        )
        .await?;
    Ok(())
}

#[async_trait]
impl ToastyDriver for Driver {
    fn url(&self) -> Cow<'_, str> {
        self.sqlite.url()
    }

    fn capability(&self) -> &'static Capability {
        self.sqlite.capability()
    }

    async fn connect(&self, cx: &ConnectContext) -> toasty_core::Result<Box<dyn Connection>> {
        let mut connection = self.sqlite.connect(cx).await?;
        // Raw SQL needs a schema to run against; pragmas touch no model.
        let schema = Arc::new(super::schema(toasty::ModelSet::new(), self.capability())?);
        for sql in [
            "PRAGMA busy_timeout=5000",
            "PRAGMA foreign_keys=ON",
            "PRAGMA journal_mode=WAL",
        ] {
            if self.wal || !sql.contains("journal_mode") {
                pragma(connection.as_mut(), &schema, sql).await?;
            }
        }
        Ok(connection)
    }

    fn max_connections(&self) -> Option<usize> {
        Some(1)
    }

    fn generate_migration(&self, schema_diff: &diff::Schema<'_>) -> Migration {
        self.sqlite.generate_migration(schema_diff)
    }

    async fn reset_db(&self) -> toasty_core::Result<()> {
        self.sqlite.reset_db().await
    }
}

#[cfg(test)]
mod tests {
    use super::read_only_uri;
    use std::path::Path;

    #[test]
    fn a_read_only_uri_escapes_what_sqlite_would_read_as_uri_syntax() {
        assert_eq!(
            read_only_uri(Path::new("/Users/me/Application Support/gespan.db")),
            "file:///Users/me/Application Support/gespan.db?mode=ro"
        );
        assert_eq!(
            read_only_uri(Path::new("/tmp/a?b#c%d/gespan.db")),
            "file:///tmp/a%3fb%23c%25d/gespan.db?mode=ro"
        );
        assert_eq!(
            read_only_uri(Path::new("relative/gespan.db")),
            "file:relative/gespan.db?mode=ro"
        );
    }
}
