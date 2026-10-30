//! The application destinations (Inbox, Notes, Automations and Board) share
//! one navigation and focus boundary.
use super::*;
use crate::conversation::composer_card::ComposerCard;
use crate::{automations, board, flows, inbox, notes};

pub(super) use crate::workspace::AppView;

impl Shell {
    pub(super) fn app_view_open(&self) -> bool {
        self.active_app_view.is_some()
    }

    pub(super) fn app_view_label(&self) -> Option<&'static str> {
        self.active_app_view.map(|view| match view {
            AppView::Inbox => "Inbox",
            AppView::Notes => "Notes",
            AppView::Automations => "Automations",
            AppView::Flows => "Flows",
            AppView::Board => "Board",
        })
    }

    pub(super) fn app_title_tab(
        &self,
        view: AppView,
        width: f32,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let active = self.active_app_view == Some(view);
        let close = active || width >= TAB_CLOSE_MIN_WIDTH;
        div()
            .id(SharedString::from(format!(
                "app-title-tab-{}",
                view.label()
            )))
            .debug_selector(move || format!("app-title-tab-{}", view.label()))
            .group("app-title-tab")
            .role(Role::Tab)
            .aria_label(view.label())
            .aria_selected(active)
            .cursor_pointer()
            .w(px(width))
            .h(px(TITLE_BAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .relative()
            .child(
                title_tab_plate(active, false)
                    .text_color(content(if active { 1. } else { 0.5 }))
                    .w_full()
                    .min_w_0()
                    .when(close && width >= TAB_CLOSE_MIN_WIDTH, |d| d.pr_7())
                    .when(width < 30., |d| d.px(px(((width - 14.) / 2.).max(0.))))
                    .gap(px(6.))
                    .group_hover("app-title-tab", |s| {
                        s.text_color(content(1.)).bg(title_tab_fill(active, true))
                    })
                    .child(icon(view.icon(), 14.))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(TITLE_TAB_TEXT_SIZE))
                            .child(view.label()),
                    ),
            )
            .when(close, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!(
                            "close-app-tab-{}",
                            view.label()
                        )))
                        .debug_selector(move || format!("close-app-tab-{}", view.label()))
                        .role(Role::Button)
                        .aria_label(format!("Close {}", view.label()))
                        .absolute()
                        .right_1()
                        .top(px(10.))
                        .size(px(20.))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(0.)
                        .group_hover("app-title-tab", |s| s.opacity(1.))
                        .hover(|s| s.bg(content(0.1)))
                        .child(icon("close", 12.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_title_tabs(&[TitleTab::App(view)], None, window, cx);
                        })),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event, _, cx| {
                    this.start_tab_drag(TitleTab::App(view), event, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.open_menu(
                        MenuTarget::AppTab(view),
                        Anchor::Point(event.position),
                        window,
                        cx,
                    )
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_title_tabs(&[TitleTab::App(view)], None, window, cx);
                }),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if std::mem::take(&mut this.skip_tab_click) {
                    return;
                }
                this.open_title_tab(&TitleTab::App(view), window, cx);
            }))
    }
    pub(super) fn leave_app_views(&mut self, cx: &mut Context<Self>) {
        if self.active_app_view == Some(AppView::Notes)
            && let Some(notes) = &self.notes
        {
            notes.update(cx, |notes, cx| notes.flush(cx));
        }
        if self.active_app_view == Some(AppView::Board)
            && let Some(board) = &self.board
        {
            board.update(cx, |board, cx| board.hidden(cx));
        }
        self.active_app_view = None;
        self.invalidate_chrome(cx);
    }

    pub(super) fn close_app_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_app_views(cx);
        if self.workspace.active.is_none() {
            let previous = self
                .workspace
                .project_focus
                .get(&self.workspace.project)
                .copied()
                .filter(|id| self.workspace.tab_of(*id).is_some());
            if let Some(id) = previous {
                self.activate_session(id, window, cx);
            }
        }
        self.ensure_project_session(window, cx);
        self.update_chat_visibility(cx);
        self.sync_right_panel(cx);
        self.focus_active(window, cx);
        if let Some(tab) = self.active_title_tab(cx) {
            self.remember_title_tab(tab);
        }
        self.persist(cx);
        cx.notify();
    }

    fn prepare_app_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_app_search(cx);
        self.close_preview(cx);
        self.close_menu(cx);
        if self.settings.is_some() {
            self.close_settings(window, cx);
        }
        if let Some(pilots) = self.pilots.clone() {
            pilots.update(cx, |pilots, cx| pilots.close(cx));
        }
        self.navigation_revision += 1;
        self.review_focused = false;
        self.pending_composer_focus = None;
    }

    fn show_app_view(&mut self, view: AppView, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_app_view != Some(view) {
            self.leave_app_views(cx);
        }
        self.active_app_view = Some(view);
        if !self.workspace.classic_layout {
            self.remember_title_tab(TitleTab::App(view));
            self.reveal_active_tab(cx);
        }
        self.update_chat_visibility(cx);
        self.sync_right_panel(cx);
        self.invalidate_chrome(cx);
        self.focus_app_view(window, cx);
        self.persist(cx);
        cx.notify();
    }

    pub(super) fn app_view_body(&self) -> Option<AnyElement> {
        match self.active_app_view? {
            AppView::Notes => self.notes.clone().map(IntoElement::into_any_element),
            AppView::Inbox => self.inbox.clone().map(IntoElement::into_any_element),
            AppView::Automations => self.automations.clone().map(IntoElement::into_any_element),
            AppView::Flows => self.flows.clone().map(IntoElement::into_any_element),
            AppView::Board => self.board.clone().map(IntoElement::into_any_element),
        }
    }

    pub(super) fn focus_app_view(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let focus = match self.active_app_view {
            Some(AppView::Notes) => self.notes.as_ref().map(|view| view.focus_handle(cx)),
            Some(AppView::Inbox) => self.inbox.as_ref().map(|view| view.focus_handle(cx)),
            Some(AppView::Automations) => {
                self.automations.as_ref().map(|view| view.focus_handle(cx))
            }
            Some(AppView::Flows) => self.flows.as_ref().map(|view| view.focus_handle(cx)),
            Some(AppView::Board) => self.board.as_ref().map(|view| view.focus_handle(cx)),
            None => return false,
        };
        if let Some(focus) = focus {
            window.focus(&focus, cx);
        }
        true
    }

    pub(super) fn open_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prepare_app_view(window, cx);
        if self.notes.is_none() {
            let view = cx.new(|cx| notes::Notes::new(self.store.clone(), cx));
            self.app_view_events.push(cx.subscribe_in(
                &view,
                window,
                |this, _, event: &notes::Event, window, cx| match event {
                    notes::Event::Close => this.close_app_view(window, cx),
                    notes::Event::OpenSource(id) => this.open(*id, window, cx),
                    notes::Event::AddToChat(note) => this.note_to_chat(note.clone(), window, cx),
                },
            ));
            self.notes = Some(view);
        }
        if let Some(notes) = &self.notes {
            let cwd = self.workspace.project.clone();
            let projects = self.workspace.projects.clone();
            notes.update(cx, |notes, cx| {
                notes.set_appearances(self.workspace.project_appearance.clone(), cx);
                notes.set_context(cwd, projects, cx);
                notes.refresh(cx);
            });
        }
        self.show_app_view(AppView::Notes, window, cx);
    }

    pub(super) fn open_board(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace.preferences.board {
            return;
        }
        // Find the database before leaving the current view, so a failure
        // leaves the user where they were.
        let path = match &self.board {
            Some(_) => None,
            None => match locked(&self.store) {
                Ok(store) => Some(store.database_path().to_path_buf()),
                Err(_) => return,
            },
        };
        self.prepare_app_view(window, cx);
        if let Some(path) = path {
            let store = self.store.clone();
            let view = cx.new(|cx| board::BoardView::new(store, path, window, cx));
            self.app_view_events.push(cx.subscribe_in(
                &view,
                window,
                |this, _, event: &board::Event, window, cx| match event {
                    board::Event::OpenSession(id) => this.open(*id, window, cx),
                    board::Event::LinkTicket(card_id) => this.open_card_ticket_dialog(
                        super::session_groups::CardTarget::Card(card_id.clone()),
                        window,
                        cx,
                    ),
                    board::Event::OpenPilot(id) => {
                        if let Some(pilots) = this.pilots.clone() {
                            pilots.update(cx, |pilots, cx| pilots.open(id, window, cx));
                        }
                    }
                    board::Event::RunWith(request) => this.start_card_run(request.clone(), cx),
                },
            ));
            let backend = self.tickets.clone();
            view.update(cx, |board, _| board.set_tickets(backend));
            self.board = Some(view);
        }
        self.show_app_view(AppView::Board, window, cx);
        self.sync_board_projects(cx);
        if let Some(board) = &self.board {
            board.update(cx, |board, cx| board.shown(cx));
        }
    }

    /// Give the Board the workspace's projects, as (path, name), so its tag
    /// and filter lists follow projects added, removed or renamed elsewhere.
    pub(super) fn sync_board_projects(&mut self, cx: &mut Context<Self>) {
        let Some(board) = &self.board else {
            return;
        };
        let projects = self
            .workspace
            .projects
            .iter()
            .map(|path| {
                let name = self
                    .workspace
                    .project_appearance
                    .get(path)
                    .map(|appearance| appearance.name.trim().to_string())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| {
                        std::path::Path::new(path)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.clone())
                    });
                (path.clone(), name)
            })
            .collect();
        let options = self.board_run_options(cx);
        board.update(cx, |board, cx| {
            board.set_projects(projects, cx);
            board.set_run_options(options);
        });
    }

    /// What the Board's "Run with…" picker offers: every provider, starting
    /// on the user's usual provider and its model for the current project.
    fn board_run_options(&self, cx: &App) -> board::run_with::RunOptions {
        let preferences = &self.workspace.preferences;
        let accounts = crate::accounts::get(cx);
        let project = preferences.project_providers.get(&self.workspace.project);
        board::run_with::RunOptions {
            providers: Kind::ALL
                .iter()
                .map(|kind| (kind.id().to_string(), kind.label().to_string()))
                .collect(),
            default_provider: Some(self.default_kind(cx).id().to_string()),
            default_models: Kind::ALL
                .iter()
                .filter_map(|kind| {
                    let model = project
                        .and_then(|project| project.models.get(kind.id()))
                        .or(preferences.default_models.get(kind.id()))?;
                    Some((kind.id().to_string(), model.clone()))
                })
                .collect(),
            // The catalogs already read; nothing is fetched for the picker.
            models: Kind::ALL
                .iter()
                .filter_map(|kind| {
                    let account = accounts
                        .as_ref()
                        .map(|accounts| accounts.read(cx).selected(*kind, &self.workspace.project))
                        .unwrap_or_default();
                    let models = gespan_core::catalog::cached_models_for(*kind, &account)?;
                    Some((
                        kind.id().to_string(),
                        models
                            .into_iter()
                            .map(|model| (model.id, model.name))
                            .collect(),
                    ))
                })
                .collect(),
        }
    }

    /// "Run with…" on a board card: make the session (prompt, ticket and card
    /// link in `prepare_run`) off the UI thread, then open and start it. The
    /// access mode is the user's default, as for any new session; plan mode
    /// and the like stay off.
    fn start_card_run(&mut self, request: board::run_with::RunRequest, cx: &mut Context<Self>) {
        let kind = Kind::from_id(&request.provider);
        let mut settings = gespan_core::agent::Settings {
            model: request.model.clone(),
            effort: crate::conversation::preferred_effort(kind.id(), cx),
            access: self
                .workspace
                .preferences
                .default_access
                .as_deref()
                .map(gespan_core::agent::AccessMode::from_id)
                .unwrap_or_default(),
            account: crate::accounts::get(cx)
                .map(|accounts| accounts.read(cx).selected(kind, &request.project))
                .unwrap_or_default(),
            ..Default::default()
        };
        if request.new_worktree {
            super::automation_runs::WorktreeChoice::New.apply(&mut settings);
        }
        let db_path = match locked(&self.store) {
            Ok(store) => store.database_path().to_path_buf(),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.start_card_run_at(request, settings, db_path, cx);
    }

    /// `start_card_run` against the board tables at `db_path`.
    pub(super) fn start_card_run_at(
        &mut self,
        request: board::run_with::RunRequest,
        settings: gespan_core::agent::Settings,
        db_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let store = self.store.clone();
        self.navigation_revision += 1;
        let revision = self.navigation_revision;
        let work = cx.background_executor().spawn(async move {
            board::run_with::prepare_run(&store, &db_path, &request, settings)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(run) => {
                    let id = run.summary.id;
                    let linked = run.summary.work_item.clone();
                    this.add(run.summary, cx);
                    if let Some(linked) = linked {
                        this.sessions[&id]
                            .update(cx, |view, _| view.summary.work_item = Some(linked));
                    }
                    let (prompt, settings) = (run.prompt, run.settings);
                    #[cfg(test)]
                    card_run_submits::record(id, &prompt);
                    if !card_run_submits::captured() {
                        this.sessions[&id]
                            .update(cx, |view, cx| view.quick_submit(prompt, settings, cx));
                    }
                    if this.navigation_revision == revision {
                        this.activate_session(id, window, cx);
                    }
                    if let Some(board) = &this.board {
                        board.update(cx, |board, cx| board.reload(cx));
                    }
                    this.persist(cx);
                }
                Err(error) => {
                    this.error = Some(error);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn open_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prepare_app_view(window, cx);
        self.ensure_background_inbox(window, cx);
        if let Some(inbox) = &self.inbox {
            inbox.update(cx, |inbox, cx| inbox.refresh(cx));
        }
        self.show_app_view(AppView::Inbox, window, cx);
    }

    pub(super) fn ensure_background_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.inbox.is_none() {
            let projects = self.workspace.projects.iter().map(Into::into).collect();
            let cwd = self.workspace.project.clone().into();
            let view = cx.new(|cx| inbox::Inbox::new(self.store.clone(), projects, cwd, cx));
            self.app_view_events.push(cx.subscribe_in(
                &view,
                window,
                |this, _, event: &inbox::Event, window, cx| match event {
                    inbox::Event::Close => this.close_app_view(window, cx),
                    inbox::Event::OpenSession(id) => this.open(*id, window, cx),
                    inbox::Event::ComposeSession { item, prompt } => {
                        this.start_inbox_session(item.as_ref().clone(), prompt.clone(), None, cx);
                    }
                    inbox::Event::StartRepair {
                        item,
                        prompt,
                        repair,
                    } => this.start_inbox_session(
                        item.as_ref().clone(),
                        prompt.clone(),
                        Some(repair.clone()),
                        cx,
                    ),
                    inbox::Event::ItemsChanged(items) => {
                        this.inbox_automation_events(items.clone(), cx)
                    }
                    inbox::Event::ConnectionsChanged => this.refresh_connected_trackers(cx),
                },
            ));
            self.app_view_events
                .push(cx.observe(&view, |this, inbox, cx| {
                    if let Some(automations) = &this.automations {
                        let connections = inbox
                            .read(cx)
                            .connection_statuses()
                            .into_iter()
                            .map(|(provider, (connected, _))| (provider, connected))
                            .collect();
                        automations
                            .update(cx, |view, cx| view.set_trigger_connections(connections, cx));
                    }
                    this.invalidate_chrome(cx);
                    this.sync_settings(cx);
                    cx.notify();
                }));
            self.inbox = Some(view);
        }
        if let Some(inbox) = &self.inbox {
            let projects = self.workspace.projects.iter().map(Into::into).collect();
            let cwd = self.workspace.project.clone().into();
            inbox.update(cx, |inbox, cx| {
                inbox.set_appearances(self.workspace.project_appearance.clone(), cx);
                inbox.set_context(projects, cwd, cx);
            });
        }
    }

    pub(super) fn open_automations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(service) = self.automation_state.service.clone() else {
            self.automation_state.open_requested = Some(self.navigation_revision);
            self.initialize_automations(cx);
            return;
        };
        self.prepare_app_view(window, cx);
        if self.automations.is_none() {
            let cwd = self.workspace.project.clone();
            let projects = self.workspace.projects.clone();
            let view = cx.new(|cx| automations::Automations::new(service, cwd, projects, cx));
            view.update(cx, |view, cx| view.set_database(self.store.clone(), cx));
            self.app_view_events.push(cx.subscribe_in(
                &view,
                window,
                |this, _, event: &automations::Event, window, cx| match event {
                    automations::Event::Close => this.close_app_view(window, cx),
                    automations::Event::Launch(run) => {
                        this.launch_automation(run.as_ref().clone(), true, cx)
                    }
                    automations::Event::OpenSession(id) => {
                        let native = locked(&this.store)
                            .ok()
                            .and_then(|store| store.resolve_existing_session_id(id));
                        if let Some(id) = native {
                            this.open(id, window, cx);
                        }
                    }
                },
            ));
            self.automations = Some(view);
        }
        if let Some(view) = &self.automations {
            let cwd = self.workspace.project.clone();
            let projects = self.workspace.projects.clone();
            view.update(cx, |view, cx| {
                view.set_appearances(self.workspace.project_appearance.clone(), cx);
                let connections = self
                    .inbox
                    .as_ref()
                    .map(|inbox| inbox.read(cx).connection_statuses())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(provider, (connected, _))| (provider, connected))
                    .collect();
                view.set_trigger_connections(connections, cx);
                view.set_context(cwd, projects, cx);
                view.refresh(cx);
            });
        }
        self.show_app_view(AppView::Automations, window, cx);
    }

    pub(super) fn open_flows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prepare_app_view(window, cx);
        if self.flows.is_none() {
            let store = self.flows_state.store.clone();
            let cwd = self.workspace.project.clone();
            let projects = self.workspace.projects.clone();
            let view = cx.new(|cx| flows::FlowsView::new(store, cwd, projects, cx));
            self.app_view_events.push(cx.subscribe_in(
                &view,
                window,
                |this, view, event: &flows::Event, window, cx| {
                    let result = match event {
                        flows::Event::Close => {
                            this.close_app_view(window, cx);
                            Ok(())
                        }
                        flows::Event::Start { flow, inputs } => {
                            match this.start_flow(flow, inputs, cx) {
                                Ok(run) => {
                                    view.update(cx, |view, cx| view.show_run(run, cx));
                                    Ok(())
                                }
                                Err(errors) => Err(errors.join("\n")),
                            }
                        }
                        flows::Event::Approve {
                            run_id,
                            approved,
                            note,
                        } => this.resolve_flow_approval(run_id, *approved, note, cx),
                        flows::Event::Cancel(run_id) => this.cancel_flow_run(run_id, cx),
                        flows::Event::Resume { run_id, state_id } => {
                            this.resume_flow_run(run_id.clone(), state_id.clone(), cx)
                        }
                        flows::Event::OpenSession(id) => {
                            this.open(*id, window, cx);
                            Ok(())
                        }
                        flows::Event::Delete(id) => this.delete_flow(id, cx),
                    };
                    if let Err(error) = result {
                        view.update(cx, |view, cx| view.set_error(error, cx));
                    }
                },
            ));
            self.flows = Some(view);
        }
        if let Some(view) = &self.flows {
            let cwd = self.workspace.project.clone();
            let projects = self.workspace.projects.clone();
            let waiting = self.flows_state.waiting.clone();
            view.update(cx, |view, cx| {
                view.set_appearances(self.workspace.project_appearance.clone(), cx);
                view.set_context(cwd, projects, cx);
                view.set_waiting(waiting, cx);
                view.refresh(cx);
            });
        }
        self.show_app_view(AppView::Flows, window, cx);
    }

    fn note_to_chat(
        &mut self,
        note: gespan_core::notes::Note,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwd = note
            .source_cwd
            .clone()
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| self.workspace.project.clone());
        let (kind, model) = crate::installed_providers::new_session_choice(
            &self.workspace.preferences,
            Some(&cwd),
            cx,
        );
        let store = self.store.clone();
        let mut settings = gespan_core::agent::Settings {
            model,
            effort: crate::conversation::preferred_effort(kind.id(), cx),
            access: self
                .workspace
                .preferences
                .default_access
                .as_deref()
                .map(gespan_core::agent::AccessMode::from_id)
                .unwrap_or_default(),
            account: crate::accounts::get(cx)
                .map(|accounts| accounts.read(cx).selected(kind, &cwd))
                .unwrap_or_default(),
            ..Default::default()
        };
        self.workspace
            .preferences
            .apply_last_controls(&mut settings, kind);
        self.leave_app_views(cx);
        self.navigation_revision += 1;
        let revision = self.navigation_revision;
        let work = cx.background_executor().spawn(async move {
            let store = locked(&store)?;
            let mut summary = store.create(kind.id(), &cwd)?;
            if !note.title.trim().is_empty() {
                store.title(summary.id, &note.title)?;
                summary.title = note.title.clone();
            }
            store.save_agent_settings(summary.id, &settings)?;
            Ok::<_, String>((summary, settings, note))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok((summary, settings, note)) => {
                    let id = summary.id;
                    this.add(summary, cx);
                    this.sessions[&id].update(cx, |view, cx| {
                        view.seed_new_settings(settings, cx);
                        view.set_composer_card(ComposerCard::Note(Box::new(note)), cx);
                    });
                    if this.navigation_revision == revision {
                        this.activate_session(id, window, cx);
                    }
                    this.persist(cx);
                }
                Err(error) => {
                    this.error = Some(error);
                    cx.notify();
                }
            });
        })
        .detach();
        self.update_chat_visibility(cx);
        self.focus_active(window, cx);
    }

    /// A check `repair` goes straight out and reports back to the Inbox.
    /// Otherwise the item waits above the composer for a note, or for Send.
    fn start_inbox_session(
        &mut self,
        item: gespan_core::inbox::Item,
        prompt: String,
        repair: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let cwd = if item.project_path.is_empty() {
            self.workspace.project.clone()
        } else {
            item.project_path.clone()
        };
        let (kind, model) = crate::installed_providers::new_session_choice(
            &self.workspace.preferences,
            Some(&cwd),
            cx,
        );
        let mut settings = gespan_core::agent::Settings {
            model,
            effort: crate::conversation::preferred_effort(kind.id(), cx),
            access: self
                .workspace
                .preferences
                .default_access
                .as_deref()
                .map(gespan_core::agent::AccessMode::from_id)
                .unwrap_or_default(),
            account: crate::accounts::get(cx)
                .map(|accounts| accounts.read(cx).selected(kind, &cwd))
                .unwrap_or_default(),
            ..Default::default()
        };
        self.workspace
            .preferences
            .apply_last_controls(&mut settings, kind);
        let store = self.store.clone();
        self.navigation_revision += 1;
        let revision = self.navigation_revision;
        let failed_repair = repair.clone();
        let work = cx.background_executor().spawn(async move {
            if !std::path::Path::new(&cwd).is_dir() {
                return Err("Choose a project folder before starting work on this item".into());
            }
            let store = locked(&store)?;
            let mut summary = store.create(kind.id(), &cwd)?;
            summary.title = format!("{} · {}", item.reference(), item.title);
            summary.work_item = Some(item.work_item().into());
            store.title(summary.id, &summary.title)?;
            store.set_work_item(summary.id, summary.work_item.as_ref())?;
            store.save_agent_settings(summary.id, &settings)?;
            Ok::<_, String>((summary, settings, prompt, item))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok((summary, settings, prompt, item)) => {
                    let id = summary.id;
                    this.add(summary, cx);
                    this.sessions[&id].update(cx, |view, cx| {
                        if repair.is_some() {
                            view.quick_submit(prompt, settings, cx);
                        } else {
                            view.seed_new_settings(settings, cx);
                            let item = Box::new(item);
                            view.set_composer_card(ComposerCard::Inbox { item, prompt }, cx);
                        }
                    });
                    if let Some(repair) = repair {
                        this.follow_repair(repair, id, cx);
                    }
                    if this.navigation_revision == revision {
                        this.activate_session(id, window, cx);
                    }
                    this.persist(cx);
                }
                Err(error) => {
                    if let (Some(repair), Some(inbox)) = (failed_repair, &this.inbox) {
                        inbox.update(cx, |inbox, cx| {
                            inbox.repair_session_failed(&repair, error.clone(), cx)
                        });
                    }
                    this.error = Some(error);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Tells the inbox which chat took a check repair and when its agent stops.
    fn follow_repair(&mut self, repair: String, id: i64, cx: &mut Context<Self>) {
        let Some(inbox) = self.inbox.clone() else {
            return;
        };
        inbox.update(cx, |inbox, cx| {
            inbox.repair_session_started(&repair, id, cx)
        });
        let mut was_busy = self.sessions[&id].read(cx).busy;
        self.app_view_events
            .push(cx.observe(&self.sessions[&id], move |_, view, cx| {
                let busy = view.read(cx).busy;
                if was_busy && !busy {
                    inbox.update(cx, |inbox, cx| inbox.repair_session_settled(id, cx));
                }
                was_busy = busy;
            }));
    }
}

/// Tests read what a card run would hand `quick_submit` instead of launching a
/// provider. Outside tests nothing is captured.
mod card_run_submits {
    #[cfg(test)]
    thread_local! {
        pub(super) static SUBMITS: std::cell::RefCell<Option<Vec<(i64, String)>>> =
            const { std::cell::RefCell::new(None) };
    }

    #[cfg(test)]
    pub(super) fn record(id: i64, prompt: &str) {
        SUBMITS.with(|submits| {
            if let Some(submits) = submits.borrow_mut().as_mut() {
                submits.push((id, prompt.to_string()));
            }
        });
    }

    #[cfg(test)]
    pub(super) fn captured() -> bool {
        SUBMITS.with(|submits| submits.borrow().is_some())
    }

    #[cfg(not(test))]
    pub(super) fn captured() -> bool {
        false
    }
}

#[cfg(test)]
mod card_run_tests {
    use super::*;
    use gespan_core::board::Service;
    use gpui::TestAppContext;

    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A board database with one card, in a folder that is also the project.
    fn board(name: &str) -> (Scratch, std::path::PathBuf, String) {
        let dir =
            std::env::temp_dir().join(format!("gespan-card-run-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("gespan.db");
        let mut service = Service::open(&db).unwrap();
        let board = service.create("Work").unwrap();
        let column = service.get(&board.id).unwrap().columns[0].id.clone();
        let card = service
            .card_create(&column, "Ship login", "Make the form work.")
            .unwrap();
        (Scratch(dir), db, card.id)
    }

    fn request(card: &str, project: &std::path::Path) -> board::run_with::RunRequest {
        board::run_with::RunRequest {
            card_id: card.into(),
            provider: "claude".into(),
            model: Some("opus".into()),
            project: project.to_string_lossy().into_owned(),
            new_worktree: false,
        }
    }

    fn capture() {
        card_run_submits::SUBMITS.with(|submits| *submits.borrow_mut() = Some(Vec::new()));
    }

    fn submits() -> Vec<(i64, String)> {
        card_run_submits::SUBMITS.with(|submits| submits.borrow().clone().unwrap_or_default())
    }

    #[gpui::test]
    fn start_card_run_opens_and_starts_the_session_with_exactly_the_run_prompt(
        cx: &mut TestAppContext,
    ) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let (dir, db, card) = board("ok");
        capture();
        let before = window
            .update(cx, |shell, _, cx| {
                let before = shell.session_summaries.keys().copied().collect::<Vec<_>>();
                shell.start_card_run_at(
                    request(&card, &dir.0),
                    gespan_core::agent::Settings::default(),
                    db.clone(),
                    cx,
                );
                before
            })
            .unwrap();
        cx.run_until_parked();
        let (id, key) = window
            .update(cx, |shell, _, _| {
                let added: Vec<i64> = shell
                    .session_summaries
                    .keys()
                    .copied()
                    .filter(|id| !before.contains(id))
                    .collect();
                assert_eq!(added.len(), 1, "one session added");
                let id = added[0];
                assert!(shell.sessions.contains_key(&id));
                assert_eq!(shell.workspace.active, Some(id), "and activated");
                assert_eq!(shell.session_summaries[&id].provider, "claude");
                assert!(shell.error.is_none());
                let key = locked(&shell.store)
                    .unwrap()
                    .persisted_session_id(id)
                    .unwrap();
                (id, key)
            })
            .unwrap();
        let mut service = Service::open(&db).unwrap();
        let prompt = service.run_prompt(&card).unwrap();
        assert_eq!(
            submits(),
            [(id, prompt)],
            "quick_submit gets the run prompt"
        );
        assert!(
            service
                .card_links(&card)
                .unwrap()
                .iter()
                .any(|link| link.kind == "session" && link.reference == key)
        );
    }

    #[gpui::test]
    fn start_card_run_failure_shows_an_error_and_leaves_no_session(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let (dir, db, _) = board("fail");
        capture();
        let (before, stored) = window
            .update(cx, |shell, _, cx| {
                let before = shell.session_summaries.len();
                let stored = locked(&shell.store).unwrap().sessions().unwrap().len();
                // A card that doesn't exist: no prompt, so nothing starts.
                shell.start_card_run_at(
                    request("00000000-0000-0000-0000-000000000000", &dir.0),
                    gespan_core::agent::Settings::default(),
                    db.clone(),
                    cx,
                );
                (before, stored)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert!(shell.error.is_some(), "the failure shows");
                assert_eq!(shell.session_summaries.len(), before);
                assert_eq!(
                    locked(&shell.store).unwrap().sessions().unwrap().len(),
                    stored
                );
            })
            .unwrap();
        assert!(submits().is_empty());
    }
}

#[cfg(test)]
mod repair_tests {
    use super::*;
    use gespan_core::inbox::ci_repair::RepairPhase;
    use gpui::TestAppContext;

    #[gpui::test]
    fn a_repair_learns_its_chat_and_settles_only_after_the_agent_ran(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        let (id, repair) = window
            .update(cx, |shell, _, cx| {
                let summary = locked(&shell.store)
                    .unwrap()
                    .create("codex", "/missing/repair")
                    .unwrap();
                let id = summary.id;
                shell.add_loaded(summary, cx);
                let store = shell.store.clone();
                let inbox = cx.new(|cx| inbox::Inbox::offline(store, cx));
                let repair = inbox.update(cx, |inbox, _| inbox.track_test_repair());
                shell.inbox = Some(inbox);
                shell.follow_repair(repair.clone(), id, cx);
                (id, repair)
            })
            .unwrap();
        let progress = |cx: &mut TestAppContext| {
            window
                .update(cx, |shell, _, cx| {
                    shell
                        .inbox
                        .as_ref()
                        .unwrap()
                        .read(cx)
                        .test_repair_progress()
                })
                .unwrap()
        };
        let set_busy = |busy: bool, cx: &mut TestAppContext| {
            window
                .update(cx, |shell, _, cx| {
                    shell.sessions[&id].update(cx, |view, cx| {
                        view.busy = busy;
                        cx.notify();
                    })
                })
                .unwrap();
            cx.run_until_parked();
        };
        assert_eq!(progress(cx), (Some(id), RepairPhase::Running));
        // Not busy yet: the agent has not started, so nothing has finished.
        set_busy(false, cx);
        assert_eq!(progress(cx).1, RepairPhase::Running);
        set_busy(true, cx);
        assert_eq!(progress(cx).1, RepairPhase::Running);
        set_busy(false, cx);
        assert_eq!(progress(cx).1, RepairPhase::Completed);
        drop(repair);
    }
}
