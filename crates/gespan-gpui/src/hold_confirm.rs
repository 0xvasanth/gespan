//! Hold-to-confirm: press and hold for [`HOLD`] to confirm, release early to
//! drain back. The model is pure so the status-bar control and the held
//! ⌘⇧H shortcut drive the same press, and tests can step its clock.
use std::time::{Duration, Instant};

pub(crate) const HOLD: Duration = Duration::from_millis(1200);
/// A release sooner than this is a tap: show the hint instead.
pub(crate) const TAP: Duration = Duration::from_millis(250);
pub(crate) const HINT: Duration = Duration::from_millis(1800);
/// Click-only input (keyboard Enter, VoiceOver) confirms on a second press
/// within this window.
pub(crate) const ARMED: Duration = Duration::from_millis(4000);
const DRAIN_MIN_MS: f32 = 180.;
const DRAIN_FULL_MS: f32 = 500.;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fill {
    Idle,
    Filling { base: f32, at: Instant },
    Draining { from: f32, at: Instant, ms: f32 },
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Released {
    Tap,
    Drain,
    /// Released after the fill completed but before a tick saw it.
    Confirmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Confirmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Label {
    Rest,
    Hint,
    Armed,
    Done,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Press {
    fill: Fill,
    pressed_at: Option<Instant>,
    hint_until: Option<Instant>,
    armed_until: Option<Instant>,
}

impl Default for Press {
    fn default() -> Self {
        Self {
            fill: Fill::Idle,
            pressed_at: None,
            hint_until: None,
            armed_until: None,
        }
    }
}

impl Press {
    /// A pointer or key went down. Resumes from whatever a drain left;
    /// further starts while filling (key auto-repeat) are ignored.
    pub(crate) fn start(&mut self, now: Instant) {
        if matches!(self.fill, Fill::Filling { .. } | Fill::Done) {
            return;
        }
        let base = self.progress(now);
        self.fill = Fill::Filling { base, at: now };
        self.pressed_at = Some(now);
        self.hint_until = None;
        self.armed_until = None;
    }

    /// The press ended. A short press shows the hint.
    pub(crate) fn release(&mut self, now: Instant) -> Released {
        if !matches!(self.fill, Fill::Filling { .. }) {
            return Released::Drain;
        }
        if self.tick(now).is_some() {
            return Released::Confirmed;
        }
        let tap = self
            .pressed_at
            .is_some_and(|at| now.saturating_duration_since(at) < TAP);
        self.drain(now);
        if tap {
            self.hint_until = Some(now + HINT);
            return Released::Tap;
        }
        Released::Drain
    }

    /// The press was abandoned (window blur, pointer left, a modifier
    /// released): drain without the hint and never confirm.
    pub(crate) fn cancel(&mut self, now: Instant) {
        if matches!(self.fill, Fill::Filling { .. }) {
            self.drain(now);
        }
    }

    fn drain(&mut self, now: Instant) {
        let from = self.progress(now);
        self.pressed_at = None;
        self.fill = if from > 0. {
            let ms = (from * DRAIN_FULL_MS).max(DRAIN_MIN_MS);
            Fill::Draining { from, at: now, ms }
        } else {
            Fill::Idle
        };
    }

    /// A click with no press behind it. The first arms; a second within
    /// [`ARMED`] confirms.
    pub(crate) fn click(&mut self, now: Instant) -> bool {
        if self.fill == Fill::Done {
            return false;
        }
        if self.armed_until.is_some_and(|until| now < until) {
            self.fill = Fill::Done;
            self.armed_until = None;
            return true;
        }
        self.armed_until = Some(now + ARMED);
        self.hint_until = None;
        false
    }

    /// Shows "Hold to confirm" without pressing, for entry points that
    /// cannot be held (the palette, the menu bar).
    pub(crate) fn hint(&mut self, now: Instant) {
        if self.fill != Fill::Done {
            self.hint_until = Some(now + HINT);
        }
    }

    pub(crate) fn progress(&self, now: Instant) -> f32 {
        match self.fill {
            Fill::Idle => 0.,
            Fill::Done => 1.,
            Fill::Filling { base, at } => {
                let elapsed = now.saturating_duration_since(at).as_secs_f32();
                (base + elapsed / HOLD.as_secs_f32()).min(1.)
            }
            Fill::Draining { from, at, ms } => {
                let t = (now.saturating_duration_since(at).as_secs_f32() * 1000. / ms).min(1.);
                let eased = 1. - (1. - t).powi(3);
                from * (1. - eased)
            }
        }
    }

    pub(crate) fn tick(&mut self, now: Instant) -> Option<Event> {
        match self.fill {
            Fill::Filling { .. } if self.progress(now) >= 1. => {
                self.fill = Fill::Done;
                self.pressed_at = None;
                Some(Event::Confirmed)
            }
            Fill::Draining { .. } if self.progress(now) <= 0. => {
                self.fill = Fill::Idle;
                None
            }
            _ => None,
        }
    }

    pub(crate) fn label(&self, now: Instant) -> Label {
        if self.fill == Fill::Done {
            Label::Done
        } else if self.armed_until.is_some_and(|until| now < until) {
            Label::Armed
        } else if self.hint_until.is_some_and(|until| now < until) {
            Label::Hint
        } else {
            Label::Rest
        }
    }

    pub(crate) fn pressing(&self) -> bool {
        matches!(self.fill, Fill::Filling { .. })
    }

    /// Whether the control must keep repainting: a fill moving, or a
    /// transient label waiting to expire.
    pub(crate) fn animating(&self, now: Instant) -> bool {
        let moving = match self.fill {
            Fill::Filling { .. } => true,
            Fill::Draining { .. } => self.progress(now) > 0.,
            _ => false,
        };
        moving || matches!(self.label(now), Label::Hint | Label::Armed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn fills_linearly_and_confirms_exactly_at_the_hold_duration() {
        let t = Instant::now();
        let mut press = Press::default();
        press.start(t);
        assert!((press.progress(t + ms(600)) - 0.5).abs() < 1e-3);
        assert_eq!(press.tick(t + ms(1199)), None);
        assert_eq!(press.tick(t + HOLD), Some(Event::Confirmed));
        assert_eq!(press.label(t + HOLD), Label::Done);
        assert_eq!(press.progress(t + HOLD + ms(500)), 1.);
    }

    #[test]
    fn confirms_only_once() {
        let t = Instant::now();
        let mut press = Press::default();
        press.start(t);
        assert_eq!(press.tick(t + HOLD), Some(Event::Confirmed));
        assert_eq!(press.tick(t + HOLD + ms(10)), None);
        press.start(t + HOLD + ms(20));
        assert_eq!(press.tick(t + HOLD * 3), None);
        assert!(!press.click(t + HOLD * 3));
        assert!(!press.click(t + HOLD * 3 + ms(1)));
    }

    #[test]
    fn repeated_starts_while_filling_are_ignored() {
        let t = Instant::now();
        let mut press = Press::default();
        press.start(t);
        // Key auto-repeat sends more key-downs; the fill keeps its own clock.
        press.start(t + ms(300));
        press.start(t + ms(600));
        assert!((press.progress(t + ms(600)) - 0.5).abs() < 1e-3);
        assert_eq!(press.tick(t + ms(1100)), None);
        assert_eq!(press.tick(t + HOLD), Some(Event::Confirmed));
    }

    #[test]
    fn early_release_drains_in_proportion_to_progress() {
        let t = Instant::now();
        let mut press = Press::default();
        press.start(t);
        assert_eq!(press.release(t + ms(600)), Released::Drain);
        // 0.5 progress drains in max(180, 250) = 250 ms, eased out.
        let mid = press.progress(t + ms(725));
        assert!(mid > 0. && mid < 0.25, "ease-out passes half early: {mid}");
        assert_eq!(press.progress(t + ms(850)), 0.);
        assert!(!press.animating(t + ms(851)));
        assert_eq!(press.tick(t + ms(5000)), None);
    }

    #[test]
    fn a_small_fill_drains_in_at_least_180ms() {
        let t = Instant::now();
        let mut press = Press::default();
        press.start(t);
        press.cancel(t + ms(300)); // 0.25 → 125 ms, floored at 180 ms
        assert!(press.progress(t + ms(300 + 170)) > 0.);
        assert_eq!(press.progress(t + ms(300 + 180)), 0.);
    }

    #[test]
    fn pressing_again_resumes_from_the_remaining_progress() {
        let t = Instant::now();
        let mut press = Press::default();
        press.start(t);
        press.release(t + ms(900)); // 0.75, drains over 375 ms
        let left = press.progress(t + ms(950));
        assert!(left > 0.4 && left < 0.75, "ease-out: {left}");
        press.start(t + ms(950));
        assert!((press.progress(t + ms(950)) - left).abs() < 1e-4);
        let remaining = ((1. - left) * HOLD.as_millis() as f32) as u64;
        assert_eq!(press.tick(t + ms(950 + remaining - 5)), None);
        assert_eq!(
            press.tick(t + ms(950 + remaining + 1)),
            Some(Event::Confirmed)
        );
    }

    #[test]
    fn a_tap_shows_the_hint_for_its_duration() {
        let t = Instant::now();
        let mut press = Press::default();
        press.start(t);
        assert_eq!(press.release(t + ms(100)), Released::Tap);
        assert_eq!(press.label(t + ms(100)), Label::Hint);
        assert_eq!(press.label(t + ms(100) + HINT - ms(1)), Label::Hint);
        assert!(press.animating(t + ms(100) + HINT - ms(1)));
        assert_eq!(press.label(t + ms(100) + HINT), Label::Rest);
        // Cancelling (blur, modifier released) never shows the hint.
        let mut press = Press::default();
        press.start(t);
        press.cancel(t + ms(100));
        assert_eq!(press.label(t + ms(100)), Label::Rest);
    }

    #[test]
    fn a_release_past_the_full_fill_confirms_when_no_tick_saw_it() {
        let t = Instant::now();
        let mut press = Press::default();
        press.start(t);
        assert_eq!(press.release(t + HOLD + ms(5)), Released::Confirmed);
        assert_eq!(press.tick(t + HOLD + ms(6)), None);
        assert_eq!(press.label(t + HOLD + ms(6)), Label::Done);
    }

    #[test]
    fn click_arms_then_confirms_within_the_window() {
        let t = Instant::now();
        let mut press = Press::default();
        assert!(!press.click(t));
        assert_eq!(press.label(t), Label::Armed);
        assert!(press.click(t + ARMED - ms(1)));
        assert_eq!(press.label(t + ARMED), Label::Done);
    }

    #[test]
    fn an_armed_click_expires() {
        let t = Instant::now();
        let mut press = Press::default();
        assert!(!press.click(t));
        assert_eq!(press.label(t + ARMED), Label::Rest);
        // The late click only arms again.
        assert!(!press.click(t + ARMED));
        assert_eq!(press.label(t + ARMED), Label::Armed);
    }

    #[test]
    fn hint_shows_without_starting_a_press() {
        let t = Instant::now();
        let mut press = Press::default();
        press.hint(t);
        assert_eq!(press.label(t), Label::Hint);
        assert_eq!(press.progress(t), 0.);
        assert!(!press.pressing());
        assert_eq!(press.tick(t + HOLD), None);
    }
}
