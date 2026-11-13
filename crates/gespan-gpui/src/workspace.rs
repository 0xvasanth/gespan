//! Viewport state: closing a tab never deletes or stops a session.
//! A title tab is keyed by one of its sessions; a tab holding more than one
//! pane keeps its split tree in `layouts` and its last focused pane in
//! `focused`. `active` is always the focused session.
use crate::pane_layout::{Edge, PaneNode, SplitDir, leaf};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum AppView {
    Home,
    MyTasks,
    Inbox,
    Notes,
    Automations,
    Flows,
    Board,
    Runs,
}
impl AppView {
    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::MyTasks => "My tasks",
            Self::Inbox => "Inbox",
            Self::Notes => "Notes",
            Self::Automations => "Automations",
            Self::Flows => "Flows",
            Self::Board => "Board",
            Self::Runs => "Runs",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::MyTasks => "circle-alert",
            Self::Inbox => "inbox",
            Self::Notes => "note",
            Self::Automations => "zap",
            Self::Flows => "git-merge",
            Self::Board => "kanban",
            Self::Runs => "clock",
        }
    }
}

/// The title strip has one order, including conversations, app views and Pilots.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum TitleTab {
    Session(i64),
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
            Self::Pilot { .. } | Self::App(_) => None,
        }
    }
}

impl TitleTab {
    /// Older builds reject an unknown `AppView` and with it the whole saved
    /// workspace, so Flows, Runs, My tasks and Home tabs live only while
    /// the app runs.
    fn saved(&self) -> bool {
        !matches!(
            self,
            Self::App(AppView::Flows | AppView::Runs | AppView::MyTasks | AppView::Home)
        )
    }
}

/// Entries that no longer parse (a retired tab, one from a newer build) are
/// dropped on their own so they cannot cost the whole saved workspace.
fn lenient_tabs<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<TitleTab>, D::Error> {
    let entries = Vec::<serde_json::Value>::deserialize(d)?;
    Ok(entries
        .into_iter()
        .filter_map(|entry| serde_json::from_value(entry).ok())
        .collect())
}

fn lenient_focus<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<HashMap<String, TitleTab>, D::Error> {
    let entries = HashMap::<String, serde_json::Value>::deserialize(d)?;
    Ok(entries
        .into_iter()
        .filter_map(|(project, entry)| Some((project, serde_json::from_value(entry).ok()?)))
        .collect())
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

/// JSON maps need string keys, so focus times are saved as `[tab, secs]` pairs.
fn save_focused_at<S: serde::Serializer>(
    times: &HashMap<TitleTab, i64>,
    s: S,
) -> Result<S::Ok, S::Error> {
    s.collect_seq(times.iter().filter(|(tab, _)| tab.saved()))
}

/// Pairs that no longer parse are dropped on their own, like `lenient_tabs`.
fn lenient_focused_at<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<HashMap<TitleTab, i64>, D::Error> {
    let entries = Vec::<serde_json::Value>::deserialize(d)?;
    Ok(entries
        .into_iter()
        .filter_map(|entry| serde_json::from_value(entry).ok())
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Workspace {
    pub window: crate::window_state::WindowState,
    pub tabs: Vec<i64>,
    #[serde(serialize_with = "save_title_order", deserialize_with = "lenient_tabs")]
    pub title_order: Vec<TitleTab>,
    /// Tabs pinned to the strip's left edge as icon cards, in pin order.
    /// Each stays in `title_order` too; this only marks and orders them.
    #[serde(serialize_with = "save_title_order", deserialize_with = "lenient_tabs")]
    pub pinned_tabs: Vec<TitleTab>,
    #[serde(
        serialize_with = "save_scope_focus",
        deserialize_with = "lenient_focus"
    )]
    pub scope_focus: HashMap<String, TitleTab>,
    /// When each title tab was last focused, in unix seconds. Tabs idle for
    /// `strip_groups::IDLE_AFTER_SECS` leave the strip.
    #[serde(
        serialize_with = "save_focused_at",
        deserialize_with = "lenient_focused_at"
    )]
    pub tab_focused_at: HashMap<TitleTab, i64>,
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
    /// Projects archived from the sidebar, newest first. Their chats stay in
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
    pub sidebar_automations_visible: bool,
    pub sidebar_inbox_visible: bool,
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
    /// Terminals pulled out of a dock into a split or onto the title strip,
    /// by their negative pane IDs. Like file panes, each borrows its owner
    /// session's project and worktree scope.
    pub terminal_panes: HashMap<i64, TerminalPane>,
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
    /// What each project's last action ran: its command, cwd and env. The
    /// footer runs it with one click only while it still runs the same.
    pub last_action_fingerprints: HashMap<String, String>,
    /// The editor the title bar's Open in editor last opened a folder in.
    pub last_external_editor: Option<String>,
    /// The new single-sidebar layout can keep its full-height, square frame.
    pub new_layout_boxy: bool,
    /// The first-run setup card to resume on; `None` once setup is done, and
    /// for every install saved before setup existed.
    pub setup: Option<crate::setup::SetupStep>,
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
            tab_focused_at: HashMap::new(),
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
        self.tab_focused_at = own.tab_focused_at;
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
pub(crate) use gespan_core::project_actions::ACTION_ICONS;
impl ProjectAction {
    pub fn icon(&self) -> &'static str {
        ACTION_ICONS
            .iter()
            .find(|icon| **icon == self.icon)
            .copied()
            .unwrap_or(ACTION_ICONS[0])
    }
    /// As core lists it, so agents see the same personal actions.
    pub fn personal(&self) -> gespan_core::project_actions::Personal {
        gespan_core::project_actions::Personal {
            id: self.id.clone(),
            name: self.name.clone(),
            command: self.command.clone(),
            icon: self.icon.clone(),
        }
    }
}

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
/// The dock entry of a tab that runs a command: it is not restored, since a
/// reopened tab would be an empty shell.
pub(crate) const COMMAND_TAB: &str = "";
/// A terminal living in the pane tree. Shells are not restored; the pane
/// reopens a fresh shell in the same directory, as a dock tab does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TerminalPane {
    pub owner: i64,
    pub cwd: String,
}

