//! "Open logs folder" and "Copy diagnostics". Both stay on this machine.
use super::{crash_dir, rotation::scan, Channel, STARTED};
use std::path::{Path, PathBuf};

/// Plain text for the clipboard. It carries no log lines: scrubbing them is
/// the redaction crate's job, so this only describes the app and its files.
pub fn summary_for(log_dir: &Path, version: &str, channel: Channel, home: Option<&Path>) -> String {
    let shown = match home.and_then(|home| log_dir.strip_prefix(home).ok()) {
        Some(rest) => Path::new("~").join(rest),
        None => log_dir.to_path_buf(),
    };
    let logs = scan(log_dir).unwrap_or_default();
    let bytes: u64 = logs.iter().map(|(_, _, size)| size).sum();
    let mut crashes: Vec<String> = std::fs::read_dir(crash_dir(log_dir))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".json"))
                .collect()
        })
        .unwrap_or_default();
    crashes.sort();
    let channel = match channel {
        Channel::Release => "release",
        Channel::Dev => "dev",
    };
    let mut text = format!(
        "Gespan Native {version} ({channel})\nOS: {} {}\nLogs: {}\nLog files: {} ({})\nCrash files: {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        shown.display(),
        logs.len(),
        size(bytes),
        crashes.len()
    );
    if let Some(latest) = crashes.last() {
        text.push_str(&format!(" (latest: {latest})"));
    }
    text
}

fn size(bytes: u64) -> String {
    const KB: f64 = 1024.;
    match bytes as f64 {
        b if b < KB => format!("{bytes} B"),
        b if b < KB * KB => format!("{:.1} KB", b / KB),
        b => format!("{:.1} MB", b / (KB * KB)),
    }
}

/// The summary for the running app.
pub fn summary() -> String {
    let Some(config) = STARTED.get() else {
        return "Gespan Native diagnostics have not started.".into();
    };
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|home| !home.is_empty())
        .map(PathBuf::from);
    summary_for(
        &config.log_dir,
        &config.version,
        config.channel,
        home.as_deref(),
    )
}

/// Reveals the log folder in Finder, Explorer or the file manager.
pub fn open_logs_folder() -> Result<(), String> {
    let config = STARTED
        .get()
        .ok_or("Diagnostics have not started, so there is no logs folder yet.")?;
    std::fs::create_dir_all(&config.log_dir).map_err(|error| error.to_string())?;
    crate::files::open_path_with_default_app(&config.log_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_hides_home_and_counts_files() {
        let home = tempfile::tempdir().unwrap();
        let logs = home.path().join("Library/Logs/dev.gespan.native");
        std::fs::create_dir_all(crash_dir(&logs)).unwrap();
        std::fs::write(
            logs.join("gespan-native.2026-10-24.jsonl"),
            vec![b'x'; 1000],
        )
        .unwrap();
        std::fs::write(
            logs.join("gespan-native.2026-10-25.jsonl"),
            vec![b'x'; 1048],
        )
        .unwrap();
        std::fs::write(logs.join(".DS_Store"), vec![b'x'; 99]).unwrap();
        for name in [
            "2026-10-20T08-00-00.000Z.json",
            "2026-10-24T09-15-30.123Z.json",
        ] {
            std::fs::write(crash_dir(&logs).join(name), "{}").unwrap();
        }
        let text = summary_for(&logs, "0.16.0", Channel::Release, Some(home.path()));
        assert_eq!(
            text,
            format!(
                "Gespan Native 0.16.0 (release)\nOS: {} {}\nLogs: ~/Library/Logs/dev.gespan.native\nLog files: 2 (2.0 KB)\nCrash files: 2 (latest: 2026-10-24T09-15-30.123Z.json)",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        );
        assert!(!text.contains(&*home.path().to_string_lossy()));
    }

    #[test]
    fn summary_with_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("absent");
        let text = summary_for(&logs, "0.16.0", Channel::Dev, None);
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines[0], "Gespan Native 0.16.0 (dev)");
        assert_eq!(lines[2], format!("Logs: {}", logs.display()));
        assert_eq!(lines[3], "Log files: 0 (0 B)");
        assert_eq!(lines[4], "Crash files: 0");
    }

    #[test]
    fn summary_outside_home_keeps_the_absolute_path() {
        let logs = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            logs.path().join("gespan-native.2026-10-25.jsonl"),
            vec![b'x'; 3 * 1024 * 1024],
        )
        .unwrap();
        let text = summary_for(logs.path(), "1.0.0", Channel::Release, Some(home.path()));
        assert!(text.contains(&format!("Logs: {}\n", logs.path().display())));
        assert!(text.contains("Log files: 1 (3.0 MB)"));
    }

    #[test]
    fn a_home_that_only_shares_a_prefix_is_not_shortened() {
        let text = summary_for(
            Path::new("/Users/adam/logs"),
            "1.0.0",
            Channel::Release,
            Some(Path::new("/Users/ada")),
        );
        assert!(text.contains("Logs: /Users/adam/logs\n"));
    }
}
