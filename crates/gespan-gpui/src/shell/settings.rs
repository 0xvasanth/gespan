//! The settings surface replaces the rail body and the workspace, preserving
//! open conversations. Its breadcrumb/actions occupy the inset title bar.
use super::*;
use crate::settings::{ArchivedSession, Section, SettingsContext, SettingsEvent, SettingsView};
use std::path::Path;
impl Shell {
    fn settings_context(&self, cx: &App) -> SettingsContext {
        let mut archived: Vec<_> = self
            .order
            .iter()
            .map(|&id| self.session_metadata(id, cx).summary)
            .filter(|s| s.archived && s.cwd == self.workspace.project)
            .map(|s| ArchivedSession {
                id: s.id,
                title: s.title.clone(),
                provider: s.provider.clone(),
                updated_at: s.updated_at,
            })
            .collect();
        archived.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        let mut catalogs = HashMap::new();
        let active = self
            .pilots
            .as_ref()
            .filter(|_| self.pilot_active(cx))
            .and_then(|m| m.read(cx).chat())
            .or_else(|| {
                self.workspace.active.and_then(|id| {
                    self.sessions
                        .get(&self.workspace.session_for_pane(id))
                        .cloned()
                })
            });
        let mut profiles: HashMap<_, _> = Kind::ALL
            .into_iter()
            .map(|kind| {
                (
                    kind,
                    crate::accounts::get(cx)
                        .map(|center| center.read(cx).selected(kind, &self.workspace.project).id)
                        .unwrap_or_else(|| gespan_core::accounts::DEFAULT.into()),
                )
            })
            .collect();
        if let Some(active) = &active {
            let v = active.read(cx);
            profiles.insert(
                Kind::from_id(&v.summary.provider),
                v.settings.account.id.clone(),
            );
        }
        for session in active.iter().chain(self.sessions.values()) {
            let session = session.read(cx);
            if profiles.get(&Kind::from_id(&session.summary.provider))
                != Some(&session.settings.account.id)
            {
                continue;
            }
            if catalogs.contains_key(&session.summary.provider) {
                continue;
            }
            let models = session.settings_models();
            if !models.is_empty() {
                catalogs.insert(session.summary.provider.clone(), models);
            }
        }
        SettingsContext {
            remote: Some(self.remote.clone()),
            mobile: self.mobile_status(),
            mobile_invite: self.mobile.invite.clone(),
            quick_error: self.quick.error.clone(),
            inbox_connections: self
                .inbox
                .as_ref()
                .map(|inbox| inbox.read(cx).connection_statuses())
                .unwrap_or_default(),
            inbox: self.inbox.clone(),
            linear_teams: self
                .inbox
                .as_ref()
                .map(|inbox| inbox.read(cx).linear_teams())
                .unwrap_or_default(),
            sessions: self
                .order
                .iter()
                .map(|&id| {
                    (
                        id,
                        self.session_working_copy(id, cx).to_owned(),
                        self.session_metadata(id, cx).busy,
                    )
                })
                .collect(),
            pilots: self
                .pilots
                .as_ref()
                .map(|m| {
                    m.read(cx)
                        .roster
                        .pilots
                        .iter()
                        .map(|m| crate::settings::PilotSetting {
                            id: m.id.clone(),
                            name: m.label(),
                            mascot: m.mascot.clone(),
                            color: m.color,
                            projects: m.projects.iter().map(|p| self.project_label(p)).collect(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            catalogs,
            classic: self.workspace.classic_layout,
            new_layout_boxy: self.workspace.new_layout_boxy,
            cwd: self.workspace.project.clone(),
            project_label: self.project_label(&self.workspace.project),
            archived,
            archived_projects: self
                .workspace
                .archived_projects
                .iter()
                .map(|path| (path.clone(), self.project_label(path)))
                .collect(),
            projects: self
                .workspace
                .projects
                .iter()
                .map(|path| (path.clone(), self.project_label(path)))
                .collect(),
        }
    }
    pub(super) fn open_settings(
        &mut self,
        section: Section,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.leave_app_search(cx);
        self.close_preview(cx);
        self.menu = None;
        self.load_inbox_for_settings(section, window, cx);
        if self.settings.is_some() {
            self.select_settings(section, cx);
            return;
        }
        self.navigation_revision += 1;
        let context = self.settings_context(cx);
        let prefs = self.workspace.preferences.clone();
        let view = cx.new(|cx| SettingsView::new(section, prefs, context, cx));
        self.settings_events = Some(cx.subscribe_in(
            &view,
            window,
            |this, _, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Section(section) => {
                    this.load_inbox_for_settings(*section, window, cx);
                    this.select_settings(*section, cx)
                }
                SettingsEvent::ManageInboxProvider(provider) => {
                    this.ensure_background_inbox(window, cx);
                    if let Some(inbox) = &this.inbox {
                        inbox.update(cx, |inbox, cx| {
                            inbox.open_connection_in_settings(provider.clone(), window, cx)
                        });
                    }
                    this.sync_settings(cx);
                }
                SettingsEvent::ToggleLinearTeam(id) => {
                    if let Some(inbox) = &this.inbox {
                        inbox.update(cx, |inbox, cx| inbox.toggle_linear_team(id.clone(), cx));
                    }
                }
                SettingsEvent::WhatsNew => {
                    this.open_whats_new(crate::notifications::updater::version().into(), cx)
                }
                SettingsEvent::CheckAppUpdates => {
                    this.notification_state
                        .updater
                        .update(cx, |updater, cx| updater.check(true, cx));
                }
                SettingsEvent::CheckHarnessUpdates => {
                    let hidden = this.workspace.preferences.hidden_providers.clone();
                    this.notification_state
                        .harness
                        .update(cx, |notice, cx| notice.check_now(hidden, cx));
                }
                SettingsEvent::Changed(prefs) => {
                    #[cfg(target_os = "linux")]
                    if this.workspace.preferences.wayland_window_decorations
                        != prefs.wayland_window_decorations
                        && let Some(decorations) = prefs.wayland_decorations(cx.compositor_name())
                    {
                        window.request_decorations(decorations);
                    }
                    this.set_preferences(*prefs.clone(), window, cx)
                }
                SettingsEvent::NewLayout(on) => {
                    if this.workspace.classic_layout == *on {
                        this.toggle_layout(cx);
                    }
                    this.sync_settings(cx);
                }
                SettingsEvent::NewLayoutBoxy(boxy) => {
                    this.workspace.new_layout_boxy = *boxy;
                    this.layout_changed(cx);
                    this.sync_settings(cx);
                }
                SettingsEvent::OpenSession(id) => {
                    this.close_settings(window, cx);
                    this.open(*id, window, cx);
                }
                SettingsEvent::Archive(id, archived) => {
                    if this.session_summaries.contains_key(id)
                        && this.session_metadata(*id, cx).summary.archived != *archived
                    {
                        if *archived {
                            this.archive_session(*id, window, cx);
                        } else {
                            this.toggle_archive(*id, cx);
                        }
                    }
                }
                SettingsEvent::Delete(id) => this.delete_from_settings(*id, window, cx),
                SettingsEvent::UnarchiveProject(path) => this.unarchive_project(path, cx),
                SettingsEvent::DeleteProject(path) => {
                    this.confirm_delete_project(path.clone(), window, cx)
                }
                SettingsEvent::RemoveWorktree {
                    cwd,
                    path,
                    delete_sessions,
                } => this.remove_settings_worktree(
                    cwd.clone(),
                    path.clone(),
                    *delete_sessions,
                    window,
                    cx,
                ),
                SettingsEvent::ChooseBackground => this.choose_chat_background(cx),
                SettingsEvent::ResetPilot(id) => {
                    if let Some(m) = this.pilots.clone() {
                        let reset = m.update(cx, |m, cx| m.reset_defaults(id, cx));
                        cx.spawn(async move |this, cx| {
                            let result = reset.await;
                            let _ = this.update(cx, |this, cx| {
                                if let Some(settings) = &this.settings {
                                    settings.update(cx, |s, cx| s.management_finished(result, cx));
                                }
                                this.sync_settings(cx);
                            });
                        })
                        .detach();
                    }
                }
                SettingsEvent::MobileRefresh => this.sync_settings(cx),
                SettingsEvent::MobileConfigure {
                    enabled,
                    open_router,
                    remote_address,
                } => this.mobile_configure(*enabled, *open_router, remote_address.clone(), cx),
                SettingsEvent::MobilePair => this.pair_mobile(cx),
                SettingsEvent::MobileCancelInvite => this.mobile_cancel_invite(cx),
                SettingsEvent::MobileRevoke(id) => this.mobile_revoke(id, cx),
            },
        ));
        // This dialog is painted at the window root, like React's portal.
        // Redraw it for countdown/copy/close changes even when only the
        // Settings entity was notified, without rebuilding the shell for
        // ordinary settings edits.
        let mut overlay_was_open = false;
        self.settings_overlay_watch = Some(cx.observe(&view, move |_, view, cx| {
            let open = view.read(cx).mobile_overlay_open();
            if open || overlay_was_open {
                cx.notify();
            }
            overlay_was_open = open;
        }));
        self.settings = Some(view.clone());
        // Settings has its own complete rail, even when the project rail was hidden.
        self.sidebar_slide = None;
        self.sidebar_fraction = 1.;
        window.focus(&view.focus_handle(cx), cx);
        this_update(self, cx);
    }
    /// The Inbox page shows each connection, which the Inbox reads.
    fn load_inbox_for_settings(
        &mut self,
        section: Section,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if section != Section::Inbox {
            return;
        }
        let first = self.inbox.is_none();
        self.ensure_background_inbox(window, cx);
        if first && let Some(inbox) = &self.inbox {
            inbox.update(cx, |inbox, cx| inbox.refresh(cx));
        }
    }
    pub(super) fn select_settings(&mut self, section: Section, cx: &mut Context<Self>) {
        if let Some(view) = &self.settings {
            view.update(cx, |v, cx| v.select_section(section, cx));
        }
        this_update(self, cx);
    }
    pub(super) fn sync_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = self.settings.clone() {
            let context = self.settings_context(cx);
            let prefs = self.workspace.preferences.clone();
            view.update(cx, |v, cx| {
                v.set_context(context, cx);
                v.set_preferences(&prefs, cx);
            });
        }
    }
    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(inbox) = &self.inbox {
            inbox.update(cx, |inbox, cx| inbox.close_settings_dialog(cx));
        }
        self.settings = None;
        self.settings_events = None;
        self.settings_overlay_watch = None;
        // A code nobody can see should not stay usable.
        if self.mobile.invite.is_some() {
            self.mobile_cancel_invite(cx);
        }
        self.sidebar_fraction = if self.workspace.sidebar_visible {
            1.
        } else {
            0.
        };
        self.focus_active(window, cx);
        this_update(self, cx);
    }
    /// MobileSettings.tsx's pair: the desktop listens only while a phone is
    /// paired or being paired, so start listening before making the code.
    fn pair_mobile(&mut self, cx: &mut Context<Self>) {
        let Some(status) = self.mobile_status() else {
            return;
        };
        if status.enabled && status.listening {
            self.mobile_invite(cx);
            return;
        }
        self.mobile_configure(true, status.open_router, status.remote_address, cx);
        cx.spawn(async move |this, cx| {
            // Binding runs in the background; a failure lands in `error`.
            for _ in 0..100 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                let Ok(ready) = this.update(cx, |this, _| {
                    this.mobile_status()
                        .is_some_and(|status| status.enabled && status.listening)
                }) else {
                    return;
                };
                if ready {
                    let _ = this.update(cx, |this, cx| this.mobile_invite(cx));
                    return;
                }
            }
        })
        .detach();
    }
    /// A new session's provider and model in this project: its default,
    /// else the last used, else the first installed provider.
    pub(super) fn new_session_choice(&self, cx: &App) -> (Kind, Option<String>) {
        crate::installed_providers::new_session_choice(
            &self.workspace.preferences,
            Some(&self.workspace.project),
            cx,
        )
    }
    pub(super) fn default_kind(&self, cx: &App) -> Kind {
        self.new_session_choice(cx).0
    }
    /// Any busy session counts, including ones in background tabs or panes.
    pub(super) fn sync_keep_awake(&mut self) {
        let mode = gespan_keep_awake::Mode::parse(&self.workspace.preferences.keep_awake)
            .unwrap_or_default();
        if let Err(error) = self.keep_awake.update(mode, self.agents_keep_awake()) {
            tracing::warn!(%error, ?mode, "keep screen awake failed");
        }
    }
    pub(super) fn set_preferences(
        &mut self,
        prefs: crate::settings::Preferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.preferences = prefs;
        self.apply_preferences(cx);
        if !self.workspace.preferences.board
            && self.active_app_view == Some(app_views::AppView::Board)
        {
            // Closing saves the workspace, including the new preferences.
            self.close_app_view(window, cx);
            // Closing the Board moves focus to the chat behind; keep it on
            // the Settings page the change came from.
            if let Some(settings) = &self.settings {
                window.focus(&settings.focus_handle(cx), cx);
            }
        } else {
            self.persist(cx);
        }
        self.invalidate_chrome(cx);
        cx.notify();
    }

    pub(super) fn apply_preferences(&mut self, cx: &mut Context<Self>) {
        self.refresh_notification_permission();
        self.configure_quick_composer(cx);
        gespan_core::cli::initialize(&self.workspace.preferences.binary_paths);
        self.keep_visible_idle_agents(cx);
        gespan_core::provider::configure_idle_agents(
            self.workspace.preferences.idle_agents as usize,
        );
        self.sync_keep_awake();
        if let Some(pilots) = &self.pilots {
            let prefs = self.workspace.preferences.clone();
            pilots.update(cx, |m, cx| m.set_preferences(prefs, cx));
        }
        crate::settings::apply_keybindings(&self.workspace.preferences, cx);
        // After the keymap, so the menu shows rebound shortcuts.
        crate::app_menu::install(&self.workspace.preferences, cx);
        cx.set_global(self.workspace.preferences.clone());
        self.sync_right_panel(cx);
        if let Some(center) = crate::accounts::get(cx) {
            center.update(cx, |center, cx| {
                let changed = center.mask_emails != self.workspace.preferences.mask_emails
                    || center.show_remaining != self.workspace.preferences.show_remaining_usage;
                center.mask_emails = self.workspace.preferences.mask_emails;
                center.show_remaining = self.workspace.preferences.show_remaining_usage;
                if changed {
                    cx.notify();
                }
            });
        }
        for session in self.sessions.values() {
            session.read(cx).transcript.clone().update(cx, |view, cx| {
                view.apply_preferences(&self.workspace.preferences, cx)
            });
        }
        self.usage_footer.update(cx, |footer, cx| {
            footer.set_remaining(self.workspace.preferences.show_remaining_usage, cx)
        });
    }
    fn choose_chat_background(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await
                && let Some(path) = paths.first()
            {
                let source = path.clone();
                let directory = crate::data_dir().join("backgrounds");
                let work = cx.background_executor().spawn(async move {
                    gespan_core::project_appearance::import_image(
                        &source, &directory, "global", false,
                    )
                });
                let result = work.await;
                let _ = this.update_in(cx, |this, window, cx| match result {
                    Ok(path) => {
                        this.workspace.preferences.chat_background = Some(path);
                        this.persist(cx);
                        this.sync_settings(cx);
                        cx.notify();
                    }
                    Err(error) => {
                        let prompt = window.prompt(
                            gpui::PromptLevel::Critical,
                            "Cannot use this image",
                            Some(&error),
                            &["OK"],
                            cx,
                        );
                        cx.spawn(async move |_, _| {
                            let _ = prompt.await;
                        })
                        .detach();
                    }
                });
            }
        })
        .detach();
    }
    fn finish_worktree_action(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        if let Some(view) = &self.settings {
            view.update(cx, |v, cx| v.management_finished(result, cx));
        }
        self.sync_git(cx);
        self.sync_settings(cx);
        self.invalidate_chrome(cx);
        cx.notify();
    }
    fn remove_settings_worktree(
        &mut self,
        cwd: String,
        path: String,
        delete_sessions: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gespan_core::files::is_equal_or_inside;
        if self
            .terminals
            .values()
            .flatten()
            .any(|t| is_equal_or_inside(&t.read(cx).cwd, &path))
        {
            self.finish_worktree_action(
                Err("Close the terminals using this worktree first.".into()),
                cx,
            );
            return;
        }
        let ids = self
            .session_summaries
            .keys()
            .filter(|id| is_equal_or_inside(self.session_working_copy(**id, cx), &path))
            .copied()
            .collect::<Vec<_>>();
        if ids
            .iter()
            .any(|id| self.sessions.get(id).is_some_and(|v| v.read(cx).busy))
        {
            self.finish_worktree_action(
                Err("Stop the running sessions using this worktree first.".into()),
                cx,
            );
            return;
        }
        let check_cwd = cwd.clone();
        let check_path = path.clone();
        let work = cx.background_executor().spawn(async move {
            gespan_core::worktree_management::check(Path::new(&check_cwd), Path::new(&check_path))
        });
        cx.spawn(async move|this,cx|{
            let result=work.await;
            if let Err(e)=result {let _=this.update(cx,|this,cx|this.finish_worktree_action(Err(e),cx));return;}
            let pending=match this.update_in(cx,|this,window,cx| {
                // Recheck before deleting any conversations after the preflight worker.
                if this.terminals.values().flatten().any(|t|is_equal_or_inside(&t.read(cx).cwd,&path))||ids.iter().any(|id|this.sessions.get(id).is_some_and(|s|s.read(cx).busy)) {return Err("This worktree is now in use. Close its terminals and stop its sessions first.".to_owned());}
                if delete_sessions {for id in &ids{this.delete_session(*id,window,cx);}}
                Ok((this.group_writes.take(),this.store.clone()))
            }) {Ok(Ok(v))=>v,Ok(Err(e))=>{let _=this.update(cx,|this,cx|this.finish_worktree_action(Err(e),cx));return;},Err(_)=>return};
            if let Some(task)=pending.0 {task.await;}
            let worker_cwd=cwd.clone();let worker_path=path.clone();let store=pending.1;
            let work=cx.background_executor().spawn(async move{
                if delete_sessions {let store=locked(&store)?;let paths=store.session_worktrees()?;if store.sessions()?.iter().any(|s|is_equal_or_inside(paths.get(&s.id).and_then(|p|p.as_deref()).unwrap_or(&s.cwd),&worker_path)){return Err("Some sessions could not be deleted, so the worktree was kept.".into());}}
                let snapshot=gespan_core::git::snapshot(Path::new(&worker_cwd))?;
                let main=snapshot.worktrees.first().ok_or("No main working copy found")?;
                gespan_core::worktree_management::remove(Path::new(&main.path),Path::new(&worker_path))?;
                locked(&store)?.detach_worktree(&worker_path,&main.path)
            });let result=work.await;
            let _=this.update(cx,|this,cx|{let result=result.map(|detached|this.worktree_detached(detached,&path,cx));this.finish_worktree_action(result,cx);});
        }).detach();
        let _ = window;
    }
    /// The worktree at `path` is gone; its sessions keep their transcripts.
    pub(super) fn worktree_detached(
        &mut self,
        detached: Vec<(Session, gespan_core::agent::Settings)>,
        path: &str,
        cx: &mut Context<Self>,
    ) {
        for (summary, settings) in detached {
            self.session_summaries.insert(summary.id, summary.clone());
            self.session_worktrees
                .insert(summary.id, settings.worktree_cwd.clone());
            if let Some(view) = self.sessions.get(&summary.id) {
                view.update(cx, |v, cx| v.worktree_deleted(summary, settings, cx));
            }
        }
        self.worktree_focus
            .retain(|_, focus| !gespan_core::files::is_equal_or_inside(&focus.path, path));
    }
    fn delete_from_settings(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let ask = session_worktree::Ask {
            title: "Delete this conversation permanently?".into(),
            detail: "Its messages and saved settings will be removed.".into(),
        };
        self.remove_session(id, session_worktree::Removal::Delete, Some(ask), window, cx);
    }
}
fn this_update(this: &mut Shell, cx: &mut Context<Shell>) {
    this.invalidate_chrome(cx);
    this.update_chat_visibility(cx);
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::window;
    use gpui::TestAppContext;

    #[gpui::test]
    fn pairing_dialog_is_a_window_overlay_and_updates_from_settings_notifications(
        cx: &mut TestAppContext,
    ) {
        cx.update(super::super::bind_keys);
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.open_settings(Section::Connections, w, cx)
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("mobile-pair-overlay").is_none());
        window
            .update(visual, |shell, _, cx| {
                shell.mobile.invite = Some(gespan_core::mobile::Invite {
                    url: format!("gespan://pair?invite={}", "q".repeat(500)),
                    expires: gespan_core::time::now_ms() / 1000 + 900,
                });
                // This notifies Settings alone. The root overlay must observe it.
                shell.sync_settings(cx);
            })
            .unwrap();
        visual.run_until_parked();
        let viewport = window.update(visual, |_, w, _| w.viewport_size()).unwrap();
        let scrim = visual
            .debug_bounds("mobile-pair-overlay")
            .expect("Settings changes show the portal at the shell root");
        assert_eq!(scrim.origin, gpui::point(px(0.), px(0.)));
        assert_eq!(scrim.size, viewport);
        let dialog = visual.debug_bounds("mobile-pair-dialog").unwrap();
        assert_eq!(dialog.size.width, px(560.));
        assert!((dialog.center().x - viewport.width / 2.).abs() < px(1.));
        assert!((dialog.center().y - viewport.height / 2.).abs() < px(1.));
        let qr = visual.debug_bounds("mobile-pair-code").unwrap();
        visual.simulate_click(qr.center(), Default::default());
        visual.run_until_parked();
        assert!(
            visual.debug_bounds("mobile-pair-overlay").is_some(),
            "clicking inside does not dismiss or reach Settings"
        );
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert!(visual.debug_bounds("mobile-pair-overlay").is_none());
        window
            .update(visual, |shell, _, _| {
                assert!(
                    shell.settings.is_some(),
                    "Escape only dismisses the pairing dialog"
                );
                assert!(shell.mobile.invite.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn settings_skill_form_opens_and_cancels_without_leaving_settings(cx: &mut TestAppContext) {
        cx.update(super::super::bind_keys);
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.open_settings(Section::Skills, w, cx)
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let add = visual.debug_bounds("settings-add-skill").unwrap();
        visual.simulate_click(add.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("settings-skill-form").is_some());
        assert!(visual.debug_bounds("settings-management-dialog").is_none());
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert!(visual.debug_bounds("settings-skill-form").is_none());
        window
            .update(visual, |shell, _, _| assert!(shell.settings.is_some()))
            .unwrap();
    }

    #[gpui::test]
    fn escape_closes_the_inner_settings_dialog_first(cx: &mut TestAppContext) {
        cx.update(super::super::bind_keys);
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, w, cx| shell.open_settings(Section::Mcp, w, cx))
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let add = visual.debug_bounds("settings-add-mcp").unwrap();
        visual.simulate_click(add.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("settings-management-dialog").is_some());
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| assert!(shell.settings.is_some()))
            .unwrap();
        assert!(visual.debug_bounds("settings-management-dialog").is_none());
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| assert!(shell.settings.is_none()))
            .unwrap();
    }
    #[gpui::test]
    fn settings_replace_native_rail_and_tabs_and_return_to_the_same_session(
        cx: &mut TestAppContext,
    ) {
        cx.update(super::super::bind_keys);
        let (window, _, active) = window(cx);
        window
            .update(cx, |shell, w, cx| shell.focus_active(w, cx))
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.simulate_keystrokes("secondary-,");
        visual.run_until_parked();
        let card = visual.debug_bounds("main-pane-frame").unwrap();
        let header = visual.debug_bounds("settings-header").unwrap();
        let page = visual.debug_bounds("settings-page-header").unwrap();
        assert!(
            header.bottom() <= px(40.),
            "breadcrumb and search live in the title bar"
        );
        assert!(
            (page.left()
                - card.left()
                - (card.size.width - px(8.) - px(1024.)).max(px(0.)) / 2.
                - px(33.))
            .abs()
                < px(1.),
            "React's 32px page padding plus the card border: page={page:?}, frame={card:?}"
        );
        assert!(visual.debug_bounds("settings-toggle-sounds").is_some());
        assert!(visual.debug_bounds("sidebar-content").is_none());
        for (section, selector) in Section::ALL.into_iter().zip([
            "settings-nav-general",
            "settings-nav-diagnostics",
            "settings-nav-connections",
            "settings-nav-appearance",
            "settings-nav-keybindings",
            "settings-nav-dictation",
            "settings-nav-chat",
            "settings-nav-providers",
            "settings-nav-mcp",
            "settings-nav-skills",
            "settings-nav-pilots",
            "settings-nav-inbox",
            "settings-nav-archive",
            "settings-nav-worktrees",
        ]) {
            let bounds = visual.debug_bounds(selector).unwrap();
            visual.simulate_click(bounds.center(), Default::default());
            visual.run_until_parked();
            window
                .update(visual, |shell, _, cx| {
                    assert_eq!(shell.settings.as_ref().unwrap().read(cx).section, section)
                })
                .unwrap();
            assert!(
                visual.debug_bounds("settings-page-header").is_some(),
                "page {} renders",
                section.label()
            );
        }
        let back = visual.debug_bounds("settings-nav-back").unwrap();
        visual.simulate_click(back.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(shell.settings.is_none());
                assert_eq!(shell.workspace.active, Some(active));
                assert_eq!(shell.workspace.tabs.len(), 2);
            })
            .unwrap();
        assert!(visual.debug_bounds("sidebar-content").is_some());
    }

    #[gpui::test]
    fn native_setting_edits_persist_and_theme_switches_live(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.open_settings(Section::Appearance, w, cx)
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let light = visual.debug_bounds("settings-theme-light").unwrap();
        visual.simulate_click(light.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.preferences.theme, "light");
                let saved = shell.latest_state.lock().unwrap().clone();
                let restored: Workspace = serde_json::from_str(&saved).unwrap();
                assert_eq!(restored.preferences.theme, "light");
                assert!(
                    content(1.).l < 0.3,
                    "the native text ink switched to the light palette"
                );
            })
            .unwrap();
        let glass = visual
            .debug_bounds("settings-toggle-main-pane-glass")
            .unwrap();
        assert!(glass.size.width > px(0.));
        let dark = visual.debug_bounds("settings-theme-dark").unwrap();
        visual.simulate_click(dark.center(), Default::default());
        visual.run_until_parked();
    }

    #[gpui::test]
    fn shortcut_recording_rebinds_native_settings_and_reset_restores_it(cx: &mut TestAppContext) {
        cx.update(crate::input::bind);
        cx.update(super::super::bind_keys);
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.open_settings(Section::Keybindings, w, cx)
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let recorder = visual.debug_bounds("shortcut-App: Settings").unwrap();
        visual.simulate_click(recorder.center(), Default::default());
        visual.simulate_keystrokes("secondary-k");
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(
                    !shell
                        .workspace
                        .preferences
                        .keybindings
                        .contains_key("App: Settings"),
                    "a default shortcut cannot be claimed twice"
                )
            })
            .unwrap();
        visual.simulate_keystrokes("secondary-alt-y");
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(
                    shell.workspace.preferences.keybindings["App: Settings"]
                        .as_ref()
                        .unwrap()
                        .ends_with('y')
                )
            })
            .unwrap();
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| assert!(shell.settings.is_none()))
            .unwrap();
        visual.simulate_keystrokes("secondary-,");
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(shell.settings.is_none(), "old chord is removed")
            })
            .unwrap();
        visual.simulate_keystrokes("secondary-alt-y");
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(
                    shell.settings.is_some(),
                    "the newly recorded chord dispatches the native action"
                )
            })
            .unwrap();
        window
            .update(visual, |shell, _, cx| {
                shell.select_settings(Section::Keybindings, cx)
            })
            .unwrap();
        visual.run_until_parked();
        let reset = visual.debug_bounds("reset-shortcut-App: Settings").unwrap();
        visual.simulate_click(reset.center(), Default::default());
        visual.run_until_parked();
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        visual.simulate_keystrokes("secondary-,");
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(
                    shell.settings.is_some(),
                    "reset restores the native default shortcut"
                )
            })
            .unwrap();
    }

    #[gpui::test]
    fn new_session_default_skips_an_uninstalled_provider(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        cx.update(|cx| {
            cx.set_global(crate::installed_providers::InstalledProviders::probed([
                Kind::Kiro,
                Kind::Hermes,
            ]))
        });
        window
            .update(cx, |shell, _, cx| {
                let project = shell.workspace.project.clone();
                let p = &mut shell.workspace.preferences;
                p.default_provider = None;
                assert_eq!(
                    shell.default_kind(cx),
                    Kind::Hermes,
                    "no preference: the first installed provider, not Codex"
                );
                shell.workspace.preferences.default_provider = Some("codex".into());
                assert_eq!(shell.default_kind(cx), Kind::Hermes);
                shell.workspace.preferences.default_provider = Some("kiro".into());
                assert_eq!(
                    shell.default_kind(cx),
                    Kind::Kiro,
                    "installed preference kept"
                );
                // A project provider that is missing falls through to the
                // last used one when installed, else the first installed.
                shell
                    .workspace
                    .preferences
                    .project_providers
                    .entry(project)
                    .or_default()
                    .provider = Some("claude".into());
                assert_eq!(shell.default_kind(cx), Kind::Kiro);
                shell.workspace.preferences.default_provider = Some("codex".into());
                assert_eq!(shell.default_kind(cx), Kind::Hermes);
            })
            .unwrap();
    }

    #[gpui::test]
    fn cold_start_keeps_the_preference_until_providers_are_probed(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.workspace.preferences.default_provider = Some("claude".into());
                assert_eq!(shell.default_kind(cx), Kind::Claude, "nothing probed yet");
                shell.workspace.preferences.default_provider = None;
                assert_eq!(shell.default_kind(cx), Kind::Codex);
                cx.set_global(crate::installed_providers::InstalledProviders::default());
                shell.workspace.preferences.default_provider = Some("grok".into());
                assert_eq!(shell.default_kind(cx), Kind::Grok, "unprobed set keeps it");
                cx.set_global(crate::installed_providers::InstalledProviders::probed([
                    Kind::Kiro,
                ]));
                assert_eq!(
                    shell.default_kind(cx),
                    Kind::Kiro,
                    "probed: Grok is missing"
                );
            })
            .unwrap();
    }

    // New-session provider and model: project default, then last used, then
    // the first installed provider; the model follows the chosen provider.
    fn only_installed(kinds: &[Kind], cx: &mut TestAppContext) {
        let kinds = kinds.to_vec();
        cx.update(|cx| {
            cx.set_global(crate::installed_providers::InstalledProviders::probed(
                kinds,
            ))
        });
    }
    fn active_chat(shell: &Shell) -> Entity<Conversation> {
        shell.sessions[&shell.workspace.active.unwrap()].clone()
    }
    fn create_default_session(
        window: gpui::WindowHandle<Shell>,
        cx: &mut TestAppContext,
    ) -> Entity<Conversation> {
        window
            .update(cx, |shell, _, cx| shell.create(shell.default_kind(cx), cx))
            .unwrap();
        cx.run_until_parked();
        window.update(cx, |shell, _, _| active_chat(shell)).unwrap()
    }

    #[gpui::test]
    fn new_session_uses_project_default_then_last_used_then_first_installed(
        cx: &mut TestAppContext,
    ) {
        let (window, _, _) = window(cx);
        only_installed(&[Kind::Claude, Kind::Hermes, Kind::Kiro], cx);
        window
            .update(cx, |shell, _, cx| {
                let project = shell.workspace.project.clone();
                let p = &mut shell.workspace.preferences;
                p.default_provider = None;
                p.last_provider = None;
                p.default_models.clear();
                assert_eq!(
                    shell.new_session_choice(cx),
                    (Kind::Claude, None),
                    "nothing chosen yet: the first installed provider"
                );
                let p = &mut shell.workspace.preferences;
                p.last_provider = Some("kiro".into());
                p.default_models.insert("kiro".into(), "kiro-fast".into());
                p.default_models
                    .insert("hermes".into(), "hermes-last".into());
                assert_eq!(
                    shell.new_session_choice(cx),
                    (Kind::Kiro, Some("kiro-fast".into())),
                    "last used beats the first installed, with its last model"
                );
                let settings = shell
                    .workspace
                    .preferences
                    .project_providers
                    .entry(project.clone())
                    .or_default();
                settings.provider = Some("hermes".into());
                assert_eq!(
                    shell.new_session_choice(cx),
                    (Kind::Hermes, Some("hermes-last".into())),
                    "the project default beats last used; no project model yet"
                );
                shell
                    .workspace
                    .preferences
                    .project_providers
                    .get_mut(&project)
                    .unwrap()
                    .models
                    .insert("hermes".into(), "hermes-project".into());
                assert_eq!(
                    shell.new_session_choice(cx),
                    (Kind::Hermes, Some("hermes-project".into())),
                    "the project's model for that provider comes first"
                );
                shell
                    .workspace
                    .preferences
                    .project_providers
                    .get_mut(&project)
                    .unwrap()
                    .provider = Some("codex".into());
                assert_eq!(
                    shell.new_session_choice(cx),
                    (Kind::Kiro, Some("kiro-fast".into())),
                    "a project default that is not installed falls to last used"
                );
                shell.workspace.preferences.last_provider = Some("grok".into());
                assert_eq!(
                    shell.new_session_choice(cx),
                    (Kind::Claude, None),
                    "nothing installed among them: the first installed provider"
                );
                // Providers hidden in this project are skipped like missing
                // ones, as the desktop's firstEnabledHarness does.
                shell
                    .workspace
                    .preferences
                    .project_providers
                    .get_mut(&project)
                    .unwrap()
                    .hidden = vec!["claude".into()];
                assert_eq!(
                    shell.new_session_choice(cx),
                    (Kind::Hermes, Some("hermes-project".into()))
                );
                shell.workspace.preferences.last_provider = Some("kiro".into());
                shell.workspace.preferences.hidden_providers = vec!["kiro".into()];
                assert_eq!(
                    shell.new_session_choice(cx),
                    (Kind::Hermes, Some("hermes-project".into())),
                    "a globally hidden last used provider is skipped too"
                );
                shell.workspace.preferences.hidden_providers.clear();
                shell
                    .workspace
                    .preferences
                    .project_providers
                    .get_mut(&project)
                    .unwrap()
                    .hidden
                    .clear();
                shell
                    .workspace
                    .preferences
                    .project_providers
                    .get_mut(&project)
                    .unwrap()
                    .provider = Some("hermes".into());
            })
            .unwrap();
        // A real new session starts there.
        let chat = create_default_session(window, cx);
        chat.read_with(cx, |chat, _| {
            assert_eq!(chat.summary.provider, "hermes");
            assert_eq!(chat.pilot_model().as_deref(), Some("hermes-project"));
        });
    }

    #[gpui::test]
    fn picking_a_model_in_a_chat_becomes_the_last_used_choice(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        only_installed(&[Kind::Claude, Kind::Codex, Kind::Kiro], cx);
        window
            .update(cx, |shell, _, cx| {
                shell.workspace.preferences.default_provider = Some("claude".into());
                shell.apply_preferences(cx);
            })
            .unwrap();
        let chat = window.update(cx, |shell, _, _| active_chat(shell)).unwrap();
        let pick = |kind: Kind, model: Option<&str>, cx: &mut TestAppContext| {
            chat.update(cx, |chat, cx| {
                assert!(chat.apply_model_pick(
                    &crate::pickers::ModelPicked(kind, model.map(Into::into)),
                    cx
                ));
            });
            cx.run_until_parked();
        };
        pick(Kind::Kiro, Some("kiro-fast"), cx);
        window
            .update(cx, |shell, _, cx| {
                let p = &shell.workspace.preferences;
                assert_eq!(p.last_provider.as_deref(), Some("kiro"));
                assert_eq!(p.default_models["kiro"], "kiro-fast");
                assert_eq!(
                    shell.new_session_choice(cx),
                    (Kind::Kiro, Some("kiro-fast".into()))
                );
            })
            .unwrap();
        // Each provider keeps its own last model.
        pick(Kind::Claude, Some("opus"), cx);
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(
                    shell.new_session_choice(cx),
                    (Kind::Claude, Some("opus".into()))
                );
                assert_eq!(
                    shell.workspace.preferences.default_models["kiro"],
                    "kiro-fast"
                );
            })
            .unwrap();
        // Picking the provider's Default entry forgets its last model.
        pick(Kind::Kiro, None, cx);
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.new_session_choice(cx), (Kind::Kiro, None));
                assert_eq!(shell.workspace.preferences.default_models["claude"], "opus");
            })
            .unwrap();
        let next = create_default_session(window, cx);
        next.read_with(cx, |next, _| {
            assert_eq!(next.summary.provider, "kiro");
            assert_eq!(next.pilot_model(), None);
        });
        // Re-picking a chat's current provider and model is still the most
        // recent pick.
        pick(Kind::Claude, Some("opus"), cx);
        next.update(cx, |next, cx| {
            assert!(next.apply_model_pick(&crate::pickers::ModelPicked(Kind::Kiro, None), cx));
        });
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.new_session_choice(cx), (Kind::Kiro, None));
            })
            .unwrap();
    }

    #[gpui::test]
    fn last_used_controls_follow_into_a_new_session(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        only_installed(&[Kind::Claude, Kind::Codex], cx);
        let chat = window.update(cx, |shell, _, _| active_chat(shell)).unwrap();
        chat.update(cx, |chat, cx| {
            assert_eq!(chat.summary.provider, "claude");
            chat.choose_setting_for_test("fast:on", cx);
            chat.choose_setting_for_test(r#"model-setting:["context","1m"]"#, cx);
            chat.choose_setting_for_test("effort:high", cx);
            chat.choose_setting_for_test("thinking", cx);
        });
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                let last = &shell.workspace.preferences.last_model_settings;
                assert_eq!(last.get("fast").map(String::as_str), Some("true"));
                assert_eq!(last.get("context").map(String::as_str), Some("1m"));
                shell.workspace.preferences.last_provider = Some("claude".into());
            })
            .unwrap();
        let next = create_default_session(window, cx);
        next.read_with(cx, |next, _| {
            assert_eq!(next.summary.provider, "claude");
            assert!(next.settings.fast, "fast follows");
            assert_eq!(
                next.settings
                    .model_settings
                    .get("context")
                    .map(String::as_str),
                Some("1m")
            );
            assert_eq!(next.settings.effort.as_deref(), Some("high"));
        });
        // Turning a control off is remembered too.
        next.update(cx, |next, cx| next.choose_setting_for_test("fast:off", cx));
        cx.run_until_parked();
        let third = create_default_session(window, cx);
        third.read_with(cx, |third, _| assert!(!third.settings.fast));
        // Controls another provider does not support stay behind.
        window
            .update(cx, |shell, _, _| {
                shell.workspace.preferences.last_provider = Some("codex".into());
            })
            .unwrap();
        let codex = create_default_session(window, cx);
        codex.read_with(cx, |codex, _| {
            assert_eq!(codex.summary.provider, "codex");
            assert!(!codex.settings.model_settings.contains_key("context"));
        });
        // Claude's fast, context and boolean thinking never reach providers
        // whose models do not define them; a Pi thinking level survives.
        only_installed(&[Kind::Omp, Kind::Cursor, Kind::Pi], cx);
        window
            .update(cx, |_, _, cx| {
                cx.update_global::<crate::settings::Preferences, _>(|p, _| {
                    p.last_model_settings.insert("fast".into(), "true".into());
                    p.provider_efforts.insert("pi".into(), "high".into());
                });
            })
            .unwrap();
        cx.run_until_parked();
        for provider in ["omp", "cursor", "pi"] {
            window
                .update(cx, |shell, _, _| {
                    shell.workspace.preferences.last_provider = Some(provider.into());
                })
                .unwrap();
            let chat = create_default_session(window, cx);
            chat.read_with(cx, |chat, _| {
                assert_eq!(chat.summary.provider, provider);
                assert!(!chat.settings.fast, "{provider}: fast");
                assert_eq!(chat.settings.thinking, None, "{provider}: thinking");
                assert!(
                    chat.settings.model_settings.is_empty(),
                    "{provider}: {:?}",
                    chat.settings.model_settings
                );
                if provider == "pi" {
                    assert_eq!(chat.settings.effort.as_deref(), Some("high"));
                }
            });
        }
    }

    #[gpui::test]
    fn first_time_uses_the_first_installed_provider_and_its_first_model(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        // Kind::ALL order: Hermes is listed before Kiro.
        only_installed(&[Kind::Kiro, Kind::Hermes], cx);
        window
            .update(cx, |shell, _, cx| {
                let p = &mut shell.workspace.preferences;
                p.default_provider = None;
                p.last_provider = None;
                p.default_models.clear();
                p.project_providers.clear();
                p.last_model_settings.clear();
                assert_eq!(shell.new_session_choice(cx), (Kind::Hermes, None));
            })
            .unwrap();
        let chat = create_default_session(window, cx);
        chat.read_with(cx, |chat, _| {
            assert_eq!(chat.summary.provider, "hermes");
            // `None` is the picker's first entry: the provider's Default.
            assert_eq!(chat.pilot_model(), None);
            assert!(!chat.settings.fast);
            assert!(chat.settings.model_settings.is_empty());
        });
    }

    #[gpui::test]
    fn settings_default_no_longer_outranks_last_used(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        only_installed(&[Kind::Claude, Kind::Kiro], cx);
        window
            .update(cx, |shell, _, cx| {
                let p = &mut shell.workspace.preferences;
                p.default_provider = Some("claude".into());
                p.last_provider = Some("kiro".into());
                assert_eq!(shell.default_kind(cx), Kind::Kiro);
                // Preferences saved before last-used existed keep their default.
                shell.workspace.preferences.last_provider = None;
                assert_eq!(shell.default_kind(cx), Kind::Claude);
                // Choosing a default in Settings → Providers is a last-used pick.
                shell.workspace.preferences.last_provider = Some("kiro".into());
                shell.workspace.preferences.use_by_default(Kind::Claude);
                assert_eq!(
                    shell.workspace.preferences.last_provider.as_deref(),
                    Some("claude")
                );
                assert_eq!(shell.default_kind(cx), Kind::Claude);
            })
            .unwrap();
        let old: crate::settings::Preferences =
            serde_json::from_str(r#"{"default_provider":"claude"}"#).unwrap();
        assert_eq!(old.last_provider, None);
        assert!(old.last_model_settings.is_empty());
    }

    #[gpui::test]
    fn the_disabled_model_picker_opens_settings_at_agent_clis(cx: &mut TestAppContext) {
        let (window, _, _) = window(cx);
        only_installed(&[], cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let pill = visual.debug_bounds("composer-model").unwrap();
        visual.simulate_click(pill.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                let settings = shell.settings.as_ref().expect("Settings opened");
                assert_eq!(settings.read(cx).section, Section::Providers);
            })
            .unwrap();
    }
}
