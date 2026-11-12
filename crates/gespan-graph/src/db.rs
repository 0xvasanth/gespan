//! Opening `graph.db` through Toasty, and waiting on it from the store's
//! blocking calls.
//!
//! gespan-core opens its own databases through its `orm` module, but it will
//! depend on this crate, so this crate cannot use it. What it needs is
//! small: Toasty's SQLite driver with the same connection settings, one
//! runtime, and the graph migrations. A graph file has no hand-made
//! predecessor to adopt: nothing wrote one outside tests before Toasty.
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use toasty_core::driver::operation::{Operation, RawSql, RawSqlRet};
use toasty_core::driver::{
    Capability, ConnectContext, Connection, Driver as ToastyDriver, ExecResponse,
};
use toasty_core::schema::db::{AppliedMigration, Migration};
use toasty_core::schema::{diff, Schema};
use toasty_driver_sqlite::Sqlite;

use crate::error::GraphError;
use crate::schema;

thread_local! {
    /// Set on the graph runtime's own worker threads.
    static WORKER: Cell<bool> = const { Cell::new(false) };
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("gespan-graph-db")
            .enable_all()
            .on_thread_start(|| WORKER.with(|worker| worker.set(true)))
            .build()
            .expect("the graph database runtime starts")
    })
}

/// Runs `future` to completion on the graph runtime. The store is blocking
/// and meant for background threads; called from a thread inside another
/// tokio runtime it waits on a helper thread instead, so it never calls
/// `block_on` from a runtime thread.
pub(crate) fn run<F>(future: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    assert!(
        !WORKER.with(Cell::get),
        "a graph call from inside the graph runtime; await the future instead"
    );
    if tokio::runtime::Handle::try_current().is_err() {
        return runtime().block_on(future);
    }
    std::thread::scope(|scope| {
        scope
            .spawn(|| runtime().block_on(future))
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// Toasty's SQLite driver with a five second busy timeout and write-ahead
/// logging on every connection. Each connection holds a clone of `live`, so
/// the store can tell when the last one has closed.
#[derive(Debug)]
struct Driver {
    sqlite: Sqlite,
    live: Arc<()>,
}

/// A connection that counts itself in [`Driver::live`] until it drops.
#[derive(Debug)]
struct Counted {
    inner: Box<dyn Connection>,
    _live: Arc<()>,
}

async fn pragma(
    connection: &mut dyn Connection,
    schema: &Arc<Schema>,
    sql: &str,
) -> toasty_core::Result<()> {
    connection
        .exec(
            schema,
            Operation::RawSql(RawSql {
                sql: sql.into(),
                params: Vec::new(),
                // `journal_mode` answers with a row; `None` would refuse it.
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
        let app = toasty_core::schema::app::Schema::from_macro(toasty::ModelSet::new())?;
        let schema =
            Arc::new(toasty_core::schema::Builder::default().build(app, self.capability())?);
        for sql in ["PRAGMA busy_timeout=5000", "PRAGMA journal_mode=WAL"] {
            pragma(connection.as_mut(), &schema, sql).await?;
        }
        Ok(Box::new(Counted {
            inner: connection,
            _live: self.live.clone(),
        }))
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

#[async_trait]
impl Connection for Counted {
    async fn exec(
        &mut self,
        schema: &Arc<Schema>,
        plan: Operation,
    ) -> toasty_core::Result<ExecResponse> {
        self.inner.exec(schema, plan).await
    }

    fn is_valid(&self) -> bool {
        self.inner.is_valid()
    }

    async fn ping(&mut self) -> toasty_core::Result<()> {
        self.inner.ping().await
    }

    async fn push_schema(&mut self, schema: &Schema) -> toasty_core::Result<()> {
        self.inner.push_schema(schema).await
    }

    async fn applied_migrations(&mut self) -> toasty_core::Result<Vec<AppliedMigration>> {
        self.inner.applied_migrations().await
    }

    async fn apply_migration(
        &mut self,
        id: u64,
        name: &str,
        migration: &Migration,
    ) -> toasty_core::Result<()> {
        self.inner.apply_migration(id, name, migration).await
    }
}

/// An open `graph.db`. Dropping it waits until its connection has closed,
/// so the file is free afterwards (to wipe, or on Windows to delete), as it
/// was when the store owned a plain connection.
pub(crate) struct Handle {
    db: Option<toasty::Db>,
    /// One clone per open connection, plus this one.
    live: Arc<()>,
}

impl Handle {
    pub(crate) fn db(&self) -> &toasty::Db {
        self.db.as_ref().expect("an open graph database")
    }

    /// Drops the database and waits until its connection has closed.
    pub(crate) fn close(mut self) -> Result<(), GraphError> {
        self.shut()
    }

    fn shut(&mut self) -> Result<(), GraphError> {
        // Toasty closes the connection on its runtime once the last handle
        // is gone.
        drop(self.db.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        while Arc::strong_count(&self.live) > 1 {
            if Instant::now() > deadline {
                return Err(GraphError::Io(std::io::Error::other(
                    "the graph database did not close",
                )));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.shut();
    }
}

/// The migrations recorded in the file at `path` that this build does not
/// have, read before Toasty touches the file. Empty for a new file.
fn unknown_migrations(path: &Path) -> Result<Vec<String>, GraphError> {
    let conn = rusqlite::Connection::open(path)?;
    let tracked: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '__toasty_migrations')",
        [],
        |row| row.get(0),
    )?;
    if !tracked {
        return Ok(Vec::new());
    }
    let known: HashSet<u64> = schema::migrations()
        .migrations()
        .iter()
        .map(|migration| migration.id())
        .collect();
    let mut stmt = conn.prepare("SELECT id, name FROM __toasty_migrations ORDER BY id")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get(1)?)))?;
    let mut unknown = Vec::new();
    for row in rows {
        let (id, name): (i64, String) = row?;
        if !known.contains(&(id as u64)) {
            unknown.push(name);
        }
    }
    Ok(unknown)
}

/// Opens `path`, refusing a file with migrations this build does not know
/// before anything is written to it, then applies the pending migrations
/// and the constraints.
pub(crate) fn open(path: &Path) -> Result<Handle, GraphError> {
    let unknown = unknown_migrations(path)?;
    if !unknown.is_empty() {
        return Err(GraphError::UnknownMigrations(unknown));
    }
    let live = Arc::new(());
    let driver = Driver {
        sqlite: Sqlite::open(path),
        live: live.clone(),
    };
    let db = run(async move {
        let db = toasty::Db::builder()
            .models(schema::models())
            .max_pool_size(1)
            .pool_health_check_interval(None)
            .build(driver)
            .await?;
        // Another process may apply the same migration in between; the
        // second attempt reads what is applied again.
        if schema::migrations().apply(&db).await.is_err() {
            schema::migrations().apply(&db).await?;
        }
        for sql in schema::constraints() {
            toasty::sql::statement(*sql).exec(&mut db.clone()).await?;
        }
        Ok::<_, toasty::Error>(db)
    })?;
    Ok(Handle { db: Some(db), live })
}
