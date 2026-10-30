//! Ask conversations. Each is a Conversation the shell owns, so background
//! running, accounts and approvals work as in any session, but it has no tab
//! or sidebar row: it stays out of `order` and `session_summaries`, and the
//! inbox renders the entity.
use super::*;
use gespan_core::inbox::{Item, ask};

type Opened = Result<(Session, ask::Context), String>;

impl Shell {
    pub(super) fn open_inbox_ask(
        &mut self,
        item: Item,
        description: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let cwd = item.project_path.clone();
        let (kind, settings) = self.inbox_session_settings(&cwd, cx);
        let context = ask::Context::new(&item, description);
        let key = context.key.clone();
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            if !std::path::Path::new(&cwd).is_dir() {
                return Err("Choose a project folder before asking about this item".into());
            }
            locked(&store)?.open_inbox_ask(kind.id(), &cwd, &context, &settings)
        });
        cx.spawn(async move |this, cx| {
            let opened = work.await;
            let _ = this.update(cx, |this, cx| this.host_inbox_ask(key, opened, cx));
        })
        .detach();
    }

    /// Stops and deletes the session, then hosts an empty one in its place.
    pub(super) fn restart_inbox_ask(&mut self, key: String, id: i64, cx: &mut Context<Self>) {
        if let Some(view) = self.sessions.get(&id) {
            view.update(cx, |view, cx| view.stop(cx));
        }
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let fresh = locked(&store)?.restart_inbox_ask(id)?;
            // Drop the old session's recorded turns and their Git refs.
            if let Some(checkpoints) = gespan_core::checkpoint::store() {
                let _ = checkpoints.exclusive(|c| c.forget(&id.to_string()));
            }
            Ok(fresh)
        });
        cx.spawn(async move |this, cx| {
            let opened: Opened = work.await;
            let _ = this.update(cx, |this, cx| {
                if opened.is_ok() {
                    this.forget_session(id, cx);
                }
                this.host_inbox_ask(key, opened, cx);
            });
        })
        .detach();
    }

    fn host_inbox_ask(&mut self, key: String, opened: Opened, cx: &mut Context<Self>) {
        let Some(inbox) = self.inbox.clone() else {
            return;
        };
        let (summary, context) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                inbox.update(cx, |inbox, cx| inbox.ask_failed(key, error, cx));
                return;
            }
        };
        let id = summary.id;
        if !self.sessions.contains_key(&id) {
            self.attach_conversation(summary, cx);
        }
        let view = self.sessions[&id].clone();
        let classic = self.workspace.boxy_layout();
        view.update(cx, |view, cx| {
            view.set_inbox_ask(context);
            view.set_classic_layout(classic, cx);
            view.set_beside_pane(true, cx);
            view.load(cx);
        });
        inbox.update(cx, |inbox, cx| inbox.host_ask(key, id, view, cx));
        self.update_chat_visibility(cx);
    }

    /// A notice from an Ask conversation opens its item, as it has no tab.
    pub(super) fn reveal_inbox_ask(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(key) = self
            .sessions
            .get(&id)
            .and_then(|view| view.read(cx).inbox_ask_key())
        else {
            return false;
        };
        self.open_inbox(window, cx);
        if let Some(inbox) = &self.inbox {
            inbox.update(cx, |inbox, cx| inbox.reveal_ask(&key, cx));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{conversation::turn_capture, inbox::Inbox, shell::app_views::AppView};
    use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px};

    /// A real folder on every platform, gone with the test.
    struct Project(std::path::PathBuf);
    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn project() -> Project {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("gespan-inbox-ask-{}-{number}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Project(dir)
    }

    fn item(id: &str, project: &Project) -> Item {
        Item {
            provider: "linear".into(),
            kind: "issue".into(),
            id: id.into(),
            identifier: id.to_uppercase(),
            title: format!("Item {id}"),
            url: format!("https://linear.app/acme/issue/{id}"),
            state: "open".into(),
            project_path: project.0.to_string_lossy().into_owned(),
            ..Default::default()
        }
    }

    struct Fixture {
        window: gpui::WindowHandle<Shell>,
        inbox: Entity<Inbox>,
        project: Project,
    }

    /// The real shell and inbox on screen, with no provider behind them.
    fn fixture(cx: &mut TestAppContext) -> Fixture {
        let (window, _, _) = crate::shell::tests::window(cx);
        let project = project();
        let items = vec![item("eng-1", &project), item("eng-2", &project)];
        let inbox = window
            .update(cx, |shell, window, cx| {
                let cwd = std::path::PathBuf::from(&shell.workspace.project);
                let store = shell.store.clone();
                let inbox = cx.new(|cx| Inbox::offline_with(store, cwd, items, cx));
                shell.adopt_inbox(inbox.clone(), window, cx);
                shell.prepare_app_view(window, cx);
                shell.show_app_view(AppView::Inbox, window, cx);
                inbox
            })
            .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        Fixture {
            window,
            inbox,
            project,
        }
    }
    impl Fixture {
        fn click(&self, selector: &'static str, cx: &mut TestAppContext) {
            let mut visual = VisualTestContext::from_window(self.window.into(), cx);
            visual.run_until_parked();
            let bounds = visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is not on screen"));
            visual.simulate_click(bounds.center(), Modifiers::default());
            visual.run_until_parked();
        }
        fn shown(&self, selector: &'static str, cx: &mut TestAppContext) -> bool {
            let mut visual = VisualTestContext::from_window(self.window.into(), cx);
            visual.run_until_parked();
            visual.debug_bounds(selector).is_some()
        }
        fn conversation(&self, cx: &TestAppContext) -> Entity<Conversation> {
            self.inbox.read_with(cx, |inbox, _| {
                inbox
                    .ask_conversation()
                    .unwrap_or_else(|| panic!("nothing hosted: {:?}", inbox.ask_error()))
            })
        }
        fn session(&self, cx: &TestAppContext) -> i64 {
            self.conversation(cx)
                .read_with(cx, |view, _| view.summary.id)
        }
        fn open_ask(&self, cx: &mut TestAppContext) {
            self.click("inbox-ask", cx);
        }
        fn select(&self, id: &str, cx: &mut TestAppContext) {
            let key = item(id, &self.project).key();
            self.inbox
                .update(cx, |inbox, cx| inbox.select_for_test(key, cx));
            cx.run_until_parked();
        }
        fn send(&self, text: &str, cx: &mut TestAppContext) {
            let view = self.conversation(cx);
            view.update(cx, |view, cx| {
                // Fails after saving the turn, without launching a provider.
                view.summary.provider = "fixture".into();
                view.input
                    .update(cx, |input, cx| input.set_text(text.to_owned(), cx));
                view.send_for_test(cx);
            });
            cx.run_until_parked();
        }
        fn hidden_from_the_shell(&self, id: i64, cx: &mut TestAppContext) {
            self.window
                .update(cx, |shell, _, _| {
                    assert!(!shell.order.contains(&id), "no sidebar row");
                    assert!(!shell.session_summaries.contains_key(&id));
                    assert!(shell.workspace.tab_of(id).is_none(), "no tab");
                    assert!(
                        !shell.workspace.open_sessions().contains(&id),
                        "not an open session"
                    );
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn the_ask_button_opens_a_panel_on_a_conversation_outside_the_session_list(
        cx: &mut TestAppContext,
    ) {
        let f = fixture(cx);
        assert!(!f.shown("inbox-ask-panel", cx));
        let (visible_before, tabs_before) = f
            .window
            .update(cx, |shell, _, _| {
                (shell.order.clone(), shell.workspace.tabs.clone())
            })
            .unwrap();

        f.open_ask(cx);

        assert!(f.shown("inbox-ask-panel", cx));
        let view = f.conversation(cx);
        let id = view.read_with(cx, |view, _| {
            assert_eq!(view.summary.title, "Ask · Item eng-1");
            assert_eq!(view.summary.cwd, f.project.0.to_string_lossy());
            view.summary.id
        });
        f.hidden_from_the_shell(id, cx);
        f.window
            .update(cx, |shell, _, _| {
                assert!(shell.sessions.contains_key(&id), "the shell owns it");
                assert_eq!(shell.order, visible_before);
                assert_eq!(shell.workspace.tabs, tabs_before);
                let store = locked(&shell.store).unwrap();
                assert!(store.sessions().unwrap().iter().all(|s| s.id != id));
            })
            .unwrap();
        // The details loaded for the selected item travel with the context.
        let key = item("eng-1", &f.project);
        let saved = f
            .window
            .update(cx, |shell, _, _| {
                locked(&shell.store)
                    .unwrap()
                    .inbox_ask(&ask::key(&key))
                    .unwrap()
                    .unwrap()
                    .1
            })
            .unwrap();
        assert_eq!(saved.description.as_deref(), Some("Body of Item eng-1"));
    }

    #[gpui::test]
    fn asking_again_returns_to_the_same_live_conversation(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.open_ask(cx);
        let first = f.conversation(cx);
        f.click("inbox-ask-close", cx);
        f.open_ask(cx);
        assert_eq!(f.conversation(cx), first);
    }

    #[gpui::test]
    fn selecting_another_item_switches_conversation_and_comes_back(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.open_ask(cx);
        let first = f.session(cx);

        f.select("eng-2", cx);
        let second = f.session(cx);
        assert_ne!(first, second);
        f.conversation(cx).read_with(cx, |view, _| {
            assert_eq!(view.summary.title, "Ask · Item eng-2")
        });
        f.hidden_from_the_shell(second, cx);

        f.select("eng-1", cx);
        assert_eq!(f.session(cx), first, "an item keeps its own conversation");
    }

    #[gpui::test]
    fn a_late_answer_for_the_previous_item_does_not_take_over_the_panel(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.open_ask(cx);
        let first = f.conversation(cx);
        f.select("eng-2", cx);
        let second = f.session(cx);
        let late = ask::key(&item("eng-1", &f.project));
        f.inbox.update(cx, |inbox, cx| {
            inbox.host_ask(late.clone(), 1, first, cx);
            inbox.ask_failed(late, "too late".into(), cx);
        });
        assert_eq!(f.session(cx), second);
        f.inbox
            .read_with(cx, |inbox, _| assert!(inbox.ask_error().is_none()));
    }

    #[gpui::test]
    fn restart_gives_an_empty_conversation_and_the_panel_stays_open(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.open_ask(cx);
        let old = f.session(cx);
        let old_view = f.conversation(cx).entity_id();
        f.send("What broke?", cx);
        f.conversation(cx).read_with(cx, |view, cx| {
            assert!(!view.transcript.read(cx).source_rows().is_empty())
        });

        // The send above swapped in a provider that cannot start.
        f.window
            .update(cx, |shell, _, _| {
                let store = locked(&shell.store).unwrap();
                store.change_provider(old, "codex").unwrap();
            })
            .unwrap();
        f.click("inbox-ask-restart", cx);

        assert!(f.shown("inbox-ask-panel", cx));
        let fresh = f.session(cx);
        // A store may hand a deleted id out again, so the view tells them apart.
        assert_ne!(f.conversation(cx).entity_id(), old_view);
        f.conversation(cx).read_with(cx, |view, cx| {
            assert!(view.transcript.read(cx).source_rows().is_empty());
            assert_eq!(view.summary.title, "Ask · Item eng-1");
            assert!(!view.busy);
        });
        f.hidden_from_the_shell(fresh, cx);
        f.window
            .update(cx, |shell, _, _| {
                let hosted = shell.sessions.get(&fresh).map(|view| view.entity_id());
                assert_ne!(hosted, Some(old_view), "the old entity is gone");
                let store = locked(&shell.store).unwrap();
                let key = ask::key(&item("eng-1", &f.project));
                assert_eq!(store.inbox_ask(&key).unwrap().unwrap().0.id, fresh);
            })
            .unwrap();
        // The context survives the restart, so the agent still gets it.
        f.send("Again", cx);
        let (_, prompt) = turn_capture::last().unwrap();
        assert!(prompt.contains("Body of Item eng-1"));
    }

    #[gpui::test]
    fn close_hides_the_panel_and_keeps_the_conversation_alive(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.open_ask(cx);
        let id = f.session(cx);

        f.click("inbox-ask-close", cx);

        assert!(!f.shown("inbox-ask-panel", cx));
        f.inbox.read_with(cx, |inbox, _| {
            assert!(!inbox.ask_open());
            assert!(inbox.shown_ask_session().is_none());
        });
        f.window
            .update(cx, |shell, _, _| assert!(shell.sessions.contains_key(&id)))
            .unwrap();
    }

    #[gpui::test]
    fn the_transcript_is_only_visible_while_the_panel_shows_it(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.open_ask(cx);
        let view = f.conversation(cx);
        view.read_with(cx, |view, cx| {
            assert!(view.transcript.read(cx).is_visible())
        });
        f.click("inbox-ask-close", cx);
        view.read_with(cx, |view, cx| {
            assert!(!view.transcript.read(cx).is_visible())
        });
    }

    #[gpui::test]
    fn dragging_the_left_edge_resizes_the_panel_between_its_limits(cx: &mut TestAppContext) {
        let f = fixture(cx);
        let mut visual = VisualTestContext::from_window(f.window.into(), cx);
        visual.simulate_resize(gpui::size(px(1600.), px(900.)));
        visual.run_until_parked();
        f.open_ask(&mut visual);
        visual.run_until_parked();
        let drag_by = |visual: &mut VisualTestContext, to: fn(f32) -> f32| {
            let handle = visual.debug_bounds("inbox-ask-resize").unwrap().center();
            let end = point(px(to(f32::from(handle.x))), handle.y);
            visual.simulate_mouse_down(handle, MouseButton::Left, Modifiers::default());
            visual.run_until_parked();
            visual.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
            visual.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
            visual.run_until_parked();
        };
        let width = |visual: &VisualTestContext| f.inbox.read_with(visual, |i, _| i.ask_width());

        drag_by(&mut visual, |x| x - 100.);
        assert_eq!(width(&visual), 540.);

        drag_by(&mut visual, |_| 0.);
        assert_eq!(width(&visual), 950., "1600 less the 650 kept free");

        drag_by(&mut visual, |_| 1590.);
        assert_eq!(width(&visual), 360.);

        // The drag ended, so a later move leaves the width alone.
        let handle = visual.debug_bounds("inbox-ask-resize").unwrap().center();
        visual.simulate_mouse_move(point(px(600.), handle.y), None, Modifiers::default());
        visual.run_until_parked();
        assert_eq!(width(&visual), 360.);
    }

    #[gpui::test]
    fn a_sent_message_keeps_what_was_typed_and_the_agent_gets_the_wrapper(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.open_ask(cx);
        turn_capture::start();

        f.send("What should I check first?", cx);

        let view = f.conversation(cx);
        view.read_with(cx, |view, cx| {
            let rows = view.transcript.read(cx).source_rows();
            let prompts: Vec<_> = rows
                .iter()
                .filter(|row| row.is_prompt())
                .map(|row| row.text.as_str())
                .collect();
            assert_eq!(prompts, ["What should I check first?"]);
        });
        let (id, prompt) = turn_capture::last().expect("a turn was launched");
        assert_eq!(id, view.read_with(cx, |view, _| view.summary.id));
        assert!(prompt.starts_with("# Inbox discussion instructions"));
        assert!(prompt.contains("INBOX ITEM (reference data):\n{\"key\":\"linear:eng-1\""));
        assert!(prompt.ends_with("USER MESSAGE:\nWhat should I check first?"));
    }

    #[gpui::test]
    fn escape_in_the_ask_composer_keeps_the_inbox_open(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.open_ask(cx);
        let mut visual = VisualTestContext::from_window(f.window.into(), cx);
        visual.run_until_parked();
        let input = f
            .conversation(&visual)
            .read_with(&visual, |view, cx| view.input.read(cx).focus_handle(cx));
        visual.update(|window, _| assert!(input.is_focused(window), "Ask focuses its composer"));

        visual.simulate_keystrokes("escape");
        visual.run_until_parked();

        f.window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.active_app_view, Some(AppView::Inbox))
            })
            .unwrap();
    }

    #[gpui::test]
    fn ask_conversations_stay_out_of_app_search(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.open_ask(cx);
        let id = f.session(cx);
        f.window
            .update(cx, |shell, window, cx| {
                shell.open_app_search(window, cx);
                shell.sync_search_session(id, cx);
                let search = shell.app_search.as_ref().unwrap().view.read(cx);
                assert!(!search.has_session_for_test(id));
            })
            .unwrap();
    }

    #[gpui::test]
    fn ordinary_sessions_are_not_wrapped(cx: &mut TestAppContext) {
        let f = fixture(cx);
        turn_capture::start();
        let plain = f
            .window
            .update(cx, |shell, _, _| {
                shell.sessions.values().next().unwrap().clone()
            })
            .unwrap();
        plain.update(cx, |view, cx| {
            view.summary.provider = "fixture".into();
            view.set_loaded_for_test();
            view.input
                .update(cx, |input, cx| input.set_text("Plain".into(), cx));
            view.send_for_test(cx);
        });
        cx.run_until_parked();
        assert_eq!(turn_capture::last().unwrap().1, "Plain");
    }

    #[gpui::test]
    fn a_failed_open_shows_the_error_and_restart_retries_it(cx: &mut TestAppContext) {
        let f = fixture(cx);
        let missing = item("eng-1", &f.project);
        std::fs::remove_dir_all(&f.project.0).unwrap();
        f.open_ask(cx);
        let error = f.inbox.read_with(cx, |inbox, _| inbox.ask_error());
        assert_eq!(
            error.as_deref(),
            Some("Choose a project folder before asking about this item")
        );
        assert!(f.shown("inbox-ask-panel", cx));

        std::fs::create_dir_all(&f.project.0).unwrap();
        f.click("inbox-ask-restart", cx);
        assert_eq!(
            f.conversation(cx)
                .read_with(cx, |view, _| view.summary.title.clone()),
            format!("Ask · {}", missing.title)
        );
    }

    #[gpui::test]
    fn a_notice_from_an_ask_conversation_reopens_its_item(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.open_ask(cx);
        let first = f.session(cx);
        f.select("eng-2", cx);
        f.click("inbox-ask-close", cx);

        f.window
            .update(cx, |shell, window, cx| {
                assert!(shell.reveal_inbox_ask(first, window, cx));
                assert!(!shell.reveal_inbox_ask(i64::MAX, window, cx));
            })
            .unwrap();
        cx.run_until_parked();

        assert_eq!(f.session(cx), first);
        f.window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.active_app_view, Some(AppView::Inbox))
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_tracker_item_waits_for_its_description_before_it_opens(cx: &mut TestAppContext) {
        let f = fixture(cx);
        f.inbox
            .update(cx, |inbox, cx| inbox.hold_details_for_test(cx));
        f.select("eng-2", cx);
        f.open_ask(cx);
        f.inbox
            .read_with(cx, |inbox, _| assert!(inbox.ask_conversation().is_none()));

        f.inbox
            .update(cx, |inbox, cx| inbox.finish_details_for_test(cx));
        cx.run_until_parked();

        let key = ask::key(&item("eng-2", &f.project));
        let saved = f
            .window
            .update(cx, |shell, _, _| {
                locked(&shell.store).unwrap().inbox_ask(&key).unwrap()
            })
            .unwrap();
        assert_eq!(
            saved.unwrap().1.description.as_deref(),
            Some("Body of Item eng-2")
        );
    }
}
