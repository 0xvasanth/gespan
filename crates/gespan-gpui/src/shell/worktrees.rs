//! Project workspace selection, independent of the project's identity. Existing
//! conversations keep their execution directory and running provider process.
use super::*;
use crate::worktree_switcher::WorktreeFocus;
use gespan_core::files::path_key;

impl Shell {
    pub(super) fn working_copy(&self) -> &str {
        self.worktree_focus
            .get(&self.workspace.project)
            .map_or(&self.workspace.project, |focus| &focus.path)
    }
    pub(super) fn session_working_copy<'a>(&'a self, id: i64, cx: &'a App) -> &'a str {
        let id = self.workspace.session_for_pane(id);
        let session = self.session_metadata(id, cx);
        self.session_worktrees
            .get(&id)
            .and_then(|p| p.as_deref())
            .or_else(|| {
                self.sessions
                    .get(&id)
                    .and_then(|v| v.read(cx).settings.worktree_cwd.as_deref())
            })
            .unwrap_or(&session.summary.cwd)
    }
    pub(super) fn in_worktree_scope(&self, id: i64, cx: &App) -> bool {
        let scope = self
            .workspace
            .tab_of(id)
            .and_then(|tab| self.workspace.tab_workspaces.get(&tab))
            .map_or_else(|| self.session_working_copy(id, cx), String::as_str);
        path_key(scope) == path_key(self.working_copy())
    }
    /// The heading a worktree's group takes in the sidebar: its branch, or
    /// its folder's name when the branch is not known yet.
    pub(super) fn worktree_label(&self, path: &str) -> String {
        self.worktree_label_in(&self.workspace.project, path)
    }
    /// [`Self::worktree_label`] for a worktree of `project`, whichever
    /// project is current.
    pub(super) fn worktree_label_in(&self, project: &str, path: &str) -> String {
        if let Some(branch) = crate::git_cache::worktree_branch(project, path) {
            return branch;
        }
        if path_key(path) == path_key(project) {
            return "Project folder".into();
        }
        crate::workspace::project_name(path)
    }
    /// A chat that made its own worktree takes its tab there, and the window
    /// follows it when it is the chat on screen, so it is never shown under
    /// the worktree it was started from.
    pub(super) fn session_made_worktree(&mut self, id: i64, path: String, cx: &mut Context<Self>) {
        self.session_worktrees.insert(id, Some(path.clone()));
        if let Some(tab) = self.workspace.tab_of(id) {
            self.workspace.tab_workspaces.insert(tab, path.clone());
        }
        let project = self.session_metadata(id, cx).summary.cwd.clone();
        let on_screen = self
            .workspace
            .active
            .map(|active| self.workspace.session_for_pane(active))
            == Some(id);
        if on_screen && path_key(&project) == path_key(&self.workspace.project) {
            let work_path = path.clone();
            let work = cx.background_executor().spawn(async move {
                crate::git_cache::load(std::path::Path::new(&work_path)).ok()
            });
            cx.spawn(async move |this, cx| {
                let branch = work.await.and_then(|git| git.current);
                let _ = this.update(cx, |this, cx| {
                    // Unless the user has moved on to another worktree since.
                    if this
                        .workspace
                        .active
                        .map(|a| this.workspace.session_for_pane(a))
                        == Some(id)
                    {
                        this.worktree_focus
                            .insert(project, WorktreeFocus { path, branch });
                        this.sync_git(cx);
                        this.persist(cx);
                        this.invalidate_chrome(cx);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        self.persist(cx);
        self.invalidate_chrome(cx);
        cx.notify();
    }
    /// A worktree the window shows took a generated branch name: show it.
    pub(super) fn worktree_renamed(&mut self, path: String, cx: &mut Context<Self>) {
        if !self
            .worktree_focus
            .values()
            .any(|focus| path_key(&focus.path) == path_key(&path))
        {
            return;
        }
        let work_path = path.clone();
        let work = cx
            .background_executor()
            .spawn(async move { crate::git_cache::load(std::path::Path::new(&work_path)).ok() });
        cx.spawn(async move |this, cx| {
            let Some(branch) = work.await.and_then(|git| git.current) else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                for focus in this.worktree_focus.values_mut() {
                    if path_key(&focus.path) == path_key(&path) {
                        focus.branch = Some(branch.clone());
                    }
                }
                this.persist(cx);
                this.invalidate_chrome(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn record_worktree_session(&mut self, id: i64, _cx: &App) {
        let project = self.workspace.project.clone();
        let path = self.working_copy().to_owned();
        // Opening a saved chat must not silently narrow the sidebar. Only an
        // explicit workspace selection changes that project-wide filter.
        if let Some(tab) = self.workspace.tab_of(id) {
            self.workspace.tab_workspaces.insert(tab, path.clone());
        }
        self.worktree_returns.insert((project, path), id);
    }
    pub(super) fn select_worktree(
        &mut self,
        focus: Option<WorktreeFocus>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self.workspace.project.clone();
        if let Some(id) = self.workspace.active {
            self.worktree_returns
                .insert((project.clone(), self.working_copy().into()), id);
        }
        let target = focus.as_ref().map_or(&project, |f| &f.path).clone();
        match focus {
            Some(focus) => {
                self.worktree_focus.insert(project.clone(), focus);
            }
            None => {
                self.worktree_focus.remove(&project);
            }
        }
        self.navigation_revision += 1;
        self.leave_app_views(cx);
        self.close_preview(cx);
        if let Some(pilots) = self.pilots.clone() {
            pilots.update(cx, |m, cx| m.close(cx));
        }
        if self.restore_scope_tab(window, cx) {
            return;
        }
        let matches = |id: &i64| {
            self.session_summaries
                .contains_key(&self.workspace.session_for_pane(*id))
                && {
                    let s = self.session_metadata(self.workspace.session_for_pane(*id), cx);
                    s.summary.cwd == project && !s.summary.archived
                }
                && self.in_worktree_scope(*id, cx)
                && self.workspace.tab_of(*id).is_some()
        };
        let active = self
            .worktree_returns
            .get(&(project.clone(), target.clone()))
            .copied()
            .filter(matches)
            .or_else(|| self.workspace.active.filter(matches))
            .or_else(|| {
                self.workspace
                    .tabs
                    .iter()
                    .rev()
                    .flat_map(|tab| self.workspace.layout(*tab).leaf_ids())
                    .find(matches)
            });
        self.workspace.active = active;
        if let Some(id) = active {
            self.activate_session(id, window, cx);
        } else {
            self.ensure_project_session(window, cx);
        }
        self.focus_active(window, cx);
        self.persist(cx);
        cx.notify();
    }
    pub(super) fn seed_worktree(
        &mut self,
        id: i64,
        focus: Option<&WorktreeFocus>,
        cx: &mut Context<Self>,
    ) {
        let path = focus.map(|f| f.path.clone());
        self.session_worktrees.insert(id, path.clone());
        if let Some(session) = self.sessions.get(&id) {
            session.update(cx, |session, cx| {
                session.restore_worktree(path, focus.and_then(|f| f.branch.clone()), cx);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::window;
    use gpui::TestAppContext;

    #[gpui::test]
    fn settings_entry_opens_settings_and_the_project_dropdown_has_no_worktree_row(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            input::bind(cx);
            crate::popover::bind(cx);
            crate::shell::bind_keys(cx);
        });
        let (window, _, active) = window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        // Settings lives in the All projects block.
        let settings = visual.debug_bounds("sidebar-navigation-Settings").unwrap();
        visual.simulate_click(settings.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, w, cx| {
                let settings = shell.settings.as_ref().expect("Settings is enabled");
                assert!(settings.focus_handle(cx).contains_focused(w, cx));
                assert!(shell.menu.is_none());
                assert_eq!(shell.workspace.active, Some(active));
            })
            .unwrap();
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| assert!(shell.settings.is_none()))
            .unwrap();
        // Worktrees are picked in the composer: the project dropdown is the
        // project picker and nothing else.
        let project = visual.debug_bounds("sidebar-project-button").unwrap();
        visual.simulate_click(project.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("menu-action-switch-worktree").is_none());
        assert!(visual.debug_bounds("worktree-switcher").is_none());
        window
            .update(visual, |shell, _, cx| {
                let open = shell.menu.as_ref().expect("the project picker opened");
                assert!(open.target == MenuTarget::Header);
                assert!(open.menu.read(cx).action_ids().is_empty());
            })
            .unwrap();
    }

    #[gpui::test]
    fn project_folder_selection_keeps_subfolder_sessions_and_new_session_cwd(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            input::bind(cx);
            crate::popover::bind(cx);
            crate::shell::bind_keys(cx);
        });
        let project = "/work/gespan";
        crate::git_cache::remember(
            project,
            &gespan_core::git::Snapshot {
                worktrees: vec![gespan_core::git::Worktree {
                    path: "/work".into(),
                    branch: Some("main".into()),
                    main: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        let (window, _, existing) = window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.sessions[&existing]
                    .update(cx, |session, _| session.summary.has_messages = true);
                // The project folder as the selected working copy.
                shell.select_worktree(
                    Some(WorktreeFocus {
                        path: project.into(),
                        branch: Some("main".into()),
                    }),
                    w,
                    cx,
                );
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                assert!(shell.sidebar_session_ids(cx).contains(&existing));
                shell.create(Kind::Claude, cx);
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                let id = shell.workspace.active.unwrap();
                assert_ne!(id, existing);
                let session = shell.sessions[&id].read(cx);
                assert_eq!(session.summary.cwd, project);
                assert_eq!(
                    session
                        .settings
                        .worktree_cwd
                        .as_deref()
                        .unwrap_or(&session.summary.cwd),
                    project
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn copies_keep_running_sessions_and_route_new_tabs_and_splits(cx: &mut TestAppContext) {
        let (window, feature_session, main_session) = window(cx);
        window
            .update(cx, |shell, w, cx| shell.open(feature_session, w, cx))
            .unwrap();
        cx.run_until_parked();
        let project = "/work/gespan";
        let feature = WorktreeFocus {
            path: "/work/gespan-worktrees/feature".into(),
            branch: Some("feature".into()),
        };
        let other = WorktreeFocus {
            path: "/work/gespan-worktrees/other".into(),
            branch: Some("other".into()),
        };
        window
            .update(cx, |shell, w, cx| {
                shell.sessions[&feature_session].update(cx, |session, cx| {
                    session.summary.cwd = project.into();
                    session.settings.worktree_cwd = Some(feature.path.clone());
                    session.busy = true;
                    session
                        .input
                        .update(cx, |input, cx| input.set_text("keep this draft".into(), cx));
                });
                shell
                    .session_worktrees
                    .insert(feature_session, Some(feature.path.clone()));
                shell
                    .workspace
                    .tab_workspaces
                    .insert(feature_session, feature.path.clone());
                shell.sessions[&main_session]
                    .update(cx, |session, _| session.summary.has_messages = true);
                shell.open(main_session, w, cx);
                shell
                    .groups
                    .entry(project.into())
                    .or_default()
                    .create_folder(&[main_session], "saved-folder".into());
                shell.select_worktree(Some(feature.clone()), w, cx);
                assert_eq!(shell.workspace.project, project);
                assert_eq!(shell.workspace.active, Some(feature_session));
                // The sidebar lists every worktree's sessions, grouped.
                assert_eq!(
                    shell
                        .sidebar_session_ids(cx)
                        .into_iter()
                        .collect::<HashSet<_>>(),
                    HashSet::from([main_session, feature_session])
                );
                assert_eq!(shell.terminal_cwd(cx), feature.path);
                shell.select_worktree(None, w, cx);
                assert_eq!(shell.workspace.active, Some(main_session));
                assert_eq!(
                    shell
                        .sidebar_session_ids(cx)
                        .into_iter()
                        .collect::<HashSet<_>>(),
                    HashSet::from([main_session, feature_session]),
                    "the default workspace lists the whole project"
                );
                assert_eq!(shell.terminal_cwd(cx), project);
                shell.open(feature_session, w, cx);
                assert_eq!(shell.working_copy(), project);
                assert!(shell.worktree_focus.is_empty());
                assert!(
                    shell
                        .visible_tabs(cx)
                        .contains(&TitleTab::Session(feature_session)),
                    "opening a worktree chat keeps its tab in the current workspace"
                );
                assert!(shell
                    .session_entries(&shell.sidebar_session_ids(cx), cx)
                    .iter()
                    .any(|entry| matches!(entry, gespan_core::session_groups::Entry::Folder { id, .. } if id == "saved-folder")),
                    "opening a worktree chat must not hide the project's other folders");
                shell.open(main_session, w, cx);
                // A late blank from the other copy cannot take over after returning.
                shell.select_worktree(Some(other.clone()), w, cx);
                shell.select_worktree(None, w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                assert_eq!(shell.workspace.active, Some(main_session));
                assert_eq!(shell.sessions.len(), 3);
                shell.select_worktree(Some(other.clone()), w, cx);
                let id = shell
                    .workspace
                    .active
                    .expect("reuse the completed blank for this copy");
                assert_eq!(shell.sessions.len(), 3);
                assert_eq!(shell.sessions[&id].read(cx).summary.cwd, project);
                assert_eq!(
                    shell.sessions[&id]
                        .read(cx)
                        .settings
                        .worktree_cwd
                        .as_deref(),
                    Some(other.path.as_str())
                );
                assert_eq!(
                    locked(&shell.store)
                        .unwrap()
                        .agent_settings(id)
                        .unwrap()
                        .unwrap()
                        .worktree_cwd
                        .as_deref(),
                    Some(other.path.as_str())
                );
                shell.create(Kind::Claude, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.sessions.len(), 4);
                assert_eq!(shell.terminal_cwd(cx), other.path);
                shell.split_session(SplitDir::Right, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                assert_eq!(shell.sessions.len(), 5);
                assert_eq!(shell.workspace.project, project);
                assert_eq!(shell.terminal_cwd(cx), other.path);
                let running = shell.sessions[&feature_session].read(cx);
                assert!(running.busy);
                assert_eq!(running.input.read(cx).content.as_ref(), "keep this draft");
                assert!(shell.worktree_focus.contains_key(project));
                // A directly opened chat joins the selected workspace without
                // changing either its checkout or the explicit sidebar filter.
                shell.open(main_session, w, cx);
                assert_eq!(shell.worktree_focus[project].path, other.path);
                assert!(
                    shell
                        .visible_tabs(cx)
                        .contains(&TitleTab::Session(main_session))
                );
                assert_eq!(shell.terminal_cwd(cx), project);
                assert_eq!(
                    shell.worktree_returns[&(project.to_owned(), other.path.clone())],
                    main_session
                );
                shell.select_project("/work/another-project".into(), w, cx);
                shell.select_project(project.into(), w, cx);
                assert_eq!(shell.worktree_focus[project].path, other.path);
                assert_eq!(shell.workspace.active, Some(main_session));
            })
            .unwrap();
    }
}
