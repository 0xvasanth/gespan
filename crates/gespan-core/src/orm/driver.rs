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
/// and (for files) write-ahead logging, as the hand-written stores did. One
/// connection does all the work.
#[derive(Debug)]
pub(crate) struct Driver {
    sqlite: Sqlite,
    file: bool,
}

impl Driver {
    pub(crate) fn new(path: &Path) -> Self {
        if super::is_memory(path) {
            Self {
                sqlite: Sqlite::in_memory(),
                file: false,
            }
        } else {
            Self {
                sqlite: Sqlite::open(path),
                file: true,
            }
        }
    }
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
            if self.file || !sql.contains("journal_mode") {
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
