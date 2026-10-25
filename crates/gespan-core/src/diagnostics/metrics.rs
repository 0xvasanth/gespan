//! Failure counters: how often each operation failed per day, by a fixed
//! error kind, in Native's own `gespan-native.db`. Never `gespan.db`, which
//! the Tauri app opens too. Counts stay on this machine; nothing here sends.
//!
//! Before [`open`] runs (unit tests, `--check-data`) every call is a no-op.
use chrono::{NaiveDate, Utc};
use rusqlite::{params, Connection};
use std::{
    cell::RefCell,
    io,
    path::{Path, PathBuf},
    sync::{Mutex, TryLockError},
    time::Duration,
};

/// Why an operation failed, as a short fixed code. Never the error text, so
/// counts are safe to show and, with consent, to send.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorKind {
    SpawnFailed,
    NotFound,
    Timeout,
    Auth,
    Network,
    Io,
    ExitStatus,
    Parse,
    Panic,
    Other,
}

impl ErrorKind {
    pub const ALL: [Self; 10] = [
        Self::SpawnFailed,
        Self::NotFound,
        Self::Timeout,
        Self::Auth,
        Self::Network,
        Self::Io,
        Self::ExitStatus,
        Self::Parse,
        Self::Panic,
        Self::Other,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::SpawnFailed => "spawn_failed",
            Self::NotFound => "not_found",
            Self::Timeout => "timeout",
            Self::Auth => "auth",
            Self::Network => "network",
            Self::Io => "io",
            Self::ExitStatus => "exit_status",
            Self::Parse => "parse",
            Self::Panic => "panic",
            Self::Other => "other",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == code)
    }

    /// Sorts an error message into a kind. The message is read, never kept.
    pub fn of_message(message: &str) -> Self {
        let message = message.to_lowercase();
        let has = |needles: &[&str]| needles.iter().any(|n| message.contains(n));
        if has(&["timed out", "timeout"]) {
            Self::Timeout
        } else if has(&[
            "permission denied",
            "unauthorized",
            "forbidden",
            "authentication",
            "401",
            "403",
        ]) {
            Self::Auth
        } else if has(&[
            "could not resolve",
            "connection refused",
            "connection reset",
            "network",
            "unreachable",
        ]) {
            Self::Network
        } else if has(&[
            "not found",
            "no such file",
            "not a git repository",
            "not installed",
        ]) {
            Self::NotFound
        } else {
            Self::Other
        }
    }

    /// A process that could not start: a missing binary is worth telling
    /// apart from everything else.
    pub fn of_spawn(error: &io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            _ => Self::SpawnFailed,
        }
    }
}

impl From<&io::Error> for ErrorKind {
    fn from(error: &io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::PermissionDenied => Self::Auth,
            io::ErrorKind::TimedOut => Self::Timeout,
            io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted => Self::Network,
            _ => Self::Io,
        }
    }
}

/// One `diagnostic_counters` row. `kind` is `None` for successes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Count {
    pub day: NaiveDate,
    pub name: String,
    pub kind: Option<ErrorKind>,
    pub count: u64,
    pub app_version: String,
}

/// Successes share the table under this code, keeping its five columns.
const SUCCESS: &str = "ok";
const SHARED_DATABASE: &str = "gespan.db";
const MAX_NAME: usize = 64;

/// The counters file beside the shared database.
pub fn database_path(gespan_db: &Path) -> PathBuf {
    gespan_db.with_file_name("gespan-native.db")
}

