//! This window's part in several windows: see `crate::windows`.
use super::*;
use std::time::Duration;

/// How long a window waits to re-read sessions, so a burst reads once.
const SESSION_SYNC_DELAY: Duration = Duration::from_millis(400);

/// What a new window starts from, taken from the primary.
pub(crate) struct WindowSeed {
    store: Database,
    summaries: Vec<Session>,
    cwd: String,
    workspace: Workspace,
    remote: Arc<gespan_core::remote::Connections>,
    projects: Vec<String>,
    metadata: gespan_core::store::SessionSidebarMetadata,
}
impl WindowSeed {
    /// Reads the sidebar's index off the UI thread.
    pub(crate) async fn load(mut self, cx: &mut gpui::AsyncApp) -> Self {
        let store = self.store.clone();
        let projects = self.projects.clone();
        self.metadata = cx
            .background_executor()
            .spawn(async move {
                locked(&store)
                    .map(|db| db.session_sidebar_metadata(&projects))
                    .unwrap_or_default()
            })
            .await;
        self
    }
    pub(crate) fn bounds(&self, cx: &App) -> gpui::WindowBounds {
        self.workspace.window.bounds(cx)
    }
    pub(crate) fn decorations(&self, cx: &App) -> Option<gpui::WindowDecorations> {
        self.workspace
            .preferences
            .wayland_decorations(cx.compositor_name())
    }
}

