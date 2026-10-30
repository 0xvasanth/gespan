//! Viewport state: closing a tab never deletes or stops a session.
//! A title tab is keyed by one of its sessions; a tab holding more than one
//! pane keeps its split tree in `layouts` and its last focused pane in
//! `focused`. `active` is always the focused session.
use crate::pane_layout::{Edge, PaneNode, SplitDir, leaf};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum AppView {
    Inbox,
    Notes,
    Automations,
    Flows,
    Board,
}
impl AppView {
    pub fn label(self) -> &'static str {
        match self {
            Self::Inbox => "Inbox",
            Self::Notes => "Notes",
            Self::Automations => "Automations",
            Self::Flows => "Flows",
            Self::Board => "Board",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Notes => "note",
            Self::Automations => "zap",
            Self::Flows => "git-merge",
            Self::Board => "kanban",
        }
    }
}

/// The title strip has one order, including conversations, app views and Pilots.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum TitleTab {
    Session(i64),
    Search,
    App(AppView),
    Pilot {
        id: String,
        project: String,
        worktree: String,
    },
}

impl TitleTab {
    pub fn session(&self) -> Option<i64> {
        match self {
            Self::Session(id) => Some(*id),
            Self::Pilot { .. } | Self::Search | Self::App(_) => None,
        }
    }
}

impl TitleTab {
    /// Builds from before Flows reject an unknown `AppView` and with it the
    /// whole saved workspace, so a Flows tab lives only while the app runs.
    fn saved(&self) -> bool {
        *self != Self::App(AppView::Flows)
    }
}

fn save_title_order<S: serde::Serializer>(tabs: &[TitleTab], s: S) -> Result<S::Ok, S::Error> {
    s.collect_seq(tabs.iter().filter(|tab| tab.saved()))
}

fn save_scope_focus<S: serde::Serializer>(
    focus: &HashMap<String, TitleTab>,
    s: S,
) -> Result<S::Ok, S::Error> {
    s.collect_map(focus.iter().filter(|(_, tab)| tab.saved()))
}

/// A named section of projects in the original project navigation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ProjectRailGroup {
    pub id: String,
    pub name: String,
    pub collapsed: bool,
    pub color_index: Option<usize>,
    pub custom_color: Option<String>,
    pub mascot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Workspace {
    pub window: crate::window_state::WindowState,
    pub tabs: Vec<i64>,
    #[serde(serialize_with = "save_title_order")]
    pub title_order: Vec<TitleTab>,
    #[serde(serialize_with = "save_scope_focus")]
    pub scope_focus: HashMap<String, TitleTab>,
    /// Sidebar narrowing belongs to this run. Reopen on the full project,
    /// while saved conversations retain their own execution directories.
    #[serde(skip)]
    pub worktree_focus: HashMap<String, crate::worktree_switcher::WorktreeFocus>,
    /// Directly opened chats join the workspace on screen without changing
    /// their execution directory. Like workspace selection, these pins are local
    /// to the current run and disappear when their tabs close.
    #[serde(skip)]
    pub tab_workspaces: HashMap<i64, String>,
    pub active: Option<i64>,
    pub projects: Vec<String>,
    /// Projects archived from the rail, newest first. Their chats stay in
    /// history; adding the project again takes it out of this list.
    pub archived_projects: Vec<String>,
    pub project: String,
    /// Last focused open session in each project, independent of tab order.
    pub project_focus: HashMap<String, i64>,
    pub recent_files: HashMap<String, Vec<String>>,
    pub collapsed: HashSet<String>,
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
    pub compact_session_cards: bool,
    pub sidebar_notes_visible: bool,
    pub sidebar_automations_visible: bool,
    pub sidebar_inbox_visible: bool,
    pub sidebar_activity_visible: bool,
    /// The inset right panel: open state, tab, width and tab order.
    pub right_panel_open: bool,
    pub right_panel_tab: crate::right_panel::RightTab,
    pub right_panel_width: f32,
    pub right_panel_tabs: Vec<crate::right_panel::RightTab>,
    pub changes_view: crate::changes::ChangesView,
    /// Each tab's file/review pane beside its chat.
    pub reviews: std::collections::HashMap<i64, crate::review::ReviewState>,
    /// Independent file panes use negative IDs in the layout tree. Their
    /// originating session supplies project/worktree scope and Add to Chat.
    pub review_owners: HashMap<i64, i64>,
    /// Pilots dragged into a split, by their negative pane IDs. Each keeps the
    /// Pilot tab it came from, which it becomes again once it stands alone.
    pub pilot_panes: HashMap<i64, TitleTab>,
    /// Split trees of tabs with more than one pane, by tab key.
    pub layouts: HashMap<i64, PaneNode>,
    /// The pane each split tab returns to when it is activated.
    pub focused: HashMap<i64, i64>,
    /// Each project's terminal dock (ProjectTerminalDock), by project path.
    pub docks: HashMap<String, TerminalDock>,
    /// Each project's saved commands, run from the footer's Actions button.
    pub actions: HashMap<String, Vec<ProjectAction>>,
    /// The id of the action each project last ran; the footer shows it.
    pub last_actions: HashMap<String, String>,
    /// The editor the title bar's Open in editor last opened a folder in.
    pub last_external_editor: Option<String>,
    /// Original layout: separate project and session sidebars.
    pub classic_layout: bool,
    /// The new single-sidebar layout can keep its full-height, square frame.
    pub new_layout_boxy: bool,
    pub project_rail_open: bool,
    pub project_rail_pilots_pinned: bool,
    pub project_rail_width: f32,
    pub project_rail_pinned: HashSet<String>,
    pub project_rail_groups: Vec<ProjectRailGroup>,
    pub project_rail_assignments: HashMap<String, String>,
    pub classic_sidebar_tabs: HashMap<String, String>,
    pub classic_sidebar_tab_order: Vec<String>,
    pub new_layout_prompt_dismissed: bool,
    pub preferences: crate::settings::Preferences,
    pub session_filters: gespan_core::session_filters::Filters,
    pub project_appearance: HashMap<String, gespan_core::project_appearance::ProjectAppearance>,
    /// Last desktop source values, used to reconcile subsequent desktop changes
    /// while retaining deliberate native project and preference edits.
    pub desktop_project_preferences: Option<serde_json::Value>,
    /// Unmodified desktop viewport metadata, including pane types and explicit
    /// tab-group assignments that do not yet have a native representation.
    pub desktop_workspace_snapshot: Option<serde_json::Value>,
    /// Exact source strings retained for preferences with no native equivalent.
    pub desktop_preferences_metadata: Option<serde_json::Value>,
}

impl Workspace {
    /// What another window starts from: this one's look and project, without
    /// the tabs and panes that show this window's chats.
    pub fn for_new_window(&self) -> Self {
        Self {
            tabs: Vec::new(),
            title_order: Vec::new(),
            scope_focus: HashMap::new(),
            worktree_focus: HashMap::new(),
            tab_workspaces: HashMap::new(),
            active: None,
            project_focus: HashMap::new(),
            reviews: HashMap::new(),
            review_owners: HashMap::new(),
            pilot_panes: HashMap::new(),
            layouts: HashMap::new(),
            focused: HashMap::new(),
            docks: HashMap::new(),
            ..self.clone()
        }
    }
    /// The settings all windows have in common, which one window's change
    /// passes to the rest: everything but what `adopt_shared` keeps.
    pub fn shared_part(&self) -> Self {
        let blank = Self::default();
        Self {
            window: blank.window,
            project: blank.project,
            sidebar_visible: blank.sidebar_visible,
            sidebar_width: blank.sidebar_width,
            right_panel_open: blank.right_panel_open,
            right_panel_tab: blank.right_panel_tab,
            right_panel_width: blank.right_panel_width,
            right_panel_tabs: blank.right_panel_tabs,
            ..self.for_new_window()
        }
    }
    /// Takes `shared`'s common settings, keeping this window's own view.
    pub fn adopt_shared(&mut self, shared: &Self) {
        let own = std::mem::replace(self, shared.clone());
        self.window = own.window;
        self.tabs = own.tabs;
        self.title_order = own.title_order;
        self.scope_focus = own.scope_focus;
        self.worktree_focus = own.worktree_focus;
        self.tab_workspaces = own.tab_workspaces;
        self.active = own.active;
        self.project = own.project;
        self.project_focus = own.project_focus;
        self.sidebar_visible = own.sidebar_visible;
        self.sidebar_width = own.sidebar_width;
        self.right_panel_open = own.right_panel_open;
        self.right_panel_tab = own.right_panel_tab;
        self.right_panel_width = own.right_panel_width;
        self.right_panel_tabs = own.right_panel_tabs;
        self.reviews = own.reviews;
        self.review_owners = own.review_owners;
        self.pilot_panes = own.pilot_panes;
        self.layouts = own.layouts;
        self.focused = own.focused;
        self.docks = own.docks;
    }
}

