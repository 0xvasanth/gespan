//! Hold everything: confirm, stop every session at its safe point, keep the
//! hold across restarts, and resume sessions a few at a time.
use super::*;
use crate::conversation::HoldChanged;
use crate::hold::Hold;
use gespan_core::hold::{HoldState, Phase, Reason, ResumeQueue};

/// At most this many held sessions start again at once.
const RESUME_LIMIT: usize = 3;

#[derive(Default)]
pub(super) struct HoldCoordinator {
    subscriptions: Vec<Subscription>,
    resume: Option<ResumeQueue>,
    reasons: HashMap<i64, Reason>,
    prompting: bool,
    details_open: bool,
    /// Mirrors the global phase for code that has no `App` at hand.
    pub(super) on_hold: bool,
}

impl Shell {
    /// ⌘⇧H, the status-bar control, the palette and the menus all land here.
    pub(super) fn toggle_hold(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match crate::hold::phase(cx) {
            None => self.confirm_hold(window, cx),
            // Hold can't be undone until every session has stopped.
            Some(Phase::Holding) => {}
            Some(Phase::OnHold) => self.resume_all(cx),
        }
    }
    pub(super) fn confirm_hold(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hold.prompting || crate::hold::active(cx) {
            return;
        }
        self.hold.prompting = true;
        let running = self
            .hold_views(cx)
            .iter()
            .filter(|(v, _)| v.read(cx).busy)
            .count();
        let sessions = match running {
            1 => "1 running session".to_owned(),
            n => format!("{n} running sessions"),
        };
        let detail = format!(
            "{sessions} will stop once their current command finishes, and nothing new will start. \
             Messages you send will wait until you resume. You can't resume until every session has stopped."
        );
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            "Hold everything?",
            Some(&detail),
            &["Hold", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                this.hold.prompting = false;
                if matches!(answer, Ok(0)) {
                    this.hold_all(cx);
                }
            });
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
                    this.hold.reasons.remove(&id);
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
        self.invalidate_chrome(cx);
        cx.notify();
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
            None => chip("hold-control")
                .aria_label("Hold everything (⌘⇧H)")
                .child(icon("queue-pause", 12.))
                .child(summary)
                .on_click(cx.listener(|this, _, window, cx| this.toggle_hold(window, cx)))
                .into_any_element(),
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

    #[gpui::test]
    fn cancelled_confirmation_changes_nothing(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 1);
        let window = shell(store, cx);
        busy_and_idle(window, &ids, cx);
        window
            .update(cx, |shell, w, cx| shell.toggle_hold(w, cx))
            .unwrap();
        cx.run_until_parked();
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(phase(cx), None);
        assert_eq!(saved(window, cx), None);
        window
            .update(cx, |shell, _, cx| {
                assert!(!shell.sessions[&ids[0]].read(cx).hold_requested);
            })
            .unwrap();
    }

    #[gpui::test]
    fn confirmed_hold_starts_holding(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ids = sessions(&store, 1);
        let window = shell(store, cx);
        busy_and_idle(window, &ids, cx);
        window
            .update(cx, |shell, w, cx| shell.toggle_hold(w, cx))
            .unwrap();
        cx.run_until_parked();
        cx.simulate_prompt_answer("Hold");
        cx.run_until_parked();
        assert_eq!(phase(cx), Some(Phase::Holding));
        window
            .update(cx, |shell, _, cx| {
                assert!(shell.sessions[&ids[0]].read(cx).hold_requested);
            })
            .unwrap();
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
        assert!(!cx.has_pending_prompt(), "no second confirmation");
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
        assert!(visual.has_pending_prompt(), "Hold always asks first");
        visual.simulate_prompt_answer("Cancel");
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
