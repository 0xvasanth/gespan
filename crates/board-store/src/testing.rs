//! A board database outside gespan-core, for this crate's tests, the seed
//! example and the scale fixtures.
//!
//! gespan-core owns the real `gespan.db`: its `orm::open` adopts an old file,
//! applies the migrations and runs the constraints. This crate cannot reach
//! that (gespan-core depends on it), so [`Fixture`] does the board's part of
//! the same work on a file of its own:
//!
//! - a new file gets the models' tables (`push_schema`), the same DDL the
//!   first migration has, and an empty `__toasty_migrations` table, which is
//!   how both tell a Toasty file from a hand-made one;
//! - a hand-made file (tables, no `__toasty_migrations`) gets the legacy
//!   upgrade, which is what adoption runs before it rebuilds the tables;
//! - every open runs [`crate::CONSTRAINTS`].
//!
//! It also keeps a plain rusqlite connection on the same file
//! ([`Fixture::raw`]) for tests that set up or check rows with SQL, and
//! counts the statements Toasty sends ([`Fixture::statements`]).
use crate::db::{Conn, Job};
use async_trait::async_trait;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use toasty_core::driver::operation::{Operation, RawSql, RawSqlRet};
use toasty_core::driver::{
    Capability, ConnectContext, Connection, Driver as ToastyDriver, ExecResponse,
};
use toasty_core::schema::db::{AppliedMigration, Migration};
use toasty_core::schema::diff;
use toasty_core::Schema;
use toasty_driver_sqlite::Sqlite;

/// What Toasty's SQLite driver creates to track migrations; its presence
/// marks a file as Toasty's.
const TRACKING_TABLE: &str = "CREATE TABLE IF NOT EXISTS __toasty_migrations (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL
            )";

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("board-store-testing")
            .enable_all()
            .build()
            .expect("the testing runtime starts")
    })
}

/// The fixture's [`crate::Runner`]: blocks on its own runtime, from a helper
/// thread when the caller is already inside a runtime.
pub fn runner(job: Job<'_>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return runtime().block_on(job);
    }
    std::thread::scope(|scope| {
        scope.spawn(|| runtime().block_on(job)).join().unwrap();
    });
}

/// A board database on a file, with a rusqlite connection beside it.
/// Derefs to the [`Conn`] every store function takes.
pub struct Fixture {
    conn: Conn,
    raw: rusqlite::Connection,
    path: PathBuf,
    statements: Arc<AtomicUsize>,
    _dir: Option<Arc<tempfile::TempDir>>,
}

impl std::ops::Deref for Fixture {
    type Target = Conn;

    fn deref(&self) -> &Conn {
        &self.conn
    }
}

/// A new, empty board database in a temporary directory.
pub fn fresh() -> Fixture {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("gespan.db");
    let mut fixture = open(&path).expect("a new board database opens");
    fixture._dir = Some(Arc::new(dir));
    fixture
}

/// A hand-made database in a temporary directory: `setup` runs on a plain
/// connection first, then the file opens as [`open`] does.
pub fn legacy(setup: impl FnOnce(&rusqlite::Connection)) -> Fixture {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("gespan.db");
    setup(&rusqlite::Connection::open(&path).expect("the file opens"));
    let mut fixture = open(&path).expect("the hand-made database opens");
    fixture._dir = Some(Arc::new(dir));
    fixture
}

/// Opens the board database at `path`, creating the file if it is missing.
pub fn open(path: &Path) -> Result<Fixture, String> {
    let raw = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
    raw.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    raw.execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(|e| e.to_string())?;
    let tables: Vec<String> = raw
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()
        })
        .map_err(|e| e.to_string())?;
    let has = |table: &str| tables.iter().any(|name| name == table);
    let tracked = has("__toasty_migrations");
    if !tracked && has("boards") {
        upgrade_legacy(&raw).map_err(|e| e.to_string())?;
    }
    // A file without the board yet gets every board table.
    let new = !tracked && !has("boards");
    let statements = Arc::new(AtomicUsize::new(0));
    let driver = Counting {
        sqlite: Sqlite::open(path),
        statements: statements.clone(),
    };
    let conn = Conn::new(
        build(driver, new).map_err(|e| format!("Cannot open the board database: {e}"))?,
        runner,
    );
    if new {
        raw.execute_batch(TRACKING_TABLE)
            .map_err(|e| e.to_string())?;
    }
    Ok(Fixture {
        conn,
        raw,
        path: path.to_path_buf(),
        statements,
        _dir: None,
    })
}