/// A project's dock: its place, size and the terminals it holds. Shells are
/// not restored; the dock reopens the same tabs in their directories.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub(crate) struct TerminalDock {
    pub side: DockSide,
    pub size: f32,
    pub open: bool,
    /// Each terminal's working directory, in tab order.
    pub terminals: Vec<String>,
    pub active: usize,
    /// Which tabs mirror an agent's run, in tab order. They are never saved,
    /// so a restart doesn't reopen them as shells.
    #[serde(skip)]
    pub mirrors: Vec<bool>,
}
impl Serialize for TerminalDock {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Saved<'a> {
            side: DockSide,
            size: f32,
            open: bool,
            terminals: Vec<&'a String>,
            active: usize,
        }
        let shells: Vec<&String> = (self.terminals.iter().enumerate())
            .filter(|(ix, _)| !self.is_mirror(*ix))
            .map(|(_, cwd)| cwd)
            .collect();
        // An active mirror saves the next shell as active, or the last one
        // when no shell follows it.
        let before = (0..self.active).filter(|ix| !self.is_mirror(*ix)).count();
        Saved {
            side: self.side,
            size: self.size,
            open: self.open,
            active: before.min(shells.len().saturating_sub(1)),
            terminals: shells,
        }
        .serialize(serializer)
    }
}
impl Default for TerminalDock {
    fn default() -> Self {
        Self {
            side: DockSide::Bottom,
            size: DockSide::Bottom.default_size(),
            open: true,
            terminals: Vec::new(),
            active: 0,
            mirrors: Vec::new(),
        }
    }
}
impl TerminalDock {
    /// A tab mirroring an agent's run. It neither opens the dock nor takes
    /// over the active tab.
    pub fn add_mirror(&mut self, cwd: String) -> usize {
        if self.terminals.is_empty() {
            self.active = 0;
            self.open = false;
        }
        self.marks().push(true);
        self.terminals.push(cwd);
        self.terminals.len() - 1
    }
    pub fn is_mirror(&self, ix: usize) -> bool {
        self.mirrors.get(ix).copied().unwrap_or(false)
    }
    /// The mirror markers, one per tab; a restored dock has none.
    fn marks(&mut self) -> &mut Vec<bool> {
        self.mirrors.resize(self.terminals.len(), false);
        &mut self.mirrors
    }
    /// addTerminalToDock: the new terminal is active and the dock opens.
    pub fn add(&mut self, cwd: String) -> usize {
        self.marks().push(false);
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
        self.marks().remove(ix);
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
        let marks = self.marks();
        let mirror = marks.remove(from);
        marks.insert(to, mirror);
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
            self.mirrors = vec![self.is_mirror(keep)];
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
            pinned_tabs: Vec::new(),
            scope_focus: HashMap::new(),
            tab_focused_at: HashMap::new(),
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
            sidebar_automations_visible: true,
            sidebar_inbox_visible: true,
            right_panel_open: false,
            right_panel_tab: Default::default(),
            right_panel_width: crate::right_panel::WIDTH_DEFAULT,
            right_panel_tabs: crate::right_panel::RightTab::normalize(&[]),
            changes_view: Default::default(),
            reviews: Default::default(),
            review_owners: Default::default(),
            terminal_panes: Default::default(),
            pilot_panes: Default::default(),
            layouts: Default::default(),
            focused: Default::default(),
            docks: Default::default(),
            actions: HashMap::new(),
            last_actions: HashMap::new(),
            last_action_fingerprints: HashMap::new(),
            last_external_editor: None,
            // Fresh installs start on the rounded frame; saved choices are kept.
            new_layout_boxy: false,
            setup: None,
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
    /// Seed the project order from the desktop's saved project order.
    fn restore_project_order(&mut self) {
        let Some(metadata) = self.desktop_preferences_metadata.as_ref() else {
            return;
        };
        let read = |key: &str| -> Option<serde_json::Value> {
            metadata
                .get(key)?
                .as_str()
                .and_then(|s| serde_json::from_str(s).ok())
        };
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
        self.new_layout_boxy
    }

    /// Moves this running workspace to a renamed project folder, using the same
    /// rewrite as a saved one. What belongs to this run is not saved, so it is
    /// carried over.
    pub(crate) fn rebase_project(&mut self, from: &str, to: &str) -> Result<(), String> {
        use gespan_core::files::path_key;
        let mut state = serde_json::to_value(&*self).map_err(|error| error.to_string())?;
        rebase_saved_project(&mut state, from, to);
        let mut rebased: Self = serde_json::from_value(state).map_err(|error| error.to_string())?;
        rebased.worktree_focus = std::mem::take(&mut self.worktree_focus);
        rebased.tab_workspaces = std::mem::take(&mut self.tab_workspaces);
        for path in rebased.tab_workspaces.values_mut() {
            if path_key(path) == path_key(from) {
                *path = to.into();
            }
        }
        *self = rebased;
        Ok(())
    }

    pub fn restore(saved: Option<&str>, sessions: &[(i64, String)], cwd: &str) -> Self {
        let mut state = saved
            .and_then(|s| serde_json::from_str::<Self>(s).ok())
            .unwrap_or_default();
        let raw = saved.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
        // The classic layout drew the boxy frame. A save from it keeps that
        // look unless it already chose a workspace style.
        if let Some(raw) = &raw
            && raw.get("new_layout_boxy").is_none()
            && legacy_classic(raw)
        {
            state.new_layout_boxy = true;
        }
        // Every native save writes `new_layout_boxy`; a workspace that only
        // the desktop import has written takes the desktop's project order.
        if raw
            .as_ref()
            .is_none_or(|raw| raw.get("new_layout_boxy").is_none())
        {
            state.restore_project_order();
        }
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
        let (owners, pilots) = (&state.review_owners, &state.pilot_panes);
        state.terminal_panes.retain(|id, pane| {
            *id < 0
                && !owners.contains_key(id)
                && !pilots.contains_key(id)
                && sessions.iter().any(|(known, _)| *known == pane.owner)
        });
        known.extend(state.terminal_panes.keys().copied());
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
                .chain(state.pinned_tabs.iter_mut())
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
        state.prune_terminal_panes();
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
        for dock in state.docks.values_mut() {
            dock.terminals.retain(|cwd| cwd != COMMAND_TAB);
        }
        state.docks.retain(|_, dock| !dock.terminals.is_empty());
        for dock in state.docks.values_mut() {
            dock.active = dock.active.min(dock.terminals.len() - 1);
            dock.size = dock.side.clamp(dock.size, (4000., 4000.));
        }
        state.add_project(cwd);
        let open: HashSet<_> = state.open_sessions().into_iter().collect();
        let focus = std::mem::take(&mut state.project_focus);
        let owner = |id: i64| state.session_for_pane(id);
        let focus: HashMap<_, _> = focus
            .into_iter()
            .filter(|(project, id)| {
                open.contains(id)
                    && sessions
                        .iter()
                        .any(|(known, path)| *known == owner(*id) && path == project)
            })
            .collect();
        state.project_focus = focus;
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
    /// Ends first-run setup on the project picked. The launch folder, added
    /// as a project on every start, goes again unless it was picked or has
    /// sessions.
    pub(crate) fn finish_setup(&mut self, chosen: &str, launch: &str, launch_has_sessions: bool) {
        self.setup = None;
        self.add_project(chosen);
        self.project = chosen.into();
        if launch != chosen && !launch_has_sessions {
            self.projects.retain(|p| p != launch);
        }
    }
    pub fn add_project(&mut self, path: &str) {
        self.archived_projects.retain(|p| p != path);
        if !self.projects.iter().any(|p| p == path) {
            self.projects.push(path.into());
        }
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
    /// Moves a project one place along the saved order. Returns whether it
    /// moved.
    pub fn step_project_order(&mut self, path: &str, step: isize) -> bool {
        let Some(to) = self
            .projects
            .iter()
            .position(|p| p == path)
            .and_then(|ix| ix.checked_add_signed(step.signum()))
            .filter(|to| *to < self.projects.len())
        else {
            return false;
        };
        self.move_project(path, to)
    }
    /// Moves a project to either end of the list: `step < 0` for the start.
    pub fn move_project_to_end(&mut self, path: &str, step: isize) -> bool {
        let to = if step < 0 {
            0
        } else {
            self.projects.len().saturating_sub(1)
        };
        self.move_project(path, to)
    }
    pub fn session_for_pane(&self, id: i64) -> i64 {
        self.review_owners
            .get(&id)
            .copied()
            .or_else(|| self.terminal_panes.get(&id).map(|t| t.owner))
            .unwrap_or(id)
    }
    /// A file or terminal pane standing in for its owner session.
    pub fn is_detached_pane(&self, id: i64) -> bool {
        self.review_owners.contains_key(&id) || self.terminal_panes.contains_key(&id)
    }
    /// File, Pilot and terminal panes share the negative IDs below every
    /// session.
    pub fn next_pane_id(&self) -> i64 {
        self.review_owners
            .keys()
            .chain(self.pilot_panes.keys())
            .chain(self.terminal_panes.keys())
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
            .chain(self.pinned_tabs.iter_mut())
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
            .chain(self.pinned_tabs.iter_mut())
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
                        .chain(self.pinned_tabs.iter_mut())
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
    /// Terminal panes no tab holds any more, forgotten. Their shells stop
    /// with them, so the caller drops their views.
    pub fn prune_terminal_panes(&mut self) -> Vec<i64> {
        let open: HashSet<_> = self.open_sessions().into_iter().collect();
        let gone: Vec<_> = self
            .terminal_panes
            .keys()
            .copied()
            .filter(|id| !open.contains(id))
            .collect();
        for id in &gone {
            self.terminal_panes.remove(id);
        }
        gone
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
    pub fn sash_snaps(&self, tab: i64, path: &[usize], index: usize) -> Vec<f32> {
        self.layouts
            .get(&tab)
            .map_or_else(Vec::new, |layout| layout.sash_snaps(path, index))
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
        self.prune_pinned_tabs();
        let order = &self.title_order;
        self.tab_focused_at.retain(|tab, _| order.contains(tab));
    }
    /// Tabs with no focus time (saved before times were kept, or rekeyed)
    /// start their clock at `now`, so they are never closed on sight.
    pub fn start_tab_clocks(&mut self, now: i64) {
        for tab in &self.title_order {
            self.tab_focused_at.entry(tab.clone()).or_insert(now);
        }
    }
    /// Pins follow their tabs: a tab that left the strip is no longer pinned.
    pub fn prune_pinned_tabs(&mut self) {
        let mut seen = HashSet::new();
        let order = &self.title_order;
        self.pinned_tabs
            .retain(|tab| order.contains(tab) && seen.insert(tab.clone()));
    }
}

/// Closed tabs are pruned rather than reopened by Back, and a new visit
/// discards Forward.
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
/// state to the new folder. This runs
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

/// Whether a save came from the retired classic layout.
fn legacy_classic(saved: &serde_json::Value) -> bool {
    saved
        .get("classic_layout")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

/// Only fields that exist are touched: indexing would add nulls that the
/// workspace cannot read back.
fn field<'a>(state: &'a mut serde_json::Value, path: &[&str]) -> Option<&'a mut serde_json::Value> {
    path.iter()
        .try_fold(state, |value, key| value.get_mut(*key))
}

/// A linked Git worktree is never a project of its own: one saved as a
/// project, or holding sessions, folds into its main checkout, and its
/// sessions keep running in the worktree. This runs before the workspace is
/// restored. `root` finds a worktree's main checkout.
pub(crate) fn fold_worktree_projects(
    store: &gespan_core::store::Store,
    saved: Option<String>,
    root: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let mut state = saved
        .as_deref()
        .and_then(|saved| serde_json::from_str::<serde_json::Value>(saved).ok());
    let mut paths: Vec<String> = Vec::new();
    if let Some(state) = &state {
        for value in std::iter::once(&state["project"])
            .chain(state["projects"].as_array().into_iter().flatten())
            .chain(state["archived_projects"].as_array().into_iter().flatten())
        {
            if let Some(path) = value.as_str() {
                paths.push(path.into());
            }
        }
    }
    paths.extend(
        store
            .sessions()
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.cwd),
    );
    let mut seen = HashSet::new();
    paths.retain(|path| {
        !path.is_empty() && !path.starts_with("remote://") && seen.insert(path.clone())
    });
    let mut folded = false;
    for path in paths {
        let Some(root) = root(&path) else {
            continue;
        };
        if let Err(error) = store.fold_worktree_sessions(&path, &root) {
            tracing::warn!(%error, worktree = %path, "cannot move a worktree's sessions to its project");
            continue;
        }
        tracing::info!(worktree = %path, project = %root, "listing a worktree under its project");
        if let Some(state) = &mut state {
            rename_saved_project(state, &path, &root);
            folded = true;
        }
    }
    let Some(state) = state.filter(|_| folded) else {
        return saved;
    };
    let folded = state.to_string();
    if let Err(error) = store.save_ui_state(&folded) {
        tracing::warn!(%error, "cannot save the workspace after folding worktrees");
    }
    Some(folded)
}

/// Renames the project `from` to `to` wherever the saved workspace keys or
/// lists projects. Paths inside the folder are left as they are.
fn rename_saved_project(state: &mut serde_json::Value, from: &str, to: &str) {
    use gespan_core::files::path_key;
    use serde_json::Value;
    let same = |path: &str| path_key(path) == path_key(from);
    let rebase = |value: &mut Value| {
        if value.as_str().is_some_and(same) {
            *value = Value::String(to.into());
        }
    };
    if let Some(project) = field(state, &["project"]) {
        rebase(project);
    }
    for list in ["projects", "archived_projects", "collapsed"] {
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
        &["last_action_fingerprints"],
        &["project_appearance"],
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
}

/// Rewrites every place the saved workspace names the project `from`.
fn rebase_saved_project(state: &mut serde_json::Value, from: &str, to: &str) {
    use gespan_core::files::{is_equal_or_inside, path_key, rebase_path};
    use serde_json::Value;
    rename_saved_project(state, from, to);
    let same = |path: &str| path_key(path) == path_key(from);
    let rebase_inside = |value: &mut Value| {
        if value.as_str().is_some_and(same) {
            *value = Value::String(to.into());
        } else if let Some(path) = value.as_str().filter(|path| is_equal_or_inside(path, from)) {
            *value = Value::String(rebase_path(path, from, to));
        }
    };
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
    fn worktree_projects_and_sessions_fold_into_their_main_checkout_at_launch() {
        let store = gespan_core::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let tree = "/work/app-worktrees/fix";
        let in_tree = store.create("claude", tree).unwrap();
        // A worktree that only holds sessions, never opened as a project.
        let other = store.create("codex", "/work/app-worktrees/other").unwrap();
        let clone = store.create("claude", "/work/app-copy").unwrap();
        let saved = serde_json::json!({
            "project": tree,
            "projects": ["/work/notes", tree, "/work/app", "/work/app-copy"],
            "archived_projects": [],
            "collapsed": [tree],
            "project_appearance": {tree: {"name": "Fix"}},
        })
        .to_string();
        let root = |path: &str| {
            path.starts_with("/work/app-worktrees/")
                .then(|| "/work/app".to_owned())
        };
        let folded = super::fold_worktree_projects(&store, Some(saved), root).unwrap();
        let state: serde_json::Value = serde_json::from_str(&folded).unwrap();
        assert_eq!(state["project"], "/work/app");
        // The root keeps the worktree's place once, an independent clone stays.
        assert_eq!(
            state["projects"],
            serde_json::json!(["/work/notes", "/work/app", "/work/app-copy"])
        );
        assert_eq!(state["collapsed"], serde_json::json!(["/work/app"]));
        assert_eq!(state["project_appearance"]["/work/app"]["name"], "Fix");
        assert_eq!(store.ui_state().unwrap().as_deref(), Some(folded.as_str()));
        let sessions = store.sessions().unwrap();
        let cwd = |id| sessions.iter().find(|s| s.id == id).unwrap().cwd.clone();
        assert_eq!(cwd(in_tree.id), "/work/app");
        assert_eq!(cwd(other.id), "/work/app");
        assert_eq!(cwd(clone.id), "/work/app-copy");
        let worktrees = store.session_worktrees().unwrap();
        assert_eq!(worktrees[&in_tree.id].as_deref(), Some(tree));
        assert_eq!(
            worktrees[&other.id].as_deref(),
            Some("/work/app-worktrees/other")
        );
        assert_eq!(worktrees[&clone.id], None);
        // A second launch has nothing left to fold and leaves the state alone.
        let again = super::fold_worktree_projects(&store, Some(folded.clone()), root);
        assert_eq!(again.as_deref(), Some(folded.as_str()));
        assert_eq!(super::fold_worktree_projects(&store, None, root), None);
    }
    #[test]
    fn a_running_workspace_follows_a_renamed_project_and_keeps_this_runs_state() {
        let mut state = Workspace::default();
        state.add_project("/work/gespan");
        state.add_project("/work/other");
        state.project = "/work/gespan".into();
        state.open(7);
        state.tab_workspaces.insert(7, "/work/gespan".into());
        state.tab_workspaces.insert(8, "/work/other".into());
        state.worktree_focus.insert(
            "/work/gespan".into(),
            crate::worktree_switcher::WorktreeFocus {
                path: "/worktrees/one".into(),
                branch: Some("one".into()),
            },
        );

        state
            .rebase_project("/work/gespan", "/work/renamed")
            .unwrap();

        assert_eq!(state.project, "/work/renamed");
        assert_eq!(state.projects, ["/work/renamed", "/work/other"]);
        assert_eq!(state.active, Some(7));
        assert_eq!(state.tab_workspaces[&7], "/work/renamed");
        assert_eq!(state.tab_workspaces[&8], "/work/other");
        assert_eq!(state.worktree_focus.len(), 1);
    }

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
    fn a_classic_save_restores_into_focused_with_everything_kept() {
        let saved = r#"{"projects":["/a","/b"],"tabs":[7],"title_order":[{"Session":7},{"App":"Board"}],
          "classic_layout":true,"classic_sidebar_tabs":{"/a":"files"},"classic_sidebar_tab_order":["files"],
          "project_rail_open":false,"project_rail_width":248.0,"project_rail_pinned":["/b"],
          "project_rail_groups":[{"id":"g","name":"G"}],"project_rail_assignments":{"/a":"g"},
          "project_rail_pilots_pinned":true,"new_layout_prompt_dismissed":true,
          "preferences":{"collapsed_project_rail":"hidden","rail_pilots_pinned":true}}"#;
        let state = super::Workspace::restore(Some(saved), &[(7, "/a".into())], "/a");
        assert_eq!(state.projects, ["/a", "/b"]);
        assert_eq!(
            state.title_order,
            [TitleTab::Session(7), TitleTab::App(AppView::Board)]
        );
        assert!(
            state.new_layout_boxy,
            "classic with no saved style implies boxy"
        );
    }

    #[test]
    fn a_saved_workspace_style_wins_over_the_classic_flag() {
        let state = super::Workspace::restore(
            Some(r#"{"classic_layout":true,"new_layout_boxy":false}"#),
            &[],
            "/work",
        );
        assert!(!state.new_layout_boxy);
    }

    #[test]
    fn the_save_never_writes_classic_or_rail_keys() {
        let json = serde_json::to_value(super::Workspace::restore(None, &[], "/work")).unwrap();
        for key in [
            "classic_layout",
            "classic_sidebar_tabs",
            "classic_sidebar_tab_order",
            "new_layout_prompt_dismissed",
            "project_rail_open",
            "project_rail_width",
            "project_rail_pinned",
            "project_rail_groups",
            "project_rail_assignments",
            "project_rail_pilots_pinned",
        ] {
            assert!(json.get(key).is_none(), "{key}");
        }
        for key in ["collapsed_project_rail", "rail_pilots_pinned"] {
            assert!(json["preferences"].get(key).is_none(), "{key}");
        }
    }

    #[test]
    fn project_moves_follow_the_saved_order() {
        let mut state = super::Workspace {
            projects: ["/a", "/b", "/c", "/e"].map(str::to_owned).to_vec(),
            ..Default::default()
        };
        assert!(state.step_project_order("/c", -1));
        assert_eq!(state.projects, ["/a", "/c", "/b", "/e"]);
        assert!(!state.step_project_order("/a", -1), "/a leads");
        assert!(!state.step_project_order("/e", 1), "/e ends");
        assert!(state.move_project_to_end("/e", -1));
        assert_eq!(state.projects[0], "/e");
        assert!(state.move_project_to_end("/e", 1));
        assert_eq!(state.projects[3], "/e");
        assert!(!state.move_project_to_end("/missing", 1));
    }

    #[test]
    fn the_desktop_project_order_seeds_only_a_workspace_native_never_saved() {
        let metadata =
            r#""desktop_preferences_metadata":{"gespan.projectRailOrder":"[\"/b\",\"/a\"]"}"#;
        let imported = super::Workspace::restore(
            Some(&format!(r#"{{"projects":["/a","/b"],{metadata}}}"#)),
            &[],
            "/a",
        );
        assert_eq!(imported.projects, ["/b", "/a"]);
        let saved = serde_json::to_string(&imported).unwrap();
        let mut reordered = super::Workspace::restore(Some(&saved), &[], "/a");
        assert!(reordered.move_project_to_end("/a", -1));
        let saved = serde_json::to_string(&reordered).unwrap();
        let restored = super::Workspace::restore(Some(&saved), &[], "/a");
        assert_eq!(restored.projects, ["/a", "/b"], "a native reorder is kept");
    }

    #[test]
    fn a_legacy_classic_save_keeps_its_boxy_frame_across_saves() {
        let state = super::Workspace::restore(Some(r#"{"classic_layout":true}"#), &[], "/work");
        assert!(state.new_layout_boxy);
        assert!(state.boxy_layout());
        let saved = serde_json::to_string(&state).unwrap();
        let state = super::Workspace::restore(Some(&saved), &[], "/work");
        assert!(state.new_layout_boxy);
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
        pinned_tabs: Vec<BeforeFlowsTab>,
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
    fn a_saved_runs_tab_does_not_cost_an_older_build_its_workspace() {
        let mut state = Workspace {
            project: "/work/gespan".into(),
            ..Default::default()
        };
        state.open(7);
        state.title_order.push(TitleTab::App(AppView::Automations));
        state.title_order.push(TitleTab::App(AppView::Runs));
        state.title_order.push(TitleTab::App(AppView::Flows));
        state.pinned_tabs.push(TitleTab::App(AppView::Runs));
        state.pinned_tabs.push(TitleTab::App(AppView::Flows));
        state.pinned_tabs.push(TitleTab::App(AppView::Automations));
        state
            .scope_focus
            .insert("/work/gespan".into(), TitleTab::App(AppView::Runs));
        state
            .scope_focus
            .insert("/work/other".into(), TitleTab::Session(7));
        let saved = serde_json::to_string(&state).unwrap();

        // A build that knows neither Runs nor Flows still reads the file.
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
        assert_eq!(
            older.pinned_tabs,
            vec![BeforeFlowsTab::App(BeforeFlowsView::Automations)]
        );
        assert_eq!(older.scope_focus.len(), 1);
        assert_eq!(older.scope_focus["/work/other"], BeforeFlowsTab::Session(7));

        // This build restores everything except the Runs tab.
        let restored = Workspace::restore(Some(&saved), &[(7, "/work/gespan".into())], "/work");
        assert_eq!(restored.project, "/work/gespan");
        assert_eq!(restored.tabs, vec![7]);
        assert!(
            restored
                .title_order
                .contains(&TitleTab::App(AppView::Automations))
        );
        assert!(!restored.title_order.contains(&TitleTab::App(AppView::Runs)));
        assert!(!restored.pinned_tabs.contains(&TitleTab::App(AppView::Runs)));
        assert!(!restored.scope_focus.contains_key("/work/gespan"));
        // The in-memory workspace keeps its open Runs tab until restart.
        assert!(state.title_order.contains(&TitleTab::App(AppView::Runs)));
        assert!(state.pinned_tabs.contains(&TitleTab::App(AppView::Runs)));
        assert_eq!(
            state.scope_focus["/work/gespan"],
            TitleTab::App(AppView::Runs)
        );
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
    fn mirror_tabs_follow_close_and_reorder_and_are_never_saved() {
        let saved = |dock: &TerminalDock| {
            let json = serde_json::to_value(dock).unwrap();
            assert!(json.get("mirrors").is_none());
            let terminals: Vec<String> = serde_json::from_value(json["terminals"].clone()).unwrap();
            (terminals, json["active"].as_u64().unwrap())
        };
        let mut dock = TerminalDock::default();
        dock.add("/a".into());
        dock.open = false;
        // Gaining a mirror neither opens the dock nor moves its active tab.
        assert_eq!(dock.add_mirror("/agent".into()), 1);
        assert!(!dock.open);
        assert_eq!(dock.active, 0);
        dock.add("/b".into());
        dock.add_mirror("/agent2".into());
        assert_eq!(dock.active, 2);
        assert_eq!(saved(&dock), (vec!["/a".into(), "/b".into()], 1));
        // The marker moves with its tab.
        dock.reorder(1, 0);
        assert_eq!(dock.terminals, ["/agent", "/a", "/b", "/agent2"]);
        let marks = |dock: &TerminalDock| {
            (0..dock.terminals.len())
                .map(|ix| dock.is_mirror(ix))
                .collect::<Vec<_>>()
        };
        assert_eq!(marks(&dock), [true, false, false, true]);
        assert_eq!(saved(&dock), (vec!["/a".into(), "/b".into()], 1));
        // Closing the mirror before the shells keeps the rest marked right.
        assert!(dock.close(0));
        assert_eq!(marks(&dock), [false, false, true]);
        assert_eq!(saved(&dock), (vec!["/a".into(), "/b".into()], 1));
        // A mirror that is active saves the shell before it as active.
        dock.active = 2;
        assert_eq!(saved(&dock), (vec!["/a".into(), "/b".into()], 1));
        dock.reorder(2, 0);
        assert_eq!(dock.terminals, ["/agent2", "/a", "/b"]);
        assert_eq!(marks(&dock), [true, false, false]);
        assert_eq!(saved(&dock), (vec!["/a".into(), "/b".into()], 0));
        // Closing a shell after the mirror leaves the mirror marked.
        assert!(dock.close(1));
        assert_eq!(marks(&dock), [true, false]);
        assert_eq!(saved(&dock), (vec!["/b".into()], 0));
        // A dock left holding only a mirror is not restored.
        dock.keep_only(0);
        assert_eq!(marks(&dock), [true]);
        assert_eq!(saved(&dock), (vec![], 0));
        let mut s = Workspace::default();
        s.docks.insert("/w".into(), dock);
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("/agent"), "{json}");
        let s = Workspace::restore(Some(&json), &[], "/w");
        assert!(!s.docks.contains_key("/w"));
        // A restored dock has no mirrors, however its tabs were set.
        let mut restored = TerminalDock {
            terminals: vec!["/x".into(), "/y".into()],
            ..Default::default()
        };
        assert_eq!(marks(&restored), [false, false]);
        restored.add_mirror("/agent".into());
        assert!(restored.close(0));
        assert_eq!(marks(&restored), [false, true]);
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
        assert!(!fresh.new_layout_boxy, "fresh installs start rounded");
        let boxy = Workspace::restore(Some(r#"{"new_layout_boxy":true}"#), &sessions, "/a");
        assert!(boxy.new_layout_boxy, "a saved boxy choice is kept");
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
        // Visibility flags older builds wrote are ignored, not an error.
        let s = Workspace::restore(
            Some(
                r#"{"sidebar_search_visible":false,"sidebar_notes_visible":false,"sidebar_activity_visible":false,"compact_session_cards":false}"#,
            ),
            &[],
            "/work/a",
        );
        assert!(!s.compact_session_cards, "the rest of the save still loads");
        assert!(s.preferences.notes && s.preferences.working_agents);
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

    #[test]
    fn restore_drops_command_tabs_and_keeps_the_shells_around_them() {
        let mut state = Workspace::restore(None, &[], "/work");
        let dock = state.docks.entry("/work".into()).or_default();
        dock.add("/work/a".into());
        dock.add(COMMAND_TAB.into());
        dock.add("/work/b".into());
        dock.add(COMMAND_TAB.into());
        let saved = serde_json::to_string(&state).unwrap();
        let restored = Workspace::restore(Some(&saved), &[], "/work");
        let dock = &restored.docks["/work"];
        assert_eq!(dock.terminals, ["/work/a", "/work/b"]);
        assert!(dock.active < dock.terminals.len());
        // A dock holding only command tabs disappears.
        let mut only = Workspace::restore(None, &[], "/work");
        only.docks
            .entry("/work".into())
            .or_default()
            .add(COMMAND_TAB.into());
        let saved = serde_json::to_string(&only).unwrap();
        assert!(
            !Workspace::restore(Some(&saved), &[], "/work")
                .docks
                .contains_key("/work")
        );
    }

    #[test]
    fn a_saved_search_tab_is_dropped_on_load() {
        let saved = r#"{"projects":["/a"],"tabs":[7],"title_order":[{"Session":7},"Search"],"pinned_tabs":["Search"],"scope_focus":{"/a":"Search"},"preferences":{"search_panel":false}}"#;
        let restored = Workspace::restore(Some(saved), &[(7, "/a".into())], "/a");
        assert_eq!(restored.projects, ["/a"]);
        assert_eq!(restored.title_order, [TitleTab::Session(7)]);
        assert!(restored.pinned_tabs.is_empty());
        assert!(restored.scope_focus.is_empty());
    }

    #[test]
    fn an_unknown_tab_does_not_discard_the_rest_of_the_workspace() {
        let saved = r#"{"projects":["/a"],"title_order":["Gone",{"Session":7},{"Mystery":1}],"scope_focus":{"/a":{"Session":7},"/b":"Gone"}}"#;
        let state: Workspace = serde_json::from_str(saved).expect("one bad entry is not fatal");
        assert_eq!(state.projects, ["/a"]);
        assert_eq!(state.title_order, [TitleTab::Session(7)]);
        assert_eq!(state.scope_focus.len(), 1);
        assert_eq!(state.scope_focus["/a"], TitleTab::Session(7));
    }
}