/// A saved command the footer runs in a terminal with one click.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ProjectAction {
    pub id: String,
    pub name: String,
    pub command: String,
    /// One of `ACTION_ICONS`.
    pub icon: String,
}
/// The icons an action can wear, in picker order; the first is the default.
pub(crate) const ACTION_ICONS: [&str; 6] =
    ["play", "test-tube", "hammer", "rocket", "bug", "package"];
impl ProjectAction {
    pub fn icon(&self) -> &'static str {
        ACTION_ICONS
            .iter()
            .find(|icon| **icon == self.icon)
            .copied()
            .unwrap_or(ACTION_ICONS[0])
    }
}

/// Port of src/features/projects/model/projectTerminal.ts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DockSide {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}
impl DockSide {
    pub fn vertical(self) -> bool {
        matches!(self, DockSide::Top | DockSide::Bottom)
    }
    /// DOCK_SIZE_DEFAULT.
    pub fn default_size(self) -> f32 {
        if self.vertical() { 220. } else { 360. }
    }
    /// clampDockSize: at least a usable strip, at most 70% of the window.
    pub fn clamp(self, value: f32, viewport: (f32, f32)) -> f32 {
        if !value.is_finite() {
            return self.default_size();
        }
        let (min, max) = self.limits(viewport);
        value.round().clamp(min, max)
    }
    pub fn limits(self, viewport: (f32, f32)) -> (f32, f32) {
        let (min, span) = if self.vertical() {
            (88., viewport.1)
        } else {
            (180., viewport.0)
        };
        (min, (span * 0.7).floor().max(min))
    }
}
/// A project's dock: its place, size and the terminals it holds. Shells are
/// not restored; the dock reopens the same tabs in their directories.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TerminalDock {
    pub side: DockSide,
    pub size: f32,
    pub open: bool,
    /// Each terminal's working directory, in tab order.
    pub terminals: Vec<String>,
    pub active: usize,
}
impl Default for TerminalDock {
    fn default() -> Self {
        Self {
            side: DockSide::Bottom,
            size: DockSide::Bottom.default_size(),
            open: true,
            terminals: Vec::new(),
            active: 0,
        }
    }
}
impl TerminalDock {
    /// addTerminalToDock: the new terminal is active and the dock opens.
    pub fn add(&mut self, cwd: String) -> usize {
        self.terminals.push(cwd);
        self.active = self.terminals.len() - 1;
        self.open = true;
        self.active
    }
    /// closeTerminalInDock: the next tab takes over a closed active one.
    /// False once the last terminal is gone.
    pub fn close(&mut self, ix: usize) -> bool {
        if ix >= self.terminals.len() {
            return true;
        }
        self.terminals.remove(ix);
        if self.terminals.is_empty() {
            self.active = 0;
            return false;
        }
        if self.active > ix || self.active >= self.terminals.len() {
            self.active = self.active.saturating_sub(1).min(self.terminals.len() - 1);
        }
        true
    }
    /// onReorderTerminals: the active terminal stays active wherever it lands.
    pub fn reorder(&mut self, from: usize, to: usize) {
        let len = self.terminals.len();
        if from >= len || to >= len || from == to {
            return;
        }
        let active = self.active;
        let cwd = self.terminals.remove(from);
        self.terminals.insert(to, cwd);
        self.active = if active == from {
            to
        } else if from < active && active <= to {
            active - 1
        } else if to <= active && active < from {
            active + 1
        } else {
            active
        };
    }
    /// onCloseOtherProjectTerminals: `keep` is left, and active.
    pub fn keep_only(&mut self, keep: usize) {
        if keep < self.terminals.len() {
            self.terminals = vec![self.terminals.swap_remove(keep)];
            self.active = 0;
        }
    }
    pub fn set_side(&mut self, side: DockSide) {
        if side != self.side {
            // A dock moved across axes starts from that axis's default.
            if side.vertical() != self.side.vertical() {
                self.size = side.default_size();
            }
            self.side = side;
        }
    }
}
impl Default for Workspace {
    fn default() -> Self {
        Self {
            window: Default::default(),
            tabs: Vec::new(),
            title_order: Vec::new(),
            scope_focus: HashMap::new(),
            worktree_focus: HashMap::new(),
            tab_workspaces: HashMap::new(),
            active: None,
            projects: Vec::new(),
            archived_projects: Vec::new(),
            project: String::new(),
            project_focus: HashMap::new(),
            recent_files: HashMap::new(),
            collapsed: HashSet::new(),
            sidebar_visible: true,
            sidebar_width: 260.,
            compact_session_cards: true,
            sidebar_notes_visible: true,
            sidebar_automations_visible: true,
            sidebar_inbox_visible: true,
            sidebar_activity_visible: true,
            right_panel_open: false,
            right_panel_tab: Default::default(),
            right_panel_width: crate::right_panel::WIDTH_DEFAULT,
            right_panel_tabs: crate::right_panel::RightTab::normalize(&[]),
            changes_view: Default::default(),
            reviews: Default::default(),
            review_owners: Default::default(),
            pilot_panes: Default::default(),
            layouts: Default::default(),
            focused: Default::default(),
            docks: Default::default(),
            actions: HashMap::new(),
            last_actions: HashMap::new(),
            last_external_editor: None,
            classic_layout: false,
            // Fresh installs start on the boxy frame; saved choices are kept.
            new_layout_boxy: true,
            project_rail_open: true,
            project_rail_pilots_pinned: false,
            project_rail_width: 220.,
            project_rail_pinned: HashSet::new(),
            project_rail_groups: Vec::new(),
            project_rail_assignments: HashMap::new(),
            classic_sidebar_tabs: HashMap::new(),
            classic_sidebar_tab_order: Vec::new(),
            new_layout_prompt_dismissed: false,
            preferences: crate::settings::Preferences::default(),
            session_filters: Default::default(),
            project_appearance: Default::default(),
            desktop_project_preferences: None,
            desktop_workspace_snapshot: None,
            desktop_preferences_metadata: None,
        }
    }
}
impl Workspace {
    /// Seed original-rail concepts from the exact desktop preference strings.
    /// Native edits then live in their own persisted fields.
    pub(crate) fn restore_project_rail_metadata(&mut self) {
        let Some(metadata) = self.desktop_preferences_metadata.as_ref() else {
            return;
        };
        let read = |key: &str| -> Option<serde_json::Value> {
            metadata
                .get(key)?
                .as_str()
                .and_then(|s| serde_json::from_str(s).ok())
        };
        if let Some(value) = read("gespan.railPilotsPinned") {
            self.project_rail_pilots_pinned = value.as_bool().unwrap_or(false);
        }
        if self.classic_sidebar_tab_order.is_empty()
            && let Some(value) = read("gespan.sidebarTabOrder")
        {
            self.classic_sidebar_tab_order = serde_json::from_value::<Vec<String>>(value)
                .unwrap_or_default()
                .into_iter()
                .filter(|tab| ["sessions", "files", "changes"].contains(&tab.as_str()))
                .collect();
        }
        if self.classic_sidebar_tabs.is_empty()
            && let Some(value) = read("gespan.projectSidebarTabs.v1")
        {
            self.classic_sidebar_tabs = serde_json::from_value::<HashMap<String, String>>(value)
                .unwrap_or_default()
                .into_iter()
                .filter(|(_, tab)| ["sessions", "files", "changes"].contains(&tab.as_str()))
                .collect();
        }
        if let Some(value) = read("gespan.projectRailPinned") {
            self.project_rail_pinned = serde_json::from_value(value).unwrap_or_default();
        }
        if let Some(value) = read("gespan.projectGroups") {
            self.project_rail_groups = serde_json::from_value(value).unwrap_or_default();
        }
        if let Some(value) = read("gespan.projectGroupAssignments") {
            self.project_rail_assignments = serde_json::from_value(value).unwrap_or_default();
        }
        if let Some(value) = read("gespan.projectRailOrder")
            && let Ok(order) = serde_json::from_value::<Vec<String>>(value)
        {
            let mut projects: Vec<_> = order
                .into_iter()
                .filter(|p| self.projects.contains(p))
                .collect();
            projects.extend(
                self.projects
                    .iter()
                    .filter(|p| !projects.contains(p))
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            self.projects = projects;
        }
    }

    pub fn boxy_layout(&self) -> bool {
        self.classic_layout || self.new_layout_boxy
    }

    pub fn restore(saved: Option<&str>, sessions: &[(i64, String)], cwd: &str) -> Self {
        let mut state = saved
            .and_then(|s| serde_json::from_str::<Self>(s).ok())
            .unwrap_or_default();
        // Before the project rail returned, "classic" meant the boxy version
        // of the single-sidebar layout. Keep that appearance for existing users.
        if let Some(saved) = saved.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            && saved.get("new_layout_boxy").is_none()
        {
            state.new_layout_boxy = state.classic_layout;
            state.classic_layout = false;
        }
        let has_rail_state = saved
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .is_some_and(|value| value.get("project_rail_pinned").is_some());
        if !has_rail_state {
            state.restore_project_rail_metadata();
        }
        // Hidden project navigation is retired; collapsed always means icons.
        state.preferences.collapsed_project_rail = "compact".into();
        state.project_rail_width = if state.project_rail_width.is_finite() {
            state.project_rail_width.clamp(176., 360.)
        } else {
            220.
        };
        if state.project.is_empty() {
            state.project = cwd.into();
        }
        for appearance in state.project_appearance.values_mut() {
            appearance.normalize();
        }
        let mut known: HashSet<_> = sessions.iter().map(|s| s.0).collect();
        state.review_owners.retain(|id, owner| {
            *id < 0
                && known.contains(owner)
                && state.reviews.get(id).is_some_and(|r| !r.tabs.is_empty())
        });
        known.extend(state.review_owners.keys().copied());
        let owners = &state.review_owners;
        state.pilot_panes.retain(|id, pilot| {
            *id < 0 && !owners.contains_key(id) && matches!(pilot, TitleTab::Pilot { .. })
        });
        known.extend(state.pilot_panes.keys().copied());
        let mut seen = HashSet::new();
        let mut layouts = std::mem::take(&mut state.layouts);
        let mut focused = std::mem::take(&mut state.focused);
        for tab in std::mem::take(&mut state.tabs) {
            let layout = layouts.remove(&tab).unwrap_or_else(|| leaf(tab));
            let Some(layout) = layout.retain(&mut |id| known.contains(&id) && seen.insert(id))
            else {
                continue;
            };
            let key = if layout.contains(tab) {
                tab
            } else {
                layout.first_leaf()
            };
            state.tabs.push(key);
            for entry in state
                .title_order
                .iter_mut()
                .chain(state.scope_focus.values_mut())
            {
                if *entry == TitleTab::Session(tab) {
                    *entry = TitleTab::Session(key);
                }
            }
            if layout.is_split() {
                if let Some(pane) = focused.remove(&tab).filter(|id| layout.contains(*id)) {
                    state.focused.insert(key, pane);
                }
                state.layouts.insert(key, layout);
            }
        }
        state.settle_pilot_panes();
        state.normalize_title_order();
        if state
            .desktop_project_preferences
            .as_ref()
            .and_then(|source| source.get("projects"))
            .is_some()
            && state.active.is_some_and(|id| {
                sessions.iter().any(|(known, path)| {
                    *known == state.session_for_pane(id) && !state.projects.contains(path)
                })
            })
        {
            // An old native sidecar could focus an internal session folder.
            // It must not restore that folder as a remembered user project.
            state.active = None;
        }
        if state.active.is_none_or(|id| state.tab_of(id).is_none()) {
            state.active = state
                .project_focus
                .get(&state.project)
                .copied()
                .filter(|id| {
                    state.tab_of(*id).is_some()
                        && sessions.iter().any(|(known, path)| {
                            *known == state.session_for_pane(*id) && *path == state.project
                        })
                })
                .or_else(|| {
                    state
                        .tabs
                        .iter()
                        .rev()
                        .map(|tab| state.tab_focus(*tab))
                        .find(|id| {
                            sessions.iter().any(|(known, path)| {
                                *known == state.session_for_pane(*id) && *path == state.project
                            })
                        })
                });
        }
        if saved.is_none()
            && let Some((id, _)) = sessions.iter().find(|(_, path)| path == cwd)
        {
            state.open(*id);
        }
        state.sidebar_width = state.sidebar_width.clamp(200., 560.);
        state.right_panel_width = state
            .right_panel_width
            .clamp(crate::right_panel::WIDTH_MIN, crate::right_panel::WIDTH_MAX)
            .round();
        state.right_panel_tabs = crate::right_panel::RightTab::normalize(&state.right_panel_tabs);
        state
            .reviews
            .retain(|id, review| known.contains(id) && !review.tabs.is_empty());
        state.docks.retain(|_, dock| !dock.terminals.is_empty());
        for dock in state.docks.values_mut() {
            dock.active = dock.active.min(dock.terminals.len() - 1);
            dock.size = dock.side.clamp(dock.size, (4000., 4000.));
        }
        state.add_project(cwd);
        let open: HashSet<_> = state.open_sessions().into_iter().collect();
        let review_owners = &state.review_owners;
        state.project_focus.retain(|project, id| {
            open.contains(id)
                && sessions.iter().any(|(known, path)| {
                    *known == review_owners.get(id).copied().unwrap_or(*id) && path == project
                })
        });
        if let Some((_, project)) = sessions.iter().find(|(id, _)| {
            state
                .active
                .is_some_and(|pane| state.session_for_pane(pane) == *id)
        }) {
            state.project = project.clone();
            state
                .project_focus
                .insert(project.clone(), state.active.unwrap());
        }
        state.add_project(&state.project.clone());
        state
    }
    pub fn add_project(&mut self, path: &str) {
        self.archived_projects.retain(|p| p != path);
        if !self.projects.iter().any(|p| p == path) {
            self.projects.push(path.into());
        }
    }
    /// The rail group a project is listed under. Assignments to deleted
    /// groups leave the project in the ungrouped list.
    pub fn project_rail_group(&self, path: &str) -> Option<&str> {
        let id = self.project_rail_assignments.get(path)?;
        self.project_rail_groups
            .iter()
            .any(|group| &group.id == id)
            .then_some(id.as_str())
    }
    /// Whether two projects sit in the same rail section and reorder together.
    pub fn project_rail_peers(&self, a: &str, b: &str) -> bool {
        let pinned = self.project_rail_pinned.contains(a);
        pinned == self.project_rail_pinned.contains(b)
            && (pinned || self.project_rail_group(a) == self.project_rail_group(b))
    }
    /// Projects in the order the current layout shows them. The rail lists
    /// pinned projects, then each group's members, then the rest.
    pub fn visible_project_order(&self) -> Vec<String> {
        if !self.classic_layout {
            return self.projects.clone();
        }
        let unpinned = |p: &&String| !self.project_rail_pinned.contains(*p);
        let mut order: Vec<String> = self
            .projects
            .iter()
            .filter(|p| self.project_rail_pinned.contains(*p))
            .cloned()
            .collect();
        for group in &self.project_rail_groups {
            order.extend(
                self.projects
                    .iter()
                    .filter(unpinned)
                    .filter(|p| self.project_rail_group(p) == Some(group.id.as_str()))
                    .cloned(),
            );
        }
        order.extend(
            self.projects
                .iter()
                .filter(unpinned)
                .filter(|p| self.project_rail_group(p).is_none())
                .cloned(),
        );
        order
    }
    /// The projects that reorder alongside `path`, in the order shown: the
    /// whole list, or just its rail section.
    pub fn project_peers(&self, path: &str) -> Vec<String> {
        self.visible_project_order()
            .into_iter()
            .filter(|p| !self.classic_layout || self.project_rail_peers(p, path))
            .collect()
    }
    /// Moves a project to `to` in the saved order. Returns whether it moved.
    pub fn move_project(&mut self, path: &str, to: usize) -> bool {
        let Some(from) = self.projects.iter().position(|p| p == path) else {
            return false;
        };
        let to = to.min(self.projects.len() - 1);
        if from == to {
            return false;
        }
        let project = self.projects.remove(from);
        self.projects.insert(to, project);
        true
    }
    /// Moves a project one place along what the layout shows, staying inside
    /// its rail section. Returns whether it moved.
    pub fn step_project_order(&mut self, path: &str, step: isize) -> bool {
        let peers = self.project_peers(path);
        let Some(ix) = peers.iter().position(|p| p == path) else {
            return false;
        };
        let Some(neighbour) = ix
            .checked_add_signed(step.signum())
            .and_then(|ix| peers.get(ix))
        else {
            return false;
        };
        let to = self.projects.iter().position(|p| p == neighbour).unwrap();
        self.move_project(path, to)
    }
    /// Moves a project to either end of its section: `step < 0` for the start.
    pub fn move_project_to_end(&mut self, path: &str, step: isize) -> bool {
        let peers = self.project_peers(path);
        let target = if step < 0 {
            peers.first()
        } else {
            peers.last()
        };
        let Some(to) = target.and_then(|t| self.projects.iter().position(|p| p == t)) else {
            return false;
        };
        self.move_project(path, to)
    }
    pub fn session_for_pane(&self, id: i64) -> i64 {
        self.review_owners.get(&id).copied().unwrap_or(id)
    }
    /// File and Pilot panes share the negative IDs below every session.
    pub fn next_pane_id(&self) -> i64 {
        self.review_owners
            .keys()
            .chain(self.pilot_panes.keys())
            .copied()
            .min()
            .unwrap_or(0)
            .min(0)
            - 1
    }
    pub fn pilot_pane(&self, pilot: &str) -> Option<i64> {
        self.pilot_panes.iter().find_map(|(pane, tab)| {
            matches!(tab, TitleTab::Pilot { id, .. } if id == pilot).then_some(*pane)
        })
    }
    /// The pane that scopes a tab to a project: its focus, unless that is a
    /// Pilot, which works across projects.
    pub fn scope_pane(&self, tab: i64) -> i64 {
        let focus = self.tab_focus(tab);
        if !self.pilot_panes.contains_key(&focus) {
            return focus;
        }
        self.layout(tab)
            .leaf_ids()
            .into_iter()
            .find(|id| !self.pilot_panes.contains_key(id))
            .unwrap_or(focus)
    }
    /// Focus a session, opening it as its own tab unless a tab holds it.
    pub fn open(&mut self, id: i64) {
        match self.tab_of(id) {
            Some(tab) => {
                if self.layouts.contains_key(&tab) {
                    self.focused.insert(tab, id);
                }
            }
            None => {
                self.tabs.push(id);
                self.title_order.push(TitleTab::Session(id));
            }
        }
        self.active = Some(id);
    }
    /// Close the whole title tab holding `id`; its sessions keep running.
    pub fn close(&mut self, id: i64) {
        let Some(tab) = self.tab_of(id) else {
            return;
        };
        let ix = self.tabs.iter().position(|t| *t == tab).unwrap_or(0);
        let layout = self.layout(tab);
        self.tabs.remove(ix);
        self.tab_workspaces.remove(&tab);
        self.title_order
            .retain(|entry| *entry != TitleTab::Session(tab));
        self.scope_focus
            .retain(|_, entry| *entry != TitleTab::Session(tab));
        self.layouts.remove(&tab);
        self.focused.remove(&tab);
        self.project_focus.retain(|_, id| !layout.contains(*id));
        if self
            .active
            .is_some_and(|active| self.tab_of(active).is_none())
        {
            self.active = self
                .tabs
                .get(ix)
                .or_else(|| self.tabs.last())
                .map(|tab| self.tab_focus(*tab));
        }
    }
    /// The title tab holding a session.
    pub fn tab_of(&self, id: i64) -> Option<i64> {
        self.tabs.iter().copied().find(|tab| {
            *tab == id
                || self
                    .layouts
                    .get(tab)
                    .is_some_and(|layout| layout.contains(id))
        })
    }
    pub fn active_tab(&self) -> Option<i64> {
        self.active.and_then(|id| self.tab_of(id))
    }
    pub fn layout(&self, tab: i64) -> PaneNode {
        self.layouts.get(&tab).cloned().unwrap_or_else(|| leaf(tab))
    }
    /// Every session shown in some tab.
    pub fn open_sessions(&self) -> Vec<i64> {
        self.tabs
            .iter()
            .flat_map(|tab| self.layout(*tab).leaf_ids())
            .collect()
    }
    /// The pane a tab activates.
    pub fn tab_focus(&self, tab: i64) -> i64 {
        self.focused
            .get(&tab)
            .copied()
            .filter(|id| self.layouts.get(&tab).is_some_and(|l| l.contains(*id)))
            .unwrap_or(tab)
    }
    /// Store a tab's tree, rekeying it when its key session left and
    /// dropping the tree once a single pane remains.
    fn set_layout(&mut self, tab: i64, layout: PaneNode) -> i64 {
        let Some(ix) = self.tabs.iter().position(|t| *t == tab) else {
            return tab;
        };
        let focus = self.focused.remove(&tab);
        self.layouts.remove(&tab);
        // A tab is keyed by a conversation when it holds one, never a Pilot.
        let key = if layout.contains(tab) && !self.pilot_panes.contains_key(&tab) {
            tab
        } else {
            let leaves = layout.leaf_ids();
            leaves
                .iter()
                .copied()
                .find(|id| !self.pilot_panes.contains_key(id))
                .unwrap_or(leaves[0])
        };
        self.tabs[ix] = key;
        if key != tab
            && let Some(scope) = self.tab_workspaces.remove(&tab)
        {
            self.tab_workspaces.insert(key, scope);
        }
        for entry in self
            .title_order
            .iter_mut()
            .chain(self.scope_focus.values_mut())
        {
            if *entry == TitleTab::Session(tab) {
                *entry = TitleTab::Session(key);
            }
        }
        if layout.is_split() {
            if let Some(focus) = focus.filter(|id| layout.contains(*id)) {
                self.focused.insert(key, focus);
            }
            self.layouts.insert(key, layout);
        }
        key
    }
    /// Take a session out of whichever tab holds it. A tab left empty
    /// closes; the active session moves to a neighbouring pane.
    fn take_out(&mut self, id: i64) {
        let Some(tab) = self.tab_of(id) else {
            return;
        };
        let layout = self.layout(tab);
        self.project_focus.retain(|_, focus| *focus != id);
        match layout.remove(id) {
            Some(rest) => {
                let next = layout.sibling(id).unwrap_or_else(|| rest.first_leaf());
                let key = self.set_layout(tab, rest);
                if self.active == Some(id) {
                    self.active = Some(next);
                }
                if self.layouts.contains_key(&key) && self.focused.get(&key) == Some(&id) {
                    self.focused.insert(key, next);
                }
            }
            None => self.close(id),
        }
    }
    /// Close one pane of a split tab. False when it is the tab's only pane.
    pub fn close_pane(&mut self, id: i64) -> bool {
        let Some(tab) = self.tab_of(id) else {
            return false;
        };
        if !self.layout(tab).is_split() {
            return false;
        }
        self.take_out(id);
        if let Some(active) = self.active
            && let Some(tab) = self.tab_of(active)
            && self.layouts.contains_key(&tab)
        {
            self.focused.insert(tab, active);
        }
        true
    }
    /// cmd-d: a new session beside the focused pane.
    pub fn split(&mut self, new: i64, dir: SplitDir) {
        let Some(focused) = self.active else {
            self.open(new);
            return;
        };
        let Some(tab) = self.tab_of(focused) else {
            self.open(new);
            return;
        };
        self.take_out(new);
        let tab = self.tab_of(focused).unwrap_or(tab);
        let key = self.set_layout(tab, self.layout(tab).split_pane(focused, dir, new));
        self.focused.insert(key, new);
        self.active = Some(new);
    }
    /// A pane, tab-strip session or sidebar session dropped on a pane edge.
    pub fn place(&mut self, id: i64, target: i64, edge: Edge) {
        if id == target || self.tab_of(target).is_none() {
            return;
        }
        let same = self.tab_of(id) == self.tab_of(target);
        if !same {
            self.take_out(id);
        }
        let Some(tab) = self.tab_of(target) else {
            return;
        };
        let key = self.set_layout(tab, self.layout(tab).place_pane(id, target, edge));
        self.focused.insert(key, id);
        self.active = Some(id);
    }
    /// A whole title tab dropped on a pane edge: its tree moves intact.
    pub fn place_tab(&mut self, source: i64, target: i64, edge: Edge) {
        let Some(target_tab) = self.tab_of(target) else {
            return;
        };
        if source == target_tab || !self.tabs.contains(&source) {
            return;
        }
        let incoming = self.layout(source);
        let focus = self.tab_focus(source);
        self.tabs.retain(|t| *t != source);
        self.tab_workspaces.remove(&source);
        self.title_order
            .retain(|entry| *entry != TitleTab::Session(source));
        self.layouts.remove(&source);
        self.focused.remove(&source);
        let key = self.set_layout(
            target_tab,
            self.layout(target_tab).place_layout(incoming, target, edge),
        );
        self.focused.insert(key, focus);
        self.active = Some(focus);
    }
    /// A Pilot tab dropped on a pane edge leaves the strip for the split.
    pub fn place_pilot(&mut self, pilot: TitleTab, target: i64, edge: Edge) -> Option<i64> {
        if !matches!(pilot, TitleTab::Pilot { .. }) || self.tab_of(target).is_none() {
            return None;
        }
        let pane = self.next_pane_id();
        self.title_order.retain(|entry| *entry != pilot);
        self.scope_focus.retain(|_, entry| *entry != pilot);
        self.pilot_panes.insert(pane, pilot);
        self.place(pane, target, edge);
        Some(pane)
    }
    /// The visible Pilot becomes a split's first pane, its tab keeping its
    /// slot, so a tab or pane can be dropped beside it.
    pub fn pilot_tab_pane(&mut self, pilot: TitleTab) -> i64 {
        let pane = self.next_pane_id();
        self.tabs.push(pane);
        for entry in self
            .title_order
            .iter_mut()
            .chain(self.scope_focus.values_mut())
        {
            if *entry == pilot {
                *entry = TitleTab::Session(pane);
            }
        }
        if !self.title_order.contains(&TitleTab::Session(pane)) {
            self.title_order.push(TitleTab::Session(pane));
        }
        self.pilot_panes.insert(pane, pilot);
        self.active = Some(pane);
        pane
    }
    /// A Pilot pane left alone in its tab becomes its Pilot tab again, in the
    /// same slot; one no tab holds is forgotten. Returns the Pilot when it
    /// had focus, for the caller to show.
    pub fn settle_pilot_panes(&mut self) -> Option<TitleTab> {
        let mut focused = None;
        let panes: Vec<_> = self.pilot_panes.keys().copied().collect();
        for pane in panes {
            match self.tab_of(pane) {
                Some(tab) if self.layout(tab).is_split() => continue,
                Some(tab) => {
                    let pilot = self.pilot_panes[&pane].clone();
                    self.tabs.retain(|t| *t != tab);
                    self.tab_workspaces.remove(&tab);
                    self.layouts.remove(&tab);
                    self.focused.remove(&tab);
                    let present = self.title_order.contains(&pilot);
                    self.title_order
                        .retain(|entry| !(present && *entry == TitleTab::Session(tab)));
                    for entry in self
                        .title_order
                        .iter_mut()
                        .chain(self.scope_focus.values_mut())
                    {
                        if *entry == TitleTab::Session(tab) {
                            *entry = pilot.clone();
                        }
                    }
                    if self.active == Some(pane) {
                        self.active = None;
                        focused = Some(pilot);
                    }
                }
                None => {}
            }
            self.pilot_panes.remove(&pane);
        }
        focused
    }
    /// A pane dragged onto the tab strip becomes its own tab at `to`.
    pub fn detach(&mut self, id: i64, to: usize) {
        if !self
            .tab_of(id)
            .is_some_and(|tab| self.layout(tab).is_split())
        {
            return;
        }
        let scope = self
            .tab_of(id)
            .and_then(|tab| self.tab_workspaces.get(&tab))
            .cloned();
        self.take_out(id);
        self.tabs.insert(to.min(self.tabs.len()), id);
        if let Some(scope) = scope {
            self.tab_workspaces.insert(id, scope);
        }
        let next = self
            .tabs
            .iter()
            .skip(to + 1)
            .find_map(|next| {
                self.title_order
                    .iter()
                    .position(|tab| *tab == TitleTab::Session(*next))
            })
            .unwrap_or(self.title_order.len());
        self.title_order.insert(next, TitleTab::Session(id));
        self.active = Some(id);
    }
    /// Show `to` in `from`'s pane (Session: Next/Previous in tab).
    pub fn swap_session(&mut self, from: i64, to: i64) {
        if self.tab_of(to).is_some() {
            return;
        }
        let Some(tab) = self.tab_of(from) else {
            return;
        };
        let key = self.set_layout(tab, self.layout(tab).replace_leaf(from, to));
        if self.layouts.contains_key(&key) {
            self.focused.insert(key, to);
        }
    }
    /// cmd-opt-arrows within the active tab.
    pub fn neighbor(&self, dir: Edge) -> Option<i64> {
        let active = self.active?;
        self.layout(self.tab_of(active)?).neighbor(active, dir)
    }
    pub fn place_sash(&mut self, tab: i64, path: &[usize], index: usize, boundary: f32) {
        if let Some(layout) = self.layouts.get_mut(&tab) {
            layout.place_sash(path, index, boundary);
        }
    }
    pub fn sash_range(&self, tab: i64, path: &[usize], index: usize) -> Option<(f32, f32, f32)> {
        self.layouts.get(&tab)?.sash_range(path, index)
    }
    pub fn normalize_title_order(&mut self) {
        self.tab_workspaces.retain(|tab, _| self.tabs.contains(tab));
        let mut seen = HashSet::new();
        self.title_order.retain(|tab| {
            tab.session().is_none_or(|id| self.tabs.contains(&id)) && seen.insert(tab.clone())
        });
        for id in &self.tabs {
            let tab = TitleTab::Session(*id);
            if seen.insert(tab.clone()) {
                self.title_order.push(tab);
            }
        }
    }
}

/// Port of src/features/workspace/model/tabVisitHistory.ts. Closed tabs are
/// pruned rather than reopened by Back, and a new visit discards Forward.
#[derive(Default)]
pub(crate) struct VisitHistory {
    pub back: Vec<i64>,
    pub forward: Vec<i64>,
    current: Option<i64>,
}
impl VisitHistory {
    fn push(&mut self, id: i64) {
        if self.back.last() != Some(&id) {
            self.back.push(id);
        }
        if self.back.len() > 50 {
            self.back.remove(0);
        }
    }
    pub fn record(&mut self, id: i64) {
        if self.current == Some(id) {
            return;
        }
        if let Some(current) = self.current {
            self.push(current);
        }
        self.current = Some(id);
        self.forward.clear();
    }
    pub fn step(&mut self, direction: isize) -> Option<i64> {
        let target = if direction < 0 {
            let target = self.back.pop()?;
            if let Some(current) = self.current {
                self.forward.insert(0, current);
            }
            target
        } else {
            if self.forward.is_empty() {
                return None;
            }
            let target = self.forward.remove(0);
            if let Some(current) = self.current {
                self.push(current);
            }
            target
        };
        self.current = Some(target);
        Some(target)
    }
    pub fn prune(&mut self, tabs: &[i64], active: Option<i64>) {
        self.back.retain(|id| tabs.contains(id));
        self.forward.retain(|id| tabs.contains(id));
        if self.current.is_none_or(|id| !tabs.contains(&id)) {
            self.current = active;
        }
        while self
            .back
            .last()
            .copied()
            .is_some_and(|id| Some(id) == self.current)
        {
            self.back.pop();
        }
        while self
            .forward
            .first()
            .copied()
            .is_some_and(|id| Some(id) == self.current)
        {
            self.forward.remove(0);
        }
    }
}
pub(crate) fn project_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .filter(|n| !n.is_empty())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.into())
}

