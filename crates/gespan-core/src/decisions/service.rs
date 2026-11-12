//! The process-wide decision log: one store, the on/off switch, and the hooks
//! the app registers (regeneration of harness files, and the user-authored
//! text of a session).
use super::entry::Config;
use super::store::DecisionStore;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

static STORE: Mutex<Option<Arc<DecisionStore>>> = Mutex::new(None);
static ENABLED: AtomicBool = AtomicBool::new(false);
static REGENERATE: Mutex<Option<Arc<dyn Fn() + Send + Sync>>> = Mutex::new(None);
static USER_TEXT: Mutex<Option<Arc<UserTextSource>>> = Mutex::new(None);

/// Returns the texts the user wrote in a session: the messages they sent and
/// their answers to its agent's questions. Never agent text, tool output or
/// attachments.
pub type UserTextSource = dyn Fn(i64) -> Vec<String> + Send + Sync;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Opens the decision log on the app's gespan.db at `database` once (its own
/// Toasty handle on the file the app's store uses). Later calls keep the
/// open store.
pub fn init(database: &Path) -> Result<(), String> {
    let mut slot = lock(&STORE);
    if slot.is_some() {
        return Ok(());
    }
    let store = DecisionStore::open(database, config()).map_err(|e| e.to_string())?;
    *slot = Some(Arc::new(store));
    Ok(())
}

fn config() -> Config {
    Config::app()
}

/// Runs `f` on the open store, or says the log isn't open.
pub fn with<T>(f: impl FnOnce(&DecisionStore) -> T) -> Result<T, String> {
    let store = lock(&STORE).clone();
    match store {
        Some(store) => Ok(f(&store)),
        None => Err("The decision log isn't open.".into()),
    }
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

/// Registers what rewrites the harness files after the log changes.
pub fn set_regenerate(hook: Box<dyn Fn() + Send + Sync>) {
    *lock(&REGENERATE) = Some(Arc::from(hook));
}

/// Calls the registered hook; nothing happens when there is none.
pub fn regenerate() {
    let hook = lock(&REGENERATE).clone();
    if let Some(hook) = hook {
        hook();
    }
}

/// Registers where the user-authored text of a session comes from. Until one
/// is registered, no proposal can be verified as the user's.
pub fn set_user_text_source(source: Box<UserTextSource>) {
    *lock(&USER_TEXT) = Some(Arc::from(source));
}

/// What the user wrote in `session`; empty without a registered source.
pub fn user_texts(session: i64) -> Vec<String> {
    let source = lock(&USER_TEXT).clone();
    source.map_or_else(Vec::new, |source| source(session))
}

/// Serializes tests that touch the process-wide state and gives each a fresh
/// store, the switch on, and no hooks.
#[cfg(test)]
pub(crate) struct Isolated {
    _lock: MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
}

#[cfg(test)]
impl Drop for Isolated {
    /// Closes the store before its directory goes, so Windows can delete it.
    fn drop(&mut self) {
        *lock(&STORE) = None;
    }
}

#[cfg(test)]
pub(crate) fn isolated() -> Isolated {
    static TESTS: Mutex<()> = Mutex::new(());
    let lock = lock(&TESTS);
    let dir = tempfile::tempdir().unwrap();
    let store = DecisionStore::open(&dir.path().join("gespan.db"), config()).unwrap();
    *self::lock(&STORE) = Some(Arc::new(store));
    *self::lock(&REGENERATE) = None;
    *self::lock(&USER_TEXT) = None;
    set_enabled(true);
    Isolated {
        _lock: lock,
        _dir: dir,
    }
}
