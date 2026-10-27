//! One session's part in Hold: stop at a safe point, queue while held, resume.
use super::*;
use gespan_core::hold::Reason;
use gespan_core::provider::HoldPoint;

/// A session reached its safe point (or was idle) after Hold.
#[derive(Clone, Debug)]
pub(crate) struct HoldChanged {
    pub session: i64,
    pub reason: Reason,
    pub last_tool: Option<String>,
    pub background: Vec<String>,
}

impl EventEmitter<HoldChanged> for Conversation {}

impl Conversation {
    /// Asks a running turn to stop at its next safe point; an idle session is held at once.
    pub(crate) fn hold(&mut self, cx: &mut Context<Self>) {
        if self.held.is_some() || self.hold_requested {
            return;
        }
        if self.busy
            && let Some(controls) = &self.controls
            && controls
                .send(Control {
                    id: String::new(),
                    decision: Decision::Hold,
                })
                .is_ok()
        {
            self.hold_requested = true;
            self.status = "Holding: finishing the current step…".into();
            cx.notify();
            return;
        }
        self.settle_hold(Reason::Idle, cx);
    }
    /// Held with something to continue: interrupted, waiting or restarted.
    pub(crate) fn hold_notice_shown(&self, cx: &gpui::App) -> bool {
        let Some(state) = cx
            .try_global::<crate::hold::Hold>()
            .and_then(|h| h.0.as_ref())
        else {
            return false;
        };
        let reason = state
            .sessions
            .iter()
            .find(|s| s.session == self.summary.id)
            .and_then(|held| held.reason)
            .or(self.held);
        reason.is_some_and(|reason| reason != Reason::Idle)
    }
    /// A thin line above the composer while this session is held.
    pub(super) fn hold_notice(&self, cx: &gpui::App) -> Option<AnyElement> {
        self.hold_notice_shown(cx).then(|| {
            div()
                .debug_selector(|| "hold-notice".into())
                .mb(px(6.))
                .px(px(12.))
                .py(px(6.))
                .rounded(px(10.))
                .bg(content(0.05))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .text_color(content(0.6))
                .child(icon("queue-pause", 13.))
                .child("Held · continues on Resume")
                .into_any_element()
        })
    }
    /// The provider loop reported where the held turn came to rest.
    pub(crate) fn hold_point(&mut self, point: HoldPoint, cx: &mut Context<Self>) {
        // After Resume the turn was released; a late report means nothing.
        if !crate::hold::active(cx) {
            return;
        }
        let reason = match point {
            // Interrupted settles once the turn has drained, so Resume can
            // never start a second turn over the one still ending.
            HoldPoint::Interrupted => {
                self.hold_interrupted = true;
                return;
            }
            HoldPoint::AwaitingInput => Reason::AwaitingInput,
            HoldPoint::Background => Reason::Background,
        };
        self.settle_hold(reason, cx);
    }
    /// The held turn ended: interrupted at its safe point, or on its own.
    pub(crate) fn hold_turn_finished(&mut self, cx: &mut Context<Self>) {
        let interrupted = std::mem::take(&mut self.hold_interrupted);
        if self.hold_requested && self.held.is_none() && crate::hold::active(cx) {
            let reason = if interrupted {
                Reason::Interrupted
            } else {
                Reason::Idle
            };
            self.settle_hold(reason, cx);
        }
    }
    /// Stop now: the user chose to end the command still running.
    pub(crate) fn stop_now(&mut self, cx: &mut Context<Self>) {
        self.stop(cx);
        self.settle_hold(Reason::Interrupted, cx);
    }
    fn settle_hold(&mut self, reason: Reason, cx: &mut Context<Self>) {
        self.hold_requested = false;
        self.held = Some(reason);
        let last_tool = self.transcript.read(cx).live_turn(&self.summary.cwd).0;
        let background = self
            .background
            .tasks
            .iter()
            .map(|task| task.description.clone())
            .collect();
        cx.emit(HoldChanged {
            session: self.summary.id,
            reason,
            last_tool,
            background,
        });
        cx.emit(SessionChanged);
        cx.notify();
    }
    /// Queued messages sitting idle: nothing is sending them.
    #[cfg(test)]
    pub(crate) fn held_queue_waiting(&self) -> bool {
        !self.follow_ups.state.messages.is_empty()
            && !self.busy
            && self.follow_ups.sending.is_none()
    }
    pub(crate) fn queued_count(&self) -> usize {
        self.follow_ups.state.messages.len()
    }
    #[cfg(test)]
    pub(crate) fn queue_for_test(&mut self, text: &str, cx: &mut Context<Self>) {
        self.loaded = true;
        self.enqueue_follow_up(text.into(), None, cx);
    }
    pub(crate) fn resume_prompt(&self, reason: Reason) -> Option<String> {
        gespan_core::hold::continuation(reason).map(str::to_owned)
    }
    /// Continues a held session: its continuation first, then its queue.
    /// Returns whether anything was sent.
    pub(crate) fn resume_from_hold(&mut self, reason: Reason, cx: &mut Context<Self>) -> bool {
        self.held = None;
        self.hold_requested = false;
        self.hold_interrupted = false;
        // A turn left waiting (an approval, background work) goes on.
        if self.busy
            && let Some(controls) = &self.controls
        {
            let _ = controls.send(Control {
                id: String::new(),
                decision: Decision::Release,
            });
        }
        let sent = if let Some(prompt) = self.resume_prompt(reason) {
            if self.follow_ups.state.messages.is_empty() {
                self.launch_prompt(prompt, cx);
            } else {
                // The paused-queue path sends the continuation, then releases the queue.
                self.follow_ups.state.paused = true;
                self.resume_queue(cx);
            }
            true
        } else {
            self.follow_ups.state.paused = false;
            self.pump_follow_ups(cx);
            !self.follow_ups.state.messages.is_empty()
        };
        cx.emit(SessionChanged);
        cx.notify();
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::hold::{HoldState, Reason};
    use gespan_core::provider::HoldPoint;
    use gespan_core::store::Store;
    use gpui::{TestAppContext, WindowHandle};
    use std::sync::Mutex;

    fn conversation(cx: &mut TestAppContext) -> WindowHandle<Conversation> {
        let store = Store::open(Path::new(":memory:")).unwrap();
        // Fails after saving the turn, without ever launching a real provider.
        let mut summary = store.create("codex", "/missing/hold-project").unwrap();
        summary.provider = "fixture".into();
        cx.update(|cx| {
            input::bind(cx);
            cx.set_global(crate::settings::Preferences {
                follow_up: "steer".into(),
                ..Default::default()
            });
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                    view.loaded = true;
                    view.summary.has_messages = true;
                    view
                })
            })
            .unwrap()
        })
    }
    fn hold_app(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(crate::hold::Hold(Some(HoldState::new(1)))));
    }
    fn changes(
        window: WindowHandle<Conversation>,
        cx: &mut TestAppContext,
    ) -> Arc<Mutex<Vec<HoldChanged>>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let view = window.root(cx).unwrap();
        cx.update(|cx| {
            cx.subscribe(&view, move |_, event: &HoldChanged, _| {
                sink.lock().unwrap().push(event.clone());
            })
            .detach()
        });
        seen
    }

    #[gpui::test]
    fn submit_while_held_queues_without_steer_and_does_not_start(cx: &mut TestAppContext) {
        let window = conversation(cx);
        hold_app(cx);
        cx.run_until_parked();
        window
            .update(cx, |view, _, cx| {
                view.input
                    .update(cx, |i, cx| i.set_text("after lunch".into(), cx));
                view.send(cx);
                assert!(!view.busy, "a held session must not start a turn");
                let messages = &view.follow_ups.state.messages;
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].text, "after lunch");
                assert!(!messages[0].steer);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, _, _| {
                let saved = locked(&view.store)
                    .unwrap()
                    .follow_ups(view.summary.id)
                    .unwrap();
                assert_eq!(saved.messages.len(), 1);
            })
            .unwrap();
    }

    #[gpui::test]
    fn messages_queued_into_a_settling_turn_are_never_steered(cx: &mut TestAppContext) {
        let window = conversation(cx);
        hold_app(cx);
        window
            .update(cx, |view, _, cx| {
                view.busy = true;
                view.enqueue_follow_up("wait for me".into(), None, cx);
                assert!(!view.follow_ups.state.messages[0].steer);
            })
            .unwrap();
    }

    #[gpui::test]
    fn held_busy_turn_sends_hold_decision_not_cancel(cx: &mut TestAppContext) {
        let window = conversation(cx);
        let (tx, rx) = std::sync::mpsc::channel();
        window
            .update(cx, |view, _, cx| {
                view.busy = true;
                view.controls = Some(tx);
                view.hold(cx);
                assert!(!view.cancel.load(Ordering::Relaxed));
                assert!(view.busy, "Hold waits for the safe point");
                assert_eq!(view.held, None);
            })
            .unwrap();
        assert!(matches!(rx.try_recv().unwrap().decision, Decision::Hold));
    }

    #[gpui::test]
    fn idle_session_is_held_at_once(cx: &mut TestAppContext) {
        let window = conversation(cx);
        let seen = changes(window, cx);
        window.update(cx, |view, _, cx| view.hold(cx)).unwrap();
        cx.run_until_parked();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].reason, Reason::Idle);
    }

    #[gpui::test]
    fn safe_point_marks_session_held_and_reports_it(cx: &mut TestAppContext) {
        let window = conversation(cx);
        let seen = changes(window, cx);
        hold_app(cx);
        window
            .update(cx, |view, _, cx| {
                view.busy = true;
                view.hold_point(HoldPoint::Background, cx);
                assert_eq!(view.held, Some(Reason::Background));
            })
            .unwrap();
        cx.run_until_parked();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].reason, Reason::Background);
        assert_eq!(
            seen[0].session,
            window.root(cx).unwrap().read_with(cx, |v, _| v.summary.id)
        );
    }

    #[gpui::test]
    fn an_interrupt_settles_only_once_the_turn_has_ended(cx: &mut TestAppContext) {
        let window = conversation(cx);
        hold_app(cx);
        let (tx, rx) = std::sync::mpsc::channel();
        window
            .update(cx, |view, _, cx| {
                view.busy = true;
                view.controls = Some(tx);
                view.hold(cx);
                view.hold_point(HoldPoint::Interrupted, cx);
                assert_eq!(view.held, None, "the turn is still draining");
                view.busy = false;
                view.hold_turn_finished(cx);
                assert_eq!(view.held, Some(Reason::Interrupted));
            })
            .unwrap();
        drop(rx);
    }

    #[gpui::test]
    fn resume_releases_a_turn_left_waiting(cx: &mut TestAppContext) {
        let window = conversation(cx);
        let (tx, rx) = std::sync::mpsc::channel();
        window
            .update(cx, |view, _, cx| {
                view.busy = true;
                view.controls = Some(tx);
                view.held = Some(Reason::AwaitingInput);
                view.resume_from_hold(Reason::AwaitingInput, cx);
                assert_eq!(view.held, None);
            })
            .unwrap();
        assert!(matches!(rx.try_recv().unwrap().decision, Decision::Release));
    }

    #[gpui::test]
    fn a_safe_point_reported_after_resume_is_ignored(cx: &mut TestAppContext) {
        let window = conversation(cx);
        window
            .update(cx, |view, _, cx| {
                view.busy = true;
                view.hold_point(HoldPoint::Interrupted, cx);
                view.hold_point(HoldPoint::Background, cx);
                assert_eq!(view.held, None);
            })
            .unwrap();
    }

    #[gpui::test]
    fn resume_from_interrupted_sends_continuation_first(cx: &mut TestAppContext) {
        let window = conversation(cx);
        hold_app(cx);
        window
            .update(cx, |view, _, cx| {
                view.enqueue_follow_up("then this".into(), None, cx);
                view.held = Some(Reason::Interrupted);
            })
            .unwrap();
        cx.update(|cx| cx.set_global(crate::hold::Hold(None)));
        window
            .update(cx, |view, _, cx| {
                let first = view.resume_prompt(Reason::Interrupted);
                assert_eq!(first.as_deref(), Some("Continue from where you left off."));
                view.resume_from_hold(Reason::Interrupted, cx);
                assert_eq!(view.held, None);
                assert!(view.busy, "the continuation starts a turn");
                assert!(
                    view.follow_ups.resuming,
                    "queued messages wait for the continuation"
                );
                assert_eq!(view.follow_ups.state.messages[0].text, "then this");
            })
            .unwrap();
    }

    #[gpui::test]
    fn resume_from_idle_sends_only_the_queue(cx: &mut TestAppContext) {
        let window = conversation(cx);
        window
            .update(cx, |view, _, cx| {
                assert_eq!(view.resume_prompt(Reason::Idle), None);
                view.held = Some(Reason::Idle);
                view.resume_from_hold(Reason::Idle, cx);
                assert_eq!(view.held, None);
                assert!(!view.busy, "nothing queued, nothing to send");
                assert!(!view.follow_ups.resuming);
            })
            .unwrap();
    }

    #[test]
    fn a_held_turn_ends_like_a_stop_without_an_error_row() {
        let cancel = AtomicBool::new(false);
        assert!(crate::runtime::ended_by_user::<()>(
            &cancel,
            &Err("Held".into())
        ));
        assert!(crate::runtime::ended_by_user::<()>(
            &cancel,
            &Err("Stopped".into())
        ));
        assert!(!crate::runtime::ended_by_user::<()>(
            &cancel,
            &Err("boom".into())
        ));
    }
}