/// Projects whose folder was renamed take their sessions and saved workspace
/// state to the new folder (App.tsx's applyProjectLocationChange). This runs
/// before the workspace is restored, so the app starts on the new paths.
pub(crate) fn follow_moved_projects(
    store: &gespan_core::store::Store,
    saved: Option<String>,
) -> Option<String> {
    let Some(mut state) = saved
        .as_deref()
        .and_then(|saved| serde_json::from_str::<serde_json::Value>(saved).ok())
    else {
        return saved;
    };
    let mut projects: Vec<String> = Vec::new();
    for value in std::iter::once(&state["project"])
        .chain(state["projects"].as_array().into_iter().flatten())
        .chain(state["archived_projects"].as_array().into_iter().flatten())
    {
        if let Some(path) = value.as_str()
            && !projects.iter().any(|p| p == path)
        {
            projects.push(path.into());
        }
    }
    let moved = gespan_core::project_location::follow_moved_projects(store, &projects);
    if moved.is_empty() {
        return saved;
    }
    for gespan_core::project_location::Moved { from, to } in &moved {
        tracing::info!(from = %from, to = %to, "project folder was renamed");
        rebase_saved_project(&mut state, from, to);
    }
    let rebased = state.to_string();
    if let Err(error) = store.save_ui_state(&rebased) {
        tracing::warn!(%error, "cannot save the workspace after a project folder moved");
    }
    Some(rebased)
}

