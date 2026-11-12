//! Notices a run that ended without a clean quit.
//!
//! Each run leaves `sessions/running-<pid>.json` next to the logs and removes
//! it on a clean quit. A marker whose process is gone at the next launch is a
//! run that ended some other way: a panic, an abort, a kill, a power loss. The
//! crash folder says which of those left a trace.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

const PREFIX: &str = "running-";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Marker {
    pid: u32,
    started: DateTime<Utc>,
    version: String,
    build: String,
}

/// A run that ended without a clean quit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UncleanExit {
    pub pid: u32,
    pub started: DateTime<Utc>,
    pub version: String,
    pub build: String,
    /// The panic hook's record from that run, when it panicked. `None` means
    /// the run left no trace: an abort, a kill, or the machine going down.
    pub crash_file: Option<PathBuf>,
}

/// This run's marker. [`RunningSession::finish`] on a clean quit; anything
/// else leaves the marker for the next launch to find.
#[derive(Debug)]
pub struct RunningSession {
    path: PathBuf,
}

impl RunningSession {
    /// Removes this run's marker: the run quit cleanly.
    pub fn finish(&self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => tracing::info!("clean quit"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(%error, "cannot remove the running-session marker"),
        }
    }
}

/// Where the markers live, beside the logs and crash files.
pub fn sessions_dir(log_dir: &Path) -> PathBuf {
    log_dir.join("sessions")
}

/// Marks this run as running and returns the earlier runs that never quit
/// cleanly. Markers of runs still alive (a second instance) are left alone.
pub fn start(
    log_dir: &Path,
    version: &str,
    build: &str,
) -> io::Result<(RunningSession, Vec<UncleanExit>)> {
    start_with(
        &sessions_dir(log_dir),
        &super::crash_dir(log_dir),
        std::process::id(),
        Utc::now(),
        version,
        build,
        process_alive,
    )
}

fn start_with(
    dir: &Path,
    crash_dir: &Path,
    pid: u32,
    now: DateTime<Utc>,
    version: &str,
    build: &str,
    alive: impl Fn(u32) -> bool,
) -> io::Result<(RunningSession, Vec<UncleanExit>)> {
    std::fs::create_dir_all(dir)?;
    let crashes = crash_times(crash_dir);
    let mut unclean = Vec::new();
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with(PREFIX) || !name.ends_with(".json") {
            continue;
        }
        let marker = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Marker>(&bytes).ok());
        let Some(marker) = marker else {
            // A marker cut short by the crash it would report; nothing to
            // attribute, so it only goes.
            tracing::warn!(file = %path.display(), "unreadable running-session marker");
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if marker.pid == pid || alive(marker.pid) {
            continue;
        }
        let crash_file = crashes
            .iter()
            .filter(|(at, _)| *at >= marker.started)
            .min_by_key(|(at, _)| *at)
            .map(|(_, path)| path.clone());
        let _ = std::fs::remove_file(&path);
        unclean.push(UncleanExit {
            pid: marker.pid,
            started: marker.started,
            version: marker.version,
            build: marker.build,
            crash_file,
        });
    }
    unclean.sort_by_key(|exit| exit.started);
    let path = dir.join(format!("{PREFIX}{pid}.json"));
    let marker = Marker {
        pid,
        started: now,
        version: version.into(),
        build: build.into(),
    };
    std::fs::write(
        &path,
        serde_json::to_vec(&marker).map_err(io::Error::other)?,
    )?;
    Ok((RunningSession { path }, unclean))
}

/// Crash files by the time in their name (`write_crash` names them).
fn crash_times(crash_dir: &Path) -> Vec<(DateTime<Utc>, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(crash_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let stem = path.file_stem()?.to_str()?;
            // `<stamp>.json`, or `<stamp>-<n>.json` for a shared millisecond.
            let stamp = stem.get(..stem.find('Z')? + 1)?;
            let at = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%dT%H-%M-%S%.3fZ").ok()?;
            Some((at.and_utc(), path))
        })
        .collect()
}

