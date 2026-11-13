//! All title-strip commands operate on the same visible, ordered snapshot.
use super::*;
use crate::strip_groups::{StripEntry, entries, idle, make_contiguous};
use crate::workspace::TitleTab;
use gespan_core::files::path_key;

/// The wall clock in unix seconds, as `Workspace::tab_focused_at` keeps it.
pub(super) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Shell {
    /// The workspace keys a tab by one of its sessions. Replacing that session
    /// changes the key, but the same slot and selected width stay on screen.
    pub(super) fn replace_title_tab_key(&mut self, from: i64, to: i64) {
        if from == to {
            return;
        }
        let previous = TitleTab::Session(from);
        let next = TitleTab::Session(to);
        self.title_tab_motion.replace_key(&previous, next.clone());
        for selected in [
            &mut self.title_tab_selection.0,
            &mut self.title_tab_selection.1,
        ] {
            if selected.as_ref() == Some(&previous) {
                *selected = Some(next.clone());
            }
        }
        if let Some(at) = self.workspace.tab_focused_at.remove(&previous) {
            self.workspace.tab_focused_at.insert(next.clone(), at);
        }
        let shift = self.tab_shifts.remove(&previous);
        self.tab_shifts.remove(&next);
        if let Some(shift) = shift {
            self.tab_shifts.insert(next, shift);
        }
        self.pane_ticks.remove(&from);
        self.pane_ticks.remove(&to);
        if self.tab_hover == Some(from) {
            self.tab_hover = Some(to);
        }
        if self.pin_card_hover.as_ref() == Some(&previous) {
            self.pin_card_hover = Some(TitleTab::Session(to));
        }
    }

    pub(crate) fn click_pilot_tab(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if std::mem::take(&mut self.skip_tab_click) {
            return;
        }
        // A Pilot tab of a peeking group belongs to that project, not this one.
        let peeked = self.strip_peek.as_ref().and_then(|project| {
            self.workspace.title_order.iter().find(|tab| {
                matches!(tab, TitleTab::Pilot { id: pilot, project: p, .. }
                    if pilot == id && path_key(p) == path_key(project))
            })
        });
        let tab = peeked.cloned().unwrap_or_else(|| self.pilot_tab(id.into()));
        self.open_title_tab(&tab, window, cx);
    }
    pub(super) fn tab_scope(&self) -> String {
        format!(
            "{}\0{}",
            path_key(&self.workspace.project),
            path_key(self.working_copy())
        )
    }

    pub(super) fn pilot_tab(&self, id: String) -> TitleTab {
        TitleTab::Pilot {
            id,
            project: self.workspace.project.clone(),
            worktree: self.working_copy().into(),
        }
    }

    /// Every tab this scope shows, pinned ones first in pin order, then the
    /// strip's own. Shortcuts and cycling walk this order.
    pub(super) fn visible_tabs(&self, cx: &App) -> Vec<TitleTab> {
        let mut tabs = self.scoped_tabs(cx);
        let pinned = &self.workspace.pinned_tabs;
        // Stable, so the strip keeps its own order behind the pins.
        tabs.sort_by_key(|tab| pinned.iter().position(|p| p == tab).unwrap_or(usize::MAX));
        tabs
    }

    /// The tabs drawn as plates in the strip, right of the pinned deck.
    /// Drags, drops and strip indices count only these.
    pub(super) fn strip_tabs(&self, cx: &App) -> Vec<TitleTab> {
        self.scoped_tabs(cx)
            .into_iter()
            .filter(|tab| !self.workspace.pinned_tabs.contains(tab))
            .collect()
    }

    fn scoped_tabs(&self, cx: &App) -> Vec<TitleTab> {
        self.workspace
            .title_order
            .iter()
            .filter(|tab| {
                self.tab_drawable(tab, cx)
                    && match tab {
                        TitleTab::App(_) => true,
                        TitleTab::Session(tab) => {
                            let id = self.workspace.scope_pane(*tab);
                            self.session_metadata(self.workspace.session_for_pane(id), cx)
                                .summary
                                .cwd
                                == self.workspace.project
                                && self.in_worktree_scope(id, cx)
                        }
                        TitleTab::Pilot {
                            project, worktree, ..
                        } => {
                            *project == self.workspace.project
                                && path_key(worktree) == path_key(self.working_copy())
                        }
                    }
            })
            .cloned()
            .collect()
    }

    /// Whether `tab` can draw at all, whichever project it belongs to: its
    /// app view is turned on, its session's summary is loaded, its pilot
    /// still exists in a project the sidebar lists. A removed project's
    /// Pilot tab, saved by an older build, stays hidden.
    pub(super) fn tab_drawable(&self, tab: &TitleTab, cx: &App) -> bool {
        match tab {
            TitleTab::App(view) => {
                (*view != app_views::AppView::Notes || self.workspace.preferences.notes)
                    && (*view != app_views::AppView::Board || self.workspace.preferences.board)
                    && (*view != app_views::AppView::Flows || crate::flows::visible(cx))
            }
            TitleTab::Session(tab) => {
                self.workspace.tabs.contains(tab)
                    && self.session_summaries.contains_key(
                        &self
                            .workspace
                            .session_for_pane(self.workspace.scope_pane(*tab)),
                    )
            }
            TitleTab::Pilot { id, project, .. } => {
                self.workspace.preferences.pilots
                    && self
                        .workspace
                        .projects
                        .iter()
                        .any(|p| path_key(p) == path_key(project))
                    && self
                        .pilots
                        .as_ref()
                        .is_some_and(|m| m.read(cx).roster.get(id).is_some())
            }
        }
    }

    /// The project folder `tab` belongs to: a session's working folder, a
    /// pilot's project. App tabs, and sessions whose summary isn't loaded,
    /// have none.
    pub(super) fn tab_project(&self, tab: &TitleTab, cx: &App) -> Option<String> {
        match tab {
            TitleTab::App(_) => None,
            TitleTab::Session(tab) => {
                let session = self
                    .workspace
                    .session_for_pane(self.workspace.scope_pane(*tab));
                self.session_summaries
                    .contains_key(&session)
                    .then(|| self.session_metadata(session, cx).summary.cwd.clone())
            }
            TitleTab::Pilot { project, .. } => Some(project.clone()),
        }
    }

    /// Every drawable tab of every project, in strip order, pins left out.
    pub(super) fn global_strip_tabs(&self, cx: &App) -> Vec<TitleTab> {
        self.workspace
            .title_order
            .iter()
            .filter(|tab| self.tab_drawable(tab, cx) && !self.tab_pinned(tab))
            .cloned()
            .collect()
    }

    /// The strip for every project: each project's tabs as one group, where
    /// its first tab is, and app tabs loose between them.
    pub(super) fn strip_entries(&self, cx: &App) -> Vec<StripEntry<TitleTab>> {
        self.strip_frame(cx).0
    }

    /// [`Self::strip_entries`], with each grouped tab's project worked out
    /// once so a frame can look it up again.
    pub(super) fn strip_frame(
        &self,
        cx: &App,
    ) -> (Vec<StripEntry<TitleTab>>, HashMap<TitleTab, String>) {
        let tabs = self.global_strip_tabs(cx);
        let projects: HashMap<TitleTab, String> = tabs
            .iter()
            .filter_map(|tab| Some((tab.clone(), self.tab_project(tab, cx)?)))
            .collect();
        (entries(&tabs, |tab| projects.get(tab).cloned()), projects)
    }

    /// `tab`'s project from a frame's `projects`, else worked out, as for a
    /// tab easing off the strip.
    pub(super) fn frame_project(
        &self,
        projects: &HashMap<TitleTab, String>,
        tab: &TitleTab,
        cx: &App,
    ) -> Option<String> {
        projects
            .get(tab)
            .cloned()
            .or_else(|| self.tab_project(tab, cx))
    }

    /// The worktree `tab` runs in when that isn't its project folder, named
    /// as the sidebar names it. The strip itself looks the project up once
    /// a frame and calls [`Self::tab_worktree_label_in`].
    #[cfg(test)]
    pub(super) fn tab_worktree_label(&self, tab: &TitleTab, cx: &App) -> Option<String> {
        let project = self.tab_project(tab, cx)?;
        self.tab_worktree_label_in(tab, &project, cx)
    }

    /// [`Self::tab_worktree_label`] for a tab whose `project` is known.
    pub(super) fn tab_worktree_label_in(
        &self,
        tab: &TitleTab,
        project: &str,
        cx: &App,
    ) -> Option<String> {
        let worktree = match tab {
            TitleTab::App(_) => return None,
            TitleTab::Session(tab) => self.workspace.tab_workspaces.get(tab).map_or_else(
                || self.session_working_copy(self.workspace.scope_pane(*tab), cx),
                String::as_str,
            ),
            TitleTab::Pilot { worktree, .. } => worktree.as_str(),
        };
        (path_key(worktree) != path_key(project)).then(|| self.worktree_label_in(project, worktree))
    }

    /// Keeps each project's tabs side by side in `title_order`, so a tab
    /// opened for a project joins its group; the session list follows.
    pub(super) fn group_title_order(&mut self, cx: &App) {
        let projects: HashMap<TitleTab, Option<String>> = self
            .workspace
            .title_order
            .iter()
            .map(|tab| (tab.clone(), self.tab_project(tab, cx)))
            .collect();
        make_contiguous(&mut self.workspace.title_order, |tab| {
            projects.get(tab).cloned().flatten()
        });
        self.workspace.tabs = self
            .workspace
            .title_order
            .iter()
            .filter_map(TitleTab::session)
            .collect();
    }

    pub(super) fn active_title_tab(&self, cx: &App) -> Option<TitleTab> {
        if let Some(view) = self.active_app_view {
            return Some(TitleTab::App(view));
        }
        self.pilots
            .as_ref()
            .and_then(|m| m.read(cx).active.clone())
            .map(|id| self.pilot_tab(id))
            .or_else(|| self.workspace.active_tab().map(TitleTab::Session))
    }

    /// The active tab follows live focus; other tabs keep their last displayed
    /// pane instead of borrowing the currently selected tab's preview focus.
    pub(super) fn tab_review_focused(&self, tab: i64, cx: &App) -> bool {
        if self.active_title_tab(cx) == Some(TitleTab::Session(tab)) {
            self.review_focused
        } else {
            self.pane_ticks
                .get(&tab)
                .is_some_and(|tick| tick.shown == (self.workspace.tab_focus(tab), true))
        }
    }

    pub(super) fn remember_title_tab(&mut self, tab: TitleTab) {
        if !self.workspace.title_order.contains(&tab) {
            self.workspace.title_order.push(tab.clone());
        }
        self.workspace.scope_focus.insert(self.tab_scope(), tab);
    }

    pub(super) fn pilot_opened(&mut self, id: String, cx: &mut Context<Self>) {
        if self
            .pilots
            .as_ref()
            .is_none_or(|m| m.read(cx).active.as_deref() != Some(&id))
            || self.redirect_pilot_to_pane(&id, cx)
        {
            return;
        }
        self.navigation_revision += 1;
        self.selected_sessions.clear();
        self.selection_anchor = None;
        self.sidebar_target = None;
        self.context_selection = false;
        self.leave_app_views(cx);
        self.leave_app_search(cx);
        self.close_preview(cx);
        self.review_focused = false;
        self.remember_title_tab(self.pilot_tab(id));
        self.reveal_active_tab(cx);
        self.persist(cx);
        cx.notify();
    }

    pub(super) fn open_title_tab(
        &mut self,
        tab: &TitleTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Opening a tab ends any peek; its group is the open one now.
        self.strip_peek = None;
        match tab {
            TitleTab::App(view) => match view {
                app_views::AppView::Home => self.open_home(window, cx),
                app_views::AppView::MyTasks => self.open_my_tasks(window, cx),
                app_views::AppView::Inbox => self.open_inbox(window, cx),
                app_views::AppView::Notes => self.open_notes(window, cx),
                app_views::AppView::Automations => self.open_automations(window, cx),
                app_views::AppView::Flows => self.open_flows(window, cx),
                app_views::AppView::Board => self.open_board(window, cx),
                app_views::AppView::Runs => self.show_runs(window, cx),
            },
            TitleTab::Session(id) => {
                let focus = self.workspace.tab_focus(*id);
                let review_focused = self.tab_review_focused(*id, cx);
                self.open(focus, window, cx);
                if review_focused
                    && self.workspace.active == Some(focus)
                    && let Some(review) = self.active_review(cx)
                {
                    self.review_focused = true;
                    review.update(cx, |r, cx| r.focus_active(window, cx));
                }
            }
            TitleTab::Pilot {
                id,
                project,
                worktree,
            } => {
                // A Pilot gone from the roster can't open; land on its project.
                let exists = self
                    .pilots
                    .as_ref()
                    .is_some_and(|m| m.read(cx).roster.get(id).is_some());
                if !exists {
                    if path_key(project) != path_key(&self.workspace.project) {
                        self.select_project(project.clone(), window, cx);
                    }
                    return;
                }
                self.enter_pilot_scope(project, worktree);
                if let Some(pilots) = self.pilots.clone() {
                    pilots.update(cx, |m, cx| m.open(id, window, cx));
                }
            }
        }
    }

    /// Switches to the project and worktree a Pilot tab belongs to, so
    /// opening it lands in its own group, as a chat tab does.
    fn enter_pilot_scope(&mut self, project: &str, worktree: &str) {
        if path_key(project) == path_key(&self.workspace.project)
            && path_key(worktree) == path_key(self.working_copy())
        {
            return;
        }
        self.workspace.project = project.to_owned();
        self.workspace.collapsed.remove(project);
        if path_key(worktree) == path_key(project) {
            self.worktree_focus.remove(project);
        } else {
            let branch = crate::git_cache::get(project).and_then(|git| {
                git.worktrees
                    .into_iter()
                    .find(|tree| path_key(&tree.path) == path_key(worktree))
                    .and_then(|tree| tree.branch)
            });
            self.worktree_focus.insert(
                project.to_owned(),
                crate::worktree_switcher::WorktreeFocus {
                    path: worktree.to_owned(),
                    branch,
                },
            );
        }
        // The Pilot takes the window; the old project's chat is not active.
        self.workspace.active = None;
    }

    pub(super) fn restore_scope_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let tab = self.workspace.scope_focus.get(&self.tab_scope()).cloned();
        if let Some(tab) = tab.filter(|tab| self.visible_tabs(cx).contains(tab)) {
            if matches!(tab, TitleTab::Pilot { .. }) {
                self.workspace.active = None;
            }
            self.open_title_tab(&tab, window, cx);
            return true;
        }
        false
    }

    pub(crate) fn close_pilot_tab(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_title_tabs(&[self.pilot_tab(id.into())], None, window, cx);
    }

    pub(super) fn run_title_tab_menu(
        &mut self,
        tab: TitleTab,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tabs = self.visible_tabs(cx);
        let Some(index) = tabs.iter().position(|entry| *entry == tab) else {
            return;
        };
        if matches!(action, "pin" | "unpin") {
            self.toggle_pin_tab(tab, cx);
            return;
        }
        if action == "close" {
            if let Some(id) = tab.session() {
                self.close(id, window, cx);
            } else {
                self.close_title_tabs(&[tab], None, window, cx);
            }
            return;
        }
        // Like a browser's pinned tabs, these survive the bulk closes.
        let closing = tabs
            .into_iter()
            .enumerate()
            .filter(|(_, entry)| !self.workspace.pinned_tabs.contains(entry))
            .filter(|(ix, entry)| match action {
                "others" => *entry != tab,
                "right" => *ix > index,
                "left" => *ix < index,
                "all" => true,
                _ => false,
            })
            .map(|(_, entry)| entry)
            .collect::<Vec<_>>();
        self.close_title_tabs(&closing, (action == "others").then_some(tab), window, cx);
    }

    pub(super) fn close_title_tabs(
        &mut self,
        tabs: &[TitleTab],
        preferred: Option<TitleTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panes = tabs
            .iter()
            .filter(|tab| self.workspace.title_order.contains(tab))
            .filter_map(TitleTab::session)
            .flat_map(|id| self.workspace.layout(id).leaf_ids())
            .collect::<Vec<_>>();
        let tabs = tabs.to_vec();
        self.confirm_close_terminal_panes(&panes, window, cx, move |this, window, cx| {
            this.finish_close_title_tabs(&tabs, preferred, false, window, cx);
        });
    }

    /// Closes `tabs`. A `sweep` closes only tabs in the background, so it
    /// leaves menus, drags, hover and focus as they are.
    fn finish_close_title_tabs(
        &mut self,
        tabs: &[TitleTab],
        preferred: Option<TitleTab>,
        sweep: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.visible_tabs(cx);
        let closing: HashSet<_> = tabs
            .iter()
            .filter(|tab| self.workspace.title_order.contains(tab))
            .cloned()
            .collect();
        if closing.is_empty() {
            return;
        }
        let active = self.active_title_tab(cx);
        let replace = active.as_ref().is_some_and(|tab| closing.contains(tab));
        // Pins never close on their own, so the strip keeps one tab beside
        // them: closing its last falls back to a blank session, not a pin.
        let strip_left = before
            .iter()
            .any(|tab| !closing.contains(tab) && !self.workspace.pinned_tabs.contains(tab));
        let pins_left = before
            .iter()
            .any(|tab| !closing.contains(tab) && self.workspace.pinned_tabs.contains(tab));
        // Closing a whole group may name a tab of another project to open.
        let elsewhere = preferred.clone().filter(|tab| {
            !before.contains(tab)
                && !closing.contains(tab)
                && self.workspace.title_order.contains(tab)
                && self.tab_drawable(tab, cx)
        });
        let next = if !replace {
            None
        } else if elsewhere.is_some() {
            elsewhere
        } else if pins_left && !strip_left {
            None
        } else {
            preferred
                .filter(|tab| before.contains(tab) && !closing.contains(tab))
                .or_else(|| {
                    let index = before.iter().position(|tab| Some(tab) == active.as_ref())?;
                    // Browsers select the next tab to the right, then the one to the left.
                    before[index + 1..]
                        .iter()
                        .chain(before[..index].iter().rev())
                        .find(|tab| !closing.contains(*tab))
                        .cloned()
                })
        };
        self.navigation_revision += 1;
        if sweep {
            self.tab_shifts.retain(|tab, _| !closing.contains(tab));
            if self
                .tab_hover
                .is_some_and(|id| closing.contains(&TitleTab::Session(id)))
            {
                self.tab_hover = None;
            }
        } else {
            self.close_menu(cx);
            self.tab_drag = None;
            self.tab_shifts.clear();
            self.tab_hover = None;
            self.tab_scroll_task = None;
            self.cancel_pane_drag(cx);
        }
        let session_active = self.workspace.active;
        let threads = self.shown_threads();
        for tab in &closing {
            if let TitleTab::App(view) = tab
                && self.active_app_view == Some(*view)
            {
                self.leave_app_views(cx);
            }
            if let TitleTab::Session(id) = tab {
                self.workspace.close(*id);
            }
        }
        self.workspace
            .title_order
            .retain(|tab| !closing.contains(tab));
        self.workspace
            .scope_focus
            .retain(|_, tab| !closing.contains(tab));
        self.workspace.prune_pinned_tabs();
        self.close_left_threads_browser_tabs(threads, cx);
        if replace {
            if let Some(pilots) = self.pilots.clone() {
                pilots.update(cx, |m, cx| m.close(cx));
            }
            self.workspace.active = None;
            self.review_focused = false;
            self.review_slide = None;
            self.pane_motion = None;
            if let Some(tab) = next {
                self.open_title_tab(&tab, window, cx);
            } else {
                self.ensure_project_session(window, cx);
            }
        } else {
            self.workspace.active =
                session_active.filter(|id| self.workspace.tab_of(*id).is_some());
        }
        self.settle_pilot_panes(window, cx);
        self.history
            .prune(&self.workspace.open_sessions(), self.workspace.active);
        self.leave_closed_dictation(cx);
        if !sweep
            && self.workspace.active.is_none()
            && !self.pilot_active(cx)
            && !self.app_view_open()
        {
            self.focus_active(window, cx);
        }
        self.persist(cx);
        cx.notify();
    }

    /// Records that `tab` is focused now.
    pub(super) fn note_tab_focus(&mut self, tab: &TitleTab) {
        self.workspace
            .tab_focused_at
            .insert(tab.clone(), unix_now());
    }

    /// Closes tabs unfocused for `IDLE_AFTER_SECS` from the strip, without
    /// asking: their chats stay in the sidebar, nothing is archived. Returns
    /// the closed tabs. While a tab or pane drag or a menu is open it closes
    /// nothing; the next tick tries again.
    pub(super) fn close_idle_tabs(
        &mut self,
        now: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<TitleTab> {
        if self.tab_drag.is_some() || self.pane_drag.is_some() || self.menu.is_some() {
            return Vec::new();
        }
        let active = self.active_title_tab(cx);
        if let Some(tab) = &active {
            self.workspace.tab_focused_at.insert(tab.clone(), now);
        }
        self.workspace.start_tab_clocks(now);
        let closing = idle(
            &self.workspace.title_order,
            &self.workspace.tab_focused_at,
            now,
            |tab| active.as_ref() == Some(tab) || self.idle_keeps(tab, cx),
        );
        if !closing.is_empty() {
            self.finish_close_title_tabs(&closing, None, true, window, cx);
        }
        closing
    }

    /// A tab the idle cleanup leaves alone: pinned, a Pilot whose chat is
    /// running, or a chat tab with any pane that is a terminal (closing one
    /// would have to ask), a running Pilot, a file pane with unsaved edits,
    /// or a chat that is busy or holds unsent input or a saved draft.
    fn idle_keeps(&self, tab: &TitleTab, cx: &App) -> bool {
        if self.tab_pinned(tab) {
            return true;
        }
        let key = match tab {
            TitleTab::Session(key) => key,
            TitleTab::Pilot { id, .. } => return self.pilot_busy(id, cx),
            TitleTab::App(_) => return false,
        };
        self.workspace.layout(*key).leaf_ids().iter().any(|id| {
            self.workspace.terminal_panes.contains_key(id)
                || self
                    .workspace
                    .pilot_panes
                    .get(id)
                    .is_some_and(|pilot| match pilot {
                        TitleTab::Pilot { id, .. } => self.pilot_busy(id, cx),
                        _ => false,
                    })
                || self
                    .reviews
                    .get(id)
                    .is_some_and(|review| review.read(cx).unsaved_files(cx))
                || self.session_summaries.contains_key(id) && {
                    let s = self.session_metadata(*id, cx);
                    s.busy
                        || s.has_saved_draft()
                        || self
                            .sessions
                            .get(id)
                            .is_some_and(|v| !v.read(cx).input.read(cx).content.is_empty())
                }
        })
    }

    /// Whether Pilot `id`'s chat is running.
    fn pilot_busy(&self, id: &str, cx: &App) -> bool {
        self.pilots.as_ref().is_some_and(|m| {
            m.read(cx)
                .chat_for(id)
                .is_some_and(|chat| chat.read(cx).busy)
        })
    }

    /// Moves `tab` to slot `to` among its own kind: a grouped tab among its
    /// group's tabs, a loose tab among the strip's top-level entries. A tab
    /// never leaves its group, and nothing lands inside another group.
    pub(super) fn reorder_title_tab(&mut self, tab: &TitleTab, to: usize, cx: &App) {
        let entries = self.strip_entries(cx);
        for (ix, entry) in entries.iter().enumerate() {
            match entry {
                StripEntry::Loose(loose) if loose == tab => {
                    return self.reorder_entry(ix, to, cx);
                }
                StripEntry::Group { tabs, .. } => {
                    if let Some(from) = tabs.iter().position(|t| t == tab) {
                        let mut tabs = tabs.clone();
                        let moved = tabs.remove(from);
                        tabs.insert(to.min(tabs.len()), moved);
                        return self.write_strip_order(tabs);
                    }
                }
                StripEntry::Loose(_) => {}
            }
        }
    }

    /// Moves the strip's top-level entry `from` (a loose tab, or a whole
    /// group with its tabs in their order) to entry slot `to`.
    pub(super) fn reorder_entry(&mut self, from: usize, to: usize, cx: &App) {
        let mut entries = self.strip_entries(cx);
        if from >= entries.len() {
            return;
        }
        let moved = entries.remove(from);
        entries.insert(to.min(entries.len()), moved);
        let tabs = entries
            .into_iter()
            .flat_map(|entry| match entry {
                StripEntry::Loose(tab) => vec![tab],
                StripEntry::Group { tabs, .. } => tabs,
            })
            .collect();
        self.write_strip_order(tabs);
    }

    /// Moves `tab` to slot `to` of the current scope's strip tabs, the
    /// slots a pane dropped on the strip counts; pinned tabs keep their places.
    pub(super) fn place_in_strip_slot(&mut self, tab: &TitleTab, to: usize, cx: &App) {
        let mut visible = self.strip_tabs(cx);
        let Some(from) = visible.iter().position(|entry| entry == tab) else {
            return;
        };
        let entry = visible.remove(from);
        visible.insert(to.min(visible.len()), entry);
        self.write_strip_order(visible);
    }

    /// Writes `tabs` back over their own places in `title_order`, in the
    /// given order; every other tab keeps its place. The session list follows.
    fn write_strip_order(&mut self, tabs: Vec<TitleTab>) {
        let set: HashSet<_> = tabs.iter().cloned().collect();
        let mut ordered = tabs.into_iter();
        for entry in &mut self.workspace.title_order {
            if set.contains(entry) {
                *entry = ordered.next().unwrap();
            }
        }
        self.workspace.tabs = self
            .workspace
            .title_order
            .iter()
            .filter_map(TitleTab::session)
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::window;
    use gpui::{TestAppContext, WindowHandle};

    fn add_session(shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) -> i64 {
        let summary = locked(&shell.store)
            .unwrap()
            .create("codex", &shell.workspace.project)
            .unwrap();
        let id = summary.id;
        shell.add(summary, cx);
        let focus = shell.worktree_focus.get(&shell.workspace.project).cloned();
        shell.seed_worktree(id, focus.as_ref(), cx);
        shell.open(id, window, cx);
        id
    }

    fn add_pilot(window: WindowHandle<Shell>, cx: &mut TestAppContext) -> TitleTab {
        window
            .update(cx, |shell, w, cx| {
                shell
                    .pilot_manager()
                    .update(cx, |m, cx| m.create(Vec::new(), w, cx));
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| shell.active_title_tab(cx).unwrap())
            .unwrap()
    }

    #[gpui::test]
    fn mixed_tabs_open_in_order_and_close_to_the_visible_neighbor(cx: &mut TestAppContext) {
        let (window, other_project, first) = window(cx);
        let pilot = add_pilot(window, cx);
        let second = window.update(cx, add_session).unwrap();
        cx.run_until_parked();
        let last = add_pilot(window, cx);
        window
            .update(cx, |shell, w, cx| {
                assert_eq!(
                    shell.visible_tabs(cx),
                    [
                        TitleTab::Session(first),
                        pilot.clone(),
                        TitleTab::Session(second),
                        last.clone()
                    ]
                );
                shell.open_title_tab(&pilot, w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                shell.close_title_tabs(std::slice::from_ref(&pilot), None, w, cx);
                assert_eq!(shell.active_title_tab(cx), Some(TitleTab::Session(second)));
                shell.close_title_tabs(&[last], None, w, cx);
                assert_eq!(
                    shell.active_title_tab(cx),
                    Some(TitleTab::Session(second)),
                    "inactive close preserves focus"
                );
                assert!(
                    shell.sessions[&second]
                        .read(cx)
                        .input
                        .focus_handle(cx)
                        .is_focused(w)
                );
                shell.close(second, w, cx);
                assert_eq!(shell.workspace.active, Some(first));
                assert_eq!(shell.workspace.project, "/work/gespan");
                assert!(shell.workspace.tabs.contains(&other_project));
            })
            .unwrap();
    }

    #[gpui::test]
    fn pilot_close_button_and_cmd_w_share_selection_and_focus(cx: &mut TestAppContext) {
        let (window, _, first) = window(cx);
        cx.update(|cx| {
            bind_keys(cx);
            input::bind(cx);
            cx.set_reduce_motion(true);
        });
        let pilot = add_pilot(window, cx);
        let second = window.update(cx, add_session).unwrap();
        window
            .update(cx, |shell, w, cx| shell.open_title_tab(&pilot, w, cx))
            .unwrap();
        cx.run_until_parked();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        let close = visual.debug_bounds("close-pilot-tab-button").unwrap();
        visual.simulate_click(close.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, w, cx| {
                assert_eq!(shell.workspace.active, Some(second));
                assert_eq!(
                    shell.visible_tabs(cx),
                    [TitleTab::Session(first), TitleTab::Session(second)]
                );
                assert!(
                    shell.sessions[&second]
                        .read(cx)
                        .input
                        .focus_handle(cx)
                        .is_focused(w)
                );
            })
            .unwrap();
        let pilot = add_pilot(window, visual);
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        visual.simulate_keystrokes(&format!("{modifier}-w"));
        visual.run_until_parked();
        window
            .update(visual, |shell, w, cx| {
                assert!(!shell.visible_tabs(cx).contains(&pilot));
                assert_eq!(shell.workspace.active, Some(second));
                assert!(
                    shell.sessions[&second]
                        .read(cx)
                        .input
                        .focus_handle(cx)
                        .is_focused(w)
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn worktree_tabs_cycle_close_and_return_without_touching_other_scopes(cx: &mut TestAppContext) {
        let (window, other_project, main) = window(cx);
        let pilot_main = add_pilot(window, cx);
        let feature = crate::worktree_switcher::WorktreeFocus {
            path: "/work/feature".into(),
            branch: Some("feature".into()),
        };
        window
            .update(cx, |shell, w, cx| {
                shell.select_worktree(Some(feature.clone()), w, cx)
            })
            .unwrap();
        cx.run_until_parked();
        let feature_session = window
            .update(cx, |shell, _, _| shell.workspace.active.unwrap())
            .unwrap();
        let pilot_feature = add_pilot(window, cx);
        window
            .update(cx, |shell, w, cx| {
                assert_eq!(
                    shell.visible_tabs(cx),
                    [TitleTab::Session(feature_session), pilot_feature.clone()]
                );
                shell.cycle(1, w, cx);
                assert_eq!(shell.workspace.active, Some(feature_session));
                shell.activate_tab(-1, w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                assert_eq!(shell.active_title_tab(cx), Some(pilot_feature.clone()));
                let TitleTab::Pilot { id, .. } = &pilot_feature else {
                    panic!("expected a Pilot");
                };
                shell.run_menu(MenuTarget::PilotTab(id.clone()), "others", None, w, cx);
                assert_eq!(
                    shell.visible_tabs(cx).as_slice(),
                    std::slice::from_ref(&pilot_feature)
                );
                assert!(shell.workspace.tabs.contains(&main));
                assert!(shell.workspace.tabs.contains(&other_project));
                shell.select_worktree(None, w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                assert_eq!(shell.active_title_tab(cx), Some(pilot_main.clone()));
                assert_eq!(
                    shell.visible_tabs(cx),
                    [TitleTab::Session(main), pilot_main.clone()]
                );
                shell.select_worktree(Some(feature), w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                assert_eq!(shell.active_title_tab(cx), Some(pilot_feature));
                shell.close_title_tabs(&shell.visible_tabs(cx), None, w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.workspace.project, "/work/gespan");
                assert_eq!(shell.working_copy(), "/work/feature");
                assert_eq!(shell.visible_tabs(cx).len(), 1);
                assert!(shell.workspace.title_order.contains(&pilot_main));
                assert!(shell.workspace.tabs.contains(&other_project));
            })
            .unwrap();
    }

    #[gpui::test]
    fn mixed_reorder_and_restart_preserve_order_and_selected_pilot(cx: &mut TestAppContext) {
        let (window, _, first) = window(cx);
        let pilot = add_pilot(window, cx);
        let second = window.update(cx, add_session).unwrap();
        window
            .update(cx, |shell, w, cx| {
                shell.reorder_title_tab(&pilot, 0, cx);
                shell.open_title_tab(&pilot, w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let (store, summaries, saved) = window
            .update(cx, |shell, _, cx| {
                assert_eq!(
                    shell.visible_tabs(cx),
                    [
                        pilot.clone(),
                        TitleTab::Session(first),
                        TitleTab::Session(second)
                    ]
                );
                let db = locked(&shell.store).unwrap();
                (
                    shell.store.clone(),
                    db.sessions().unwrap(),
                    serde_json::to_string(&shell.workspace).unwrap(),
                )
            })
            .unwrap();
        let restored = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Shell::new(store, summaries, "/work/gespan".into(), Some(saved), cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        restored
            .update(cx, |shell, _, cx| {
                assert_eq!(
                    shell.visible_tabs(cx),
                    [
                        pilot.clone(),
                        TitleTab::Session(first),
                        TitleTab::Session(second)
                    ]
                );
                assert_eq!(shell.active_title_tab(cx), Some(pilot));
            })
            .unwrap();
    }

    #[gpui::test]
    fn rapid_new_tabs_follow_request_order(cx: &mut TestAppContext) {
        let (window, _, first) = window(cx);
        let providers = [Kind::Claude, Kind::Codex, Kind::Claude, Kind::Codex];
        window
            .update(cx, |shell, _, cx| {
                for provider in providers {
                    shell.create(provider, cx);
                }
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                let tabs = shell.visible_tabs(cx);
                assert_eq!(tabs.len(), 5);
                assert_eq!(tabs[0], TitleTab::Session(first));
                for (tab, provider) in tabs[1..].iter().zip(providers) {
                    assert_eq!(
                        shell.sessions[&tab.session().unwrap()]
                            .read(cx)
                            .summary
                            .provider,
                        provider.id()
                    );
                }
                assert_eq!(shell.active_title_tab(cx).as_ref(), tabs.last());
            })
            .unwrap();
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use crate::shell::tests::window;
    use gpui::{Modifiers, TestAppContext, point};

    fn sidebar_highlight(
        window: gpui::WindowHandle<Shell>,
        selector: String,
        cx: &mut TestAppContext,
    ) -> bool {
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        let selector = Box::leak(selector.into_boxed_str());
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("the sidebar row {selector} is visible"));
        window
            .update(visual, |_, window, _| {
                let scale = window.scale_factor();
                window.painted_quads().into_iter().any(|quad| {
                    (quad.bounds.left().0 - f32::from(bounds.left()) * scale).abs() < 1.
                        && (quad.bounds.top().0 - f32::from(bounds.top()) * scale).abs() < 1.
                        && (quad.bounds.size.width.0 - f32::from(bounds.size.width) * scale).abs()
                            < 1.
                        && (quad.bounds.size.height.0 - f32::from(bounds.size.height) * scale).abs()
                            < 1.
                        && quad
                            .background
                            .as_solid()
                            .is_some_and(|color| (color.a - 0.1).abs() < 0.001)
                })
            })
            .unwrap()
    }

    #[gpui::test]
    fn sidebar_highlights_only_the_visible_pilot_or_session(cx: &mut TestAppContext) {
        let (window, other, session) = window(cx);
        window
            .update(cx, |shell, w, cx| {
                for id in [other, session] {
                    shell.sessions[&id].update(cx, |view, _| {
                        view.summary.cwd = "/work/gespan".into();
                        view.summary.has_messages = true;
                    });
                }
                shell
                    .pilot_manager()
                    .update(cx, |m, cx| m.create(vec!["/work/gespan".into()], w, cx));
            })
            .unwrap();
        cx.run_until_parked();
        let pilot = window
            .update(cx, |shell, w, cx| {
                let pilot = shell.active_title_tab(cx).unwrap();
                shell.open(session, w, cx);
                pilot
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        let TitleTab::Pilot { id, .. } = &pilot else {
            panic!("expected a Pilot");
        };
        let pilot_row = format!("pilot-rail-{id}");
        let session_row = format!("sidebar-session-{session}");
        assert!(sidebar_highlight(window, session_row.clone(), cx));
        assert!(!sidebar_highlight(window, pilot_row.clone(), cx));
        let click = |selector: &str, cx: &mut TestAppContext| {
            let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
            let bounds = visual
                .debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
                .unwrap();
            visual.simulate_click(bounds.center(), Default::default());
            visual.simulate_mouse_move(point(px(5.), px(5.)), None, Default::default());
            visual.run_until_parked();
            visual.refresh().unwrap();
        };
        click(&pilot_row, cx);
        assert!(sidebar_highlight(window, pilot_row.clone(), cx));
        assert!(
            !sidebar_highlight(window, session_row.clone(), cx),
            "a remembered session must not look active while a Pilot is visible"
        );
        window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.workspace.active, Some(session));
            })
            .unwrap();
        click(&session_row, cx);
        assert!(sidebar_highlight(window, session_row.clone(), cx));
        assert!(!sidebar_highlight(window, pilot_row.clone(), cx));
        // Opening from the title strip must also clear an old bulk selection.
        window
            .update(cx, |shell, w, cx| {
                for id in [other, session] {
                    shell.select_sidebar_session(
                        id,
                        Modifiers {
                            platform: true,
                            ..Default::default()
                        },
                        w,
                        cx,
                    );
                }
                assert_eq!(shell.selected_sessions.len(), 2);
                shell.open_title_tab(&pilot, w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        window
            .update(cx, |shell, _, _| {
                assert!(shell.selected_sessions.is_empty());
                assert!(shell.selection_anchor.is_none());
                assert!(shell.sidebar_target.is_none());
            })
            .unwrap();
        assert!(sidebar_highlight(window, pilot_row, cx));
        assert!(!sidebar_highlight(window, session_row, cx));
        assert!(!sidebar_highlight(
            window,
            format!("sidebar-session-{other}"),
            cx
        ));
    }

    #[gpui::test]
    fn closing_a_session_button_selects_the_adjacent_pilot_and_restores_its_composer(
        cx: &mut TestAppContext,
    ) {
        let (window, _, first) = window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell
                    .pilot_manager()
                    .update(cx, |m, cx| m.create(Vec::new(), w, cx));
            })
            .unwrap();
        cx.run_until_parked();
        let pilot = window
            .update(cx, |shell, w, cx| {
                let pilot = shell.active_title_tab(cx).unwrap();
                shell.open(first, w, cx);
                pilot
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let close = visual
            .debug_bounds(Box::leak(
                format!("close-title-tab-{first}").into_boxed_str(),
            ))
            .unwrap();
        visual.simulate_click(close.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, w, cx| {
                assert_eq!(shell.active_title_tab(cx), Some(pilot));
                let chat = shell.pilot_manager().read(cx).chat().unwrap();
                assert!(chat.read(cx).input.focus_handle(cx).is_focused(w));
                assert_eq!(shell.workspace.project, "/work/gespan");
            })
            .unwrap();
    }

    #[gpui::test]
    fn dragging_a_pilot_uses_the_mixed_strip_slots_and_keeps_the_active_session(
        cx: &mut TestAppContext,
    ) {
        let (window, _, first) = window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell
                    .pilot_manager()
                    .update(cx, |m, cx| m.create(Vec::new(), w, cx));
            })
            .unwrap();
        cx.run_until_parked();
        let pilot = window
            .update(cx, |shell, w, cx| {
                let pilot = shell.active_title_tab(cx).unwrap();
                shell.open(first, w, cx);
                pilot
            })
            .unwrap();
        let TitleTab::Pilot { id, .. } = &pilot else {
            panic!("expected a Pilot");
        };
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let session_bounds = visual
            .debug_bounds(Box::leak(format!("title-tab-{first}").into_boxed_str()))
            .unwrap();
        let pilot_bounds = visual
            .debug_bounds(Box::leak(format!("pilot-title-tab-{id}").into_boxed_str()))
            .unwrap();
        visual.simulate_mouse_down(
            pilot_bounds.center(),
            MouseButton::Left,
            Modifiers::default(),
        );
        visual.simulate_mouse_move(
            point(session_bounds.center().x, pilot_bounds.center().y),
            MouseButton::Left,
            Modifiers::default(),
        );
        visual.run_until_parked();
        visual.simulate_mouse_up(
            point(session_bounds.center().x, pilot_bounds.center().y),
            MouseButton::Left,
            Modifiers::default(),
        );
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(shell.visible_tabs(cx), [pilot, TitleTab::Session(first)]);
                assert_eq!(shell.workspace.active, Some(first));
                assert!(!shell.pilot_active(cx));
            })
            .unwrap();
    }
}
