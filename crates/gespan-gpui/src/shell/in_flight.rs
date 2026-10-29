//! Chats with a running turn survive a quit, a crash or an update restart:
//! the list is kept current while they run, and the next launch marks each
//! one interrupted and continues it (as `inFlight.ts` does in the Tauri app).
use super::*;

impl Shell {
    /// Local chats with a turn running, in sidebar order. Remote turns belong to
    /// their host; Pilot chats are held and resumed by Pilots.
    pub(super) fn running_chats(&self, cx: &App) -> Vec<i64> {
        let busy = |id: &i64| {
            self.sessions
                .get(id)
                .is_some_and(|view| view.read(cx).busy && !view.read(cx).is_remote())
        };
        let mut ids: Vec<i64> = self.order.iter().copied().filter(busy).collect();
        let mut rest: Vec<i64> = self
            .sessions
            .keys()
            .copied()
            .filter(|id| !ids.contains(id) && busy(id))
            .collect();
        rest.sort_unstable();
        ids.extend(rest);
        ids
    }

    /// Called whenever a chat changes. Frozen once a quit starts, so turns the
    /// shutdown stops are not taken off the list.
    pub(super) fn sync_in_flight(&mut self, cx: &App) {
        if crate::quitting(cx) {
            return;
        }
        let ids = self.running_chats(cx);
        if self.in_flight_written.as_ref() == Some(&ids) {
            return;
        }
        match locked(&self.store).and_then(|store| store.set_in_flight(&ids)) {
            Ok(()) => self.in_flight_written = Some(ids),
            Err(error) => tracing::warn!(%error, "cannot record running chats"),
        }
    }

    /// At launch: end each listed chat's turn as interrupted, then continue it
    /// unless Gespan is on hold (Resume continues held chats itself).
    pub(super) fn resume_interrupted(&mut self, cx: &mut Context<Self>) {
        let ids = match locked(&self.store).and_then(|store| store.take_in_flight()) {
            Ok(ids) => ids,
            Err(error) => {
                tracing::warn!(%error, "cannot read chats interrupted by the last quit");
                return;
            }
        };
        let now = gespan_core::time::now_ms();
        let mut marked = Vec::new();
        for id in ids {
            let Some(summary) = self.session_summaries.get(&id) else {
                continue;
            };
            // Continuing needs the provider's thread; without one the agent
            // would start over with no context.
            let resumable = summary.provider_id.is_some();
            match locked(&self.store).and_then(|mut store| store.mark_turn_interrupted(id, now)) {
                Ok(_) if resumable => marked.push(id),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, session = id, "cannot mark interrupted chat"),
            }
        }
        if crate::hold::active(cx) {
            return;
        }
        for id in marked {
            if let Some(view) = self.ensure_session(id, cx) {
                view.update(cx, |view, cx| view.continue_after_quit(cx));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::interrupted::{CONTINUE, MESSAGE};
    use gespan_core::store::Store;
    use gespan_core::transcript::Row;
    use gpui::{TestAppContext, WindowHandle};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    const CWD: &str = "/missing/in-flight";

    fn row(role: &str, text: &str) -> Row {
        Row {
            role: role.into(),
            text: text.into(),
            first: true,
            sent_at: None,
        }
    }

    /// A chat the agent was answering, with a provider thread to continue.
    fn cut_off_chat(store: &mut Store) -> i64 {
        let id = store.create("codex", CWD).unwrap().id;
        store
            .write_rows(
                id,
                0,
                &[
                    row("user", "Fix the tests"),
                    row("assistant", "Running them"),
                ],
            )
            .unwrap();
        store.provider_id(id, "thread-1").unwrap();
        id
    }

    fn shell(store: Store, cx: &mut TestAppContext) -> WindowHandle<Shell> {
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

    fn rows(window: WindowHandle<Shell>, id: i64, cx: &mut TestAppContext) -> Vec<Row> {
        window
            .update(cx, |shell, _, _| {
                locked(&shell.store).unwrap().load(id).unwrap().rows
            })
            .unwrap()
    }

    fn taken(window: WindowHandle<Shell>, cx: &mut TestAppContext) -> Vec<i64> {
        window
            .update(cx, |shell, _, _| {
                locked(&shell.store).unwrap().take_in_flight().unwrap()
            })
            .unwrap()
    }

    fn set_busy(window: WindowHandle<Shell>, id: i64, busy: bool, cx: &mut TestAppContext) {
        window
            .update(cx, |shell, _, cx| {
                let view = shell.ensure_session(id, cx).unwrap();
                view.update(cx, |view, cx| {
                    view.busy = busy;
                    cx.emit(SessionChanged);
                });
            })
            .unwrap();
        cx.run_until_parked();
    }

    fn user_rows(rows: &[Row]) -> Vec<&str> {
        rows.iter()
            .filter(|row| row.role == "user")
            .map(|row| row.text.as_str())
            .collect()
    }

    #[gpui::test]
    fn a_running_chat_is_recorded_and_cleared_when_its_turn_ends(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let a = store.create("codex", CWD).unwrap().id;
        let b = store.create("codex", CWD).unwrap().id;
        let window = shell(store, cx);
        set_busy(window, a, true, cx);
        set_busy(window, b, true, cx);
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.running_chats(cx).len(), 2)
            })
            .unwrap();
        set_busy(window, a, false, cx);
        assert_eq!(taken(window, cx), vec![b]);
        // A finished turn writes the now-empty list.
        set_busy(window, b, false, cx);
        window
            .update(cx, |shell, _, _| {
                locked(&shell.store).unwrap().set_in_flight(&[a]).unwrap();
            })
            .unwrap();
        set_busy(window, b, true, cx);
        set_busy(window, b, false, cx);
        assert!(taken(window, cx).is_empty());
    }

