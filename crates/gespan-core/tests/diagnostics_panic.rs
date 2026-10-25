//! The panic hook is process-wide, so it gets its own test binary and the
//! scenarios run in order inside one test.
use gespan_core::diagnostics::{self, install_panic_hook};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tracing_subscriber::layer::SubscriberExt;

static PREVIOUS_HOOK_RUNS: AtomicUsize = AtomicUsize::new(0);

fn json_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| entries.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default()
}

/// The JSON lines the hook and the `catch_unwind` wrapper log, from every thread.
#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Lines {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Lines {
    fn json(&self) -> Vec<serde_json::Value> {
        String::from_utf8(self.0.lock().unwrap().clone())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn panic_lines(&self) -> Vec<serde_json::Value> {
        self.json()
            .into_iter()
            .filter(|line| line["fields"]["kind"] == "panic")
            .collect()
    }
}

#[test]
fn panics_write_one_crash_file_and_keep_the_previous_hook() {
    let lines = Lines::default();
    let writer = lines.clone();
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(diagnostics::json_layer(move || writer.clone(), "9.8.7")),
    )
    .unwrap();
    std::panic::set_hook(Box::new(|info| {
        PREVIOUS_HOOK_RUNS.fetch_add(1, Ordering::SeqCst);
        // Keep failed assertions readable: this stands in for the default hook.
        eprintln!("{info}");
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
    let logged = lines.panic_lines();
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0]["level"], "error");
    assert_eq!(logged[0]["fields"]["thread"], "worker");
    // A plain path that later tooling can open, not `Some("…")`.
    assert_eq!(
        logged[0]["fields"]["crash_file"],
        files[0].to_string_lossy().as_ref()
    );

    // A panic that `catch_unwind` recovers from is not a crash: no crash
    // file and no `kind = "panic"` error, but a `warn` line, and the
    // previous hook still runs.
    let caught = diagnostics::catch_unwind("probe", || -> u8 { panic!("caught one") });
    assert_eq!(caught, Err("caught one".to_owned()));
    assert_eq!(diagnostics::catch_unwind("probe", || 7u8), Ok(7));
    assert_eq!(json_files(&crashes).len(), 1);
    assert_eq!(lines.panic_lines().len(), 1);
    assert_eq!(PREVIOUS_HOOK_RUNS.load(Ordering::SeqCst), 2);
    let warned = lines
        .json()
        .into_iter()
        .find(|line| line["level"] == "warn")
        .expect("the caught panic is logged at warn");
    assert_eq!(warned["fields"]["what"], "probe");
    assert_eq!(warned["fields"]["panic"], "caught one");
    // A real panic on this same thread afterwards is a crash again.
    assert!(std::panic::catch_unwind(|| panic!("after")).is_err());
    assert_eq!(json_files(&crashes).len(), 2);

    // An unwritable crash folder (a regular file sits at its path) must not
    // stop the previous hook or turn the panic into an abort.
    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, "not a folder").unwrap();
    install_panic_hook(blocked.clone(), "9.8.7".into());
    let joined = std::thread::spawn(|| panic!("second")).join();
    assert!(joined.is_err());
    // The second hook wraps the first, which still writes its own file.
    assert_eq!(json_files(&crashes).len(), 3);
    assert_eq!(PREVIOUS_HOOK_RUNS.load(Ordering::SeqCst), 4);
    let failed = lines
        .panic_lines()
        .into_iter()
        .find(|line| {
            line["message"] == "panic: second" && line["fields"]["crash_error"].is_string()
        })
        .expect("the unwritable folder is reported");
    assert!(failed["fields"].get("crash_file").is_none());
}
