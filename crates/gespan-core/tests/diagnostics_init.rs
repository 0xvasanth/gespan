//! `init` installs process-wide state (the global subscriber, the `log`
//! bridge and the panic hook), so it runs once, in its own test binary.
use gespan_core::diagnostics::{self, Channel, Config};
use std::time::{Duration, Instant};

#[test]
fn init_prunes_writes_both_apis_and_flushes() {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let today = chrono::Utc::now().date_naive();
    let stale = logs.join(diagnostics::file_name(today - chrono::Duration::days(30)));
    std::fs::write(&stale, "{}\n").unwrap();

    let guard = diagnostics::init(Config {
        log_dir: logs.clone(),
        version: "3.2.1".into(),
        channel: Channel::Release,
    })
    .unwrap();
    assert!(!stale.exists(), "startup prunes files older than 7 days");
    assert!(diagnostics::init(Config {
        log_dir: logs.clone(),
        version: "3.2.1".into(),
        channel: Channel::Release,
    })
    .is_err());

    tracing::error!(code = 5, "e2e");
    log::warn!("from log");
    tracing::debug!("below the default level");
    diagnostics::flush();

    let text = std::fs::read_to_string(logs.join(diagnostics::file_name(today))).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let find = |message: &str| {
        lines
            .iter()
            .find(|line| line["message"] == message)
            .unwrap_or_else(|| panic!("no {message:?} in {text}"))
    };
    let e2e = find("e2e");
    assert_eq!(e2e["level"], "error");
    assert_eq!(e2e["fields"]["code"], 5);
    assert_eq!(e2e["app"], "native");
    assert_eq!(e2e["version"], "3.2.1");
    assert_eq!(find("from log")["level"], "warn");
    assert!(lines
        .iter()
        .all(|l| l["message"] != "below the default level"));

    // Flushing twice, and logging after the flush, must neither panic nor block.
    let started = Instant::now();
    diagnostics::flush();
    tracing::info!("after flush");
    drop(guard);
    assert!(started.elapsed() < Duration::from_secs(2));
}