fn upgrade_legacy(raw: &rusqlite::Connection) -> rusqlite::Result<()> {
    raw.execute_batch("PRAGMA foreign_keys=OFF; BEGIN IMMEDIATE;")?;
    let result = crate::migrate::legacy_upgrade(raw);
    raw.execute_batch(if result.is_ok() {
        "COMMIT;"
    } else {
        "ROLLBACK;"
    })?;
    raw.execute_batch("PRAGMA foreign_keys=ON;")?;
    result
}

fn build(driver: Counting, new: bool) -> toasty::Result<toasty::Db> {
    let mut out = None;
    runner(Box::pin(async {
        out = Some(
            async {
                let db = toasty::Db::builder()
                    .models(crate::models::models())
                    .max_pool_size(1)
                    .pool_health_check_interval(None)
                    .build(driver)
                    .await?;
                if new {
                    db.push_schema().await?;
                }
                for sql in crate::CONSTRAINTS {
                    toasty::sql::statement(*sql).exec(&mut db.clone()).await?;
                }
                Ok(db)
            }
            .await,
        );
    }));
    out.expect("the runner finishes the job")
}

impl Fixture {
    /// A plain rusqlite connection on the same file.
    pub fn raw(&self) -> &rusqlite::Connection {
        &self.raw
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The store handle; the fixture also derefs to it.
    pub fn conn(&self) -> &Conn {
        &self.conn
    }

    /// Opens the same file again, as another app or window would.
    pub fn reopen(&self) -> Fixture {
        let mut fixture = open(&self.path).expect("the file opens again");
        fixture._dir = self._dir.clone();
        fixture
    }

    /// How many statements `work` sent to SQLite through this fixture's
    /// store connection. Transaction control (`BEGIN`, `COMMIT`) doesn't
    /// count; a trigger runs inside the statement that fired it.
    pub fn statements<T>(&self, work: impl FnOnce() -> T) -> (usize, T) {
        let before = self.statements.load(Ordering::SeqCst);
        let value = work();
        (self.statements.load(Ordering::SeqCst) - before, value)
    }
}

/// Toasty's SQLite driver, opening connections as the app does and counting
/// the statements they run.
#[derive(Debug)]
struct Counting {
    sqlite: Sqlite,
    statements: Arc<AtomicUsize>,
}

#[derive(Debug)]
struct CountingConnection {
    inner: Box<dyn Connection>,
    statements: Arc<AtomicUsize>,
}

#[async_trait]
impl ToastyDriver for Counting {
    fn url(&self) -> Cow<'_, str> {
        self.sqlite.url()
    }

    fn capability(&self) -> &'static Capability {
        self.sqlite.capability()
    }

    async fn connect(&self, cx: &ConnectContext) -> toasty_core::Result<Box<dyn Connection>> {
        let mut inner = self.sqlite.connect(cx).await?;
        let app = toasty_core::schema::app::Schema::from_macro(toasty::ModelSet::new())?;
        let schema =
            Arc::new(toasty_core::schema::Builder::default().build(app, self.capability())?);
        for sql in [
            "PRAGMA busy_timeout=5000",
            "PRAGMA foreign_keys=ON",
            "PRAGMA journal_mode=WAL",
        ] {
            inner
                .exec(
                    &schema,
                    Operation::RawSql(RawSql {
                        sql: sql.into(),
                        params: Vec::new(),
                        ret: RawSqlRet::Infer,
                    }),
                )
                .await?;
        }
        Ok(Box::new(CountingConnection {
            inner,
            statements: self.statements.clone(),
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
impl Connection for CountingConnection {
    async fn exec(
        &mut self,
        schema: &Arc<Schema>,
        op: Operation,
    ) -> toasty_core::Result<ExecResponse> {
        if !matches!(op, Operation::Transaction(_)) {
            self.statements.fetch_add(1, Ordering::SeqCst);
        }
        self.inner.exec(schema, op).await
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