    #[gpui::test]
    fn the_list_is_frozen_once_a_quit_starts(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let id = store.create("codex", CWD).unwrap().id;
        let window = shell(store, cx);
        set_busy(window, id, true, cx);
        cx.update(|cx| cx.set_global(crate::Quitting));
        // The shutdown ends the turn; it must stay listed for the next launch.
        set_busy(window, id, false, cx);
        assert_eq!(taken(window, cx), vec![id]);
    }

    #[gpui::test]
    fn an_interrupted_chat_is_marked_and_continued_at_launch(cx: &mut TestAppContext) {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let id = cut_off_chat(&mut store);
        store.set_in_flight(&[id]).unwrap();
        let window = shell(store, cx);
        let rows = rows(window, id, cx);
        let marker = rows
            .iter()
            .position(|row| row.role == "system" && row.text == MESSAGE);
        let marker = marker.expect("the cut-off turn is marked");
        assert_eq!(rows[marker - 1].text, "Running them");
        assert_eq!(user_rows(&rows[marker..]), vec![CONTINUE]);
        assert!(taken(window, cx).is_empty(), "the list is taken once");
    }

    #[gpui::test]
    fn a_chat_is_continued_only_once(cx: &mut TestAppContext) {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let id = cut_off_chat(&mut store);
        store.set_in_flight(&[id]).unwrap();
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, cx| {
                // A second resume with the chat listed again sends nothing more.
                locked(&shell.store).unwrap().set_in_flight(&[id]).unwrap();
                shell.resume_interrupted(cx);
            })
            .unwrap();
        cx.run_until_parked();
        let rows = rows(window, id, cx);
        assert_eq!(user_rows(&rows), vec!["Fix the tests", CONTINUE]);
    }

    #[gpui::test]
    fn on_hold_an_interrupted_chat_is_marked_but_waits_for_resume(cx: &mut TestAppContext) {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let id = cut_off_chat(&mut store);
        store.set_in_flight(&[id]).unwrap();
        let mut hold = gespan_core::hold::HoldState::new(1);
        hold.track(id, false);
        store.save_hold(&hold).unwrap();
        let window = shell(store, cx);
        let rows = rows(window, id, cx);
        assert!(gespan_core::interrupted::ends_interrupted(&rows));
        assert_eq!(user_rows(&rows), vec!["Fix the tests"]);
    }

    #[gpui::test]
    fn a_chat_with_no_provider_thread_is_marked_but_not_continued(cx: &mut TestAppContext) {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let id = store.create("codex", CWD).unwrap().id;
        store.write_rows(id, 0, &[row("user", "Hello")]).unwrap();
        store.set_in_flight(&[id]).unwrap();
        let window = shell(store, cx);
        let rows = rows(window, id, cx);
        assert!(gespan_core::interrupted::ends_interrupted(&rows));
        assert_eq!(user_rows(&rows), vec!["Hello"]);
    }

    #[gpui::test]
    fn quitting_with_a_running_chat_asks_first(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let id = store.create("codex", CWD).unwrap().id;
        let window = shell(store, cx);
        set_busy(window, id, true, cx);
        window
            .update(cx, |shell, window, cx| shell.request_quit(window, cx))
            .unwrap();
        cx.run_until_parked();
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert!(
            !cx.update(|cx| crate::quitting(cx)),
            "Cancel keeps Gespan open"
        );
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.running_chats(cx), vec![id])
            })
            .unwrap();
    }

    #[gpui::test]
    fn quitting_with_nothing_running_does_not_ask(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        store.create("codex", CWD).unwrap();
        let window = shell(store, cx);
        window
            .update(cx, |shell, window, cx| shell.request_quit(window, cx))
            .unwrap();
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt());
        assert!(cx.update(|cx| crate::quitting(cx)));
    }

    #[gpui::test]
    fn closing_the_window_with_a_running_chat_asks_first(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let id = store.create("codex", CWD).unwrap().id;
        let window = shell(store, cx);
        // Windows hides to the tray instead unless Close to tray is off.
        window
            .update(cx, |shell, _, _| {
                shell.workspace.preferences.close_to_tray = false
            })
            .unwrap();
        set_busy(window, id, true, cx);
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let closed = visual.simulate_close();
        visual.run_until_parked();
        assert!(!closed, "the window stays open until the user answers");
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
    }

    #[cfg(windows)]
    #[gpui::test]
    fn with_close_to_tray_closing_hides_without_asking(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let id = store.create("codex", CWD).unwrap().id;
        let window = shell(store, cx);
        window
            .update(cx, |shell, _, _| {
                shell.workspace.preferences.close_to_tray = true
            })
            .unwrap();
        set_busy(window, id, true, cx);
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let closed = visual.simulate_close();
        visual.run_until_parked();
        assert!(!closed, "the window is hidden, not closed");
        assert!(!cx.has_pending_prompt(), "hiding asks nothing");
        assert!(
            !cx.update(|cx| crate::quitting(cx)),
            "and Gespan keeps running"
        );
    }
}