impl Shell {
    pub(crate) fn new_window_seed(&self, cx: &App) -> WindowSeed {
        let workspace = self.workspace.for_new_window();
        let mut projects = workspace.projects.clone();
        if !projects.contains(&workspace.project) {
            projects.push(workspace.project.clone());
        }
        let summaries = self
            .order
            .iter()
            .filter_map(|id| self.latest_summary(*id, cx).cloned())
            .collect();
        WindowSeed {
            store: self.store.clone(),
            summaries,
            cwd: workspace.project.clone(),
            workspace,
            remote: self.remote.clone(),
            projects,
            metadata: Default::default(),
        }
    }
    fn latest_summary<'a>(&'a self, id: i64, cx: &'a App) -> Option<&'a Session> {
        let live = self.sessions.get(&id).map(|view| &view.read(cx).summary);
        live.or_else(|| self.session_summaries.get(&id))
    }
    pub(crate) fn for_new_window(seed: WindowSeed, cx: &mut Context<Self>) -> Self {
        let saved = serde_json::to_string(&seed.workspace).ok();
        Self::build(
            seed.store,
            seed.summaries,
            seed.cwd,
            saved,
            seed.metadata,
            Some(seed.remote),
            cx,
        )
    }
    pub(crate) fn live_chats(&self) -> Vec<Entity<Conversation>> {
        self.sessions.values().cloned().collect()
    }
    pub(crate) fn show_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_window_visible(window, true, cx);
    }
    pub(crate) fn show_session(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.show_window(window, cx);
        self.activate_session(id, window, cx);
    }

    /// Passes app-wide workspace settings to the other windows. Only the
    /// primary saves, so this is how another window's changes reach the store.
    pub(super) fn share_workspace(&self, cx: &mut Context<Self>) {
        if crate::windows::count(cx) < 2 {
            return;
        }
        let shared = self.workspace.shared_part();
        let Ok(json) = serde_json::to_string(&shared) else {
            return;
        };
        let others = crate::windows::share(cx.entity_id(), json, cx);
        if others.is_empty() {
            return;
        }
        crate::windows::sessions_changed(cx.entity_id(), cx);
        cx.defer(move |cx| {
            for window in others {
                let _ = window.update(cx, |shell, _, cx| shell.adopt_shared(&shared, cx));
            }
        });
    }
    fn adopt_shared(&mut self, shared: &Workspace, cx: &mut Context<Self>) {
        let preferences = self.workspace.preferences != shared.preferences;
        self.workspace.adopt_shared(shared);
        if preferences {
            self.apply_preferences(cx);
        }
        self.save_state(cx);
        self.invalidate_chrome(cx);
        cx.notify();
    }

    /// Re-reads sessions soon, for rows another window added, renamed or
    /// removed. A lone window has nothing to catch up with.
    pub(crate) fn schedule_session_sync(&mut self, cx: &mut Context<Self>) {
        if self.session_sync.is_some() || crate::windows::count(cx) < 2 {
            return;
        }
        let store = self.store.clone();
        self.session_sync = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SESSION_SYNC_DELAY).await;
            let read = cx
                .background_executor()
                .spawn(async move { locked(&store)?.sessions() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.session_sync = None;
                match read {
                    Ok(sessions) => this.merge_sessions(sessions, cx),
                    Err(error) => tracing::warn!(%error, "cannot re-read sessions"),
                }
            });
        }));
    }
    fn merge_sessions(&mut self, saved: Vec<Session>, cx: &mut Context<Self>) {
        let ids: HashSet<i64> = saved.iter().map(|s| s.id).collect();
        let live = &self.sessions;
        let gone = |id: &i64| !ids.contains(id) && !live.contains_key(id);
        self.order.retain(|id| !gone(id));
        self.session_summaries.retain(|id, _| !gone(id));
        let pilots = self.pilots.clone().or_else(|| {
            let primary = crate::windows::primary(cx)?;
            primary.read(cx).ok()?.pilots.clone()
        });
        for summary in saved {
            let id = summary.id;
            let pilot = pilots.as_ref();
            if self.sessions.contains_key(&id)
                || self.deleting.contains(&id)
                || pilot.is_some_and(|pilots| pilots.read(cx).is_pilot_session(id))
            {
                continue;
            }
            self.session_summaries.insert(id, summary);
            if !self.order.contains(&id) {
                self.order.push(id);
            }
        }
        self.invalidate_chrome(cx);
        cx.notify();
    }

    /// The window's close button. Returns whether the window goes now.
    pub(super) fn close_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.save_window_state(window, cx);
        if self.secondary {
            return self.close_further_window(window, cx);
        }
        // App-wide work stays here; the other windows still need it.
        if crate::windows::visible_elsewhere(cx.entity_id(), cx) > 0 {
            self.set_window_visible(window, false, cx);
            return false;
        }
        #[cfg(all(target_os = "macos", not(test)))]
        if self.workspace.preferences.close_to_tray
            && self.workspace.preferences.pilot_menu_bar_icon
            && self
                .pilots
                .as_ref()
                .is_some_and(|m| !m.read(cx).roster.pilots.is_empty())
            && !self.unsaved_files(cx)
        {
            self.set_window_visible(window, false, cx);
            return false;
        }
        let to_tray = cfg!(windows) && self.workspace.preferences.close_to_tray;
        match crate::tray::on_close(
            to_tray,
            cfg!(target_os = "macos"),
            !self.running_chats(cx).is_empty(),
            self.unsaved_files(cx),
        ) {
            crate::tray::Close::Hide => {
                self.set_window_visible(window, false, cx);
                false
            }
            // Closing the window quits: ask first, as a quit does.
            crate::tray::Close::Ask => {
                self.set_window_visible(window, true, cx);
                self.request_quit(window, cx);
                false
            }
            crate::tray::Close::Quit => true,
        }
    }
    fn close_further_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.unsaved_files(cx) {
            self.leave_window(cx);
            return true;
        }
        if self.quit_prompted {
            return false;
        }
        self.quit_prompted = true;
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            "Close this window with unsaved file edits?",
            Some("Save your files before closing to keep your changes."),
            &["Close Without Saving", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.quit_prompted = false;
                if matches!(answer, Ok(0)) {
                    this.leave_window(cx);
                    window.remove_window();
                }
            });
        })
        .detach();
        false
    }
    /// Hands running chats to the primary, so closing this window leaves
    /// their agents running. They move with the runs they belong to.
    fn leave_window(&mut self, cx: &mut Context<Self>) {
        let key = cx.entity_id();
        let last = crate::windows::visible_elsewhere(key, cx) == 0;
        let leads: HashSet<i64> = self.orchestrations.keys().copied().collect();
        let staying: Vec<i64> = self
            .sessions
            .iter()
            .filter(|(id, view)| view.read(cx).busy || leads.contains(id))
            .map(|(id, _)| *id)
            .collect();
        let chats: Vec<_> = staying
            .iter()
            .filter_map(|id| self.sessions.remove(id))
            .collect();
        let runs = std::mem::take(&mut self.orchestrations);
        let summaries = std::mem::take(&mut self.orchestration_summaries);
        let started_by = std::mem::take(&mut self.started_by);
        crate::windows::remove(key, cx);
        crate::windows::on_primary(cx, move |primary, window, cx| {
            primary.orchestrations.extend(runs);
            primary.orchestration_summaries.extend(summaries);
            primary.started_by.extend(started_by);
            for chat in chats {
                primary.adopt_session(chat, cx);
            }
            primary.sync_in_flight(cx);
            cx.notify();
            // The primary was hidden and this was the last window on screen,
            // so this close means what closing the last window means.
            if last && primary.close_requested(window, cx) {
                window.remove_window();
            }
        });
    }
    /// Takes over a chat from a closing window, agent and all.
    fn adopt_session(&mut self, view: Entity<Conversation>, cx: &mut Context<Self>) {
        let summary = view.read(cx).summary.clone();
        let id = summary.id;
        if self.sessions.contains_key(&id) || !crate::windows::claim(id, cx.entity_id(), cx) {
            return;
        }
        self.session_summaries.insert(id, summary.clone());
        self.session_marks
            .insert(id, cx.new(|_| SessionMark::new(summary.provider.clone())));
        self.tab_marks
            .insert(id, cx.new(|_| SessionMark::tab(summary.provider.clone())));
        if !self.order.contains(&id) {
            self.order.push(id);
        }
        if let Some(bridge) = &self.app_bridge {
            view.update(cx, |view, _| {
                view.set_app_environment(bridge.environment(id))
            });
        }
        self.attach_session(id, view, cx);
        // A lead's capability named the closing window's bridge.
        self.orchestration_view_added(id, cx);
        self.refresh_session_mark(id, cx);
        self.sync_keep_awake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::store::Store;
    use gpui::{MenuItem, TestAppContext, VisualTestContext, WindowHandle};
    use std::path::Path;

    const CWD: &str = "/work/gespan";

    fn first_window(store: Store, cx: &mut TestAppContext) -> WindowHandle<Shell> {
        let summaries = store.sessions().unwrap();
        let store = Arc::new(Mutex::new(store));
        cx.update(crate::shell::bind_keys);
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Shell::new(store, summaries, CWD.into(), None, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        window
    }

    fn windows(cx: &mut TestAppContext) -> Vec<WindowHandle<Shell>> {
        cx.update(|cx| {
            cx.windows()
                .into_iter()
                .filter_map(|window| window.downcast::<Shell>())
                .collect()
        })
    }

    fn new_window(first: WindowHandle<Shell>, cx: &mut TestAppContext) -> WindowHandle<Shell> {
        cx.update(crate::windows::open_new_window);
        cx.run_until_parked();
        windows(cx)
            .into_iter()
            .rfind(|window| *window != first)
            .expect("a new window")
    }

    fn two_windows(cx: &mut TestAppContext) -> (WindowHandle<Shell>, WindowHandle<Shell>, i64) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let id = store.create("codex", CWD).unwrap().id;
        let first = first_window(store, cx);
        let second = new_window(first, cx);
        (first, second, id)
    }

    fn active(window: WindowHandle<Shell>, cx: &mut TestAppContext) -> i64 {
        window
            .update(cx, |shell, _, _| shell.workspace.active.unwrap())
            .unwrap()
    }

    fn set_busy(window: WindowHandle<Shell>, id: i64, busy: bool, cx: &mut TestAppContext) {
        window
            .update(cx, |shell, _, cx| {
                shell.sessions[&id].update(cx, |view, cx| {
                    view.busy = busy;
                    cx.emit(SessionChanged);
                });
            })
            .unwrap();
        cx.run_until_parked();
    }

    /// As the platform does: asks the window, and closes it if it agrees.
    fn close(window: WindowHandle<Shell>, cx: &mut TestAppContext) -> bool {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let closed = visual.simulate_close();
        visual.run_until_parked();
        if closed {
            let _ = window.update(cx, |_, window, _| window.remove_window());
            cx.run_until_parked();
            cx.update(crate::windows::forget_closed);
        }
        closed
    }

    fn hidden(window: WindowHandle<Shell>, cx: &mut TestAppContext) -> bool {
        window
            .update(cx, |_, _, cx| crate::windows::is_hidden(cx.entity_id(), cx))
            .unwrap()
    }

    #[gpui::test]
    fn closing_a_window_hands_its_run_to_the_primary_still_under_control(cx: &mut TestAppContext) {
        use gespan_core::orchestration::{Plan, Task};
        let (first, second, _) = two_windows(cx);
        let lead = active(second, cx);
        second
            .update(cx, |shell, _, cx| {
                shell.start_orchestration(
                    lead,
                    Plan {
                        title: "Work".into(),
                        summary: String::new(),
                        tasks: vec![Task {
                            id: "a".into(),
                            title: "A".into(),
                            prompt: "Work".into(),
                            files: vec!["src".into()],
                            depends_on: Vec::new(),
                            harness: None,
                            model: None,
                            model_settings: Default::default(),
                        }],
                        max_workers: 1,
                    },
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();
        close(second, cx);
        let (reply, result) = std::sync::mpsc::channel();
        first
            .update(cx, |shell, _, cx| {
                assert!(shell.sessions.contains_key(&lead));
                shell.handle_control_call(
                    gespan_core::pilot::bridge::Call {
                        session: lead,
                        request_id: "r1".into(),
                        action: "list".into(),
                        input: serde_json::json!({}),
                        reply,
                    },
                    cx,
                );
                shell.stop_orchestration(lead, cx);
            })
            .unwrap();
        let listed = result.try_recv().unwrap().unwrap();
        assert_eq!(listed["run"]["tasks"][0]["title"], "A");
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_new_window_is_a_further_workspace_with_its_own_chat(cx: &mut TestAppContext) {
        let (first, second, id) = two_windows(cx);
        assert_eq!(windows(cx).len(), 2);
        assert_eq!(active(first, cx), id);
        assert_ne!(active(second, cx), id, "the first window keeps its chat");
        second
            .update(cx, |shell, _, _| {
                assert!(shell.secondary);
                assert!(shell.pilots.is_none(), "Pilots stay with the first window");
                assert!(shell.app_bridge.is_none());
                assert!(shell.session_summaries.contains_key(&id));
                assert!(!shell.sessions.contains_key(&id));
                assert_eq!(shell.workspace.project, CWD);
                assert!(!shell.workspace.tabs.contains(&id));
            })
            .unwrap();
        first
            .update(cx, |shell, _, _| assert!(!shell.secondary))
            .unwrap();
    }

    #[gpui::test]
    fn a_chat_open_elsewhere_shows_there_instead_of_starting_a_second_agent(
        cx: &mut TestAppContext,
    ) {
        let (first, second, id) = two_windows(cx);
        let own = active(second, cx);
        second
            .update(cx, |shell, window, cx| {
                assert!(shell.ensure_session(id, cx).is_none());
                shell.activate_session(id, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(active(second, cx), own);
        second
            .update(cx, |shell, _, _| assert!(!shell.sessions.contains_key(&id)))
            .unwrap();
        // The other way round, too, and a hidden window comes back to show it.
        first
            .update(cx, |shell, window, cx| {
                shell.set_window_visible(window, false, cx);
                shell.activate_session(own, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        first
            .update(cx, |shell, _, _| {
                assert!(!shell.sessions.contains_key(&own))
            })
            .unwrap();
        assert_eq!(active(first, cx), id);
    }

    #[gpui::test]
    fn showing_a_chat_from_another_window_brings_its_hidden_window_back(cx: &mut TestAppContext) {
        let (first, second, id) = two_windows(cx);
        assert!(!close(first, cx), "the first window hides");
        assert!(hidden(first, cx));
        second
            .update(cx, |shell, window, cx| {
                shell.activate_session(id, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert!(!hidden(first, cx));
    }

    #[gpui::test]
    fn closing_a_further_window_hands_its_running_chat_to_the_first(cx: &mut TestAppContext) {
        let (first, second, _) = two_windows(cx);
        let id = active(second, cx);
        set_busy(second, id, true, cx);
        let chat = second
            .update(cx, |shell, _, _| shell.sessions[&id].clone())
            .unwrap();
        assert!(close(second, cx), "the window goes, the agent does not");
        assert!(!cx.has_pending_prompt());
        assert_eq!(windows(cx).len(), 1);
        first
            .update(cx, |shell, _, cx| {
                let kept = &shell.sessions[&id];
                assert_eq!(kept.entity_id(), chat.entity_id(), "the same live chat");
                assert!(kept.read(cx).busy);
                assert!(shell.order.contains(&id));
                assert_eq!(shell.running_chats(cx), vec![id]);
                assert_eq!(
                    locked(&shell.store).unwrap().take_in_flight().unwrap(),
                    vec![id],
                    "a quit from here still resumes it"
                );
            })
            .unwrap();
        // It still reports to the window that took it.
        set_busy(first, id, false, cx);
        first
            .update(cx, |shell, _, cx| {
                assert!(shell.running_chats(cx).is_empty())
            })
            .unwrap();
    }

    #[gpui::test]
    fn an_idle_further_window_just_closes_and_frees_its_chats(cx: &mut TestAppContext) {
        let (first, second, _) = two_windows(cx);
        let id = active(second, cx);
        assert!(close(second, cx));
        first
            .update(cx, |shell, window, cx| {
                assert!(!shell.sessions.contains_key(&id));
                shell.session_summaries.entry(id).or_insert_with(|| {
                    locked(&shell.store)
                        .unwrap()
                        .sessions()
                        .unwrap()
                        .into_iter()
                        .find(|s| s.id == id)
                        .unwrap()
                });
                shell.activate_session(id, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(active(first, cx), id, "free to open here now");
    }

    #[gpui::test]
    fn closing_the_first_window_hides_it_while_another_is_open(cx: &mut TestAppContext) {
        let (first, second, id) = two_windows(cx);
        set_busy(first, id, true, cx);
        assert!(!close(first, cx));
        assert!(!cx.has_pending_prompt(), "nothing stops, so nothing to ask");
        assert!(hidden(first, cx));
        assert_eq!(windows(cx).len(), 2);
        assert!(!cx.update(crate::windows::forget_closed));
        second
            .update(cx, |_, _, cx| assert!(crate::windows::is_running(id, cx)))
            .unwrap();
    }

    #[gpui::test]
    fn new_window_brings_back_a_hidden_first_window_rather_than_a_third(cx: &mut TestAppContext) {
        let (first, _second, _) = two_windows(cx);
        assert!(!close(first, cx));
        cx.update(crate::windows::open_new_window);
        cx.run_until_parked();
        assert_eq!(windows(cx).len(), 2);
        assert!(!hidden(first, cx));
    }

    #[gpui::test]
    fn the_dock_icon_brings_back_a_hidden_window_only(cx: &mut TestAppContext) {
        let (first, second, _) = two_windows(cx);
        assert!(!close(first, cx));
        cx.update(crate::shell::reopen);
        cx.run_until_parked();
        assert!(!hidden(first, cx));
        assert!(!hidden(second, cx));
    }

    #[gpui::test]
    fn closing_the_last_window_on_screen_ends_the_app_when_nothing_runs(cx: &mut TestAppContext) {
        let (first, second, _) = two_windows(cx);
        // Close to tray, on by default, would keep the app going on Windows.
        first
            .update(cx, |shell, _, _| {
                shell.workspace.preferences.close_to_tray = false
            })
            .unwrap();
        assert!(!close(first, cx));
        assert!(close(second, cx));
        cx.run_until_parked();
        assert!(windows(cx).is_empty(), "the hidden first window goes too");
        assert!(cx.update(crate::windows::forget_closed));
    }

    #[gpui::test]
    fn a_reminder_click_brings_a_hidden_window_back(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let id = store.create("codex", CWD).unwrap().id;
        let first = first_window(store, cx);
        first
            .update(cx, |shell, window, cx| {
                shell.set_window_visible(window, false, cx)
            })
            .unwrap();
        assert!(hidden(first, cx));
        first
            .update(cx, |shell, _, cx| {
                shell.open_notice_target(
                    crate::notifications::delivery::Target::Reminder {
                        session_id: id,
                        due_at: 1,
                    },
                    cx,
                )
            })
            .unwrap();
        cx.run_until_parked();
        assert!(!hidden(first, cx));
        assert_eq!(active(first, cx), id);
    }

    #[gpui::test]
    fn pilot_chat_and_the_quick_composer_bring_the_first_window_back(cx: &mut TestAppContext) {
        let (first, _second, _) = two_windows(cx);
        assert!(!close(first, cx));
        cx.update(crate::windows::show_primary);
        cx.run_until_parked();
        assert!(!hidden(first, cx));
    }

    #[gpui::test]
    fn the_running_list_covers_every_window(cx: &mut TestAppContext) {
        let (first, second, id) = two_windows(cx);
        let other = active(second, cx);
        set_busy(first, id, true, cx);
        set_busy(second, other, true, cx);
        let store = first.update(cx, |shell, _, _| shell.store.clone()).unwrap();
        let mut saved = locked(&store).unwrap().take_in_flight().unwrap();
        saved.sort_unstable();
        assert_eq!(saved, vec![id, other]);
        // A chat ending in one window keeps the other's on the list.
        set_busy(first, id, false, cx);
        assert_eq!(
            locked(&store).unwrap().take_in_flight().unwrap(),
            vec![other]
        );
    }

    #[gpui::test]
    fn quitting_from_one_window_counts_chats_running_in_another(cx: &mut TestAppContext) {
        let (first, second, id) = two_windows(cx);
        set_busy(first, id, true, cx);
        second
            .update(cx, |shell, window, cx| shell.request_quit(window, cx))
            .unwrap();
        cx.run_until_parked();
        assert!(cx.has_pending_prompt());
        assert!(!cx.update(|cx| crate::quitting(cx)));
    }

    #[gpui::test]
    fn a_settings_change_reaches_every_window_and_only_the_first_saves(cx: &mut TestAppContext) {
        let (first, second, id) = two_windows(cx);
        let before = first
            .update(cx, |shell, _, _| shell.workspace.preferences.autosave)
            .unwrap();
        second
            .update(cx, |shell, _, cx| {
                shell.workspace.preferences.autosave = !before;
                shell.persist(cx);
            })
            .unwrap();
        cx.run_until_parked();
        first
            .update(cx, |shell, _, _| {
                assert_eq!(shell.workspace.preferences.autosave, !before);
                assert_eq!(shell.workspace.active, Some(id), "its own view stays");
                let saved = locked(&shell.store).unwrap().ui_state().unwrap().unwrap();
                let saved: Workspace = serde_json::from_str(&saved).unwrap();
                assert_eq!(saved.preferences.autosave, !before);
                assert_eq!(saved.active, Some(id), "the saved layout is the first's");
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_chat_started_in_one_window_joins_the_others_sidebar(cx: &mut TestAppContext) {
        let (first, second, _) = two_windows(cx);
        let started = first
            .update(cx, |shell, _, cx| {
                let id = locked(&shell.store)
                    .unwrap()
                    .create("codex", CWD)
                    .unwrap()
                    .id;
                shell.persist(cx);
                crate::windows::sessions_changed(cx.entity_id(), cx);
                id
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(SESSION_SYNC_DELAY * 2);
        cx.run_until_parked();
        second
            .update(cx, |shell, _, _| {
                assert!(shell.session_summaries.contains_key(&started));
                assert!(shell.order.contains(&started));
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_chat_deleted_elsewhere_leaves_the_sidebar(cx: &mut TestAppContext) {
        let (first, second, _) = two_windows(cx);
        let store = first.update(cx, |s, _, _| s.store.clone()).unwrap();
        let gone = locked(&store).unwrap().create("codex", CWD).unwrap().id;
        second
            .update(cx, |shell, _, cx| {
                let sessions = locked(&store).unwrap().sessions().unwrap();
                shell.merge_sessions(sessions, cx);
                assert!(shell.order.contains(&gone));
                let sessions = locked(&store).unwrap().sessions().unwrap();
                let left: Vec<_> = sessions.into_iter().filter(|s| s.id != gone).collect();
                shell.merge_sessions(left, cx);
                assert!(!shell.order.contains(&gone));
                assert!(!shell.session_summaries.contains_key(&gone));
            })
            .unwrap();
    }

    #[gpui::test]
    fn deleting_a_chat_held_elsewhere_goes_to_its_window(cx: &mut TestAppContext) {
        let (first, second, id) = two_windows(cx);
        // A blank chat would just be reused for the tab closing leaves empty.
        first
            .update(cx, |shell, _, cx| {
                shell.sessions[&id].update(cx, |view, _| view.summary.has_messages = true)
            })
            .unwrap();
        second
            .update(cx, |shell, window, cx| shell.delete_session(id, window, cx))
            .unwrap();
        cx.run_until_parked();
        first
            .update(cx, |shell, _, _| assert!(!shell.sessions.contains_key(&id)))
            .unwrap();
    }

    #[gpui::test]
    fn hold_from_a_further_window_covers_every_window(cx: &mut TestAppContext) {
        let (first, second, id) = two_windows(cx);
        let other = active(second, cx);
        second
            .update(cx, |shell, _, cx| shell.hold_all(cx))
            .unwrap();
        cx.run_until_parked();
        let held: Vec<i64> = cx.update(|cx| {
            let state = cx.global::<crate::hold::Hold>().0.clone().unwrap();
            state.sessions.iter().map(|s| s.session).collect()
        });
        assert!(held.contains(&id) && held.contains(&other));
        second
            .update(cx, |shell, _, _| assert!(shell.hold.on_hold))
            .unwrap();
        second
            .update(cx, |shell, _, cx| shell.resume_all(cx))
            .unwrap();
        cx.run_until_parked();
        assert!(!cx.update(|cx| crate::hold::active(cx)));
        first
            .update(cx, |shell, _, _| assert!(!shell.hold.on_hold))
            .unwrap();
        second
            .update(cx, |shell, _, _| assert!(!shell.hold.on_hold))
            .unwrap();
    }

    #[gpui::test]
    fn the_action_opens_a_window_from_inside_the_window_it_was_pressed_in(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        store.create("codex", CWD).unwrap();
        let first = first_window(store, cx);
        cx.update(crate::app_menu::bind);
        cx.dispatch_action(first.into(), NewWindow);
        cx.run_until_parked();
        assert_eq!(windows(cx).len(), 2);
    }

    #[gpui::test]
    fn a_new_primary_on_its_own_store_does_not_inherit_the_old_ones_chats(cx: &mut TestAppContext) {
        let open = |cx: &mut TestAppContext| {
            let store = Store::open(Path::new(":memory:")).unwrap();
            let id = store.create("codex", CWD).unwrap().id;
            (first_window(store, cx), id)
        };
        let (_, id) = open(cx);
        let (second, same_id) = open(cx);
        assert_eq!(id, same_id, "each store numbers its chats from the start");
        second
            .update(cx, |shell, _, _| assert!(shell.sessions.contains_key(&id)))
            .unwrap();
    }

    #[gpui::test]
    fn the_shortcut_opens_a_window_from_one_keymap_entry(cx: &mut TestAppContext) {
        cx.update(crate::shell::bind_keys);
        cx.update(|cx| {
            let keymap = cx.key_bindings();
            let keymap = keymap.borrow();
            let chord: Vec<_> = keymap
                .bindings()
                .filter(|binding| {
                    let keys = binding.keystrokes()[0].unparse();
                    keys == "cmd-shift-n" || keys == "ctrl-shift-n"
                })
                .map(|binding| binding.action().name())
                .collect();
            assert_eq!(chord, vec!["workspace::NewWindow"]);
        });
    }

    #[test]
    fn file_and_the_dock_offer_new_window() {
        let prefs = crate::settings::Preferences::default();
        let named =
            |item: &MenuItem| matches!(item, MenuItem::Action { name, .. } if name == "New Window");
        for mac in [true, false] {
            let menus = crate::app_menu::menus(&prefs, mac);
            let file = menus.iter().find(|menu| menu.name == "File").unwrap();
            assert!(file.items.iter().any(named));
        }
        assert!(crate::app_menu::dock_menu().iter().any(named));
    }

    #[test]
    fn windows_share_settings_but_not_what_they_show() {
        let mut one = Workspace {
            tabs: vec![1],
            active: Some(1),
            project: "/a".into(),
            projects: vec!["/a".into()],
            ..Default::default()
        };
        let mut two = one.for_new_window();
        assert!(two.tabs.is_empty() && two.active.is_none());
        assert_eq!(two.project, "/a");
        two.tabs = vec![2];
        two.active = Some(2);
        two.project = "/b".into();
        two.projects.push("/b".into());
        one.adopt_shared(&two);
        assert_eq!(one.projects, vec!["/a".to_string(), "/b".into()]);
        assert_eq!((one.tabs.clone(), one.active), (vec![1], Some(1)));
        assert_eq!(one.project, "/a");
        // Two windows differing only in what they show agree.
        let same = |w: &Workspace| serde_json::to_string(&w.shared_part()).unwrap();
        assert_eq!(same(&one), same(&two));
    }
}
