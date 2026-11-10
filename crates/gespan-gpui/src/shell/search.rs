use super::*;
use crate::search::{Event, SearchView, Sources};
use gespan_core::search::Target;

pub(super) struct Open {
    pub(super) view: Entity<SearchView>,
    restore: Option<FocusHandle>,
    _events: Subscription,
}

impl Shell {
    /// The panel floats over whatever is open, like Quick Open, so nothing
    /// underneath is left or rearranged while searching.
    pub(super) fn search_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let open = self.app_search.as_ref()?;
        let viewport = window.viewport_size();
        let width = 680f32.min(viewport.width.as_f32() - 32.);
        let anchor = Anchor::Point(gpui::point(
            px((viewport.width.as_f32() - width) / 2.),
            px((viewport.height.as_f32() * 0.14).max(32.)),
        ));
        let weak = cx.weak_entity();
        Some(
            div()
                .absolute()
                .size_full()
                .child(crate::modal::scrim("app-search-scrim"))
                .child(
                    popover("app-search-modal", anchor)
                        .modal_surface(self.workspace.boxy_layout())
                        .width(width)
                        .gap(0.)
                        .priority(80)
                        .on_dismiss(move |window, cx| {
                            let _ = weak.update(cx, |this, cx| this.close_app_search(window, cx));
                        })
                        .child(open.view.clone()),
                )
                .into_any_element(),
        )
    }
    pub(super) fn leave_app_search(&mut self, cx: &mut Context<Self>) {
        if let Some(search) = self.app_search.take() {
            self.saved_search = Some(search.view.read(cx).draft(cx));
        }
    }
    pub(super) fn close_app_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.app_search.take() {
            self.saved_search = Some(search.view.read(cx).draft(cx));
            self.update_chat_visibility(cx);
            if let Some(restore) = search.restore {
                window.focus(&restore, cx);
            } else {
                self.focus_active(window, cx);
            }
            if let Some(tab) = self.active_title_tab(cx) {
                self.remember_title_tab(tab);
            }
            self.persist(cx);
            self.invalidate_chrome(cx);
            cx.notify();
        }
    }
    pub(super) fn open_app_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = &self.app_search {
            window.focus(&search.view.focus_handle(cx), cx);
            return;
        }
        self.close_menu(cx);
        if self.quick_open.is_some() {
            self.close_quick_open(window, cx);
        }
        self.navigation_revision += 1;
        let restore = window.focused(cx);
        let conversations = self
            .sessions
            .values()
            .filter(|view| view.read(cx).inbox_ask_key().is_none())
            .cloned()
            .chain(
                self.pilots
                    .as_ref()
                    .map(|m| m.read(cx).search_conversations())
                    .unwrap_or_default(),
            )
            .collect::<Vec<_>>();
        let mut summaries: HashMap<_, _> = self
            .order
            .iter()
            .map(|&id| (id, self.session_metadata(id, cx).summary.clone()))
            .collect();
        summaries.extend(
            conversations
                .iter()
                .map(|s| (s.read(cx).summary.id, s.read(cx).summary.clone())),
        );
        let summaries = summaries.into_values().collect();
        let live = conversations
            .iter()
            // A paged Pilot's saved search covers turns not loaded yet.
            .filter(|s| {
                s.read(cx).history_loaded() && s.read(cx).transcript.read(cx).history_complete()
            })
            .map(|s| {
                (
                    s.read(cx).summary.id,
                    Arc::new(s.read(cx).transcript.read(cx).history_rows().to_vec()),
                )
            })
            .collect();
        let cwd = self.working_copy().to_owned();
        let store = self.store.clone();
        let projects = self.workspace.projects.clone();
        let recent = self
            .workspace
            .recent_files
            .get(&cwd)
            .cloned()
            .unwrap_or_default();
        let view = cx.new(|cx| {
            SearchView::new(
                cwd,
                store,
                Sources {
                    summaries,
                    live,
                    projects,
                    recent,
                    logos: self
                        .workspace
                        .project_appearance
                        .iter()
                        .filter_map(|(path, a)| a.logo.clone().map(|logo| (path.clone(), logo)))
                        .collect(),
                },
                cx,
            )
        });
        if let Some(draft) = self.saved_search.clone() {
            view.update(cx, |v, cx| v.restore_draft(draft, cx));
        }
        let events = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &Event, window, cx| match event {
                Event::Close => this.close_app_search(window, cx),
                Event::Open(target, query) => {
                    this.close_app_search(window, cx);
                    match target {
                        Target::Conversation(id) => this.open_search_session(*id, None, window, cx),
                        Target::Message { session, row } => this.open_search_session(
                            *session,
                            Some((*row, query.clone())),
                            window,
                            cx,
                        ),
                        Target::File { path, location } => {
                            let cwd = this.working_copy().to_owned();
                            this.open_review(
                                crate::review::ReviewTab::File {
                                    cwd,
                                    path: path.clone(),
                                },
                                window,
                                cx,
                            );
                            if let Some((line, column)) = location
                                && let Some(pane) = this.active_review(cx)
                            {
                                pane.update(cx, |p, cx| {
                                    p.navigate_file(*line as usize, *column as usize, cx)
                                });
                            }
                        }
                        Target::Project(path) => this.select_project(path.clone(), window, cx),
                    }
                }
            },
        );
        window.focus(&view.focus_handle(cx), cx);
        self.app_search = Some(Open {
            view,
            restore,
            _events: events,
        });
        cx.notify();
    }
    fn open_search_session(
        &mut self,
        id: i64,
        jump: Option<(usize, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.ensure_session(id, cx) {
            self.open(id, window, cx);
            if let Some((row, query)) = jump {
                view.update(cx, |v, cx| v.reveal_search(row, query, cx));
            }
            return;
        }
        if let Some(pilots) = &self.pilots
            && let Some(view) = pilots.update(cx, |m, cx| m.open_search_session(id, window, cx))
        {
            if let Some((row, query)) = jump {
                view.update(cx, |v, cx| v.reveal_search(row, query, cx));
            }
            return;
        }
        let store = self.store.clone();
        let revision = self.navigation_revision;
        let work = cx.background_executor().spawn(async move {
            locked(&store)?
                .sessions()?
                .into_iter()
                .find(|s| s.id == id)
                .ok_or_else(|| "Conversation no longer exists".to_owned())
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.navigation_revision != revision {
                    return;
                }
                match result {
                    Ok(summary) => {
                        this.add(summary, cx);
                        this.open(id, window, cx);
                        if let Some((row, query)) = jump {
                            this.sessions[&id].update(cx, |v, cx| v.reveal_search(row, query, cx));
                        }
                    }
                    Err(error) => {
                        this.error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    pub(super) fn sync_pilot_search(&mut self, cx: &mut Context<Self>) {
        let Some(search) = &self.app_search else {
            return;
        };
        let views = self
            .pilots
            .as_ref()
            .map(|m| m.read(cx).search_conversations())
            .unwrap_or_default();
        for view in views {
            let transcript = view.read(cx).transcript.read(cx);
            if !transcript.history_complete() {
                continue;
            }
            let rows = Arc::new(transcript.history_rows().to_vec());
            let summary = view.read(cx).summary.clone();
            search
                .view
                .update(cx, |v, cx| v.update_session(summary, rows, cx));
        }
    }
    pub(super) fn sync_search_session(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(search) = &self.app_search else {
            return;
        };
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let session = session.read(cx);
        let transcript = session.transcript.read(cx);
        if !transcript.history_complete() || session.inbox_ask_key().is_some() {
            return;
        }
        let summary = session.summary.clone();
        let rows = Arc::new(transcript.history_rows().to_vec());
        search
            .view
            .update(cx, |v, cx| v.update_session(summary, rows, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    #[gpui::test]
    fn app_search_restores_draft_and_opens_cross_project_result(cx: &mut TestAppContext) {
        cx.update(|cx| {
            input::bind(cx);
            super::super::bind_keys(cx);
            cx.set_reduce_motion(true);
        });
        let (window, a, b) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.open(b, w, cx);
                let input = shell.sessions[&b].read(cx).input.clone();
                input.update(cx, |i, cx| i.set_text("preserve this draft".into(), cx));
                w.focus(&input.focus_handle(cx), cx);
                shell.sessions[&a]
                    .update(cx, |v, _| v.summary.title = "Needle conversation".into());
                shell.focus_search(w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        window
            .update(cx, |shell, w, cx| {
                let view = &shell.app_search.as_ref().unwrap().view;
                assert!(view.focus_handle(cx).is_focused(w));
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "escape");
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                assert!(shell.app_search.is_none());
                let input = &shell.sessions[&b].read(cx).input;
                assert_eq!(input.read(cx).content, "preserve this draft");
                assert!(input.focus_handle(cx).is_focused(w));
                shell.focus_search(w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_input(window.into(), "Needle");
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "enter");
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert!(shell.app_search.is_none());
                assert_eq!(shell.workspace.active, Some(a));
                assert_eq!(shell.workspace.project, "/work/another-project");
                assert_eq!(
                    shell.sessions[&b].read(cx).input.read(cx).content,
                    "preserve this draft"
                );
            })
            .unwrap();
    }
    #[gpui::test]
    fn search_floats_over_the_workspace_without_a_tab(cx: &mut TestAppContext) {
        cx.update(|cx| {
            input::bind(cx);
            super::super::bind_keys(cx);
            cx.set_reduce_motion(true);
        });
        let (window, _, active) = crate::shell::tests::window(cx);
        let tabs = window
            .update(cx, |shell, w, cx| {
                let tabs = shell.visible_tabs(cx);
                shell.open_app_search(w, cx);
                tabs
            })
            .unwrap();
        cx.run_until_parked();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("app-search").is_some());
        assert!(visual.debug_bounds("title-tab-strip").is_some());
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(shell.visible_tabs(cx), tabs);
                assert_eq!(shell.active_title_tab(cx), Some(TitleTab::Session(active)));
            })
            .unwrap();
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(shell.app_search.is_none());
                assert!(shell.saved_search.is_some());
            })
            .unwrap();
    }
    #[gpui::test]
    fn search_over_open_settings_keeps_settings_open(cx: &mut TestAppContext) {
        cx.update(|cx| {
            input::bind(cx);
            super::super::bind_keys(cx);
            cx.set_reduce_motion(true);
        });
        let (window, _, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.open_settings(crate::settings::Section::General, w, cx);
                assert!(shell.settings.is_some());
                shell.open_app_search(w, cx);
                assert!(shell.settings.is_some());
                assert!(shell.app_search.is_some());
            })
            .unwrap();
    }
}