/// `[a-z0-9_]` segments joined by dots, so error text cannot become a name.
fn valid_name(name: &str) -> bool {
    name.len() <= MAX_NAME
        && name.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

pub struct Counters {
    conn: Connection,
    version: String,
}

impl Counters {
    pub fn open(path: &Path, version: &str) -> Result<Self, String> {
        if path.file_name().is_some_and(|name| name == SHARED_DATABASE) {
            return Err(format!(
                "Failure counters never go in the shared {SHARED_DATABASE}."
            ));
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.busy_timeout(Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(|e| e.to_string())?;
        Self::with(conn, version)
    }

    pub fn open_in_memory(version: &str) -> Result<Self, String> {
        Self::with(
            Connection::open_in_memory().map_err(|e| e.to_string())?,
            version,
        )
    }

    fn with(conn: Connection, version: &str) -> Result<Self, String> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS diagnostic_counters (
                day TEXT NOT NULL,
                name TEXT NOT NULL,
                error_kind TEXT NOT NULL,
                count INTEGER NOT NULL,
                app_version TEXT NOT NULL,
                PRIMARY KEY (day, name, error_kind, app_version)
            ) WITHOUT ROWID;",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            conn,
            version: version.to_owned(),
        })
    }

    /// Adds one to `(day, name, kind)` for this app version. `None` is a success.
    pub fn record(
        &self,
        day: NaiveDate,
        name: &str,
        kind: Option<ErrorKind>,
    ) -> Result<(), String> {
        if !valid_name(name) {
            // The name may be error text, so only its length is reported.
            return Err(format!(
                "A counter name must be a short dotted code; got {} bytes.",
                name.len()
            ));
        }
        self.conn
            .execute(
                "INSERT INTO diagnostic_counters (day, name, error_kind, count, app_version)
                 VALUES (?1, ?2, ?3, 1, ?4)
                 ON CONFLICT (day, name, error_kind, app_version)
                 DO UPDATE SET count = count + 1",
                params![
                    day.to_string(),
                    name,
                    kind.map_or(SUCCESS, ErrorKind::code),
                    self.version
                ],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Rows from `first` on, by day, name, kind code and version.
    pub fn since(&self, first: NaiveDate) -> Result<Vec<Count>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT day, name, error_kind, count, app_version FROM diagnostic_counters
                 WHERE day >= ?1 ORDER BY day, name, error_kind, app_version",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([first.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut counts = Vec::new();
        for row in rows {
            let (day, name, kind, count, app_version) = row.map_err(|e| e.to_string())?;
            let kind = match kind.as_str() {
                SUCCESS => None,
                // A code from a newer build: keep the count, under `other`.
                code => Some(ErrorKind::from_code(code).unwrap_or(ErrorKind::Other)),
            };
            counts.push(Count {
                day: day
                    .parse()
                    .map_err(|_| format!("Bad counter day {day:?}"))?,
                name,
                kind,
                count: u64::try_from(count).unwrap_or(0),
                app_version,
            });
        }
        Ok(counts)
    }
}

/// The process's counters, set once by [`open`].
static COUNTERS: Mutex<Option<Counters>> = Mutex::new(None);

thread_local! {
    /// Set by [`capture`]; takes the place of the process's counters.
    static CAPTURE: RefCell<Option<Counters>> = const { RefCell::new(None) };
}

/// Opens the process's counters, logging why if it cannot. Later calls
/// replace the file.
pub fn open(path: &Path, version: &str) -> Result<(), String> {
    let counters = Counters::open(path, version)
        .inspect_err(|error| tracing::warn!(%error, "failure counters are off"))?;
    *COUNTERS.lock().unwrap_or_else(|e| e.into_inner()) = Some(counters);
    Ok(())
}

pub fn failure(name: &str, kind: ErrorKind) {
    record(name, Some(kind));
}

pub fn success(name: &str) {
    record(name, None);
}

/// A success, or a failure sorted by [`ErrorKind::of_message`].
pub fn track<T, E: AsRef<str>>(name: &str, result: &Result<T, E>) {
    match result {
        Ok(_) => success(name),
        Err(error) => failure(name, ErrorKind::of_message(error.as_ref())),
    }
}

fn record(name: &str, kind: Option<ErrorKind>) {
    let today = Utc::now().date_naive();
    let captured = CAPTURE
        .try_with(|capture| {
            capture
                .borrow()
                .as_ref()
                .map(|counters| counters.record(today, name, kind))
        })
        .ok()
        .flatten();
    let result = match captured {
        Some(result) => result,
        None => match COUNTERS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(counters) => counters.record(today, name, kind),
            None => return,
        },
    };
    if let Err(error) = result {
        tracing::warn!(%error, "cannot update a failure counter");
    }
}

