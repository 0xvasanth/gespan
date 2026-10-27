//! `init` installs process-wide state (the global subscriber, the `log`
//! bridge and the panic hook), so it runs once, in its own test binary.
use gespan_core::diagnostics::{self, Channel, Config};
use std::time::{Duration, Instant};

#[test]
fn init_prunes_writes_both_apis_and_flushes() {
    // Hermetic: a developer's exported GESPAN_LOG must not change the levels.
    // This binary has one test, so nothing else reads the environment.
    std::env::remove_var("GESPAN_LOG");
    let dir = tempfile::tempdir().unwrap();
    // HOME is empty, but the profile folder still finds the home (as a
    // passwd lookup would): the summary must hide it either way.
    std::env::set_var("HOME", "");
    std::env::set_var("USERPROFILE", dir.path());
    let logs = dir.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let today = chrono::Utc::now().date_naive();
    let stale = logs.join(diagnostics::file_name(today - chrono::Duration::days(30)));
    std::fs::write(&stale, "{}\n").unwrap();

    let guard = diagnostics::init(Config {
        log_dir: logs.clone(),
        version: "3.2.1".into(),
        build: "4f2a9c1e0b7d".into(),
        channel: Channel::Release,
    })
    .unwrap();
    assert!(!stale.exists(), "startup prunes files older than 7 days");
    // Filtered-out `log::debug!` calls in dependencies stop at `log`'s own
    // level check instead of reaching the subscriber.
    assert_eq!(log::max_level(), log::LevelFilter::Info);
    assert!(diagnostics::init(Config {
        log_dir: logs.clone(),
        version: "3.2.1".into(),
        build: String::new(),
        channel: Channel::Release,
    })
    .is_err());

    tracing::error!(code = 5, "e2e");
    log::warn!(target: "gespan_core::git", "from log");
    tracing::debug!("below the default level");
    diagnostics::flush();

    // Every daily file, in case the test ran across UTC midnight.
    let text: String = std::fs::read_dir(&logs)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "jsonl"))
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect();
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
    assert_eq!(e2e["build"], "4f2a9c1e0b7d");
    let bridged = find("from log");
    assert_eq!(bridged["level"], "warn");
    assert_eq!(bridged["target"], "gespan_core::git");
    assert!(lines
        .iter()
        .all(|l| l["message"] != "below the default level"));

    let summary = diagnostics::summary();
    assert!(
        summary.starts_with("Gespan Native 3.2.1 build 4f2a9c1e0b7d (release)\n"),
        "{summary}"
    );
    assert!(
        summary.contains(&format!("Logs: ~{}logs\n", std::path::MAIN_SEPARATOR)),
        "{summary}"
    );
    assert!(
        !summary.contains(&*dir.path().to_string_lossy()),
        "{summary}"
    );

    // Flushing twice, and logging after the flush, must neither panic nor block.
    let started = Instant::now();
    diagnostics::flush();
    tracing::info!("after flush");
    drop(guard);
    assert!(started.elapsed() < Duration::from_secs(2));
}
