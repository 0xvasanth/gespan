//! The send queue: one file per pending background send, by kind, under
//! `<log dir>/outbox/`. [`drain`] is the only way out, and it hands the
//! transport only what the consent record allows.
use super::consent::Consent;
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Automatic crash sends (crash reports "Always send").
    Crash,
    /// The daily error metrics aggregate.
    Metrics,
}

impl Kind {
    pub const ALL: [Self; 2] = [Self::Crash, Self::Metrics];

    fn dir_name(self) -> &'static str {
        match self {
            Self::Crash => "crash",
            Self::Metrics => "metrics",
        }
    }
}

/// Where queued sends of `kind` wait.
pub fn queue_dir(log_dir: &Path, kind: Kind) -> PathBuf {
    log_dir.join("outbox").join(kind.dir_name())
}

/// What actually sends; the HTTP client in the app, a recorder in tests.
pub trait Transport {
    fn send(&mut self, kind: Kind, body: &[u8]) -> Result<(), String>;
}

/// The queued files of `kind`, oldest name first.
fn queued(log_dir: &Path, kind: Kind) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(queue_dir(log_dir, kind))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .collect();
    files.sort();
    files
}

/// Deletes everything queued of `kinds`. A missing queue is already empty.
pub fn clear(log_dir: &Path, kinds: &[Kind]) -> io::Result<usize> {
    let mut removed = 0;
    for kind in kinds {
        for file in queued(log_dir, *kind) {
            match std::fs::remove_file(&file) {
                Ok(()) => removed += 1,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(removed)
}

/// [`clear`] for consent changes: a queue that cannot be emptied is logged,
/// since nothing else would notice.
pub fn revoke(log_dir: &Path, kinds: &[Kind]) {
    if let Err(error) = clear(log_dir, kinds) {
        tracing::warn!(%error, ?kinds, "could not empty the diagnostics send queue");
    }
}

/// Sends what `consent` allows, deleting each file once it is sent. A
/// failed send stays queued. Returns how many were sent.
pub fn drain(log_dir: &Path, consent: &Consent, transport: &mut dyn Transport) -> usize {
    let mut sent = 0;
    for kind in Kind::ALL.into_iter().filter(|kind| consent.sends(*kind)) {
        for file in queued(log_dir, kind) {
            let Ok(body) = std::fs::read(&file) else {
                continue;
            };
            if transport.send(kind, &body).is_ok() {
                let _ = std::fs::remove_file(&file);
                sent += 1;
            }
        }
    }
    sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::consent::{Answer, ConsentState, CrashReports};

    /// Stands in for the HTTP client: records every call it gets.
    #[derive(Default)]
    struct Recorder {
        calls: Vec<(Kind, Vec<u8>)>,
        fail: bool,
    }

    impl Transport for Recorder {
        fn send(&mut self, kind: Kind, body: &[u8]) -> Result<(), String> {
            self.calls.push((kind, body.to_vec()));
            if self.fail {
                Err("offline".into())
            } else {
                Ok(())
            }
        }
    }

    fn queue(log_dir: &Path, kind: Kind, name: &str) -> PathBuf {
        let dir = queue_dir(log_dir, kind);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, name).unwrap();
        path
    }

    /// Queued sends of both kinds and an unsent crash record.
    fn full(log_dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let crash = queue(log_dir, Kind::Crash, "a.json");
        let metrics = queue(log_dir, Kind::Metrics, "2026-10-24.json");
        let record = crate::diagnostics::crash_dir(log_dir).join("2026-10-24T09-00-00.000Z.json");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, "{}").unwrap();
        (crash, metrics, record)
    }

    #[test]
    fn default_consent_makes_no_calls() {
        let dir = tempfile::tempdir().unwrap();
        let (crash, metrics, record) = full(dir.path());
        let mut http = Recorder::default();
        assert_eq!(
            drain(dir.path(), &ConsentState::default().current(), &mut http),
            0
        );
        assert!(http.calls.is_empty());
        assert!(crash.exists() && metrics.exists() && record.exists());
    }

    #[test]
    fn first_run_answers_other_than_always_make_no_calls() {
        let dir = tempfile::tempdir().unwrap();
        full(dir.path());
        for answer in [Answer::Ask, Answer::Never, Answer::Dismissed] {
            let mut state = ConsentState::default();
            state.answer(answer, "2026-10-25T09:00:00.000Z");
            let mut http = Recorder::default();
            assert_eq!(drain(dir.path(), &state.current(), &mut http), 0);
            assert!(http.calls.is_empty(), "{answer:?}");
        }
    }

    #[test]
    fn always_sends_only_the_crash_queue() {
        let dir = tempfile::tempdir().unwrap();
        let (crash, metrics, record) = full(dir.path());
        let mut consent = ConsentState::default().current();
        consent.crash_reports = CrashReports::Always;
        let mut http = Recorder::default();
        assert_eq!(drain(dir.path(), &consent, &mut http), 1);
        assert_eq!(http.calls, vec![(Kind::Crash, b"a.json".to_vec())]);
        assert!(!crash.exists(), "sent, so no longer queued");
        assert!(metrics.exists());
        // Pending crash records wait for the crash notice; they are not the queue.
        assert!(record.exists());
    }

    #[test]
    fn metrics_on_sends_only_the_metrics_queue() {
        let dir = tempfile::tempdir().unwrap();
        let (crash, metrics, _) = full(dir.path());
        let mut consent = ConsentState::default().current();
        consent.metrics = true;
        let mut http = Recorder::default();
        assert_eq!(drain(dir.path(), &consent, &mut http), 1);
        assert_eq!(http.calls[0].0, Kind::Metrics);
        assert!(crash.exists() && !metrics.exists());
    }

    #[test]
    fn failed_send_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let crash = queue(dir.path(), Kind::Crash, "a.json");
        let mut consent = ConsentState::default().current();
        consent.crash_reports = CrashReports::Always;
        let mut http = Recorder {
            fail: true,
            ..Default::default()
        };
        assert_eq!(drain(dir.path(), &consent, &mut http), 0);
        assert_eq!(http.calls.len(), 1);
        assert!(crash.exists());
    }

    #[test]
    fn clear_empties_only_named_queues() {
        let dir = tempfile::tempdir().unwrap();
        let (crash, metrics, record) = full(dir.path());
        queue(dir.path(), Kind::Crash, "b.json");
        assert_eq!(clear(dir.path(), &[Kind::Crash]).unwrap(), 2);
        assert!(!crash.exists());
        assert!(metrics.exists() && record.exists());
        assert_eq!(clear(dir.path(), &Kind::ALL).unwrap(), 1);
        assert!(!metrics.exists());
    }

    #[cfg(unix)]
    #[test]
    fn revoke_logs_a_queue_it_cannot_empty() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let queued = queue(dir.path(), Kind::Crash, "a.json");
        let folder = queue_dir(dir.path(), Kind::Crash);
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        let lines = crate::diagnostics::test_capture(|| revoke(dir.path(), &[Kind::Crash]));
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(queued.exists());
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["level"], "warn");
        assert_eq!(
            lines[0]["message"],
            "could not empty the diagnostics send queue"
        );
    }

    #[test]
    fn clear_missing_outbox_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(clear(&dir.path().join("absent"), &Kind::ALL).unwrap(), 0);
    }

    #[test]
    fn drain_skips_non_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(queue_dir(dir.path(), Kind::Crash).join("nested")).unwrap();
        let mut consent = ConsentState::default().current();
        consent.crash_reports = CrashReports::Always;
        let mut http = Recorder::default();
        assert_eq!(drain(dir.path(), &consent, &mut http), 0);
        assert!(http.calls.is_empty());
    }

    #[test]
    fn queues_live_under_the_log_folder() {
        let log_dir = Path::new("logs");
        assert_eq!(
            queue_dir(log_dir, Kind::Crash),
            log_dir.join("outbox").join("crash")
        );
        assert_eq!(
            queue_dir(log_dir, Kind::Metrics),
            log_dir.join("outbox").join("metrics")
        );
    }
}
