//! The panic hook is process-wide, so it gets its own test binary and the
//! scenarios run in order inside one test.
use gespan_core::diagnostics::install_panic_hook;
use std::sync::atomic::{AtomicUsize, Ordering};

static PREVIOUS_HOOK_RUNS: AtomicUsize = AtomicUsize::new(0);

fn json_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| entries.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default()
}

#[test]
fn panics_write_one_crash_file_and_keep_the_previous_hook() {
    std::panic::set_hook(Box::new(|_| {
        PREVIOUS_HOOK_RUNS.fetch_add(1, Ordering::SeqCst);
    }));
    let dir = tempfile::tempdir().unwrap();
    let crashes = dir.path().join("crashes");
    install_panic_hook(crashes.clone(), "9.8.7".into());

    let line = line!() + 3;
    let joined = std::thread::Builder::new()
        .name("worker".into())
        .spawn(|| panic!("boom {}", 42))
        .unwrap()
        .join();
    assert!(joined.is_err());
    let files = json_files(&crashes);
    assert_eq!(files.len(), 1, "exactly one crash file: {files:?}");
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&files[0]).unwrap()).unwrap();
    assert_eq!(record["kind"], "panic");
    assert_eq!(record["message"], "boom 42");
    assert_eq!(record["thread"], "worker");
    assert_eq!(record["app"], "native");
    assert_eq!(record["version"], "9.8.7");
    assert_eq!(record["os"], std::env::consts::OS);
    assert_eq!(record["arch"], std::env::consts::ARCH);
    let location = record["location"].as_str().unwrap();
    assert!(
        location.contains(&format!("diagnostics_panic.rs:{line}:")),
        "{location}"
    );
    assert!(record["backtrace"].is_string());
    chrono::DateTime::parse_from_rfc3339(record["ts"].as_str().unwrap()).unwrap();
    assert_eq!(PREVIOUS_HOOK_RUNS.load(Ordering::SeqCst), 1);

    // An unwritable crash folder (a regular file sits at its path) must not
    // stop the previous hook or turn the panic into an abort.
    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, "not a folder").unwrap();
    install_panic_hook(blocked.clone(), "9.8.7".into());
    let joined = std::thread::spawn(|| panic!("second")).join();
    assert!(joined.is_err());
    // The second hook wraps the first, which still writes its own file.
    assert_eq!(json_files(&crashes).len(), 2);
    assert_eq!(PREVIOUS_HOOK_RUNS.load(Ordering::SeqCst), 2);
}
