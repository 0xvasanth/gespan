//! What the app embedding the browser module provides: its database, where
//! state changes go, and the few facts only the app's windows know.
//!
//! The Tauri app and the native (GPUI) app each implement [`Embedder`] once
//! and pass it to [`crate::init`].

use std::future::Future;
use std::pin::Pin;
use std::process::Child;

use crate::protocol::{BrowserSettings, BrowserState};

/// Work on the browser database, for [`BrowserDb`]'s `wait` to drive.
pub type DbJob<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// The browser database as [`Embedder::with_db`] lends it: a Toasty handle
/// on a database the app opened with [`crate::audit::models`], migrated, with
/// [`crate::audit::CONSTRAINTS`] run; and how to wait on it from synchronous
/// code.
#[derive(Clone)]
pub struct BrowserDb {
    db: toasty::Db,
    wait: fn(DbJob<'_>),
}

impl BrowserDb {
    /// `wait` runs a job to completion on the runtime the database lives on.
    pub fn new(db: toasty::Db, wait: fn(DbJob<'_>)) -> BrowserDb {
        BrowserDb { db, wait }
    }

    /// Runs `work` with a handle on the database and waits for it.
    pub(crate) fn run<T, F, Fut>(&self, work: F) -> toasty::Result<T>
    where
        F: FnOnce(toasty::Db) -> Fut,
        Fut: Future<Output = toasty::Result<T>> + Send,
        T: Send,
    {
        let mut out = None;
        let slot = &mut out;
        let future = work(self.db.clone());
        (self.wait)(Box::pin(async move {
            *slot = Some(future.await);
        }));
        out.unwrap_or_else(|| {
            Err(toasty::Error::invalid_result(
                "the app's database did not run the browser's work",
            ))
        })
    }
}

pub trait Embedder: Send + Sync + 'static {
    /// Run `f` on the app's browser database, which holds the audit tables,
    /// the lease rows and the settings. Not calling `f` means "no database
    /// now": rows are skipped, nothing fails.
    fn with_db(&self, f: &mut dyn FnMut(&BrowserDb));

    /// The browser state changed (toggle, connection, grants, tabs,
    /// requests). The app shows it; the Tauri app sends
    /// [`crate::protocol::STATE_EVENT`] to its windows.
    fn state_changed(&self, state: &BrowserState);

    /// Pixel ratio of the display the app's main window is on; the
    /// Gespan browser starts at that scale.
    fn display_scale(&self) -> Option<f64> {
        None
    }

    /// The Gespan browser process was just started. On Windows the app
    /// puts it in its job object, so it ends with the app even after a
    /// crash; OFF and app exit stop it either way.
    fn browser_started(&self, _child: &Child) {}

    /// The settings the app runs with, from what was stored (at start) or
    /// what its UI saves. An app that offers fewer choices than the module
    /// fixes the rest here; both the host and the database get the result.
    fn fix_settings(&self, settings: BrowserSettings) -> BrowserSettings {
        settings
    }
}

/// Tests: the browser tables in an in-memory database, on a runtime of
/// their own. The app's real database runs the embedded migrations; this
/// one is built straight from the models, which those migrations match.
#[cfg(any(test, feature = "testing"))]
mod memory {
    use super::{BrowserDb, DbJob};
    use std::sync::OnceLock;

    fn runtime() -> &'static tokio::runtime::Runtime {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("gespan-browser-test-db")
                .enable_all()
                .build()
                .expect("the test database runtime starts")
        })
    }

    /// Blocks on `job`, from a helper thread when the caller is itself on a
    /// tokio runtime (where `block_on` would panic).
    fn wait(job: DbJob<'_>) {
        if tokio::runtime::Handle::try_current().is_err() {
            return runtime().block_on(job);
        }
        std::thread::scope(|scope| {
            scope
                .spawn(|| runtime().block_on(job))
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        });
    }

    impl BrowserDb {
        pub(crate) fn memory() -> BrowserDb {
            let mut db = None;
            let slot = &mut db;
            wait(Box::pin(async move {
                let built = async {
                    let db = toasty::Db::builder()
                        .models(crate::audit::models())
                        .max_pool_size(1)
                        // An in-memory database lives as long as its one connection.
                        .pool_health_check_interval(None)
                        .connect("sqlite::memory:")
                        .await?;
                    db.push_schema().await?;
                    for sql in crate::audit::CONSTRAINTS {
                        toasty::sql::statement(*sql).exec(&mut db.clone()).await?;
                    }
                    Ok::<_, toasty::Error>(db)
                };
                *slot = Some(built.await.expect("in-memory browser db"));
            }));
            BrowserDb::new(db.expect("in-memory browser db"), wait)
        }
    }

    #[cfg(test)]
    impl BrowserDb {
        /// Runs one raw statement.
        pub(crate) fn execute(&self, sql: &str) -> u64 {
            let sql = sql.to_owned();
            self.run(|mut db| async move { toasty::sql::statement(sql).exec(&mut db).await })
                .expect("statement")
        }

        /// The first column of the first row, as text.
        pub(crate) fn text(&self, sql: &str) -> String {
            let sql = sql.to_owned();
            let rows = self
                .run(|mut db| async move { toasty::sql::query(sql).exec(&mut db).await })
                .expect("query");
            let row = rows.into_iter().next().expect("a row").into_record();
            String::try_from(row.fields.into_iter().next().expect("a column")).expect("text")
        }
    }
}

/// Tests: an in-memory database and no windows.
#[cfg(any(test, feature = "testing"))]
pub(crate) struct MemoryEmbedder(BrowserDb);

#[cfg(any(test, feature = "testing"))]
impl MemoryEmbedder {
    pub(crate) fn new() -> MemoryEmbedder {
        MemoryEmbedder(BrowserDb::memory())
    }
}

#[cfg(any(test, feature = "testing"))]
impl Embedder for MemoryEmbedder {
    fn with_db(&self, f: &mut dyn FnMut(&BrowserDb)) {
        f(&self.0);
    }

    fn state_changed(&self, _state: &BrowserState) {}
}
