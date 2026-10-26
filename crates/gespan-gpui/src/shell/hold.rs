//! Hold everything: press and hold to confirm, stop every session at its
//! safe point, keep the hold across restarts, and resume sessions a few at a
//! time.
use super::*;
use crate::conversation::HoldChanged;
use crate::hold::Hold;
use crate::hold_confirm::{Label, Press, Released};
use gespan_core::hold::{HoldState, Phase, Reason, ResumeQueue};
use gpui::{ClickEvent, Div, Hsla, Stateful, relative};
use std::time::Duration;

/// At most this many held sessions start again at once.
const RESUME_LIMIT: usize = 3;
/// Wide enough for the longest label, so the button never resizes.
const PRESS_WIDTH: f32 = 172.;
const PRESS_FRAME: Duration = Duration::from_millis(16);
/// How often a waiting hint or armed label checks whether it expired.
const LABEL_FRAME: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(super) struct HoldCoordinator {
    subscriptions: Vec<Subscription>,
    resume: Option<ResumeQueue>,
    reasons: HashMap<i64, Reason>,
    pub(super) press: Press,
    /// ⌘⇧H is down: its auto-repeats are ignored until it is released.
    key_held: bool,
    ticking: bool,
    details_open: bool,
    /// Mirrors the global phase for code that has no `App` at hand.
    pub(super) on_hold: bool,
}

/// The Hold shortcut's modifiers are still down, so the press can be held.
fn shortcut_down(window: &Window) -> bool {
    let modifiers = window.modifiers();
    modifiers.secondary() && modifiers.shift
}

