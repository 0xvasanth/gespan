//! The panic hook: a crash file that survives the process, then the previous
//! hook, so the default message (or a test harness's hook) still runs.
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{
    any::Any,
    cell::Cell,
    io::{self, Write},
    path::{Path, PathBuf},
};

thread_local! {
    /// How many `diagnostics::catch_unwind` calls this thread is inside.
    static CATCHING: Cell<usize> = const { Cell::new(0) };
}

/// Runs `f` with this thread's panics marked as expected, so the hook writes
/// no crash file for them. The caller catches and logs them.
pub(super) fn expecting_panics<T>(f: impl FnOnce() -> T) -> T {
    struct Depth;
    impl Drop for Depth {
        fn drop(&mut self) {
            CATCHING.with(|depth| depth.set(depth.get() - 1));
        }
    }
    CATCHING.with(|depth| depth.set(depth.get() + 1));
    let _depth = Depth;
    f()
}

pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "Box<dyn Any>".into()
    }
}

/// Writes `record` to `crash_dir/<ts>.json` (`<ts>-1.json`, … when two panics
/// share a millisecond) and fsyncs it before returning.
pub fn write_crash(crash_dir: &Path, record: &Value, at: DateTime<Utc>) -> io::Result<PathBuf> {
    std::fs::create_dir_all(crash_dir)?;
    let stamp = at.format("%Y-%m-%dT%H-%M-%S%.3fZ");
    let body = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
    for attempt in 0.. {
        let path = crash_dir.join(if attempt == 0 {
            format!("{stamp}.json")
        } else {
            format!("{stamp}-{attempt}.json")
        });
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        file.write_all(&body)?;
        file.sync_all()?;
        // Make the new directory entry durable too, where the OS allows it.
        if let Ok(dir) = std::fs::File::open(crash_dir) {
            let _ = dir.sync_all();
        }
        return Ok(path);
    }
    unreachable!("an unbounded range always yields another attempt")
}

/// Records every panic to `crash_dir`, logs it at `error` with
/// `kind = "panic"`, then runs the hook that was installed before. The hook
/// itself never panics (a panic inside a panic hook aborts the process), so
/// a crash folder it cannot write is skipped, not fatal.
pub fn install_panic_hook(crash_dir: PathBuf, version: String, build: String) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A panic a `diagnostics::catch_unwind` recovers from is not a crash.
        if CATCHING.try_with(Cell::get).unwrap_or(0) > 0 {
            previous(info);
            return;
        }
        let message = panic_message(info.payload());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_owned();
        let at = Utc::now();
        let record = serde_json::json!({
            "kind": "panic",
            "ts": at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "message": message,
            "location": location,
            "thread": thread,
            "backtrace": std::backtrace::Backtrace::force_capture().to_string(),
            "app": "native",
            "version": version,
            "build": build,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        });
        let crash_file = write_crash(&crash_dir, &record, at);
        tracing::error!(
            kind = "panic",
            location = %location,
            thread = %thread,
            crash_file = crash_file.as_ref().ok().map(|p| p.display().to_string()),
            crash_error = crash_file.as_ref().err().map(|e| e.to_string()),
            "panic: {message}"
        );
        super::metrics::record_panic();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_millisecond_panics_get_distinct_files() {
        let dir = tempfile::tempdir().unwrap();
        let crashes = dir.path().join("crashes");
        let at = DateTime::parse_from_rfc3339("2026-10-25T09:15:30.123Z")
            .unwrap()
            .with_timezone(&Utc);
        let first = write_crash(&crashes, &serde_json::json!({"message": "a"}), at).unwrap();
        let second = write_crash(&crashes, &serde_json::json!({"message": "b"}), at).unwrap();
        assert_eq!(first.file_name().unwrap(), "2026-10-25T09-15-30.123Z.json");
        assert_eq!(
            second.file_name().unwrap(),
            "2026-10-25T09-15-30.123Z-1.json"
        );
        let read = |path: &Path| -> Value {
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        assert_eq!(read(&first)["message"], "a");
        assert_eq!(read(&second)["message"], "b");
    }

    #[test]
    fn panic_message_reads_common_payloads() {
        let caught = |f: fn()| std::panic::catch_unwind(f).unwrap_err();
        assert_eq!(
            panic_message(&*caught(|| panic!("static text"))),
            "static text"
        );
        assert_eq!(
            panic_message(&*caught(|| panic!("formatted {}", 42))),
            "formatted 42"
        );
        assert_eq!(
            panic_message(&*caught(|| std::panic::panic_any(42u8))),
            "Box<dyn Any>"
        );
    }
}