/// Counts a crash panic, from the panic hook. It never waits for the lock:
/// the panicking thread may be the one holding it.
pub(super) fn record_panic() {
    let name = "panic";
    let kind = Some(ErrorKind::Panic);
    let today = Utc::now().date_naive();
    let captured = CAPTURE
        .try_with(|capture| {
            capture
                .try_borrow()
                .ok()?
                .as_ref()
                .map(|counters| counters.record(today, name, kind))
        })
        .ok()
        .flatten();
    if captured.is_some() {
        return;
    }
    let counters = match COUNTERS.try_lock() {
        Ok(counters) => counters,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    if let Some(counters) = counters.as_ref() {
        let _ = counters.record(today, name, kind);
    }
}

/// The last `days` days of counts, today included. Empty before [`open`].
pub fn last_days(days: u32) -> Result<Vec<Count>, String> {
    let first = Utc::now().date_naive() - chrono::Days::new(u64::from(days.saturating_sub(1)));
    match COUNTERS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        Some(counters) => counters.since(first),
        None => Ok(Vec::new()),
    }
}

/// Runs `body` with this thread's counts going to a fresh in-memory store,
/// and returns what it recorded. For tests in this crate and in Native.
pub fn capture<T>(body: impl FnOnce() -> T) -> (T, Vec<Count>) {
    let counters = Counters::open_in_memory("test").expect("in-memory counters");
    let previous = CAPTURE.with(|capture| capture.replace(Some(counters)));
    let value = body();
    let counters = CAPTURE
        .with(|capture| capture.replace(previous))
        .expect("capture was set above");
    let counts = counters
        .since(NaiveDate::MIN)
        .expect("read captured counts");
    (value, counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn day(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, n).unwrap()
    }
    fn rows(counters: &Counters) -> Vec<(NaiveDate, String, Option<ErrorKind>, u64)> {
        counters
            .since(day(1))
            .unwrap()
            .into_iter()
            .map(|c| (c.day, c.name, c.kind, c.count))
            .collect()
    }

    #[test]
    fn failures_increment_one_row_per_day_name_and_kind() {
        let counters = Counters::open_in_memory("1.0.0").unwrap();
        let name = "worktree.create";
        counters
            .record(day(20), name, Some(ErrorKind::Timeout))
            .unwrap();
        counters
            .record(day(20), name, Some(ErrorKind::Auth))
            .unwrap();
        counters
            .record(day(20), name, Some(ErrorKind::Timeout))
            .unwrap();
        counters
            .record(day(21), name, Some(ErrorKind::Timeout))
            .unwrap();
        assert_eq!(
            rows(&counters),
            [
                (day(20), name.into(), Some(ErrorKind::Auth), 1),
                (day(20), name.into(), Some(ErrorKind::Timeout), 2),
                (day(21), name.into(), Some(ErrorKind::Timeout), 1),
            ]
        );
    }

    #[test]
    fn successes_are_stored_as_ok_and_read_back_as_none() {
        let counters = Counters::open_in_memory("1.0.0").unwrap();
        counters.record(day(20), "app_update.check", None).unwrap();
        counters.record(day(20), "app_update.check", None).unwrap();
        let stored: String = counters
            .conn
            .query_row("SELECT error_kind FROM diagnostic_counters", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stored, "ok");
        assert_eq!(
            rows(&counters),
            [(day(20), "app_update.check".into(), None, 2)]
        );
    }

    #[test]
    fn a_new_app_version_gets_its_own_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan-native.db");
        Counters::open(&path, "1.0.0")
            .unwrap()
            .record(day(20), "panic", Some(ErrorKind::Panic))
            .unwrap();
        let updated = Counters::open(&path, "1.1.0").unwrap();
        updated
            .record(day(20), "panic", Some(ErrorKind::Panic))
            .unwrap();
        updated
            .record(day(20), "panic", Some(ErrorKind::Panic))
            .unwrap();
        let versions: Vec<_> = updated
            .since(day(1))
            .unwrap()
            .into_iter()
            .map(|c| (c.app_version, c.count))
            .collect();
        assert_eq!(versions, [("1.0.0".into(), 1), ("1.1.0".into(), 2)]);
    }

    #[test]
    fn names_with_free_text_are_dropped() {
        let counters = Counters::open_in_memory("1.0.0").unwrap();
        for name in [
            "git failed: /Users/someone/project",
            "",
            "Agent.Start",
            "agent..start",
            ".agent",
            "agent.",
            &"a".repeat(65),
        ] {
            assert!(
                counters
                    .record(day(20), name, Some(ErrorKind::Other))
                    .is_err(),
                "{name:?}"
            );
        }
        assert!(rows(&counters).is_empty());
        counters
            .record(day(20), "agent.start.claude", Some(ErrorKind::NotFound))
            .unwrap();
        counters
            .record(day(20), &"a".repeat(64), Some(ErrorKind::Other))
            .unwrap();
        assert_eq!(rows(&counters).len(), 2);
    }

    #[test]
    fn since_skips_older_days() {
        let counters = Counters::open_in_memory("1.0.0").unwrap();
        counters
            .record(day(18), "git.command", Some(ErrorKind::Io))
            .unwrap();
        counters
            .record(day(19), "git.command", Some(ErrorKind::Io))
            .unwrap();
        let days: Vec<_> = counters
            .since(day(19))
            .unwrap()
            .into_iter()
            .map(|c| c.day)
            .collect();
        assert_eq!(days, [day(19)]);
    }

    #[test]
    fn every_kind_round_trips_through_a_unique_snake_case_code() {
        let mut seen = std::collections::HashSet::new();
        for kind in ErrorKind::ALL {
            let code = kind.code();
            assert!(
                code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "{code}"
            );
            assert_ne!(code, "ok", "ok is reserved for successes");
            assert!(seen.insert(code), "{code} is used twice");
            assert_eq!(ErrorKind::from_code(code), Some(kind));
        }
        assert_eq!(ErrorKind::from_code("ok"), None);
        assert_eq!(ErrorKind::from_code("something else"), None);
    }

    #[test]
    fn messages_map_to_fixed_kinds() {
        for (message, kind) in [
            ("Operation timed out", ErrorKind::Timeout),
            ("Model discovery timed out", ErrorKind::Timeout),
            ("Request timeout after 30s", ErrorKind::Timeout),
            ("Permission denied (publickey).", ErrorKind::Auth),
            ("HTTP 401 Unauthorized", ErrorKind::Auth),
            ("403 Forbidden", ErrorKind::Auth),
            (
                "No such file or directory (os error 2)",
                ErrorKind::NotFound,
            ),
            (
                "fatal: not a git repository (or any parent)",
                ErrorKind::NotFound,
            ),
            ("Could not resolve host: github.com", ErrorKind::Network),
            ("Connection refused", ErrorKind::Network),
            ("boom", ErrorKind::Other),
            ("", ErrorKind::Other),
        ] {
            assert_eq!(ErrorKind::of_message(message), kind, "{message:?}");
        }
    }

    #[test]
    fn spawn_errors_separate_a_missing_binary_from_other_failures() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(ErrorKind::of_spawn(&missing), ErrorKind::NotFound);
        assert_eq!(ErrorKind::of_spawn(&denied), ErrorKind::SpawnFailed);
        assert_eq!(ErrorKind::from(&denied), ErrorKind::Auth);
        assert_eq!(
            ErrorKind::from(&std::io::Error::from(std::io::ErrorKind::TimedOut)),
            ErrorKind::Timeout
        );
        assert_eq!(ErrorKind::from(&std::io::Error::other("x")), ErrorKind::Io);
    }

    #[test]
    fn calls_before_open_are_ignored() {
        // Nothing is open in unit tests; these must neither panic nor write.
        failure("worktree.create", ErrorKind::Io);
        success("worktree.create");
        track("worktree.create", &Err::<(), _>("timed out"));
        assert!(last_days(7).unwrap().is_empty());
    }

    #[test]
    fn capture_sees_this_thread_s_calls() {
        let ((), counts) = capture(|| {
            track("remote.ssh_connect", &Err::<(), _>("Operation timed out"));
            track("remote.ssh_connect", &Ok::<_, String>(()));
            failure("bad name with text", ErrorKind::Other);
        });
        let got: Vec<_> = counts
            .into_iter()
            .map(|c| (c.name, c.kind, c.count))
            .collect();
        assert_eq!(
            got,
            [
                ("remote.ssh_connect".into(), None, 1),
                ("remote.ssh_connect".into(), Some(ErrorKind::Timeout), 1),
            ]
        );
        let other_thread = std::thread::spawn(|| capture(|| ()).1).join().unwrap();
        assert!(other_thread.is_empty());
    }

    #[test]
    fn the_table_has_the_specified_columns() {
        let counters = Counters::open_in_memory("1.0.0").unwrap();
        let mut statement = counters
            .conn
            .prepare("SELECT name FROM pragma_table_info('diagnostic_counters')")
            .unwrap();
        let columns: Vec<String> = statement
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            columns,
            ["day", "name", "error_kind", "count", "app_version"]
        );
    }

    #[test]
    fn refuses_the_shared_database() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("gespan.db");
        assert!(Counters::open(&shared, "1.0.0").is_err());
        assert!(!shared.exists());
        assert_eq!(database_path(&shared), dir.path().join("gespan-native.db"));
    }

    #[test]
    fn open_creates_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new/gespan-native.db");
        Counters::open(&path, "1.0.0").unwrap();
        assert!(path.exists());
    }

    #[test]
    fn gespan_db_gains_no_tables() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("gespan.db");
        drop(crate::store::Store::open_gespan(&shared).unwrap());
        let schema = |path: &Path| -> Vec<(String, String)> {
            let conn = Connection::open(path).unwrap();
            let mut statement = conn
                .prepare("SELECT type, name FROM sqlite_master ORDER BY type, name")
                .unwrap();
            let rows = statement
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            rows
        };
        let before = schema(&shared);
        assert!(!before.is_empty());
        let path = database_path(&shared);
        let counters = Counters::open(&path, "1.0.0").unwrap();
        counters
            .record(day(20), "worktree.create", Some(ErrorKind::Io))
            .unwrap();
        drop(counters);
        assert_eq!(schema(&shared), before);
        assert!(!before.iter().any(|(_, name)| name == "diagnostic_counters"));
        assert_eq!(path, dir.path().join("gespan-native.db"));
        assert!(schema(&path)
            .iter()
            .any(|(_, name)| name == "diagnostic_counters"));
    }

    #[test]
    fn crash_panics_are_counted() {
        let ((), counts) = capture(record_panic);
        let got: Vec<_> = counts
            .into_iter()
            .map(|c| (c.name, c.kind, c.count))
            .collect();
        assert_eq!(got, [("panic".into(), Some(ErrorKind::Panic), 1)]);
    }

    #[test]
    fn a_held_lock_skips_the_panic_count_instead_of_blocking() {
        // A panic raised while this thread records a count must not deadlock.
        let _held = COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
        record_panic();
    }
}