impl Shell {
    /// ⌘⇧H, the palette and the menus all land here. Held ⌘⇧H drives the
    /// press; anything that cannot be held only shows "Hold to confirm".
    pub(super) fn toggle_hold(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hold.key_held {
            return;
        }
        let key = shortcut_down(window);
        match crate::hold::phase(cx) {
            None if key => {
                self.hold.key_held = true;
                self.start_hold_press(cx);
            }
            None => self.arm_hold_hint(cx),
            // Hold can't be undone until every session has stopped.
            Some(Phase::Holding) => {}
            Some(Phase::OnHold) => {
                self.hold.key_held = key;
                self.resume_all(cx);
            }
        }
    }
    /// The palette and the tray can't be held, and the modifiers the window
    /// remembers may be stale: they show the hint, or resume once on hold.
    pub(super) fn hold_from_menu(&mut self, cx: &mut Context<Self>) {
        if self.hold.key_held {
            return;
        }
        match crate::hold::phase(cx) {
            None => self.arm_hold_hint(cx),
            Some(Phase::Holding) => {}
            Some(Phase::OnHold) => self.resume_all(cx),
        }
    }
    /// Shows "Hold to confirm" on the status-bar button.
    pub(super) fn arm_hold_hint(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        self.hold.press.hint(now);
        self.drive_hold_press(cx);
    }
    fn start_hold_press(&mut self, cx: &mut Context<Self>) {
        if crate::hold::active(cx) {
            return;
        }
        let now = cx.background_executor().now();
        self.hold.press.start(now);
        self.drive_hold_press(cx);
    }
    fn release_hold_press(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        if self.hold.press.release(now) == Released::Confirmed {
            self.hold_press_confirmed(cx);
        } else {
            self.drive_hold_press(cx);
        }
    }
    /// The pointer left, the window lost focus or a modifier was released.
    pub(super) fn cancel_hold_press(&mut self, cx: &mut Context<Self>) {
        self.hold.key_held = false;
        if self.hold.press.pressing() {
            let now = cx.background_executor().now();
            self.hold.press.cancel(now);
            self.drive_hold_press(cx);
        }
    }
    pub(super) fn hold_key_up(&mut self, event: &gpui::KeyUpEvent, cx: &mut Context<Self>) {
        if self.hold.key_held && event.keystroke.key.eq_ignore_ascii_case("h") {
            self.hold.key_held = false;
            if self.hold.press.pressing() {
                self.release_hold_press(cx);
            }
        }
    }
    pub(super) fn hold_modifiers_changed(
        &mut self,
        event: &gpui::ModifiersChangedEvent,
        cx: &mut Context<Self>,
    ) {
        // macOS sends no key-up for H while ⌘ is down, so a modifier going up
        // is what usually ends a held ⌘⇧H.
        if self.hold.key_held && !(event.modifiers.secondary() && event.modifiers.shift) {
            self.hold.key_held = false;
            if self.hold.press.pressing() {
                self.release_hold_press(cx);
            }
        }
    }
    /// Enter, Space or VoiceOver on the button: no press to hold, so the
    /// first activation arms and a second one confirms.
    pub(super) fn hold_button_keyboard_press(&mut self, cx: &mut Context<Self>) {
        if crate::hold::active(cx) {
            return;
        }
        let now = cx.background_executor().now();
        if self.hold.press.click(now) {
            self.hold_press_confirmed(cx);
        } else {
            self.drive_hold_press(cx);
        }
    }
    fn hold_press_confirmed(&mut self, cx: &mut Context<Self>) {
        self.hold.press = Press::default();
        self.hold_all(cx);
    }
    /// Repaints while the fill moves, and confirms when it completes.
    fn drive_hold_press(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        if self.hold.ticking {
            return;
        }
        self.hold.ticking = true;
        cx.spawn(async move |this, cx| {
            let mut frame = PRESS_FRAME;
            loop {
                cx.background_executor().timer(frame).await;
                let next = this
                    .update(cx, |this, cx| {
                        let now = cx.background_executor().now();
                        if this.hold.press.tick(now).is_some() {
                            this.hold_press_confirmed(cx);
                        }
                        cx.notify();
                        let press = &this.hold.press;
                        let next = if press.pressing() || press.progress(now) > 0. {
                            Some(PRESS_FRAME)
                        } else if press.animating(now) {
                            Some(LABEL_FRAME)
                        } else {
                            None
                        };
                        this.hold.ticking = next.is_some();
                        next
                    })
                    .ok()
                    .flatten();
                match next {
                    Some(next) => frame = next,
                    None => break,
                }
            }
        })
        .detach();
    }
    /// Conversations Hold stops: loaded local sessions, then Pilot-owned ones.
    fn hold_views(&self, cx: &App) -> Vec<(Entity<Conversation>, bool)> {
        let mut views: Vec<_> = self
            .sessions
            .values()
            .filter(|view| !view.read(cx).is_remote())
            .map(|view| (view.clone(), false))
            .collect();
        if let Some(pilots) = &self.pilots {
            views.extend(pilots.read(cx).hold_views());
        }
        views
    }
    pub(super) fn hold_all(&mut self, cx: &mut Context<Self>) {
        if crate::hold::active(cx) {
            return;
        }
        let views = self.hold_views(cx);
        let mut state = HoldState::new(gespan_core::time::now_ms());
        for (view, pilot_chat) in &views {
            state.track(view.read(cx).summary.id, *pilot_chat);
            self.watch_hold(view, cx);
        }
        // Their agents, and any background work in them, outlive the
        // warm pool's idle limit until Resume.
        gespan_core::provider::pin_idle_agents(state.sessions.iter().map(|s| s.session));
        // With nothing to wait for, Gespan is on hold at once.
        state.refresh_phase();
        cx.set_global(Hold(Some(state)));
        for (view, _) in &views {
            view.update(cx, |view, cx| view.hold(cx));
            // Idle sessions settle at once; busy ones report through HoldChanged.
            let (id, held) = {
                let view = view.read(cx);
                (view.summary.id, view.held)
            };
            if let Some(reason) = held {
                self.settle_hold(id, reason, None, Vec::new(), cx);
            }
        }
        self.hold_state_changed(cx);
    }
    fn watch_hold(&mut self, view: &Entity<Conversation>, cx: &mut Context<Self>) {
        self.hold
            .subscriptions
            .push(cx.subscribe(view, |this, _, event: &HoldChanged, cx| {
                this.settle_hold(
                    event.session,
                    event.reason,
                    event.last_tool.clone(),
                    event.background.clone(),
                    cx,
                );
                this.hold_state_changed(cx);
            }));
        let id = view.read(cx).summary.id;
        self.hold.subscriptions.push(cx.subscribe(
            view,
            move |this, view, _: &SessionChanged, cx| {
                if view.read(cx).busy {
                    // Its resumed turn began: from here it reports as usual.
                    if this.hold.reasons.remove(&id).is_some() {
                        this.refresh_session_mark(id, cx);
                    }
                } else {
                    this.resumed_session_settled(id, cx);
                }
            },
        ));
    }
    fn settle_hold(
        &mut self,
        session: i64,
        reason: Reason,
        last_tool: Option<String>,
        background: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        cx.update_default_global::<Hold, _>(|hold, _| {
            if let Some(state) = hold.0.as_mut() {
                state.settle(
                    session,
                    reason,
                    gespan_core::time::now_ms(),
                    last_tool,
                    background,
                );
                state.refresh_phase();
            }
        });
    }
    /// Saves the state, mirrors the phase and lets the Mac sleep once on hold.
    fn hold_state_changed(&mut self, cx: &mut Context<Self>) {
        let state = cx.try_global::<Hold>().and_then(|hold| hold.0.clone());
        self.hold.on_hold = state.as_ref().is_some_and(|s| s.phase == Phase::OnHold);
        let saved = match &state {
            Some(state) => locked(&self.store).and_then(|store| store.save_hold(state)),
            None => locked(&self.store).and_then(|store| store.clear_hold()),
        };
        if let Err(error) = saved {
            self.error = Some(format!("Could not save the hold: {error}"));
        }
        self.sync_keep_awake();
        if let Some(pilots) = self.pilots.clone() {
            pilots.update(cx, |pilots, cx| pilots.hold_changed(cx));
        }
        self.refresh_hold_marks(cx);
        self.invalidate_chrome(cx);
        cx.notify();
    }
    /// Marks and composer notices follow the hold, which no view observes.
    fn refresh_hold_marks(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<i64> = self.sessions.keys().copied().collect();
        for id in ids {
            self.refresh_session_mark(id, cx);
            self.sessions[&id].update(cx, |_, cx| cx.notify());
        }
    }
    /// After a restart the hold continues; nothing that was running survived.
    pub(super) fn restore_hold(&mut self, cx: &mut Context<Self>) {
        let restored = locked(&self.store).and_then(|store| store.hold());
        match restored {
            Ok(Some(state)) => {
                cx.set_global(Hold(Some(state.after_restart())));
                self.hold_state_changed(cx);
            }
            Ok(None) => {}
            Err(error) => self.error = Some(format!("Could not read the hold: {error}")),
        }
    }
    pub(super) fn hold_view(
        &mut self,
        id: i64,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Conversation>> {
        if let Some(view) = self.pilots.as_ref().and_then(|pilots| {
            pilots
                .read(cx)
                .hold_views()
                .into_iter()
                .find(|(v, _)| v.read(cx).summary.id == id)
        }) {
            return Some(view.0);
        }
        self.ensure_session(id, cx)
    }
    /// Stop now: the user chose to end the command a session is still waiting on.
    pub(super) fn stop_now(&mut self, session: i64, cx: &mut Context<Self>) {
        if let Some(view) = self.hold_view(session, cx) {
            view.update(cx, |view, cx| view.stop_now(cx));
        }
    }
    pub(super) fn resume_all(&mut self, cx: &mut Context<Self>) {
        let Some(state) = cx.try_global::<Hold>().and_then(|hold| hold.0.clone()) else {
            return;
        };
        if state.phase != Phase::OnHold {
            return;
        }
        // Lift the hold first: resumed turns and schedulers may start work again.
        cx.set_global(Hold(None));
        gespan_core::provider::unpin_idle_agents();
        self.hold.subscriptions.clear();
        self.hold.reasons = state
            .sessions
            .iter()
            .filter_map(|s| s.reason.map(|reason| (s.session, reason)))
            .collect();
        let known: HashSet<i64> = self
            .session_summaries
            .keys()
            .copied()
            .chain(
                self.hold_views(cx)
                    .iter()
                    .map(|(v, _)| v.read(cx).summary.id),
            )
            .collect();
        let plan = gespan_core::hold::resume_plan(&state, |id| known.contains(&id));
        // Sessions opened or started during the hold weren't tracked; their
        // held messages go out now too.
        let tracked: HashSet<i64> = state.sessions.iter().map(|s| s.session).collect();
        for (view, _) in self.hold_views(cx) {
            if !tracked.contains(&view.read(cx).summary.id) && view.read(cx).queued_count() > 0 {
                view.update(cx, |view, cx| view.resume_from_hold(Reason::Idle, cx));
            }
        }
        self.hold.resume = Some(ResumeQueue::new(plan));
        if let Some(pilots) = self.pilots.clone() {
            let now = gespan_core::time::now_ms();
            pilots.update(cx, |pilots, cx| {
                pilots.resumed(state.held_ms(now), state.since, cx)
            });
        }
        self.hold_state_changed(cx);
        self.pump_resume(cx);
        let leads: Vec<_> = self.orchestrations.keys().copied().collect();
        for lead in leads {
            self.schedule_orchestration(lead, cx);
        }
    }
    fn pump_resume(&mut self, cx: &mut Context<Self>) {
        loop {
            let Some(next) = self
                .hold
                .resume
                .as_mut()
                .map(|queue| queue.next(RESUME_LIMIT))
            else {
                return;
            };
            if next.is_empty() {
                break;
            }
            for id in next {
                let reason = self.hold.reasons.get(&id).copied().unwrap_or(Reason::Idle);
                let sent = self.hold_view(id, cx).is_some_and(|view| {
                    self.watch_hold(&view, cx);
                    view.update(cx, |view, cx| view.resume_from_hold(reason, cx))
                });
                // A turn that was left waiting simply goes on; it holds no slot.
                let waiting = matches!(reason, Reason::AwaitingInput | Reason::Background);
                if !sent || waiting {
                    self.hold.reasons.remove(&id);
                    if let Some(queue) = self.hold.resume.as_mut() {
                        queue.finished(id);
                    }
                } else if self
                    .hold_view(id, cx)
                    .is_some_and(|view| view.read(cx).busy)
                {
                    self.hold.reasons.remove(&id);
                }
                // Otherwise it stays held until its turn begins (history may
                // still be loading), so nothing reports it finished early.
            }
        }
        if self.hold.resume.as_ref().is_some_and(ResumeQueue::done) {
            self.hold.resume = None;
            self.hold.subscriptions.clear();
        }
        self.refresh_hold_marks(cx);
    }
    fn resumed_session_settled(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.hold.reasons.contains_key(&id) {
            return;
        }
        let Some(queue) = self.hold.resume.as_mut() else {
            return;
        };
        if queue.is_running(id) {
            queue.finished(id);
            self.pump_resume(cx);
        }
    }
    /// Sessions still finishing a step, for the status-bar details and the quit guard.
    pub(super) fn settling_sessions(&self, cx: &App) -> Vec<(i64, String, Option<String>)> {
        let Some(state) = cx.try_global::<Hold>().and_then(|hold| hold.0.as_ref()) else {
            return Vec::new();
        };
        state
            .sessions
            .iter()
            .filter(|held| held.reason.is_none())
            .map(|held| {
                let title = self
                    .session_summaries
                    .get(&held.session)
                    .map(|s| s.title.clone())
                    .unwrap_or_else(|| "Pilot".into());
                let tool = self.sessions.get(&held.session).and_then(|view| {
                    let view = view.read(cx);
                    view.transcript.read(cx).live_turn(&view.summary.cwd).0
                });
                (held.session, title, tool)
            })
            .collect()
    }
    /// A session that is held, or not yet resumed, has not finished its work.
    pub(super) fn session_held(&self, id: i64, live: Option<&Conversation>, cx: &App) -> bool {
        live.is_some_and(|view| view.held.is_some() || view.hold_requested)
            || self.hold.reasons.contains_key(&id)
            || cx
                .try_global::<Hold>()
                .and_then(|hold| hold.0.as_ref())
                .is_some_and(|state| state.sessions.iter().any(|s| s.session == id))
    }
    /// The sidebar and tab mark: stopped with something to continue on
    /// Resume. A session that was simply idle isn't marked, and one still
    /// finishing its step shows as working.
    pub(super) fn held_mark(&self, id: i64, live: Option<&Conversation>, cx: &App) -> bool {
        let reason = cx
            .try_global::<Hold>()
            .and_then(|hold| hold.0.as_ref())
            .and_then(|state| state.sessions.iter().find(|s| s.session == id))
            .and_then(|held| held.reason)
            .or_else(|| self.hold.reasons.get(&id).copied())
            .or_else(|| live.and_then(|view| view.held));
        reason.is_some_and(|reason| reason != Reason::Idle)
    }
    fn queued_messages(&self, state: &HoldState, cx: &App) -> usize {
        let pilot_views = self
            .pilots
            .as_ref()
            .map(|pilots| pilots.read(cx).hold_views())
            .unwrap_or_default();
        state
            .sessions
            .iter()
            .filter_map(|held| {
                self.sessions.get(&held.session).or_else(|| {
                    pilot_views
                        .iter()
                        .map(|(view, _)| view)
                        .find(|view| view.read(cx).summary.id == held.session)
                })
            })
            .map(|view| view.read(cx).queued_count())
            .sum()
    }
    /// The status-bar text for the current hold.
    pub(super) fn hold_summary(&self, cx: &App) -> String {
        let Some(state) = cx.try_global::<Hold>().and_then(|hold| hold.0.as_ref()) else {
            return "Hold all".into();
        };
        let total = state.sessions.len();
        if state.phase == Phase::Holding {
            let settled = state.sessions.iter().filter(|s| s.reason.is_some()).count();
            return format!("Holding {settled}/{total}…");
        }
        let sessions = if total == 1 {
            "1 session".to_owned()
        } else {
            format!("{total} sessions")
        };
        format!(
            "On hold · {sessions} · {} queued",
            self.queued_messages(state, cx)
        )
    }
    /// The far right of the status bar: Hold all, Holding… or On hold with Resume.
    pub(super) fn hold_control(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let chip = |id: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .flex_shrink_0()
                .h(px(22.))
                .px(px(8.))
                .rounded_md()
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(12.))
                .text_color(content(0.6))
                .hover(|s| s.bg(content(0.08)).text_color(content(0.9)))
        };
        let summary = self.hold_summary(cx);
        let control = match crate::hold::phase(cx) {
            None => self.hold_press_button(chip("hold-control"), cx),
            Some(Phase::Holding) => chip("hold-control")
                .aria_label("Holding: show what is still finishing")
                .child(crate::pulse::looping(
                    "hold-spinner",
                    std::time::Duration::from_secs(1),
                    crate::pulse::Rate::Smooth,
                    |delta| {
                        icon("loader", 12.).with_transformation(gpui::Transformation::rotate(
                            gpui::percentage(delta),
                        ))
                    },
                ))
                .child(summary)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.hold.details_open = !this.hold.details_open;
                    cx.notify();
                }))
                .into_any_element(),
            Some(Phase::OnHold) => div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(
                    div()
                        .debug_selector(|| "hold-control".into())
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .text_size(px(12.))
                        .text_color(content(0.6))
                        .child(icon("queue-pause", 12.))
                        .child(summary),
                )
                .child(
                    chip("hold-resume")
                        .aria_label("Resume everything (⌘⇧H)")
                        .text_color(content(0.9))
                        .child(icon("queue-play", 12.))
                        .child("Resume")
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_hold(window, cx))),
                )
                .into_any_element(),
        };
        div()
            .flex_shrink_0()
            .mr(px(6.))
            .child(control)
            .into_any_element()
    }
    /// Hold all: press and hold. The fill is an inverted copy of the label,
    /// clipped to the progress, so the text changes colour under the edge.
    fn hold_press_button(&self, chip: Stateful<Div>, cx: &mut Context<Self>) -> AnyElement {
        let now = cx.background_executor().now();
        let progress = self.hold.press.progress(now);
        let text = match self.hold.press.label(now) {
            Label::Rest => "Hold all",
            Label::Hint => "Hold to confirm",
            Label::Armed => "Press again to confirm",
            Label::Done => "Holding…",
        };
        let face = |color: Hsla| {
            div()
                .w(px(PRESS_WIDTH))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(5.))
                .text_color(color)
                .child(icon("queue-pause", 12.).text_color(color))
                .child(text)
        };
        let mut ink = crate::theme::shell_fill();
        ink.a = 1.;
        chip.relative()
            .w(px(PRESS_WIDTH))
            .px_0()
            .overflow_hidden()
            .aria_label("Hold everything: press and hold (⌘⇧H)")
            .child(face(content(0.6)))
            .when(progress > 0., |chip| {
                chip.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .bottom_0()
                        .w(relative(progress))
                        .overflow_hidden()
                        .bg(content(0.85))
                        .child(face(ink)),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.start_hold_press(cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.hold.press.pressing() {
                        this.release_hold_press(cx);
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.cancel_hold_press(cx)),
            )
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered && !this.hold.key_held {
                    this.cancel_hold_press(cx);
                }
            }))
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if event.is_keyboard() {
                    this.hold_button_keyboard_press(cx);
                }
            }))
            // VoiceOver's press can't be held: without this, gpui turns it
            // into an instant mouse click, which only ever shows the hint.
            .on_a11y_action(gpui::AccessibleAction::Click, {
                let shell = cx.entity().downgrade();
                move |_, _, cx| {
                    let _ = shell.update(cx, |this, cx| this.hold_button_keyboard_press(cx));
                }
            })
            .into_any_element()
    }
    /// What Hold is still waiting for, each with Stop now, above the status bar.
    pub(super) fn hold_details(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.hold.details_open || crate::hold::phase(cx) != Some(Phase::Holding) {
            self.hold.details_open = false;
            return None;
        }
        let viewport = window.viewport_size();
        let width = 340f32.min(viewport.width.as_f32() - 24.);
        let anchor = Anchor::Point(gpui::point(
            viewport.width - px(width + 12.),
            viewport.height - px(44.),
        ));
        let rows = self
            .settling_sessions(cx)
            .into_iter()
            .map(|(id, title, tool)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(6.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().truncate().text_size(px(13.)).child(title))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(content(0.5))
                                    .child(
                                        tool.unwrap_or_else(|| "Finishing its current step".into()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id(("hold-stop-now", id as usize))
                            .debug_selector(|| "hold-stop-now".into())
                            .role(Role::Button)
                            .aria_label("Stop now")
                            .flex_shrink_0()
                            .px(px(8.))
                            .h(px(22.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .text_size(px(12.))
                            .text_color(content(0.7))
                            .hover(|s| s.bg(content(0.08)))
                            .child("Stop now")
                            .on_click(cx.listener(move |this, _, _, cx| this.stop_now(id, cx))),
                    )
            });
        let owner = cx.weak_entity();
        Some(
            popover("hold-details", anchor)
                .width(width)
                .gap(0.)
                .on_dismiss(move |_, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.hold.details_open = false;
                        cx.notify();
                    });
                })
                .child(
                    div()
                        .debug_selector(|| "hold-details".into())
                        .flex()
                        .flex_col()
                        .py(px(4.))
                        .child(
                            div()
                                .px(px(10.))
                                .py(px(4.))
                                .text_size(px(12.))
                                .text_color(content(0.5))
                                .child("Still finishing. Resume is available once these stop."),
                        )
                        .children(rows),
                )
                .into_any_element(),
        )
    }
    /// Keep the Mac awake for agents only while some are working and Gespan is not on hold.
    pub(super) fn agents_keep_awake(&self) -> bool {
        !self.busy_sessions.is_empty() && !self.hold.on_hold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::hold::{HoldState, Phase, Reason};
    use gespan_core::provider::HoldPoint;
    use gespan_core::store::Store;
    use gpui::{TestAppContext, WindowHandle};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    fn shell(store: Store, cx: &mut TestAppContext) -> WindowHandle<Shell> {
        let store = Arc::new(Mutex::new(store));
        cx.update(crate::shell::bind_keys);
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Shell::new(store, vec![], "/missing/hold".into(), None, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        window
    }
    fn sessions(store: &Store, n: usize) -> Vec<i64> {
        (0..n)
            .map(|_| store.create("codex", "/missing/hold").unwrap().id)
            .collect()
    }
    fn load(shell: &mut Shell, id: i64, cx: &mut Context<Shell>) {
        let summary = locked(&shell.store)
            .unwrap()
            .sessions()
            .unwrap()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap();
        shell.add_loaded(summary, cx);
    }
    fn phase(cx: &mut TestAppContext) -> Option<Phase> {
        cx.update(|cx| crate::hold::phase(cx))
    }
    fn saved(window: WindowHandle<Shell>, cx: &mut TestAppContext) -> Option<HoldState> {
        window
            .update(cx, |shell, _, _| {
                locked(&shell.store).unwrap().hold().unwrap()
            })
            .unwrap()
    }
    /// Loads `ids` and makes the first one a busy turn that waits for its safe point.
    fn busy_and_idle(window: WindowHandle<Shell>, ids: &[i64], cx: &mut TestAppContext) {
        window
            .update(cx, |shell, _, cx| {
                for &id in ids {
                    load(shell, id, cx);
                }
                let (tx, rx) = std::sync::mpsc::channel();
                std::mem::forget(rx);
                shell.sessions[&ids[0]].update(cx, |view, _| {
                    view.busy = true;
                    view.controls = Some(tx);
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn hold_all_reaches_on_hold_once_every_session_settles(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 2);
        let window = shell(store, cx);
        busy_and_idle(window, &ids, cx);
        window
            .update(cx, |shell, _, cx| shell.hold_all(cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(phase(cx), Some(Phase::Holding));
        assert_eq!(saved(window, cx).unwrap().phase, Phase::Holding);
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&ids[0]].update(cx, |view, cx| {
                    view.hold_point(HoldPoint::Interrupted, cx);
                    view.busy = false;
                    view.hold_turn_finished(cx);
                });
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(phase(cx), Some(Phase::OnHold));
        let state = saved(window, cx).unwrap();
        assert_eq!(state.phase, Phase::OnHold);
        let reasons: Vec<_> = state
            .sessions
            .iter()
            .map(|s| (s.session, s.reason))
            .collect();
        assert!(reasons.contains(&(ids[0], Some(Reason::Interrupted))));
        assert!(reasons.contains(&(ids[1], Some(Reason::Idle))));
    }

    #[gpui::test]
    fn holding_with_nothing_running_is_on_hold_at_once(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, cx| shell.hold_all(cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(phase(cx), Some(Phase::OnHold));
    }

    #[gpui::test]
    fn sessions_resumed_while_still_waiting_do_not_hold_resume_slots(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 4);
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, cx| {
                for &id in &ids {
                    load(shell, id, cx);
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::mem::forget(rx);
                    shell.sessions[&id].update(cx, |view, _| {
                        view.summary.provider = "fixture".into();
                        view.busy = true;
                        view.controls = Some(tx);
                    });
                }
                shell.hold_all(cx);
                for &id in &ids {
                    shell.sessions[&id]
                        .update(cx, |view, cx| view.hold_point(HoldPoint::Background, cx));
                }
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(phase(cx), Some(Phase::OnHold));
        window
            .update(cx, |shell, _, cx| {
                shell.resume_all(cx);
                assert!(
                    shell.hold.resume.is_none(),
                    "every waiting session counts as resumed"
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_session_stays_held_until_its_resumed_turn_begins(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 1);
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, cx| {
                load(shell, ids[0], cx);
                shell.sessions[&ids[0]].update(cx, |view, _| {
                    view.summary.provider = "fixture".into();
                    view.hold_history_for_test();
                });
                // As after a restart: the turn needs a continuation.
                cx.set_global(Hold(Some({
                    let mut state = HoldState::new(1);
                    state.track(ids[0], false);
                    state.settle(ids[0], Reason::Restarted, 2, None, vec![]);
                    state.refresh_phase();
                    state
                })));
                shell.resume_all(cx);
                let view = shell.sessions[&ids[0]].clone();
                assert!(!view.read(cx).busy, "history is still loading");
                assert!(
                    shell.session_held(ids[0], Some(view.read(cx)), cx),
                    "not finished: its continuation hasn't started"
                );
                view.update(cx, |view, cx| {
                    view.busy = true;
                    cx.emit(SessionChanged);
                });
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                let view = shell.sessions[&ids[0]].clone();
                assert!(!shell.session_held(ids[0], Some(view.read(cx)), cx));
            })
            .unwrap();
    }

    fn mark(window: WindowHandle<Shell>, id: i64, cx: &mut TestAppContext) -> MarkState {
        window
            .update(cx, |shell, _, cx| shell.session_marks[&id].read(cx).state())
            .unwrap()
    }
    fn notice(window: WindowHandle<Shell>, id: i64, cx: &mut TestAppContext) -> bool {
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&id].read(cx).hold_notice_shown(cx)
            })
            .unwrap()
    }

    #[gpui::test]
    fn restored_sessions_are_marked_held_until_their_resumed_turn_begins(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 2);
        // Quit while on hold: one was interrupted, one was idle.
        let mut state = HoldState::new(1);
        state.track(ids[0], false);
        state.track(ids[1], false);
        state.settle(ids[1], Reason::Idle, 2, None, vec![]);
        store.save_hold(&state).unwrap();
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, cx| {
                for &id in &ids {
                    load(shell, id, cx);
                }
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            mark(window, ids[0], cx),
            MarkState::Held,
            "no view state, still held"
        );
        assert_eq!(
            mark(window, ids[1], cx),
            MarkState::Idle,
            "nothing to continue"
        );
        assert!(notice(window, ids[0], cx));
        assert!(!notice(window, ids[1], cx));

        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&ids[0]].update(cx, |view, _| {
                    view.summary.provider = "fixture".into();
                    view.hold_history_for_test();
                });
                shell.resume_all(cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            mark(window, ids[0], cx),
            MarkState::Held,
            "its continuation hasn't started"
        );
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&ids[0]].update(cx, |view, cx| {
                    view.busy = true;
                    cx.emit(SessionChanged);
                });
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(mark(window, ids[0], cx), MarkState::Working);
        assert!(!notice(window, ids[0], cx));
    }

    #[gpui::test]
    fn a_held_session_not_yet_loaded_is_marked_in_the_sidebar(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                // Listed but metadata only, as a background session after a restart.
                let mut summary = locked(&shell.store)
                    .unwrap()
                    .create("codex", "/work/gespan")
                    .unwrap();
                summary.has_messages = true;
                let id = summary.id;
                shell.order.insert(0, id);
                shell.session_summaries.insert(id, summary);
                assert!(!shell.sessions.contains_key(&id));
                cx.set_global(Hold(Some({
                    let mut state = HoldState::new(1);
                    state.track(id, false);
                    state.settle(id, Reason::Restarted, 2, None, vec![]);
                    state.refresh_phase();
                    state
                })));
                shell.hold_state_changed(cx);
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("session-held-unloaded").is_some());
    }

    #[gpui::test]
    fn a_settling_session_works_until_it_reaches_its_safe_point(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 2);
        let window = shell(store, cx);
        busy_and_idle(window, &ids, cx);
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&ids[0]].update(cx, |_, cx| cx.emit(SessionChanged));
                shell.hold_all(cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            mark(window, ids[0], cx),
            MarkState::Working,
            "a command is running"
        );
        assert!(!notice(window, ids[0], cx));
        assert_eq!(mark(window, ids[1], cx), MarkState::Idle);
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&ids[0]]
                    .update(cx, |view, cx| view.hold_point(HoldPoint::AwaitingInput, cx));
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(mark(window, ids[0], cx), MarkState::Held);
        assert!(notice(window, ids[0], cx));
    }

    #[gpui::test]
    fn the_held_notice_shows_above_the_composer(cx: &mut TestAppContext) {
        let (window, a, b) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                cx.set_global(Hold(Some({
                    let mut state = HoldState::new(1);
                    state.track(a, false);
                    state.track(b, false);
                    state.settle(a, Reason::Interrupted, 2, None, vec![]);
                    state.settle(b, Reason::Interrupted, 2, None, vec![]);
                    state.refresh_phase();
                    state
                })));
                shell.hold_state_changed(cx);
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("hold-notice").is_some());
    }

    #[gpui::test]
    fn messages_queued_in_untracked_sessions_are_released_on_resume(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 1);
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, cx| {
                shell.hold_all(cx);
                // Opened during the hold, so Hold never tracked it.
                load(shell, ids[0], cx);
                shell.sessions[&ids[0]].update(cx, |view, cx| {
                    view.summary.provider = "fixture".into();
                    view.load(cx);
                });
            })
            .unwrap();
        // Its history finishes loading while still held.
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&ids[0]]
                    .update(cx, |view, cx| view.queue_for_test("after lunch", cx));
            })
            .unwrap();
        // Saved, and blocked by the hold.
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| shell.resume_all(cx))
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                let view = shell.sessions[&ids[0]].read(cx);
                assert!(
                    !view.held_queue_waiting(),
                    "the held message is released on Resume"
                );
            })
            .unwrap();
    }

    fn advance(cx: &mut TestAppContext, ms: u64) {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(ms));
        cx.run_until_parked();
    }
    fn label(window: WindowHandle<Shell>, cx: &mut TestAppContext) -> crate::hold_confirm::Label {
        window
            .update(cx, |shell, _, cx| {
                shell.hold.press.label(cx.background_executor().now())
            })
            .unwrap()
    }
    fn requested(window: WindowHandle<Shell>, id: i64, cx: &mut TestAppContext) -> bool {
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&id].read(cx).hold_requested
            })
            .unwrap()
    }
    const SHORTCUT: &str = if cfg!(target_os = "macos") {
        "cmd-shift-h"
    } else {
        "ctrl-shift-h"
    };
    fn shortcut_modifiers() -> gpui::Modifiers {
        gpui::Modifiers {
            shift: true,
            ..gpui::Modifiers::secondary_key()
        }
    }
    /// ⌘⇧H goes down: the modifiers first, then the key.
    fn shortcut_down(visual: &mut gpui::VisualTestContext) {
        visual.simulate_modifiers_change(shortcut_modifiers());
        visual.simulate_keystrokes(SHORTCUT);
    }
    fn shortcut_up(visual: &mut gpui::VisualTestContext) {
        visual.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse(SHORTCUT).unwrap(),
        });
        visual.simulate_modifiers_change(gpui::Modifiers::none());
    }
    fn press_control(visual: &mut gpui::VisualTestContext) -> gpui::Point<gpui::Pixels> {
        visual.refresh().unwrap();
        let at = visual.debug_bounds("hold-control").unwrap().center();
        visual.simulate_mouse_move(at, None, Default::default());
        visual.simulate_mouse_down(at, gpui::MouseButton::Left, Default::default());
        at
    }

    #[gpui::test]
    fn holding_the_button_for_the_full_duration_starts_holding(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 1);
        let window = shell(store, cx);
        busy_and_idle(window, &ids, cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        press_control(visual);
        advance(visual, 1100);
        assert_eq!(phase(visual), None, "not before the fill completes");
        advance(visual, 150);
        assert!(!visual.has_pending_prompt(), "no dialog");
        assert_eq!(phase(visual), Some(Phase::Holding));
        assert!(requested(window, ids[0], visual));
    }

    #[gpui::test]
    fn early_release_changes_nothing(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 1);
        let window = shell(store, cx);
        busy_and_idle(window, &ids, cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        let at = press_control(visual);
        advance(visual, 500);
        visual.simulate_mouse_up(at, gpui::MouseButton::Left, Default::default());
        advance(visual, 3000);
        assert_eq!(phase(visual), None);
        assert_eq!(saved(window, visual), None);
        assert!(!requested(window, ids[0], visual));
    }

    #[gpui::test]
    fn leaving_the_button_mid_press_never_confirms(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        press_control(visual);
        advance(visual, 400);
        visual.simulate_mouse_move(
            gpui::point(gpui::px(5.), gpui::px(5.)),
            gpui::MouseButton::Left,
            Default::default(),
        );
        advance(visual, 3000);
        assert_eq!(phase(visual), None);
    }

    #[gpui::test]
    fn a_tap_shows_hold_to_confirm(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        let at = press_control(visual);
        advance(visual, 100);
        visual.simulate_mouse_up(at, gpui::MouseButton::Left, Default::default());
        visual.run_until_parked();
        assert_eq!(label(window, visual), crate::hold_confirm::Label::Hint);
        assert_eq!(phase(visual), None);
        advance(visual, 2000);
        assert_eq!(label(window, visual), crate::hold_confirm::Label::Rest);
    }

    #[gpui::test]
    fn holding_the_shortcut_holds_and_an_early_key_up_does_not(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 1);
        let window = shell(store, cx);
        busy_and_idle(window, &ids, cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        shortcut_down(visual);
        advance(visual, 300);
        shortcut_up(visual);
        advance(visual, 3000);
        assert_eq!(phase(visual), None, "released after 0.3 s");

        shortcut_down(visual);
        for _ in 0..12 {
            // Key auto-repeat must not restart or speed up the fill.
            advance(visual, 90);
            visual.simulate_keystrokes(SHORTCUT);
        }
        assert_eq!(phase(visual), None, "1.08 s in");
        advance(visual, 200);
        assert!(!visual.has_pending_prompt());
        assert_eq!(phase(visual), Some(Phase::Holding));
    }

    #[gpui::test]
    fn releasing_a_modifier_cancels_the_shortcut(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        shortcut_down(visual);
        advance(visual, 600);
        visual.simulate_modifiers_change(gpui::Modifiers::secondary_key());
        advance(visual, 3000);
        assert_eq!(phase(visual), None);
        assert_eq!(label(window, visual), crate::hold_confirm::Label::Rest);
    }

    #[gpui::test]
    fn a_quick_shortcut_tap_shows_hold_to_confirm(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        shortcut_down(visual);
        advance(visual, 100);
        // macOS sends no key-up for H while ⌘ is down: ⌘ going up ends it.
        visual.simulate_modifiers_change(gpui::Modifiers::none());
        visual.run_until_parked();
        assert_eq!(label(window, visual), crate::hold_confirm::Label::Hint);
        assert_eq!(phase(visual), None);
    }

    #[gpui::test]
    fn blurring_the_window_mid_press_never_confirms(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        shortcut_down(visual);
        advance(visual, 600);
        visual.deactivate_window();
        advance(visual, 3000);
        assert_eq!(phase(visual), None);
    }

    #[gpui::test]
    fn key_repeat_after_the_hold_lands_does_not_resume(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        shortcut_down(visual);
        advance(visual, 1250);
        assert_eq!(phase(visual), Some(Phase::OnHold), "nothing running");
        visual.simulate_keystrokes(SHORTCUT);
        advance(visual, 100);
        assert_eq!(phase(visual), Some(Phase::OnHold), "still held down");
        shortcut_up(visual);
        shortcut_down(visual);
        assert_eq!(phase(visual), None, "a fresh press resumes");
    }

    #[gpui::test]
    fn the_palette_and_menus_show_the_hint_instead_of_holding(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        window
            .update(cx, |_, w, cx| w.dispatch_action(Box::new(HoldFromMenu), cx))
            .unwrap();
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt());
        assert_eq!(phase(cx), None);
        assert_eq!(label(window, cx), crate::hold_confirm::Label::Hint);
        advance(cx, 5000);
        assert_eq!(phase(cx), None);
    }

    #[gpui::test]
    fn the_tray_and_palette_never_start_a_press_even_with_stale_modifiers(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        // Left with ⌘⇧Tab: the window still remembers ⌘⇧ as down.
        visual.simulate_modifiers_change(shortcut_modifiers());
        window
            .update(visual, |shell, w, cx| shell.pilot_toggle_hold(w, cx))
            .unwrap();
        window
            .update(visual, |_, w, cx| {
                w.dispatch_action(Box::new(HoldFromMenu), cx)
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(label(window, visual), crate::hold_confirm::Label::Hint);
        advance(visual, 3000);
        assert_eq!(
            phase(visual),
            None,
            "only the held shortcut or button holds"
        );
    }

    #[gpui::test]
    fn a_keyboard_press_of_the_button_arms_then_confirms(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, cx| shell.hold_button_keyboard_press(cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(phase(cx), None, "the first press only arms");
        assert_eq!(label(window, cx), crate::hold_confirm::Label::Armed);
        advance(cx, 1000);
        window
            .update(cx, |shell, _, cx| shell.hold_button_keyboard_press(cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(phase(cx), Some(Phase::OnHold));
    }

    #[gpui::test]
    fn hold_keeps_the_agents_of_held_sessions_until_resume(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 2);
        let window = shell(store, cx);
        busy_and_idle(window, &ids, cx);
        window
            .update(cx, |shell, _, cx| shell.hold_all(cx))
            .unwrap();
        assert!(
            ids.iter()
                .all(|&id| gespan_core::provider::idle_agent_pinned(id))
        );
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&ids[0]]
                    .update(cx, |view, cx| view.hold_point(HoldPoint::AwaitingInput, cx));
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(phase(cx), Some(Phase::OnHold));
        assert!(gespan_core::provider::idle_agent_pinned(ids[0]));
        window
            .update(cx, |shell, _, cx| shell.resume_all(cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(phase(cx), None);
        assert!(
            ids.iter()
                .all(|&id| !gespan_core::provider::idle_agent_pinned(id))
        );
    }

    #[gpui::test]
    fn toggle_is_ignored_while_holding(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 1);
        let window = shell(store, cx);
        busy_and_idle(window, &ids, cx);
        window
            .update(cx, |shell, _, cx| shell.hold_all(cx))
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| shell.toggle_hold(w, cx))
            .unwrap();
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt());
        assert_eq!(phase(cx), Some(Phase::Holding), "no resume while holding");
    }

    #[gpui::test]
    fn resume_clears_the_hold_and_restarts_nothing_for_idle_sessions(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 2);
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, cx| {
                for &id in &ids {
                    load(shell, id, cx);
                }
                shell.hold_all(cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(phase(cx), Some(Phase::OnHold));
        window
            .update(cx, |shell, w, cx| shell.toggle_hold(w, cx))
            .unwrap();
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt(), "Resume needs no confirmation");
        assert_eq!(phase(cx), None);
        assert_eq!(saved(window, cx), None);
        window
            .update(cx, |shell, _, cx| {
                for id in &ids {
                    let view = shell.sessions[id].read(cx);
                    assert_eq!(view.held, None);
                    assert!(!view.busy);
                }
            })
            .unwrap();
    }

    #[gpui::test]
    fn restart_keeps_the_hold_and_marks_unsettled_sessions_restarted(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 2);
        let mut state = HoldState::new(1);
        state.track(ids[0], false);
        state.track(ids[1], false);
        state.settle(ids[1], Reason::Idle, 2, None, vec![]);
        store.save_hold(&state).unwrap();
        let window = shell(store, cx);
        assert_eq!(phase(cx), Some(Phase::OnHold));
        let restored = saved(window, cx).unwrap();
        assert_eq!(restored.sessions[0].reason, Some(Reason::Restarted));
        assert_eq!(restored.sessions[1].reason, Some(Reason::Idle));
    }

    #[gpui::test]
    fn quit_anyway_still_asks_about_unsaved_files(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        cx.update(|cx| cx.set_reduce_motion(true));
        let path = std::env::temp_dir().join(format!("gespan-hold-quit-{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let (window, a, _) = crate::shell::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, w, cx| {
                shell.open(a, w, cx);
                shell.open_review(
                    crate::review::ReviewTab::File {
                        cwd: "/tmp".into(),
                        path: path.to_string_lossy().into_owned(),
                    },
                    w,
                    cx,
                );
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(visual, |shell, w, cx| {
                let pane = shell.reviews[&a].clone();
                pane.update(cx, |pane, cx| {
                    pane.navigate_file(1, 4, cx);
                    let editor = pane.active_editor().unwrap();
                    let input = editor.read(cx).input.clone();
                    input.update(cx, |i, cx| i.replace_text_in_range(None, " edited", w, cx));
                });
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(visual, |shell, w, cx| {
                let (tx, rx) = std::sync::mpsc::channel();
                std::mem::forget(rx);
                shell.sessions[&a].update(cx, |view, _| {
                    view.busy = true;
                    view.controls = Some(tx);
                });
                shell.hold_all(cx);
                assert!(shell.unsaved_files(cx));
                shell.request_quit(w, cx);
            })
            .unwrap();
        visual.run_until_parked();
        visual.simulate_prompt_answer("Quit Anyway");
        visual.run_until_parked();
        assert!(
            visual.has_pending_prompt(),
            "unsaved edits are asked about next"
        );
        visual.simulate_prompt_answer("Cancel");
        let _ = std::fs::remove_file(path);
    }

    #[gpui::test]
    fn quitting_while_holding_asks_first(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 1);
        let window = shell(store, cx);
        busy_and_idle(window, &ids, cx);
        window
            .update(cx, |shell, _, cx| shell.hold_all(cx))
            .unwrap();
        window
            .update(cx, |shell, w, cx| shell.request_quit(w, cx))
            .unwrap();
        cx.run_until_parked();
        assert!(cx.has_pending_prompt(), "a command is still finishing");
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(phase(cx), Some(Phase::Holding));
    }

    fn click(selector: &'static str, visual: &mut gpui::VisualTestContext) {
        visual.refresh().unwrap();
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not painted"));
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
    }

    #[gpui::test]
    fn the_status_bar_offers_hold_all_at_its_far_right(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        let control = visual.debug_bounds("hold-control").unwrap();
        let width = window
            .update(visual, |_, w, _| w.viewport_size().width.as_f32())
            .unwrap();
        assert!(
            control.right().as_f32() > width - 160.,
            "{control:?} in {width}"
        );
        assert!(visual.debug_bounds("hold-resume").is_none());
        click("hold-control", visual);
        assert!(
            !visual.has_pending_prompt(),
            "a click is a tap, not a dialog"
        );
        assert_eq!(visual.update(|_, cx| crate::hold::phase(cx)), None);
    }

    #[gpui::test]
    fn holding_shows_progress_details_with_stop_now_and_no_resume(cx: &mut TestAppContext) {
        let (window, a, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                let (tx, rx) = std::sync::mpsc::channel();
                std::mem::forget(rx);
                shell.sessions[&a].update(cx, |view, _| {
                    view.busy = true;
                    view.controls = Some(tx);
                });
                shell.hold_all(cx);
                assert_eq!(shell.hold_summary(cx), "Holding 1/2…");
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        assert!(
            visual.debug_bounds("hold-resume").is_none(),
            "no resume while holding"
        );
        click("hold-control", visual);
        assert!(visual.debug_bounds("hold-details").is_some());
        click("hold-stop-now", visual);
        assert_eq!(
            visual.update(|_, cx| crate::hold::phase(cx)),
            Some(Phase::OnHold)
        );
    }

    #[gpui::test]
    fn on_hold_counts_sessions_and_queued_messages_and_offers_resume(cx: &mut TestAppContext) {
        let (window, a, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.hold_all(cx);
                shell.sessions[&a].update(cx, |view, cx| view.queue_for_test("later", cx));
                assert_eq!(shell.hold_summary(cx), "On hold · 2 sessions · 1 queued");
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        click("hold-resume", visual);
        assert_eq!(visual.update(|_, cx| crate::hold::phase(cx)), None);
    }

    #[gpui::test]
    fn the_palette_holds_and_resumes(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |_, _, cx| {
                assert_eq!(crate::quick_open::hold_label(cx), "Hold everything");
                cx.set_global(crate::hold::Hold(Some({
                    let mut state = HoldState::new(1);
                    state.refresh_phase();
                    state
                })));
                assert_eq!(crate::quick_open::hold_label(cx), "Resume everything");
            })
            .unwrap();
    }

    #[gpui::test]
    fn automations_are_not_polled_while_held(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, cx| {
                shell.hold_all(cx);
                shell.automation_state.polling = false;
                shell.poll_automations(cx);
                assert!(!shell.automation_state.polling, "no claim while held");
            })
            .unwrap();
    }

    #[gpui::test]
    fn keep_awake_is_released_once_on_hold(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 1);
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, cx| {
                load(shell, ids[0], cx);
                shell.busy_sessions.insert(ids[0]);
                assert!(shell.agents_keep_awake());
                shell.hold_all(cx);
                assert_eq!(crate::hold::phase(cx), Some(Phase::OnHold));
                assert!(!shell.agents_keep_awake());
            })
            .unwrap();
    }
}