/// Rewrites every place the saved workspace names the project `from`.
fn rebase_saved_project(state: &mut serde_json::Value, from: &str, to: &str) {
    use gespan_core::files::{is_equal_or_inside, path_key, rebase_path};
    use serde_json::Value;
    // Only fields that exist are touched: indexing would add nulls that the
    // workspace cannot read back.
    fn field<'a>(state: &'a mut Value, path: &[&str]) -> Option<&'a mut Value> {
        path.iter()
            .try_fold(state, |value, key| value.get_mut(*key))
    }
    let same = |path: &str| path_key(path) == path_key(from);
    let rebase = |value: &mut Value| {
        if value.as_str().is_some_and(same) {
            *value = Value::String(to.into());
        }
    };
    let rebase_inside = |value: &mut Value| {
        if value.as_str().is_some_and(same) {
            *value = Value::String(to.into());
        } else if let Some(path) = value.as_str().filter(|path| is_equal_or_inside(path, from)) {
            *value = Value::String(rebase_path(path, from, to));
        }
    };
    if let Some(project) = field(state, &["project"]) {
        rebase(project);
    }
    for list in [
        "projects",
        "archived_projects",
        "project_rail_pinned",
        "collapsed",
    ] {
        if let Some(paths) = field(state, &[list]).and_then(Value::as_array_mut) {
            paths.iter_mut().for_each(rebase);
            let mut seen = HashSet::new();
            paths.retain(|path| seen.insert(path.clone()));
        }
    }
    for map in [
        &["project_focus"][..],
        &["recent_files"],
        &["docks"],
        &["actions"],
        &["last_actions"],
        &["classic_sidebar_tabs"],
        &["project_appearance"],
        &["project_rail_assignments"],
        &["preferences", "project_providers"],
    ] {
        let Some(map) = field(state, map).and_then(Value::as_object_mut) else {
            continue;
        };
        if let Some(key) = map.keys().find(|key| same(key)).cloned()
            && let Some(value) = map.remove(&key)
        {
            map.entry(to).or_insert(value);
        }
    }
    for list in [&["recent_files", to][..], &["docks", to, "terminals"]] {
        if let Some(paths) = field(state, list).and_then(Value::as_array_mut) {
            paths.iter_mut().for_each(rebase_inside);
        }
    }
    // Title tabs remember Pilots by project, and each tab scope is keyed by
    // the project and working copy.
    let rebase_pilot = |tab: &mut Value| {
        for key in ["project", "worktree"] {
            if let Some(path) = field(tab, &["Pilot", key]) {
                rebase_inside(path);
            }
        }
    };
    if let Some(tabs) = field(state, &["title_order"]).and_then(Value::as_array_mut) {
        tabs.iter_mut().for_each(rebase_pilot);
    }
    if let Some(focus) = field(state, &["scope_focus"]).and_then(Value::as_object_mut) {
        *focus = std::mem::take(focus)
            .into_iter()
            .map(|(scope, mut tab)| {
                rebase_pilot(&mut tab);
                let scope = scope
                    .split('\0')
                    .map(|part| {
                        if is_equal_or_inside(part, from) {
                            path_key(&rebase_path(part, from, to))
                        } else {
                            part.to_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\0");
                (scope, tab)
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_renamed_project_moves_its_saved_workspace_state() {
        use std::path::Path;
        let root = Path::new("work");
        let path = |p: &Path| p.to_string_lossy().into_owned();
        let (old, new, other) = (
            path(&root.join("gespan")),
            path(&root.join("gespan-personal")),
            path(&root.join("other")),
        );
        let file = path(&root.join("gespan").join("README.md"));
        let mut state = serde_json::json!({
            "project": old,
            "projects": [old, other],
            "project_focus": {old.clone(): 3},
            "recent_files": {old.clone(): [file]},
            "docks": {old.clone(): {"terminals": [old, other]}},
            "project_rail_pinned": [old],
            "preferences": {"project_providers": {old.clone(): {}}},
            "scope_focus": {
                format!("{}\0{}", gespan_core::files::path_key(&old), gespan_core::files::path_key(&old)): {"Session": 3},
                format!("{}\0", gespan_core::files::path_key(&other)): {"Session": 4},
            },
            "title_order": [{"Pilot": {"id": "p", "project": old, "worktree": ""}}],
        });
        super::rebase_saved_project(&mut state, &old, &new);
        assert_eq!(state["project"], new);
        assert_eq!(state["projects"], serde_json::json!([new, other]));
        assert_eq!(state["project_focus"], serde_json::json!({new.clone(): 3}));
        assert_eq!(
            gespan_core::files::path_key(state["recent_files"][&new][0].as_str().unwrap()),
            gespan_core::files::path_key(&path(&root.join("gespan-personal").join("README.md")))
        );
        assert_eq!(
            state["docks"][&new]["terminals"],
            serde_json::json!([new, other])
        );
        assert_eq!(state["project_rail_pinned"], serde_json::json!([new]));
        assert!(state["preferences"]["project_providers"][&new].is_object());
        let scopes: Vec<_> = state["scope_focus"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert!(scopes.contains(&format!(
            "{}\0{}",
            gespan_core::files::path_key(&new),
            gespan_core::files::path_key(&new)
        )));
        assert!(scopes.contains(&format!("{}\0", gespan_core::files::path_key(&other))));
        assert_eq!(state["title_order"][0]["Pilot"]["project"], new);
        assert_eq!(state["title_order"][0]["Pilot"]["worktree"], "");
        let restored = super::Workspace::restore(Some(&state.to_string()), &[], &new);
        assert_eq!(restored.project, new);

        // Fields an older workspace never saved are not added as nulls,
        // which would make the whole workspace unreadable.
        let mut sparse = serde_json::json!({"projects": [old]});
        super::rebase_saved_project(&mut sparse, &old, &new);
        assert_eq!(sparse, serde_json::json!({"projects": [new]}));
    }

    #[test]
    fn rail_moves_stay_inside_their_section_and_follow_the_drawn_order() {
        let mut state = super::Workspace {
            classic_layout: true,
            projects: ["/a", "/b", "/c", "/d", "/e", "/f"]
                .map(str::to_owned)
                .to_vec(),
            project_rail_pinned: ["/e".to_owned()].into(),
            project_rail_groups: vec![super::ProjectRailGroup {
                id: "g".into(),
                name: "Group".into(),
                ..Default::default()
            }],
            project_rail_assignments: [("/b", "g"), ("/d", "g"), ("/f", "deleted")]
                .map(|(p, g)| (p.to_owned(), g.to_owned()))
                .into(),
            ..Default::default()
        };
        // Pinned, then the group, then the rest. A deleted group's member
        // falls back to the ungrouped list.
        assert_eq!(
            state.visible_project_order(),
            ["/e", "/b", "/d", "/a", "/c", "/f"]
        );
        assert_eq!(state.project_peers("/f"), ["/a", "/c", "/f"]);
        assert!(state.step_project_order("/f", -1));
        assert_eq!(state.project_peers("/a"), ["/a", "/f", "/c"]);
        assert!(!state.step_project_order("/b", -1), "/b leads its group");
        assert!(state.move_project_to_end("/b", 1));
        assert_eq!(state.project_peers("/d"), ["/d", "/b"]);
        assert!(!state.step_project_order("/e", 1), "/e is the only pin");
        // The projects row shows the saved order as it is.
        state.classic_layout = false;
        assert_eq!(state.visible_project_order(), state.projects);
        assert!(state.move_project_to_end("/e", -1));
        assert_eq!(state.projects[0], "/e");
    }

    #[test]
    fn legacy_boxy_layout_and_hidden_rail_migrate_without_changing_appearance() {
        let state = super::Workspace::restore(
            Some(r#"{"classic_layout":true,"preferences":{"collapsed_project_rail":"hidden"}}"#),
            &[],
            "/work",
        );
        assert!(!state.classic_layout);
        assert!(state.new_layout_boxy);
        assert!(state.boxy_layout());
        assert_eq!(state.preferences.collapsed_project_rail, "compact");
        let saved = serde_json::to_string(&state).unwrap();
        let mut state = super::Workspace::restore(Some(&saved), &[], "/work");
        state.classic_layout = true;
        state.project_rail_open = false;
        state.project_rail_width = 248.;
        let saved = serde_json::to_string(&state).unwrap();
        let state = super::Workspace::restore(Some(&saved), &[], "/work");
        assert!(state.classic_layout && state.new_layout_boxy);
        assert!(!state.project_rail_open);
        assert_eq!(state.project_rail_width, 248.);
    }

    use super::*;

    /// The workspace as a build from before Flows reads it: its `AppView`
    /// has no `Flows`, and one unknown variant fails the whole document.
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct BeforeFlows {
        project: String,
        tabs: Vec<i64>,
        title_order: Vec<BeforeFlowsTab>,
        scope_focus: HashMap<String, BeforeFlowsTab>,
    }
    #[derive(Deserialize, Debug, PartialEq)]
    enum BeforeFlowsView {
        Inbox,
        Notes,
        Automations,
        Board,
    }
    #[derive(Deserialize, Debug, PartialEq)]
    enum BeforeFlowsTab {
        Session(i64),
        Search,
        App(BeforeFlowsView),
        Pilot {
            id: String,
            project: String,
            worktree: String,
        },
    }

    #[test]
    fn a_saved_flows_tab_does_not_cost_an_older_build_its_workspace() {
        let mut state = Workspace {
            project: "/work/gespan".into(),
            ..Default::default()
        };
        state.open(7);
        state.title_order.push(TitleTab::App(AppView::Automations));
        state.title_order.push(TitleTab::App(AppView::Flows));
        state
            .scope_focus
            .insert("/work/gespan".into(), TitleTab::App(AppView::Flows));
        state
            .scope_focus
            .insert("/work/other".into(), TitleTab::Session(7));
        let saved = serde_json::to_string(&state).unwrap();

        let older: BeforeFlows = serde_json::from_str(&saved).expect("older build parses it");
        assert_eq!(older.project, "/work/gespan");
        assert_eq!(older.tabs, vec![7]);
        assert_eq!(
            older.title_order,
            vec![
                BeforeFlowsTab::Session(7),
                BeforeFlowsTab::App(BeforeFlowsView::Automations)
            ]
        );
        assert_eq!(older.scope_focus.len(), 1);
        assert_eq!(older.scope_focus["/work/other"], BeforeFlowsTab::Session(7));

        // This build restores everything except the Flows tab.
        let restored = Workspace::restore(Some(&saved), &[(7, "/work/gespan".into())], "/work");
        assert_eq!(restored.project, "/work/gespan");
        assert_eq!(restored.tabs, vec![7]);
        assert!(
            restored
                .title_order
                .contains(&TitleTab::App(AppView::Automations))
        );
        assert!(
            !restored
                .title_order
                .contains(&TitleTab::App(AppView::Flows))
        );
        // The in-memory workspace keeps its Flows tab while the app runs.
        assert!(state.title_order.contains(&TitleTab::App(AppView::Flows)));
    }

    #[test]
    fn navigation_prunes_closed_tabs_and_a_new_visit_discards_forward() {
        let mut h = VisitHistory::default();
        h.record(1);
        h.record(2);
        h.record(3);
        assert_eq!(h.step(-1), Some(2));
        h.prune(&[1, 3], Some(3));
        assert_eq!(h.step(-1), Some(1));
        assert_eq!(h.step(1), Some(3));
        h.step(-1);
        h.record(4);
        assert!(h.forward.is_empty());
        for id in 5..100 {
            h.record(id);
        }
        assert_eq!(h.back.len(), 50);
    }
    fn pilot(id: &str) -> TitleTab {
        TitleTab::Pilot {
            id: id.into(),
            project: "/work".into(),
            worktree: "/work".into(),
        }
    }
    #[test]
    fn a_pilot_joins_a_split_and_becomes_its_tab_again_when_alone() {
        let mut s = Workspace::default();
        s.open(1);
        s.title_order.push(pilot("m"));
        s.open(2);
        let pane = s.place_pilot(pilot("m"), 1, Edge::Right).unwrap();
        assert!(pane < 0);
        assert_eq!(s.layout(1).leaf_ids(), vec![1, pane]);
        assert_eq!(s.active, Some(pane));
        assert_eq!(s.scope_pane(1), 1, "a Pilot never scopes its tab");
        assert_eq!(
            s.title_order,
            vec![TitleTab::Session(1), TitleTab::Session(2)]
        );
        assert_eq!(s.pilot_pane("m"), Some(pane));
        assert_eq!(s.settle_pilot_panes(), None);
        // Restarting keeps the Pilot in its split.
        let saved = serde_json::to_string(&s).unwrap();
        let restored = Workspace::restore(
            Some(&saved),
            &[(1, "/work".into()), (2, "/work".into())],
            "/work",
        );
        assert_eq!(restored.layout(1).leaf_ids(), vec![1, pane]);
        assert_eq!(restored.pilot_pane("m"), Some(pane));
        // Detached to the strip, it is the Pilot tab again in the same slot.
        s.detach(pane, 1);
        assert_eq!(s.settle_pilot_panes(), Some(pilot("m")));
        assert_eq!(
            s.title_order,
            vec![TitleTab::Session(1), pilot("m"), TitleTab::Session(2)]
        );
        assert!(s.pilot_panes.is_empty());
        assert_eq!(s.tabs, vec![1, 2]);
    }
    #[test]
    fn a_tab_dropped_on_the_visible_pilot_splits_beside_it() {
        let mut s = Workspace::default();
        s.open(1);
        s.title_order.push(pilot("m"));
        let pane = s.pilot_tab_pane(pilot("m"));
        assert_eq!(
            s.title_order,
            vec![TitleTab::Session(1), TitleTab::Session(pane)]
        );
        s.place_tab(1, pane, Edge::Left);
        assert_eq!(s.tabs, vec![1], "the conversation keys the tab");
        assert_eq!(s.layout(1).leaf_ids(), vec![1, pane]);
        assert_eq!(s.title_order, vec![TitleTab::Session(1)]);
        assert_eq!(s.settle_pilot_panes(), None);
        // Closing the conversation's pane leaves the Pilot on its own.
        assert!(s.close_pane(1));
        assert_eq!(s.settle_pilot_panes(), Some(pilot("m")));
        assert_eq!(s.title_order, vec![pilot("m")]);
        assert!(s.tabs.is_empty());
        // A closed tab forgets its Pilot pane.
        s.open(1);
        s.place_pilot(pilot("m"), 1, Edge::Bottom);
        s.close(1);
        assert_eq!(s.settle_pilot_panes(), None);
        assert!(s.pilot_panes.is_empty());
    }
    #[test]
    fn close_and_reopen_keep_session_identity() {
        let mut s = Workspace::default();
        s.open(1);
        s.open(2);
        s.open(3);
        s.open(2);
        assert_eq!(s.tabs, vec![1, 2, 3]);
        s.close(1);
        assert_eq!(s.active, Some(2));
        s.close(2);
        assert_eq!(s.active, Some(3));
        s.open(2);
        s.open(3);
        assert_eq!(s.active, Some(3));
        s.close(3);
        s.close(2);
        assert_eq!(s.active, None);
        s.open(1);
        assert_eq!(s.tabs, vec![1]);
    }
    #[test]
    fn split_tabs_place_detach_and_close_panes_without_losing_sessions() {
        let mut s = Workspace::default();
        s.open(1);
        s.open(2);
        s.open(1);
        s.split(3, SplitDir::Right);
        s.tab_workspaces.insert(1, "/selected".into());
        s.tab_workspaces.insert(2, "/other".into());
        assert_eq!(s.tabs, [1, 2]);
        assert_eq!(s.layout(1).leaf_ids(), [1, 3]);
        assert_eq!((s.active, s.active_tab()), (Some(3), Some(1)));
        // Another tab's session joins this one; its old tab closes.
        s.place(2, 1, Edge::Bottom);
        assert_eq!(s.tabs, [1]);
        assert!(!s.tab_workspaces.contains_key(&2));
        assert_eq!(s.layout(1).leaf_ids(), [1, 2, 3]);
        assert_eq!(s.open_sessions(), [1, 2, 3]);
        // Closing the key pane rekeys the tab and keeps its place.
        s.open(4);
        s.open(1);
        assert!(s.close_pane(1));
        assert_eq!(s.tabs, [2, 4]);
        assert_eq!(s.tab_workspaces[&2], "/selected");
        assert!(!s.tab_workspaces.contains_key(&1));
        assert_eq!(s.active, Some(2));
        assert_eq!(s.tab_focus(2), 2);
        s.open(3);
        s.open(s.tab_focus(4));
        assert_eq!(s.active, Some(4));
        s.open(s.tab_focus(2));
        assert_eq!(s.active, Some(3));
        // A pane dragged to the strip becomes a tab where it was dropped.
        s.detach(3, 0);
        assert_eq!(s.tabs, [3, 2, 4]);
        assert_eq!(s.tab_workspaces[&3], "/selected");
        assert!(!s.layouts.contains_key(&2));
        assert!(!s.close_pane(3));
        // A whole tab dropped on a pane moves in intact.
        s.split(5, SplitDir::Down);
        s.place_tab(3, 4, Edge::Left);
        assert_eq!(s.tabs, [2, 4]);
        assert!(!s.tab_workspaces.contains_key(&3));
        assert_eq!(s.layout(4).leaf_ids(), [3, 5, 4]);
        assert_eq!(s.active, Some(5));
        s.close(5);
        assert_eq!(s.tabs, [2]);
        assert_eq!(s.tab_workspaces.len(), 1);
        assert_eq!(s.active, Some(2));
    }
    #[test]
    fn restore_keeps_split_tabs_and_prunes_their_missing_panes() {
        let mut s = Workspace::default();
        s.open(1);
        s.split(2, SplitDir::Right);
        s.split(3, SplitDir::Down);
        s.open(9);
        s.open(2);
        let json = serde_json::to_string(&s).unwrap();
        let sessions: Vec<_> = [2, 3].iter().map(|id| (*id, "/w".into())).collect();
        let s = Workspace::restore(Some(&json), &sessions, "/w");
        assert_eq!(s.tabs, [2]);
        assert_eq!(s.layout(2).leaf_ids(), [2, 3]);
        assert_eq!(s.active, Some(2));
        assert_eq!(s.tab_focus(2), 2);
    }
    #[test]
    fn terminal_docks_add_close_move_and_clamp_like_project_terminal() {
        let mut dock = TerminalDock::default();
        assert_eq!((dock.side, dock.size), (DockSide::Bottom, 220.));
        dock.add("/w/a".into());
        dock.add("/w/b".into());
        dock.add("/w/c".into());
        assert_eq!(dock.active, 2);
        dock.active = 1;
        assert!(dock.close(1));
        assert_eq!((dock.terminals.len(), dock.active), (2, 1));
        assert!(dock.close(0));
        assert_eq!(dock.active, 0);
        assert!(!dock.close(0));
        dock.set_side(DockSide::Right);
        assert_eq!(dock.size, 360.);
        dock.size = 500.;
        dock.set_side(DockSide::Left);
        assert_eq!(dock.size, 500.);
        assert_eq!(DockSide::Bottom.clamp(10., (1280., 800.)), 88.);
        assert_eq!(DockSide::Bottom.clamp(900., (1280., 800.)), 560.);
        assert_eq!(DockSide::Left.clamp(f32::NAN, (1280., 800.)), 360.);
        let mut s = Workspace::default();
        s.docks.insert("/w".into(), dock.clone());
        let mut kept = dock.clone();
        kept.add("/w".into());
        s.docks.insert("/k".into(), kept);
        let json = serde_json::to_string(&s).unwrap();
        let s = Workspace::restore(Some(&json), &[], "/w");
        assert!(!s.docks.contains_key("/w"));
        assert_eq!(s.docks["/k"].terminals, ["/w"]);
    }
    #[test]
    fn terminal_docks_reorder_keeping_the_active_terminal_and_close_others() {
        let mut dock = TerminalDock::default();
        for cwd in ["/a", "/b", "/c", "/d"] {
            dock.add(cwd.into());
        }
        dock.active = 1;
        dock.reorder(1, 3);
        assert_eq!(dock.terminals, ["/a", "/c", "/d", "/b"]);
        assert_eq!(dock.active, 3);
        dock.reorder(0, 2);
        assert_eq!(dock.terminals, ["/c", "/d", "/a", "/b"]);
        assert_eq!(dock.active, 3);
        dock.reorder(3, 0);
        assert_eq!(dock.terminals, ["/b", "/c", "/d", "/a"]);
        assert_eq!(dock.active, 0);
        dock.active = 2;
        dock.reorder(3, 1);
        assert_eq!(dock.terminals, ["/b", "/a", "/c", "/d"]);
        assert_eq!(dock.active, 3);
        dock.keep_only(2);
        assert_eq!(
            (dock.terminals.clone(), dock.active),
            (vec!["/c".to_string()], 0)
        );
    }
    #[test]
    fn restore_keeps_project_selection_and_prunes_stale_project_focus() {
        let sessions = vec![(3, "/b".into()), (2, "/a".into()), (1, "/a".into())];
        let mut state = Workspace::default();
        for id in [1, 2, 3] {
            state.open(id);
        }
        state.project = "/a".into();
        state.active = Some(99);
        state.project_focus.insert("/a".into(), 1);
        state.project_focus.insert("/b".into(), 99);
        state.worktree_focus.insert(
            "/a".into(),
            crate::worktree_switcher::WorktreeFocus {
                path: "/a-tree".into(),
                branch: Some("feature".into()),
            },
        );
        state.tab_workspaces.insert(1, "/a-tree".into());
        let mut saved = serde_json::to_value(&state).unwrap();
        assert!(saved.get("worktree_focus").is_none());
        assert!(saved.get("tab_workspaces").is_none());
        // Older native sidecars saved this transient filter. Ignore it too.
        saved["worktree_focus"] = serde_json::json!({
            "/a": {"path":"/a-tree", "branch":"feature"}
        });
        let json = serde_json::to_string(&saved).unwrap();
        let restored = Workspace::restore(Some(&json), &sessions, "/b");
        assert_eq!(restored.active, Some(1));
        assert_eq!(restored.project, "/a");
        assert!(restored.worktree_focus.is_empty());
        assert!(restored.tab_workspaces.is_empty());
        assert!(!restored.project_focus.contains_key("/b"));
        state.active = Some(3);
        let json = serde_json::to_string(&state).unwrap();
        let restored = Workspace::restore(Some(&json), &sessions, "/a");
        assert_eq!(restored.active, Some(3));
        assert_eq!(restored.project, "/b");
        let fresh = Workspace::restore(None, &sessions, "/a");
        assert_eq!(fresh.active, Some(2));
        assert_eq!(fresh.project, "/a");
        assert!(fresh.new_layout_boxy);
        let rounded = Workspace::restore(Some(r#"{"new_layout_boxy":false}"#), &sessions, "/a");
        assert!(!rounded.new_layout_boxy);
    }
    #[test]
    fn restoring_history_keeps_only_remembered_or_selected_projects() {
        let saved = serde_json::json!({
            "project": "/code/app",
            "projects": ["/code/app", "/code/other"],
            "tabs": [2],
            "active": 2,
            "desktop_project_preferences": {"projects": ["/code/app", "/code/other"]}
        })
        .to_string();
        let sessions = vec![
            (1, "/code/app".into()),
            (2, "/tmp/internal-pilot-chat".into()),
            (3, "/private/var/folders/temporary".into()),
        ];
        let restored = Workspace::restore(Some(&saved), &sessions, "/code/app");
        assert_eq!(restored.projects, ["/code/app", "/code/other"]);
        assert_eq!(restored.project, "/code/app");
        assert_eq!(restored.active, None);
        let serialized = serde_json::to_value(restored).unwrap();
        assert_eq!(
            serialized["desktop_project_preferences"]["projects"],
            serde_json::json!(["/code/app", "/code/other"])
        );
    }
    #[test]
    fn restore_prunes_stale_tabs_and_preserves_an_intentionally_empty_workspace() {
        let mut s = Workspace::default();
        s.open(1);
        s.open(8);
        s.tabs.push(1);
        s.sidebar_width = 900.;
        let json = serde_json::to_string(&s).unwrap();
        let s = Workspace::restore(Some(&json), &[(1, "/work/a".into())], "/work/b");
        assert_eq!(s.tabs, vec![1]);
        assert_eq!(s.active, None);
        assert_eq!(s.project, "/work/b");
        assert_eq!(s.sidebar_width, 560.);
        assert_eq!(s.projects, vec!["/work/b"]);
        let s = Workspace::restore(Some("{}"), &[(1, "/work/a".into())], "/work/a");
        assert!(s.tabs.is_empty());
        assert!(
            s.compact_session_cards,
            "older workspaces retain compact cards"
        );
        assert!(
            s.sidebar_activity_visible,
            "older workspaces keep the activity card visible"
        );
    }
    #[test]
    fn desktop_snapshot_translation_deserializes_and_retains_unported_metadata() {
        let source = serde_json::json!({
            "projectCwd":"/work", "activeTabId":"tab",
            "tabs":[{"id":"tab", "focusedId":"b", "groupId":"assigned", "editorPanes":[], "layout":{"type":"split", "dir":"down", "children":[{"type":"leaf","id":"a"},{"type":"leaf","id":"b"}], "sizes":[0.4,0.6]}}],
            "projectTerminals":[{"projectPath":"/work", "side":"bottom", "size":240, "open":false, "pane":{"activeFileId":"term", "files":[{"id":"term", "terminal":true, "cwd":"/work/sub"}]}}]
        });
        let mapped = gespan_core::desktop_preferences::DesktopPreferences::workspace_snapshot(
            &source,
            |id| match id {
                "a" => Some(1),
                "b" => Some(2),
                _ => None,
            },
        );
        let saved = mapped.to_string();
        let restored = Workspace::restore(
            Some(&saved),
            &[(1, "/work".into()), (2, "/work".into())],
            "/work",
        );
        assert_eq!(restored.tabs, [1]);
        assert_eq!(restored.active, Some(2));
        assert!(restored.layouts[&1].is_split());
        assert_eq!(restored.focused[&1], 2);
        assert_eq!(restored.docks["/work"].terminals, ["/work/sub"]);
        assert!(!restored.docks["/work"].open);
        assert_eq!(restored.desktop_workspace_snapshot.as_ref(), Some(&source));
    }
}
