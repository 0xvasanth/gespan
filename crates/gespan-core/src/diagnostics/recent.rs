//! The `event_id`s of the last few `error` lines, so an error toast's
//! "Report" can point the report at the lines behind it.
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};

/// How many recent error ids are kept.
const KEPT: usize = 32;

static RECENT: Mutex<VecDeque<(Instant, String)>> = Mutex::new(VecDeque::new());

/// Remembers an `error` line's `event_id`. The JSON layer calls this.
pub fn note_error_event(event_id: String) {
    let mut recent = RECENT.lock().unwrap_or_else(|e| e.into_inner());
    if recent.len() == KEPT {
        recent.pop_front();
    }
    recent.push_back((Instant::now(), event_id));
}

/// The ids of `error` lines logged in the last `within`, oldest first.
pub fn recent_error_events(within: Duration) -> Vec<String> {
    let now = Instant::now();
    RECENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|(at, _)| now.duration_since(*at) <= within)
        .map(|(_, id)| id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ring is process-wide: one test filling it must not evict the
    /// other's id.
    static RING: Mutex<()> = Mutex::new(());

    #[test]
    fn error_lines_are_remembered_for_reports() {
        let _ring = RING.lock().unwrap_or_else(|e| e.into_inner());
        let lines = crate::diagnostics::test_capture(|| {
            tracing::error!("disk full");
            tracing::info!("routine");
        });
        let id = |n: usize| lines[n]["event_id"].as_str().unwrap().to_owned();
        let recent = recent_error_events(Duration::from_secs(10));
        assert!(recent.contains(&id(0)), "{recent:?}");
        assert!(!recent.contains(&id(1)));
    }

    #[test]
    fn only_the_newest_ids_within_the_window_are_kept() {
        let _ring = RING.lock().unwrap_or_else(|e| e.into_inner());
        note_error_event("stale-id".into());
        RECENT.lock().unwrap().iter_mut().for_each(|(at, id)| {
            if id == "stale-id" {
                *at -= Duration::from_secs(60);
            }
        });
        let ids: Vec<String> = (0..KEPT + 5).map(|n| format!("kept-limit-{n}")).collect();
        for id in &ids {
            note_error_event(id.clone());
        }
        let recent = recent_error_events(Duration::from_secs(10));
        assert!(!recent.contains(&"stale-id".to_string()));
        assert!(!recent.contains(&ids[0]), "the oldest are dropped");
        assert!(recent.len() <= KEPT);
        assert!(recent.contains(&ids[KEPT + 4]), "the newest stays");
    }
}
