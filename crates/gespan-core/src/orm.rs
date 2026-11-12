//! Gespan's databases through the Toasty ORM.
//!
//! [`open`] gives one [`Database`] per file and [`Kind`]. Opening adopts a
//! file the hand-written stores made (see `adopt`), backs the file up, and
//! applies every migration the file has not had yet. Synchronous code reaches
//! Toasty's async API through [`run`], [`Database::run`] and
//! [`Database::transaction`]. `docs/toasty.md` explains how to add a model
//! and its migration.
mod adopt;
mod driver;
mod kind;
pub mod migrations;
mod runtime;
#[cfg(test)]
pub(crate) mod tests;

pub use kind::Kind;
pub use runtime::run;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use toasty_core::driver::operation::TransactionMode;

/// A future borrowed from a transaction, as [`Database::transaction`] takes.
pub type TxFuture<'t, T> = Pin<Box<dyn Future<Output = toasty::Result<T>> + Send + 't>>;

/// One Toasty database file (or an in-memory database for `:memory:`).
pub struct Database {
    db: toasty::Db,
    kind: Kind,
    path: PathBuf,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("kind", &self.kind)
            .field("path", &self.path)
            .finish()
    }
}

/// Opens `path` as a `kind` database, adopting and migrating it first.
/// `:memory:` opens a fresh in-memory database with every migration applied.
pub fn open(kind: Kind, path: &Path) -> Result<Database, String> {
    let memory = is_memory(path);
    if !memory {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        adopt::prepare(kind, path)?;
    }
    let db = run(async {
        let db = toasty::Db::builder()
            .models(kind.models())
            .max_pool_size(1)
            // An in-memory database lives only as long as its one connection.
            .pool_health_check_interval(None)
            .build(driver::Driver::new(path))
            .await?;
        // Another process may apply the same migration in between; the
        // second attempt reads what is applied again.
        if kind.migrations().apply(&db).await.is_err() {
            kind.migrations().apply(&db).await?;
        }
        for sql in kind.constraints() {
            toasty::sql::statement(*sql).exec(&mut db.clone()).await?;
        }
        Ok::<_, toasty::Error>(db)
    })
    .map_err(|e| format!("Cannot open the {} database: {e}", kind.name()))?;
    Ok(Database {
        db,
        kind,
        path: path.into(),
    })
}

impl Database {
    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A handle for async code; clones share the one connection.
    pub fn db(&self) -> toasty::Db {
        self.db.clone()
    }

    /// Runs `work` with a database handle and waits for it.
    ///
    /// ```ignore
    /// let task = database.run(|mut db| async move {
    ///     MyTask::filter_by_key(key).first().exec(&mut db).await
    /// })?;
    /// ```
    pub fn run<T, F, Fut>(&self, work: F) -> Result<T, String>
    where
        F: FnOnce(toasty::Db) -> Fut,
        Fut: Future<Output = toasty::Result<T>> + Send,
        T: Send,
    {
        run(work(self.db.clone())).map_err(|e| e.to_string())
    }

    /// Runs `work` in one transaction that takes the write lock up front
    /// (`BEGIN IMMEDIATE`, like `store::write_transaction`). It commits when
    /// `work` returns `Ok` and rolls back otherwise. Inside, use `tx` as the
    /// executor: the database has one connection, which the transaction holds.
    ///
    /// ```ignore
    /// database.transaction(|tx| Box::pin(async move {
    ///     toasty::sql::statement("UPDATE t SET n = n + 1").exec(tx).await?;
    ///     Ok(())
    /// }))?;
    /// ```
    pub fn transaction<T, F>(&self, work: F) -> Result<T, String>
    where
        F: for<'c, 't> FnOnce(&'c mut toasty::Transaction<'t>) -> TxFuture<'c, T> + Send,
        T: Send,
    {
        let mut db = self.db.clone();
        run(async move {
            let mut tx = db
                .transaction_builder()
                .mode(TransactionMode::Immediate)
                .begin()
                .await?;
            match work(&mut tx).await {
                Ok(value) => {
                    tx.commit().await?;
                    Ok(value)
                }
                Err(error) => {
                    tx.rollback().await?;
                    Err(error)
                }
            }
        })
        .map_err(|e: toasty::Error| e.to_string())
    }
}

pub(crate) fn is_memory(path: &Path) -> bool {
    path == Path::new(":memory:")
}

/// The full schema `models` make for this driver, without a connection.
pub(crate) fn schema(
    models: toasty::ModelSet,
    capability: &toasty::Capability,
) -> toasty::Result<toasty_core::Schema> {
    let app = toasty_core::schema::app::Schema::from_macro(models)?;
    toasty_core::schema::Builder::default().build(app, capability)
}

/// `path`'s bytes once every committed change is in the file. Toasty closes
/// a dropped database's connection on its own task, so its last commits can
/// still be in the WAL when a test reads the file; checkpointing first makes
/// a byte comparison mean "the data changed", not "the WAL was folded in".
#[cfg(test)]
pub(crate) fn settled_bytes(path: &Path) -> Vec<u8> {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    std::fs::read(path).unwrap()
}

/// Removes a database file a dropped [`Database`] used. Toasty closes the
/// connection on its own task, and Windows refuses to delete a file that is
/// still open, so this retries until the file is released (five seconds at
/// most) instead of failing on the first try.
#[cfg(test)]
pub(crate) fn remove_released(path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match std::fs::remove_file(path) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) if std::time::Instant::now() > deadline => {
                panic!("{} was never released: {error}", path.display())
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
}