/// Logs each unclean exit at `warn` and counts it in the diagnostics counters,
/// so the Diagnostics page and a shared report both show it.
pub fn report(unclean: &[UncleanExit]) {
    for exit in unclean {
        let how = if exit.crash_file.is_some() {
            "panicked"
        } else {
            "ended without a trace"
        };
        tracing::warn!(
            kind = "unclean_exit",
            pid = exit.pid,
            started = %exit.started.to_rfc3339(),
            previous_version = %exit.version,
            previous_build = %exit.build,
            crash_file = exit.crash_file.as_ref().map(|p| p.display().to_string()),
            "the previous run {how} instead of quitting"
        );
        super::metrics::failure(
            "app.exit",
            if exit.crash_file.is_some() {
                super::metrics::ErrorKind::Panic
            } else {
                super::metrics::ErrorKind::Other
            },
        );
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // EPERM means it exists under another user: still alive.
    pid != 0
        && (unsafe { libc::kill(pid as i32, 0) } == 0
            || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    if pid == 0 {
        return false;
    }
    // SAFETY: the handle is checked, read once, and closed.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let read = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        read != 0 && code == STILL_ACTIVE as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 12, h, m, 0).unwrap()
    }

    fn dirs() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let crashes = root.path().join("crashes");
        (root, sessions, crashes)
    }

    fn launch(
        sessions: &Path,
        crashes: &Path,
        pid: u32,
        now: DateTime<Utc>,
        alive: impl Fn(u32) -> bool,
    ) -> (RunningSession, Vec<UncleanExit>) {
        start_with(sessions, crashes, pid, now, "0.1.3", "abc123", alive).unwrap()
    }

    fn markers(sessions: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(sessions)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_clean_quit_leaves_nothing_for_the_next_launch() {
        let (_root, sessions, crashes) = dirs();
        let (run, unclean) = launch(&sessions, &crashes, 100, at(9, 0), |_| false);
        assert!(unclean.is_empty());
        assert_eq!(markers(&sessions), ["running-100.json"]);
        run.finish();
        assert!(markers(&sessions).is_empty());
        let (_run, unclean) = launch(&sessions, &crashes, 200, at(10, 0), |_| false);
        assert!(unclean.is_empty());
    }

    #[test]
    fn a_run_that_vanished_is_reported_once_with_its_version() {
        let (_root, sessions, crashes) = dirs();
        let (_killed, _) = start_with(&sessions, &crashes, 100, at(9, 0), "0.1.2", "old", |_| {
            false
        })
        .unwrap();
        // No finish: the process died without quitting.
        let (_run, unclean) = launch(&sessions, &crashes, 200, at(10, 0), |_| false);
        assert_eq!(
            unclean,
            [UncleanExit {
                pid: 100,
                started: at(9, 0),
                version: "0.1.2".into(),
                build: "old".into(),
                crash_file: None,
            }]
        );
        assert_eq!(markers(&sessions), ["running-200.json"]);
        // The stale marker is gone, so a third launch doesn't report it again.
        let (_run, unclean) = launch(&sessions, &crashes, 300, at(11, 0), |pid| pid == 200);
        assert!(unclean.is_empty());
    }

    #[test]
    fn a_second_instance_still_running_is_not_an_unclean_exit() {
        let (_root, sessions, crashes) = dirs();
        let (_first, _) = launch(&sessions, &crashes, 100, at(9, 0), |_| false);
        let (_second, unclean) = launch(&sessions, &crashes, 200, at(9, 5), |pid| pid == 100);
        assert!(unclean.is_empty());
        assert_eq!(markers(&sessions), ["running-100.json", "running-200.json"]);
    }

    #[test]
    fn a_panic_after_the_run_started_is_attributed_to_it() {
        let (_root, sessions, crashes) = dirs();
        std::fs::create_dir_all(&crashes).unwrap();
        // An older crash from a run before this one, and the run's own panic
        // (second of two in the same millisecond, so it carries `-1`).
        std::fs::write(crashes.join("2026-11-12T08-00-00.000Z.json"), "{}").unwrap();
        std::fs::write(crashes.join("2026-11-12T09-30-00.250Z-1.json"), "{}").unwrap();
        std::fs::write(crashes.join("notes.txt"), "").unwrap();
        let (_crashed, _) = launch(&sessions, &crashes, 100, at(9, 0), |_| false);
        let (_run, unclean) = launch(&sessions, &crashes, 200, at(10, 0), |_| false);
        assert_eq!(unclean.len(), 1);
        assert_eq!(
            unclean[0].crash_file.as_deref(),
            Some(crashes.join("2026-11-12T09-30-00.250Z-1.json").as_path())
        );
    }

    #[test]
    fn a_crash_from_before_the_run_does_not_explain_its_exit() {
        let (_root, sessions, crashes) = dirs();
        std::fs::create_dir_all(&crashes).unwrap();
        std::fs::write(crashes.join("2026-11-12T08-59-59.999Z.json"), "{}").unwrap();
        let (_killed, _) = launch(&sessions, &crashes, 100, at(9, 0), |_| false);
        let (_run, unclean) = launch(&sessions, &crashes, 200, at(10, 0), |_| false);
        assert_eq!(unclean[0].crash_file, None);
    }

    #[test]
    fn a_torn_marker_is_dropped_without_failing_startup() {
        let (_root, sessions, crashes) = dirs();
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("running-100.json"), b"{\"pid\":10").unwrap();
        std::fs::write(sessions.join("unrelated.txt"), b"keep").unwrap();
        let (_run, unclean) = launch(&sessions, &crashes, 200, at(10, 0), |_| false);
        assert!(unclean.is_empty());
        assert_eq!(markers(&sessions), ["running-200.json", "unrelated.txt"]);
    }

    #[test]
    fn finishing_twice_is_harmless() {
        let (_root, sessions, crashes) = dirs();
        let (run, _) = launch(&sessions, &crashes, 100, at(9, 0), |_| false);
        run.finish();
        run.finish();
        assert!(markers(&sessions).is_empty());
    }

    #[test]
    fn liveness_tells_this_process_from_one_that_exited() {
        assert!(process_alive(std::process::id()));
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!process_alive(pid));
    }
}
