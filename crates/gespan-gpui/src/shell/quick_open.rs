use super::*;
use crate::quick_open::{Command, Picked, QuickOpen};
pub(super) struct Open {
    picker: Entity<QuickOpen>,
    restore: Option<FocusHandle>,
    _events: Subscription,
}
impl Shell {
    pub(super) fn close_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.quick_open.take() {
            if let Some(restore) = open.restore {
                window.focus(&restore, cx);
            } else {
                self.focus_active(window, cx);
            }
        }
        cx.notify();
    }
    pub(super) fn open_quick_open(
        &mut self,
        palette: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let restore = self
            .quick_open
            .as_ref()
            .and_then(|o| o.restore.clone())
            .or_else(|| window.focused(cx));
        self.close_menu(cx);
        self.close_preview(cx);
        let picker = cx.new(|cx| QuickOpen::new(self.working_copy().to_owned(), palette, cx));
        let events = cx.subscribe_in(&picker, window, |this, _, event: &Picked, window, cx| {
            this.close_quick_open(window, cx);
            match event {
                Picked::File(path) => {
                    let cwd = this.working_copy().to_owned();
                    this.open_review(
                        crate::review::ReviewTab::File {
                            cwd,
                            path: path.clone(),
                        },
                        window,
                        cx,
                    );
                }
                Picked::Command(command) => match command {
                    Command::New => window.dispatch_action(Box::new(NewCodex), cx),
                    Command::Project => window.dispatch_action(Box::new(OpenProject), cx),
                    Command::Model => window.dispatch_action(Box::new(SwitchModel), cx),
                    Command::Settings => window.dispatch_action(Box::new(OpenSettings), cx),
                    Command::Sidebar => window.dispatch_action(Box::new(ToggleSidebar), cx),
                    Command::Panel => window.dispatch_action(Box::new(ToggleRightPanel), cx),
                    Command::Terminal => window.dispatch_action(Box::new(ToggleTerminal), cx),
                    Command::Layout => window.dispatch_action(Box::new(ToggleLayout), cx),
                    Command::LinkTicket => this.link_ticket_to_active_session(window, cx),
                    Command::NewSessionFromTicket => this.open_new_ticket_dialog(window, cx),
                    Command::OpenLogs => {
                        let open = cx
                            .background_executor()
                            .spawn(async { gespan_core::diagnostics::open_logs_folder() });
                        cx.spawn(async move |this, cx| {
                            if let Err(error) = open.await {
                                let _ = this.update(cx, |this, cx| {
                                    this.error = Some(format!("Open logs folder: {error}"));
                                    cx.notify();
                                });
                            }
                        })
                        .detach();
                    }
                    Command::ReportIssue => {
                        window.dispatch_action(Box::new(crate::report::ReportIssue), cx)
                    }
                    Command::CopyDiagnostics => {
                        // It scans the log and crash folders: keep that off
                        // the UI thread.
                        let summary = cx
                            .background_executor()
                            .spawn(async { gespan_core::diagnostics::summary() });
                        cx.spawn(async move |_, cx| {
                            let summary = summary.await;
                            cx.update(|cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(summary))
                            })
                        })
                        .detach();
                    }
                    Command::Plan | Command::Operator | Command::Orchestrator => {
                        if let Some(view) =
                            this.workspace.active.and_then(|id| this.sessions.get(&id))
                        {
                            view.update(cx, |v, cx| {
                                v.toggle_mode(
                                    match command {
                                        Command::Plan => "plan",
                                        Command::Operator => "operator",
                                        _ => "orchestrator",
                                    },
                                    cx,
                                )
                            });
                        }
                    }
                },
                Picked::Dismiss => (),
            }
        });
        window.focus(&picker.focus_handle(cx), cx);
        self.quick_open = Some(Open {
            picker,
            restore,
            _events: events,
        });
        cx.notify();
    }
    pub(super) fn quick_open_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let open = self.quick_open.as_ref()?;
        let viewport = window.viewport_size();
        let width = 560f32.min(viewport.width.as_f32() - 32.);
        let anchor = Anchor::Point(gpui::point(
            px((viewport.width.as_f32() - width) / 2.),
            px((viewport.height.as_f32() * 0.16).max(32.)),
        ));
        let weak = cx.weak_entity();
        Some(
            div()
                .absolute()
                .size_full()
                .child(crate::modal::scrim("quick-open-scrim"))
                .child(
                    popover("quick-open-modal", anchor)
                        .modal_surface(self.workspace.boxy_layout())
                        .width(width)
                        .gap(0.)
                        .priority(80)
                        .on_dismiss(move |window, cx| {
                            let _ = weak.update(cx, |this, cx| this.close_quick_open(window, cx));
                        })
                        .child(open.picker.clone()),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tickets::fakes::FakeTickets;
    use gespan_core::{session_groups::WorkItem, tickets::Tracker};
    use gpui::{TestAppContext, WindowHandle};

    #[gpui::test]
    fn titlebar_search_opens_files_and_sidebar_search_opens_global_search(cx: &mut TestAppContext) {
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                let input = shell.sessions[&active].read(cx).input.clone();
                input.update(cx, |input, cx| input.set_text("keep this draft".into(), cx));
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        for classic in [false, true] {
            window
                .update(visual, |shell, _, cx| {
                    shell.workspace.classic_layout = classic;
                    shell.invalidate_chrome(cx);
                    cx.notify();
                })
                .unwrap();
            visual.run_until_parked();
            let search = visual.debug_bounds("titlebar-file-search").unwrap();
            visual.simulate_click(search.center(), Default::default());
            visual.run_until_parked();
            window
                .update(visual, |shell, w, cx| {
                    assert!(shell.quick_open.is_some());
                    assert!(shell.app_search.is_none());
                    assert_eq!(
                        shell.sessions[&active].read(cx).input.read(cx).content,
                        "keep this draft"
                    );
                    shell.close_quick_open(w, cx);
                })
                .unwrap();
            visual.run_until_parked();
            let search = visual.debug_bounds("sidebar-search-field").unwrap();
            visual.simulate_click(search.center(), Default::default());
            visual.run_until_parked();
            window
                .update(visual, |shell, w, cx| {
                    assert!(shell.app_search.is_some());
                    assert!(shell.quick_open.is_none());
                    shell.close_app_search(w, cx);
                })
                .unwrap();
            visual.run_until_parked();
        }
    }
    #[gpui::test]
    fn cmd_p_and_command_palette_preserve_the_composer_draft(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::input::bind(cx);
            super::super::bind_keys(cx);
            crate::popover::bind(cx);
        });
        let (window, _, active) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, w, cx| {
                let input = shell.sessions[&active].read(cx).input.clone();
                input.update(cx, |i, cx| i.set_text("keep this draft".into(), cx));
                w.focus(&input.focus_handle(cx), cx);
            })
            .unwrap();
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.simulate_keystrokes(window.into(), &format!("{modifier}-p"));
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| assert!(shell.quick_open.is_some()))
            .unwrap();
        cx.simulate_keystrokes(window.into(), "escape");
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                assert!(shell.quick_open.is_none());
                let input = &shell.sessions[&active].read(cx).input;
                assert_eq!(input.read(cx).content, "keep this draft");
                assert!(input.focus_handle(cx).is_focused(w));
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), &format!("{modifier}-shift-p"));
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(
                    shell
                        .quick_open
                        .as_ref()
                        .unwrap()
                        .picker
                        .read(cx)
                        .search
                        .read(cx)
                        .content,
                    ">"
                )
            })
            .unwrap();
    }
    #[gpui::test]
    fn palette_opens_the_report_sheet(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::input::bind(cx);
            super::super::bind_keys(cx);
            crate::popover::bind(cx);
        });
        let (window, _, _) = crate::shell::tests::window(cx);
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.simulate_keystrokes(window.into(), &format!("{modifier}-shift-p"));
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                let search = shell
                    .quick_open
                    .as_ref()
                    .unwrap()
                    .picker
                    .read(cx)
                    .search
                    .clone();
                search.update(cx, |input, cx| {
                    input.set_text(">Report an issue".into(), cx)
                });
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "enter");
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert!(shell.quick_open.is_none());
                assert!(shell.report.open.is_some());
            })
            .unwrap();
    }

    #[gpui::test]
    fn palette_copies_diagnostics_and_reports_a_missing_logs_folder(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::input::bind(cx);
            super::super::bind_keys(cx);
            crate::popover::bind(cx);
        });
        let (window, _, _) = crate::shell::tests::window(cx);
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        let run = |query: &str, cx: &mut TestAppContext| {
            cx.simulate_keystrokes(window.into(), &format!("{modifier}-shift-p"));
            cx.run_until_parked();
            window
                .update(cx, |shell, _, cx| {
                    let search = shell
                        .quick_open
                        .as_ref()
                        .unwrap()
                        .picker
                        .read(cx)
                        .search
                        .clone();
                    search.update(cx, |input, cx| input.set_text(query.into(), cx));
                })
                .unwrap();
            cx.run_until_parked();
            cx.simulate_keystrokes(window.into(), "enter");
            cx.run_until_parked();
        };
        run(">Copy diagnostics", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some(gespan_core::diagnostics::summary())
        );
        // Tests never start diagnostics, so the folder action must say why it
        // did nothing rather than fail silently.
        run(">Open logs folder", cx);
        window
            .update(cx, |shell, _, _| {
                assert!(shell.quick_open.is_none());
                let error = shell.error.clone().unwrap_or_default();
                assert!(error.contains("Diagnostics have not started"), "{error}");
            })
            .unwrap();
    }

    fn palette_pick(
        cx: &mut TestAppContext,
        window: WindowHandle<Shell>,
        query: &str,
        connected: &[Tracker],
    ) -> Arc<FakeTickets> {
        let fake = Arc::new(FakeTickets::new(connected));
        let query = query.to_string();
        window
            .update(cx, |shell, w, cx| {
                shell.tickets = fake.clone();
                shell.open_quick_open(true, w, cx);
                shell
                    .quick_open
                    .as_ref()
                    .unwrap()
                    .picker
                    .update(cx, |picker, cx| {
                        picker
                            .search
                            .update(cx, |input, cx| input.set_text(query, cx))
                    });
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                shell
                    .quick_open
                    .as_ref()
                    .unwrap()
                    .picker
                    .update(cx, |picker, cx| picker.pick(0, cx));
            })
            .unwrap();
        cx.run_until_parked();
        fake
    }

    fn start_from_ticket(
        cx: &mut TestAppContext,
        window: WindowHandle<Shell>,
        key: &str,
    ) -> (i64, Option<WorkItem>, String) {
        let key = key.to_string();
        let before = window
            .update(cx, |shell, _, _| shell.session_summaries.len())
            .unwrap();
        window
            .update(cx, |shell, _, cx| {
                let dialog = shell.link_dialog.as_ref().expect("the ticket dialog opens");
                assert_eq!(dialog.id, None, "no session is linked yet");
                dialog.input.update(cx, |input, cx| input.set_text(key, cx));
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| shell.save_link(true, w, cx))
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.session_summaries.len(), before + 1);
                let id = shell.workspace.active.expect("the new session opens");
                let item = shell.session_metadata(id, cx).summary.work_item.clone();
                (id, item, shell.sessions[&id].read(cx).draft_text(cx))
            })
            .unwrap()
    }

    #[gpui::test]
    fn palette_lists_the_ticket_commands(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::shell::bind_keys(cx);
            crate::input::bind(cx);
            crate::popover::bind(cx);
        });
        let (window, _, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.open_quick_open(true, w, cx);
                shell
                    .quick_open
                    .as_ref()
                    .unwrap()
                    .picker
                    .update(cx, |picker, cx| {
                        picker
                            .search
                            .update(cx, |input, cx| input.set_text(">ticket".into(), cx))
                    });
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                let labels = shell.quick_open.as_ref().unwrap().picker.read(cx).labels();
                assert!(labels.contains(&"Link ticket…".to_string()), "{labels:?}");
                assert!(
                    labels.contains(&"New session from ticket…".to_string()),
                    "{labels:?}"
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn palette_link_ticket_opens_the_dialog_for_the_active_session(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::shell::bind_keys(cx);
            crate::input::bind(cx);
            crate::popover::bind(cx);
        });
        let (window, a, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, w, cx| shell.open(a, w, cx))
            .unwrap();
        cx.run_until_parked();
        palette_pick(cx, window, ">link ticket", &[Tracker::Linear]);
        window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.link_dialog.as_ref().unwrap().id, Some(a));
            })
            .unwrap();
    }

    #[gpui::test]
    fn palette_new_session_from_ticket_links_and_drafts_title(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::shell::bind_keys(cx);
            crate::input::bind(cx);
            crate::popover::bind(cx);
        });
        let (window, _, _) = crate::shell::tests::window(cx);
        let fake = palette_pick(cx, window, ">new session from", &[Tracker::Linear]);
        let (id, item, draft) = start_from_ticket(cx, window, "ENG-7");
        let Some(WorkItem::Ticket(ticket)) = item else {
            panic!("the new session links the ticket")
        };
        assert_eq!(
            (ticket.tracker, ticket.key.as_str()),
            (Tracker::Linear, "ENG-7")
        );
        assert_eq!(ticket.title.as_deref(), Some("Fix login"));
        assert_eq!(
            draft, "Fix login",
            "the title replaces the key once fetched"
        );
        assert_eq!(fake.fetch_count(), 1);
        window
            .update(cx, |shell, _, _| {
                let saved = gespan_core::store::Store::sessions(&locked(&shell.store).unwrap())
                    .unwrap()
                    .into_iter()
                    .find(|s| s.id == id)
                    .unwrap();
                assert!(matches!(saved.work_item, Some(WorkItem::Ticket(_))));
            })
            .unwrap();
    }

    #[gpui::test]
    fn new_session_from_ticket_keeps_the_key_draft_when_the_fetch_fails(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::shell::bind_keys(cx);
            crate::input::bind(cx);
            crate::popover::bind(cx);
        });
        let (window, _, _) = crate::shell::tests::window(cx);
        let fake = palette_pick(cx, window, ">new session from", &[Tracker::Linear]);
        *fake.offline.lock().unwrap() = true;
        let (_, item, draft) = start_from_ticket(cx, window, "ENG-7");
        let Some(WorkItem::Ticket(ticket)) = item else {
            panic!("the ticket is still linked")
        };
        assert_eq!(ticket.title, None);
        assert_eq!(draft, "ENG-7");
    }

    #[gpui::test]
    fn new_session_from_ticket_refuses_a_github_url_and_creates_nothing(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::shell::bind_keys(cx);
            crate::input::bind(cx);
            crate::popover::bind(cx);
        });
        let (window, _, _) = crate::shell::tests::window(cx);
        palette_pick(cx, window, ">new session from", &[Tracker::Linear]);
        let before = window
            .update(cx, |shell, _, cx| {
                let before = shell.session_summaries.len();
                shell
                    .link_dialog
                    .as_ref()
                    .unwrap()
                    .input
                    .update(cx, |input, cx| {
                        input.set_text("https://github.com/acme/app/issues/5".into(), cx)
                    });
                before
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| shell.save_link(true, w, cx))
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert!(shell.link_dialog.as_ref().unwrap().error.is_some());
                assert_eq!(shell.session_summaries.len(), before);
            })
            .unwrap();
    }
}
