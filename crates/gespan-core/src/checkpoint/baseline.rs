//! A turn's "before" snapshot, recorded while its provider starts.
//!
//! Recording the workspace takes a dozen `git` processes and several fsyncs.
//! Taking it before the provider launches put all of that in front of the
//! first token. Nothing can edit the workspace on the turn's behalf until
//! the provider has its prompt, so the provider starts at once and only the
//! prompt waits for the recording (`provider` calls [`Baseline::wait`]).
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// How often a wait looks at its cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(10);

/// A recording that runs on its own thread. Clones share it.
#[derive(Clone)]
pub struct Baseline {
    done: Arc<(Mutex<bool>, Condvar)>,
}

impl std::fmt::Debug for Baseline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Baseline")
            .field("recorded", &self.recorded())
            .finish()
    }
}

/// Marks the recording finished when dropped, so a panic in it still
/// releases the prompt and the turn's end.
struct Finished(Arc<(Mutex<bool>, Condvar)>);
impl Drop for Finished {
    fn drop(&mut self) {
        let (done, changed) = &*self.0;
        *done.lock().unwrap_or_else(|e| e.into_inner()) = true;
        changed.notify_all();
    }
}

impl Baseline {
    /// Runs `record` on a thread of its own and returns at once. If no
    /// thread can be started, `record` runs here before returning.
    pub fn record(record: impl FnOnce() + Send + 'static) -> Self {
        let done = Arc::new((Mutex::new(false), Condvar::new()));
        let finished = Finished(done.clone());
        let work = move || {
            let _finished = finished;
            record();
        };
        // `spawn` hands the closure back only by dropping it, so keep a slot
        // the fallback can still take it from.
        let slot = Arc::new(Mutex::new(Some(work)));
        let thread_slot = slot.clone();
        let spawned = std::thread::Builder::new()
            .name("gespan-checkpoint".into())
            .spawn(move || {
                let work = thread_slot.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let Some(work) = work {
                    work();
                }
            });
        if spawned.is_err() {
            let work = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(work) = work {
                work();
            }
        }
        Self { done }
    }

    /// A baseline with nothing to record.
    pub fn ready() -> Self {
        Self {
            done: Arc::new((Mutex::new(true), Condvar::new())),
        }
    }

    pub fn recorded(&self) -> bool {
        *self.done.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Blocks until the recording has finished: `true`. Returns `false` as
    /// soon as `cancel` is set instead; the recording itself carries on.
    pub fn wait(&self, cancel: &AtomicBool) -> bool {
        let (done, changed) = &*self.done;
        let mut recorded = done.lock().unwrap_or_else(|e| e.into_inner());
        while !*recorded {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            recorded = changed
                .wait_timeout(recorded, CANCEL_POLL)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
    }

    /// Blocks until the recording has finished, even after a Stop: the
    /// turn's closing snapshot must come after its opening one.
    pub fn finish(&self) {
        let (done, changed) = &*self.done;
        let mut recorded = done.lock().unwrap_or_else(|e| e.into_inner());
        while !*recorded {
            recorded = changed.wait(recorded).unwrap_or_else(|e| e.into_inner());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    /// A recording held open until the test sends on the returned channel.
    fn held() -> (Baseline, mpsc::Sender<()>, mpsc::Receiver<()>) {
        let (release, released) = mpsc::channel::<()>();
        let (finished_tx, finished) = mpsc::channel();
        let baseline = Baseline::record(move || {
            let _ = released.recv();
            let _ = finished_tx.send(());
        });
        (baseline, release, finished)
    }

    #[test]
    fn record_returns_before_the_recording_finishes() {
        let (baseline, release, finished) = held();
        // The caller got control back while the recording is still blocked.
        assert!(!baseline.recorded());
        assert!(finished.try_recv().is_err());
        release.send(()).unwrap();
        baseline.finish();
        assert!(baseline.recorded());
        finished.recv().unwrap();
    }

    #[test]
    fn wait_returns_only_after_the_recording_finished() {
        let (baseline, release, finished) = held();
        let waiter = {
            let baseline = baseline.clone();
            std::thread::spawn(move || baseline.wait(&AtomicBool::new(false)))
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !waiter.is_finished(),
            "the prompt went out before the baseline"
        );
        release.send(()).unwrap();
        assert!(waiter.join().unwrap());
        // The recording's own work completed before the waiter was released.
        finished.try_recv().unwrap();
    }

    #[test]
    fn stop_releases_a_wait_but_not_the_closing_snapshot() {
        let (baseline, release, _finished) = held();
        let cancel = Arc::new(AtomicBool::new(false));
        let waiter = {
            let (baseline, cancel) = (baseline.clone(), cancel.clone());
            std::thread::spawn(move || baseline.wait(&cancel))
        };
        cancel.store(true, Ordering::Relaxed);
        let stopped = Instant::now();
        assert!(!waiter.join().unwrap(), "a stopped turn reports the stop");
        assert!(stopped.elapsed() < Duration::from_secs(2));
        // The turn's end still waits for the opening snapshot.
        let closing = {
            let baseline = baseline.clone();
            std::thread::spawn(move || baseline.finish())
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(!closing.is_finished());
        release.send(()).unwrap();
        closing.join().unwrap();
    }

    #[test]
    fn a_recording_that_panics_still_releases_the_turn() {
        let baseline = Baseline::record(|| panic!("git crashed"));
        assert!(baseline.wait(&AtomicBool::new(false)));
        baseline.finish();
    }

    #[test]
    fn a_ready_baseline_never_blocks() {
        let baseline = Baseline::ready();
        assert!(baseline.wait(&AtomicBool::new(true)));
        baseline.finish();
    }
}
