//! Settings → Advanced → Keep screen awake. Each workspace window reports the
//! shared mode and whether its own agents are working; one assertion covers
//! every window, so a busy agent in any window keeps the screen unlocked.
use rebase_keep_awake::{Inhibitor, KeepAwake, Mode, SystemInhibitor};
use std::collections::HashSet;
use std::sync::Mutex;

pub(crate) struct KeepAwakeState(Mutex<Windows<SystemInhibitor>>);

impl Default for KeepAwakeState {
    fn default() -> Self {
        Self(Mutex::new(Windows::new(KeepAwake::new())))
    }
}

struct Windows<I: Inhibitor> {
    mode: Mode,
    running: HashSet<String>,
    awake: KeepAwake<I>,
}

impl<I: Inhibitor> Windows<I> {
    fn new(awake: KeepAwake<I>) -> Self {
        Self {
            mode: Mode::Off,
            running: HashSet::new(),
            awake,
        }
    }

    fn report(&mut self, label: &str, mode: Mode, running: bool) {
        self.mode = mode;
        if running {
            self.running.insert(label.to_owned());
        } else {
            self.running.remove(label);
        }
        self.apply();
    }

    fn closed(&mut self, label: &str) {
        if self.running.remove(label) {
            self.apply();
        }
    }

    fn apply(&mut self) {
        if let Err(error) = self.awake.update(self.mode, !self.running.is_empty()) {
            eprintln!("[rebase] keep awake: {error}");
        }
    }
}

#[tauri::command]
pub(crate) fn set_keep_awake(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, KeepAwakeState>,
    mode: String,
    running: bool,
) {
    let mode = Mode::parse(&mode).unwrap_or_default();
    if let Ok(mut windows) = state.0.lock() {
        windows.report(window.label(), mode, running);
    }
}

pub(crate) fn window_closed(handle: &tauri::AppHandle, label: &str) {
    use tauri::Manager;
    if let Some(state) = handle.try_state::<KeepAwakeState>() {
        if let Ok(mut windows) = state.0.lock() {
            windows.closed(label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    #[derive(Clone, Default)]
    struct Fake(Arc<StdMutex<(usize, usize)>>);
    impl Inhibitor for Fake {
        fn acquire(&mut self) -> Result<(), String> {
            self.0.lock().unwrap().0 += 1;
            Ok(())
        }
        fn release(&mut self) {
            self.0.lock().unwrap().1 += 1;
        }
    }

    fn windows() -> (Windows<Fake>, Fake) {
        let fake = Fake::default();
        (Windows::new(KeepAwake::with(fake.clone())), fake)
    }

    fn counts(fake: &Fake) -> (usize, usize) {
        *fake.0.lock().unwrap()
    }

    #[test]
    fn stays_awake_until_the_last_busy_window_goes_idle() {
        let (mut state, fake) = windows();
        state.report("main", Mode::AgentsRunning, true);
        state.report("second", Mode::AgentsRunning, true);
        state.report("main", Mode::AgentsRunning, false);
        assert!(state.awake.is_held());
        assert_eq!(counts(&fake), (1, 0));
        state.report("second", Mode::AgentsRunning, false);
        assert!(!state.awake.is_held());
        assert_eq!(counts(&fake), (1, 1));
    }

    #[test]
    fn closing_a_window_with_a_running_agent_releases_its_hold() {
        let (mut state, fake) = windows();
        state.report("second", Mode::AgentsRunning, true);
        state.closed("second");
        assert!(!state.awake.is_held());
        assert_eq!(counts(&fake), (1, 1));
        // Closing an idle window is not a transition.
        state.closed("main");
        assert_eq!(counts(&fake), (1, 1));
    }

    #[test]
    fn the_latest_reported_mode_applies_to_every_window() {
        let (mut state, fake) = windows();
        state.report("main", Mode::AppOpen, false);
        assert!(state.awake.is_held());
        // Switching to Off from another window drops the hold even with agents busy.
        state.report("second", Mode::Off, true);
        assert!(!state.awake.is_held());
        state.report("main", Mode::AgentsRunning, false);
        assert!(
            state.awake.is_held(),
            "second window still has a running agent"
        );
        assert_eq!(counts(&fake), (2, 1));
    }
}
