//! The "Two zones" left nav. An **Everywhere** card holds what spans every
//! project: Search as a field, then Home, My tasks, Git and Automations as
//! labelled rows. Under it, a **This project** card tinted with the
//! project's own colour holds the switcher, New thread and (in the virtual
//! list) the Pilots, Now and Sessions. The running card and Settings close
//! the rail. Measured from the design board `SoTSidebar.dc.html`.
//!
//! What goes in which section is decided in [`super::rail_plan`]; this
//! module gathers the inputs and draws the rows.
// A panic while drawing takes the whole app down, so render code here may
// not index, slice, unwrap or expect.
#![deny(clippy::indexing_slicing, clippy::unwrap_used, clippy::expect_used)]
use super::rail_plan::{self, Folder, FolderInput, FolderKind, Live};
use super::session_groups::{PILOT_PREFIX, RenameTarget, toggle_group};
use super::*;
use gespan_core::session_groups::Entry;
use gpui::{BoxShadow, Hsla, Stateful};

/// The rail's side padding: rows sit 10px in from its edges.
pub(super) const RAIL_PAD: f32 = 10.;
/// `.rl-row`: sessions, New thread and Settings.
pub(super) const ROW_HEIGHT: f32 = 28.;
const ROW_FONT: f32 = 12.5;
/// `.fold`: a folder header.
const FOLD_HEIGHT: f32 = 26.;
/// A row's own left padding in the flat Sessions list.
pub(super) const ROW_INSET: f32 = 10.;
/// `.kid`: a session under an open folder, in line with the folder's name.
pub(super) const KID_INDENT: f32 = 33.;
/// How much further in each level of started sessions sits.
pub(super) const CHILD_STEP: f32 = 23.;
/// `.zone`: a card's padding, inside its 1px border.
pub(super) const CARD_PAD: f32 = 4.;
/// The space between the Everywhere and This project cards.
const CARD_GAP: f32 = 10.;
const SEARCH_HEIGHT: f32 = 30.;
/// The space under the Search field and under the switcher, before the
/// rows that follow each: the same breathing room in both cards.
pub(super) const HEAD_GAP: f32 = 4.;
const MORE_HEIGHT: f32 = 24.;
/// The time column, right-aligned.
const TIME_WIDTH: f32 = 26.;
/// A row's branch label, before it truncates (`.wt`).
const BRANCH_LABEL_WIDTH: f32 = 74.;
const SWITCHER_HEIGHT: f32 = 44.;
/// The switcher's tooltip: its shortcut moved here to make room for "…".
const SWITCHER_HINT: &str = if cfg!(target_os = "macos") {
    "Switch project  ⌘O"
} else {
    "Switch project  Ctrl+O"
};

/// The waiting yellow and its ink, light-theme variants second.
const WAIT: u32 = 0xe8c547;
const WAIT_INK: (u32, u32) = (0xf5dc84, 0xa86b00);

pub(super) fn wait_ink() -> Hsla {
    rgb(if is_light() { WAIT_INK.1 } else { WAIT_INK.0 }).into()
}
/// Ink at the board's greys: `#d0d0d0` rows, `#a3a3a3` closed folders and
/// `#8a8a8a` captions.
pub(super) fn row_ink() -> Hsla {
    content(0.86)
}
pub(super) fn muted_ink() -> Hsla {
    content(0.64)
}
pub(super) fn faint_ink() -> Hsla {
    content(0.54)
}

/// A page icon's colour on the rail: the board's tints in the dark theme,
/// the navigation tints in light, where the pastels would wash out.
pub(super) fn rail_tint(label: &str) -> Option<Hsla> {
    if is_light() {
        return nav_tint(label);
    }
    let tint = match label {
        "New thread" => 0x6ee7b7,
        "My tasks" => WAIT,
        "Git" => 0x2dd4bf,
        "Automations" => 0xfbbf24,
        _ => return nav_tint(label).filter(|_| label != "Home" && label != "Search"),
    };
    Some(rgb(tint).into())
}

/// The 7px status dot, with the board's 3px halo while live. The halo is a
/// 13px circle behind the dot, drawn outside its 7px box as CSS's spread
/// shadow is, so rows line up on the dot itself.
pub(super) fn live_dot(live: Option<Live>) -> gpui::Div {
    let (color, halo): (Hsla, f32) = match live {
        Some(Live::NeedsYou) => (rgb(WAIT).into(), 0.2),
        Some(Live::Running) => (accent_color(), 0.22),
        None => (content(0.22), 0.),
    };
    div()
        .relative()
        .size(px(7.))
        .flex_shrink_0()
        .when(halo > 0., |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(-3.))
                    .left(px(-3.))
                    .size(px(13.))
                    .rounded_full()
                    .bg(color.opacity(halo)),
            )
        })
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size(px(7.))
                .rounded_full()
                .bg(color),
        )
}

/// `.kbd`: a shortcut in a hairline box.
pub(super) fn kbd(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .flex_shrink_0()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_color(crate::glass::edge())
        .font_family(crate::fonts::monospace())
        .text_size(px(10.5))
        .line_height(px(14.))
        .text_color(faint_ink())
        .child(text.into())
}

/// `.rl-sec`: a section heading, with an optional count and a trailing
/// "N · All N →".
pub(super) fn section(label: &'static str) -> gpui::Div {
    div()
        .debug_selector(move || format!("rail-section-{label}"))
        .w_full()
        .pt(px(12.))
        .px(px(10.))
        .pb(px(4.))
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(11.))
        // CSS's normal line for 11px text; a count pill still makes the
        // line 16px, as on the board.
        .line_height(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(faint_ink())
        .child(label)
}

/// `.rl-sec .more`.
pub(super) fn section_more(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .ml_auto()
        .flex_shrink_0()
        .font_weight(FontWeight::MEDIUM)
        .text_color(faint_ink())
        .child(text.into())
}

/// The fill and highlight of a selected `.rl-row.on`.
fn selected(d: Stateful<gpui::Div>) -> Stateful<gpui::Div> {
    d.bg(content(0.11))
        .text_color(content(1.))
        .shadow(vec![BoxShadow::new(px(0.), px(1.), white(0.08)).inset()])
}

/// A project's icon as the switcher and its picker show it: the project's
/// pixel mascot in its colour on a `content(0.08)` tile, its logo when it
/// has one, or the globe for a remote folder (see
/// [`crate::project_settings::project_icon`]). The switcher uses a 26px tile
/// with a 14px mascot; picker rows a 20px tile with a 10px mascot.
pub(crate) fn project_tile(
    appearance: &gespan_core::project_appearance::ProjectAppearance,
    project: &str,
    side: f32,
    radius: f32,
    mascot: f32,
) -> AnyElement {
    if let Some(logo) = &appearance.logo {
        return crate::image_memory::thumbnail(std::path::PathBuf::from(logo))
            .size(px(side))
            .rounded(px(radius))
            .object_fit(gpui::ObjectFit::Contain)
            .flex_shrink_0()
            .into_any_element();
    }
    div()
        .size(px(side))
        .flex_shrink_0()
        .rounded(px(radius))
        .bg(content(0.08))
        .flex()
        .items_center()
        .justify_center()
        .child(crate::project_settings::project_icon(
            appearance, project, mascot,
        ))
        .into_any_element()
}

/// Moves `project` to the front of the recent list, which keeps a bounded
/// history.
pub(super) fn note_recent(recent: &mut Vec<String>, project: &str) {
    recent.retain(|p| p != project);
    recent.insert(0, project.to_owned());
    recent.truncate(32);
}

/// Pins `project` to the switcher's top, or unpins it.
pub(super) fn toggle_pin(pinned: &mut Vec<String>, project: &str) {
    if pinned.iter().any(|p| p == project) {
        pinned.retain(|p| p != project);
    } else {
        pinned.push(project.to_owned());
    }
}

/// Lays an anchor probe over its whole row, from the row's corner rather
/// than after its padding.
fn anchor_cover(probe: impl IntoElement) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(probe)
        .into_any_element()
}

/// What the bolt says to a screen reader: the page, then what its badge
/// shows.
pub(super) fn automations_aria(going: usize, failing: usize) -> String {
    let mut parts = vec!["Automations".to_string()];
    if going > 0 {
        parts.push(format!("{going} running"));
    }
    if failing > 0 {
        parts.push(format!("{failing} failing"));
    }
    parts.join(", ")
}

/// The Everywhere card's pages, in order, with their glyphs. Flows, Board
/// and Notes join when Settings turns them on.
const PAGES: [(&str, &str); 7] = [
    ("Home", "home"),
    ("My tasks", "circle-alert"),
    ("Git", "git-pull-request"),
    ("Automations", "zap"),
    ("Flows", "git-merge"),
    ("Board", "kanban"),
    ("Notes", "note"),
];

/// `.zone`: a rounded card on the rail. `tint` colours its border and fill,
/// for the project card; without it, the neutral Everywhere card.
pub(super) fn zone(tint: Option<Hsla>) -> gpui::Div {
    let (edge, fill) = match tint {
        Some(tint) => (tint.opacity(0.30), tint.opacity(0.045)),
        None => (content(0.07), content(0.025)),
    };
    div()
        .w_full()
        .rounded(px(12.))
        .border_1()
        .border_color(edge)
        .bg(fill)
        .p(px(CARD_PAD))
        .flex()
        .flex_col()
        .gap_px()
        .overflow_hidden()
}

/// `.zlab`: a card's small caps label, "EVERYWHERE".
fn zone_label(text: &'static str) -> gpui::Div {
    div()
        .debug_selector(move || format!("rail-zone-{text}"))
        .pt(px(10.))
        .px(px(10.))
        .pb(px(4.))
        .text_size(px(10.5))
        .line_height(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(faint_ink())
        .child(text.to_uppercase())
}

/// What a row's trailing column shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Trailing {
    /// Its age, as in Now.
    Age,
    /// Its branch as a small label, or its age while the branch is unknown.
    Branch,
}

impl Shell {
    /// The project switcher's groups and, per project, its branch, what
    /// needs you and runs there, and how long since its last session.
    pub(super) fn switcher_data(&self, cx: &App) -> crate::pickers::Switcher {
        let mut info: HashMap<String, crate::pickers::ProjectInfo> = self
            .workspace
            .projects
            .iter()
            .map(|project| {
                (
                    project.clone(),
                    crate::pickers::ProjectInfo {
                        branch: crate::git_cache::worktree_branch(project, project),
                        ..Default::default()
                    },
                )
            })
            .collect();
        for agent in self.open_agents(cx) {
            let Some(project) = info.get_mut(&agent.cwd) else {
                continue;
            };
            match agent.state {
                super::agents_panel::AgentState::NeedsYou => project.needs_you += 1,
                super::agents_panel::AgentState::Running => project.running += 1,
                super::agents_panel::AgentState::Idle => {}
            }
        }
        let now = gespan_core::time::now_ms();
        let mut last: HashMap<&str, u64> = HashMap::new();
        for summary in self.session_summaries.values() {
            let at = last.entry(summary.cwd.as_str()).or_default();
            *at = (*at).max(summary.updated_at);
        }
        for (project, row) in &mut info {
            row.age = last
                .get(project.as_str())
                .map(|at| rail_plan::age(*at, now))
                .unwrap_or_default();
        }
        crate::pickers::Switcher {
            pinned: self.workspace.pinned_projects.clone(),
            recent: self.workspace.recent_projects.clone(),
            info,
        }
    }

    /// Every loaded chat that waits on the user or is running. Cheap: it
    /// only reads chats already in memory.
    pub(super) fn rail_live(&self, cx: &App) -> HashMap<i64, Live> {
        self.sessions
            .iter()
            .filter_map(|(id, view)| {
                let chat = view.read(cx);
                if chat.is_remote() {
                    None
                } else if chat.needs_approval() {
                    Some((*id, Live::NeedsYou))
                } else if chat.busy {
                    Some((*id, Live::Running))
                } else {
                    None
                }
            })
            .collect()
    }

    /// The user's own folders for this project, when Settings groups by
    /// folders, and every session in none of them. By worktree there are
    /// no folders: every session is in the one flat list. A Pilot's
    /// sessions are listed here too; the plan lists them under the Pilot.
    pub(super) fn rail_folders(
        &self,
        entries: &[Entry],
        _cx: &App,
    ) -> (Vec<FolderInput>, Vec<i64>) {
        let mut loose = Vec::new();
        if self.workspace.preferences.groups_by_worktree() {
            for entry in entries {
                loose.extend(match entry {
                    Entry::Session(id) => vec![*id],
                    Entry::Reminders { sessions, .. }
                    | Entry::Folder { sessions, .. }
                    | Entry::Fixed { sessions, .. }
                    | Entry::Pinned { sessions } => sessions.clone(),
                });
            }
            return (Vec::new(), loose);
        }
        let groups = self.project_groups();
        let mut inputs = Vec::new();
        for entry in entries {
            let (key, label, kind, sessions, open) = match entry {
                Entry::Reminders {
                    collapsed,
                    sessions,
                } => (
                    "reminders".to_owned(),
                    "Reminders".to_owned(),
                    FolderKind::Reminders,
                    sessions.clone(),
                    !collapsed,
                ),
                Entry::Folder { id, sessions } => {
                    let Some(folder) = groups.folder(id) else {
                        continue;
                    };
                    (
                        format!("folder:{id}"),
                        folder.name.clone(),
                        FolderKind::Folder,
                        sessions.clone(),
                        !folder.collapsed,
                    )
                }
                // A Pilot's sessions go under the Pilot, never a folder.
                Entry::Fixed { id, sessions, .. } if id.starts_with(PILOT_PREFIX) => {
                    loose.extend(sessions.iter().copied());
                    continue;
                }
                Entry::Fixed { id, name, sessions } => (
                    id.clone(),
                    name.clone(),
                    FolderKind::Automation,
                    sessions.clone(),
                    !groups.fixed_is_collapsed(id),
                ),
                Entry::Pinned { sessions } => (
                    "pinned".to_owned(),
                    "Pinned".to_owned(),
                    FolderKind::Pinned,
                    sessions.clone(),
                    !groups.pinned_collapsed,
                ),
                Entry::Session(id) => {
                    loose.push(*id);
                    continue;
                }
            };
            inputs.push(FolderInput {
                key,
                label,
                kind,
                sessions,
                open,
            });
        }
        (inputs, loose)
    }

    /// A session row's branch label, from the worktree it runs in. `None`
    /// while the branch is unknown: the row shows its age instead.
    pub(super) fn rail_branch_label(&self, id: i64, cx: &App) -> Option<SharedString> {
        let path = self.session_working_copy(id, cx);
        crate::git_cache::worktree_branch(&self.workspace.project, path)
            .map(|branch| rail_plan::branch_label(&branch).into())
    }

    /// Opens or closes a rail folder, remembering the choice. The user's
    /// folders keep their own stored state, as before.
    pub(super) fn toggle_rail_folder(&mut self, folder: &Folder, cx: &mut Context<Self>) {
        let key = folder.key.clone();
        self.update_groups(cx, |groups| toggle_group(groups, &key));
    }

    /// The Everywhere card, pinned at the top of the rail: Search, then the
    /// pages that span every project as labelled rows.
    pub(super) fn rail_top(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_shrink_0()
            .w_full()
            .px(px(RAIL_PAD))
            .pt(px(4.))
            .pb(px(CARD_GAP))
            .child(
                zone(None)
                    .debug_selector(|| "rail-everywhere".into())
                    .child(zone_label("Everywhere"))
                    .child(self.rail_search_field(cx))
                    .children(self.rail_page_rows(cx)),
            )
            .into_any()
    }

    /// The This project card: tinted with the project's colour, the one its
    /// title-bar tab group wears. It holds the switcher, New thread and the
    /// scrolling list of Pilots, Now and Sessions.
    pub(super) fn rail_project_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let project = self.workspace.project.clone();
        let tint: Hsla = rgb(self.appearance(&project).color(&project)).into();
        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .px(px(RAIL_PAD))
            .flex()
            .flex_col()
            .child(
                zone(Some(tint))
                    .debug_selector(|| "rail-project-card".into())
                    .flex_1()
                    .min_h_0()
                    .child(div().mb(px(HEAD_GAP)).child(self.rail_switcher(cx)))
                    .child(self.sidebar_new_thread(cx))
                    .child(self.virtual_sidebar(cx)),
            )
            .into_any()
    }

    /// Search as a real field, ⌘K.
    fn rail_search_field(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.app_search.is_some();
        div()
            .id("Search")
            .debug_selector(|| "sidebar-navigation-Search".into())
            .role(Role::Button)
            .aria_label("Search sessions")
            .aria_selected(on)
            .cursor_pointer()
            .mx(px(2.))
            .mb(px(HEAD_GAP))
            .h(px(SEARCH_HEIGHT))
            .px(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(content(if on { 0.16 } else { 0.08 }))
            .bg(content(if on { 0.08 } else { 0.04 }))
            .hover(|s| s.bg(content(0.07)))
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.5))
            .text_color(faint_ink())
            .child(icon("search", 14.).text_color(muted_ink()))
            .child(div().flex_1().min_w_0().truncate().child("Search"))
            .child(kbd(format!("{MOD}K")))
            .on_click(cx.listener(|this, _, window, cx| this.focus_search(window, cx)))
            .into_any()
    }

    /// The project switcher: tile, name and "N worktrees · N sessions". It
    /// opens the project picker; a right click opens the project's menu.
    fn rail_switcher(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let project = self.workspace.project.clone();
        let label = self.project_label(&project);
        let open = self.menu_open(&MenuTarget::Header);
        let (folders, sessions) = self.sidebar_list.summary;
        let summary = rail_plan::switcher_summary(
            folders,
            sessions,
            self.workspace.preferences.groups_by_worktree(),
        );
        let fill = if is_light() { white(0.7) } else { white(0.07) };
        div()
            .id("workspace-project")
            .debug_selector(|| "sidebar-project-button".into())
            .relative()
            .role(Role::Button)
            .aria_label(format!("Switch project, {label}"))
            .aria_description(project.clone())
            .aria_expanded(open)
            .cursor_pointer()
            .w_full()
            .h(px(SWITCHER_HEIGHT))
            .px(px(10.))
            .rounded(px(10.))
            .border_1()
            .border_color(if is_light() {
                content(0.12)
            } else {
                white(0.14)
            })
            .bg(if open {
                fill.opacity(fill.a + 0.04)
            } else {
                fill
            })
            .shadow(vec![
                BoxShadow::new(px(0.), px(1.), white(if is_light() { 0.9 } else { 0.1 })).inset(),
            ])
            .hover(|s| s.bg(content(0.1)))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(project_tile(
                &self.appearance(&project),
                &project,
                26.,
                7.,
                14.,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .debug_selector(|| "sidebar-project-name".into())
                            .truncate()
                            .text_size(px(13.5))
                            .line_height(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(1.))
                            .child(label),
                    )
                    .child(
                        div()
                            .debug_selector(|| "sidebar-project-summary".into())
                            .truncate()
                            .text_size(px(11.5))
                            .line_height(px(16.))
                            .text_color(faint_ink())
                            .child(summary),
                    ),
            )
            .child(self.rail_project_actions(cx))
            .child(self.project_anchor.probe())
            .tooltip(|_, cx| {
                cx.new(|_| crate::conversation::HintTooltip(SWITCHER_HINT))
                    .into()
            })
            .on_click(cx.listener(|this, _, window, cx| this.toggle_project_switcher(window, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    let target = MenuTarget::Project {
                        path: this.workspace.project.clone(),
                    };
                    this.open_menu(target, Anchor::Point(event.position), window, cx);
                    cx.stop_propagation();
                }),
            )
            .into_any()
    }

    /// The switcher's "…": opens the project menu (appearance, Move, Pin,
    /// Reveal, Open in editor, Mute, Import sessions, Archive, Delete), the
    /// menu a right click on the switcher also opens.
    fn rail_project_actions(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let label = self.project_label(&self.workspace.project);
        let open = self.menu_open(&MenuTarget::Project {
            path: self.workspace.project.clone(),
        });
        icon_button_hover(
            "sidebar-project-actions",
            "Project actions",
            "more-horizontal",
            0.12,
        )
        .debug_selector(|| "sidebar-project-actions".into())
        .aria_label(format!("Project actions for {label}"))
        .aria_expanded(open)
        .size(px(28.))
        .rounded(px(7.))
        .relative()
        .when(open, |d| d.bg(content(0.14)).text_color(content(1.)))
        .child(self.project_actions_anchor.probe())
        .on_click(cx.listener(|this, _, window, cx| {
            cx.stop_propagation();
            this.toggle_project_actions(window, cx);
        }))
        .into_any()
    }

    /// The switcher's "…": opens the current project's menu, or closes it.
    pub(super) fn toggle_project_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = MenuTarget::Project {
            path: self.workspace.project.clone(),
        };
        if self.menu_open(&target) {
            self.close_menu(cx);
        } else {
            let anchor = Anchor::Slot(self.project_actions_anchor.clone());
            self.open_menu(target, anchor, window, cx);
        }
    }

    /// ⌘O and the switcher: opens the project picker, or closes it.
    pub(super) fn toggle_project_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu_open(&MenuTarget::Header) {
            self.close_menu(cx);
        } else {
            let anchor = Anchor::Slot(self.project_anchor.clone());
            self.open_menu(MenuTarget::Header, anchor, window, cx);
        }
    }

    /// The Everywhere card's pages as labelled rows: My tasks with its
    /// yellow count, Git with an unread dot, Automations with a run dot or
    /// a failed count.
    fn rail_page_rows(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let flows_visible = crate::flows::visible(cx);
        let waiting = self.my_task_count(cx);
        let unread = self
            .inbox
            .as_ref()
            .map_or(0, |inbox| inbox.read(cx).unread_count());
        let (runs_going, runs_failing) = self.automation_state.rail_runs;
        let open = self.app_view_label();
        PAGES
            .into_iter()
            .filter(|(label, _)| match *label {
                "Notes" => self.workspace.preferences.notes,
                "Automations" => self.workspace.sidebar_automations_visible,
                "Flows" => flows_visible,
                "Board" => self.workspace.preferences.board,
                "Git" => self.workspace.sidebar_inbox_visible,
                _ => true,
            })
            .map(|(label, glyph)| {
                let on = open == Some(label);
                let aria: SharedString = match label {
                    "My tasks" if waiting > 0 => format!("My tasks, {waiting} waiting").into(),
                    "Git" if unread > 0 => format!("Git, {unread} new").into(),
                    "Automations" => automations_aria(runs_going, runs_failing).into(),
                    _ => label.into(),
                };
                let ink = rail_tint(label).unwrap_or_else(|| content(if on { 1. } else { 0.74 }));
                let trailing: Option<AnyElement> = match label {
                    "My tasks" if waiting > 0 => Some(
                        div()
                            .debug_selector(|| "sidebar-navigation-count-My tasks".into())
                            .h(px(16.))
                            .min_w(px(16.))
                            .px(px(5.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(Hsla::from(rgb(WAIT)).opacity(0.22))
                            .text_size(px(11.))
                            .text_color(wait_ink())
                            .child(waiting.to_string())
                            .into_any_element(),
                    ),
                    "Git" if unread > 0 => Some(
                        div()
                            .debug_selector(|| "sidebar-navigation-unread-Git".into())
                            .size(px(7.))
                            .rounded_full()
                            .bg(rail_tint("Git").unwrap_or_else(accent_color))
                            .into_any_element(),
                    ),
                    "Automations" if runs_going > 0 || runs_failing > 0 => Some(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .when(runs_going > 0, |d| {
                                d.child(live_dot(Some(Live::Running)).debug_selector(|| {
                                    "sidebar-navigation-running-Automations".into()
                                }))
                            })
                            .when(runs_failing > 0, |d| {
                                d.child(
                                    div()
                                        .debug_selector(|| {
                                            "sidebar-navigation-failing-Automations".into()
                                        })
                                        .h(px(16.))
                                        .min_w(px(16.))
                                        .px(px(5.))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .bg(crate::glass::Tone::Bad.ink().opacity(0.22))
                                        .text_size(px(11.))
                                        .text_color(crate::glass::Tone::Bad.ink())
                                        .child(runs_failing.to_string()),
                                )
                            })
                            .into_any_element(),
                    ),
                    _ => None,
                };
                div()
                    .id(label)
                    .debug_selector(move || format!("sidebar-navigation-{label}"))
                    .role(Role::Button)
                    .aria_label(aria)
                    .aria_selected(on)
                    .cursor_pointer()
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .px(px(10.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .text_size(px(ROW_FONT))
                    .text_color(if on { content(1.) } else { row_ink() })
                    .hover(|s| s.bg(content(0.06)).text_color(content(1.)))
                    .when(on, selected)
                    .child(icon(glyph, 15.).flex_shrink_0().text_color(ink))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .children(trailing)
                    .when(label == "Git", |d| {
                        d.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, e: &gpui::MouseDownEvent, w, cx| {
                                this.open_menu(
                                    MenuTarget::InboxNav,
                                    Anchor::Point(e.position),
                                    w,
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| match label {
                        "Home" => this.open_home(window, cx),
                        "My tasks" => this.open_my_tasks(window, cx),
                        "Git" => this.open_inbox(window, cx),
                        "Notes" => this.open_notes(window, cx),
                        "Automations" => this.open_automations(window, cx),
                        "Flows" => this.open_flows(window, cx),
                        "Board" => this.open_board(window, cx),
                        _ => (),
                    }))
                    .into_any_element()
            })
            .collect()
    }

    /// A session as the rail lists it: dot, title, then its age (in Now)
    /// or its branch as a small label (in Sessions; its age while the branch
    /// is unknown). Hovering swaps either for Archive. `indent` is its left
    /// padding.
    pub(super) fn rail_session_row(
        &self,
        id: i64,
        live: Option<Live>,
        trailing: Trailing,
        indent: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = match trailing {
            Trailing::Age => None,
            Trailing::Branch => self.rail_branch_label(id, cx),
        };
        if let Some(row) = self.rename_row(&RenameTarget::Session(id), cx) {
            return row;
        }
        let session = self.session_metadata(id, cx);
        let active = self.selected_sessions.contains(&id)
            || (self.workspace.active == Some(id)
                && !self.pilot_active(cx)
                && !self.app_view_open());
        let anchor = self
            .session_row_bounds
            .get(&id)
            .cloned()
            .unwrap_or_default();
        self.sidebar_list.track_anchor(anchor.clone());
        let trailing = if self.session_hover == Some(id) {
            self.session_archive_button(id, &session, cx)
        } else if let Some(label) = label {
            // A labelled row shows no age.
            div()
                .debug_selector(move || format!("rail-session-branch-{id}"))
                .flex_shrink_0()
                .max_w(px(BRANCH_LABEL_WIDTH))
                .truncate()
                .text_size(px(11.))
                .text_color(faint_ink())
                .child(label)
                .into_any_element()
        } else {
            div()
                .debug_selector(move || format!("rail-session-age-{id}"))
                .w(px(TIME_WIDTH))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .text_size(px(11.))
                .text_color(faint_ink())
                .child(rail_plan::age(
                    session.summary.updated_at,
                    gespan_core::time::now_ms(),
                ))
                .into_any_element()
        };
        let title = session.display_title();
        div()
            .id(("sidebar-session", id as usize))
            .debug_selector(move || format!("sidebar-session-{id}"))
            .group("session-row")
            .role(Role::Button)
            .aria_label(title.clone())
            .aria_description(match live {
                Some(Live::NeedsYou) => "Needs you".to_owned(),
                Some(Live::Running) => "Running".to_owned(),
                None => format!(
                    "{} conversation",
                    Kind::from_id(&session.summary.provider).label()
                ),
            })
            .aria_selected(active)
            .cursor_pointer()
            .relative()
            .w_full()
            .h(px(ROW_HEIGHT))
            .pl(px(indent))
            .pr(px(10.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap(px(9.))
            .text_size(px(ROW_FONT))
            .text_color(row_ink())
            .hover(|s| s.bg(content(0.06)))
            .when(active, selected)
            .when(
                self.sidebar_drop == Some(session_groups::ListDrop::Session(id)),
                |d| d.bg(content(0.14)),
            )
            .when(self.deleting.contains(&id), |d| d.opacity(0.5))
            .child(
                // Hold all pauses a session; it shows the pause mark instead.
                if live.is_none()
                    && self.held_mark(id, self.sessions.get(&id).map(|v| v.read(cx)), cx)
                {
                    div()
                        .size(px(7.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(crate::session_mark::held(
                            if self.sessions.contains_key(&id) {
                                "session-held"
                            } else {
                                "session-held-unloaded"
                            },
                        ))
                        .into_any_element()
                } else {
                    live_dot(live)
                        .debug_selector(move || format!("rail-session-dot-{id}"))
                        .into_any_element()
                },
            )
            .child(
                div()
                    .debug_selector(move || format!("session-title-{id}"))
                    .flex_1()
                    .min_w_0()
                    .h(px(18.))
                    .line_height(px(18.))
                    .truncate()
                    .child(crate::particle_text::particle_text(title)),
            )
            .child(trailing)
            // At the row's corner, not after its padding, so the anchor
            // covers the whole row.
            .child(anchor_cover(anchor.probe()))
            .on_hover(cx.listener(move |this, hovered, window, cx| {
                this.hover_session(id, anchor.clone(), *hovered, true, window, cx);
            }))
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    if std::mem::take(&mut this.skip_session_click) {
                        return;
                    }
                    this.select_sidebar_session(id, event.modifiers(), window, cx)
                }),
            )
            // A row dragged onto a pane edge opens there, or onto a folder
            // files it.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, _| {
                    this.skip_session_click = false;
                    this.tab_drag = None;
                    this.pane_drag = Some(PaneDrag {
                        source: DragSource::Session(id),
                        start: event.position,
                        position: event.position,
                        active: false,
                        over: None,
                        insert: None,
                    });
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    let anchor = Anchor::Point(event.position);
                    this.context_selection = !this.selected_sessions.contains(&id);
                    if this.context_selection {
                        this.selected_sessions.clear();
                        this.selected_sessions.insert(id);
                    }
                    this.open_menu(MenuTarget::Session(id), anchor, window, cx);
                }),
            )
            .into_any()
    }

    /// The mark before a folder's name: a branch, a bolt, the Pilot's
    /// mascot, or the folder's own glyph and colour.
    fn folder_mark(&self, key: &str, kind: &FolderKind) -> AnyElement {
        let glyph = |name: &'static str, color: Option<Hsla>| {
            icon(name, 12.)
                .flex_shrink_0()
                .text_color(color.unwrap_or_else(muted_ink))
                .into_any_element()
        };
        match kind {
            FolderKind::Automation => glyph("zap", rail_tint("Automations")),
            FolderKind::Folder => glyph(
                "folder",
                key.strip_prefix("folder:")
                    .and_then(|id| self.project_groups().folder(id).cloned())
                    .and_then(|folder| session_groups::folder_tint(&folder)),
            ),
            FolderKind::Reminders => glyph("clock", Some(rgb(0xf59e0b).into())),
            FolderKind::Pinned => glyph("pin", None),
        }
    }

    /// `.fold`: a folder's header, its chevron, mark, name and count, with
    /// a dot while anything inside needs you or runs.
    pub(super) fn rail_folder_row(&self, folder: &Folder, cx: &mut Context<Self>) -> AnyElement {
        let user_folder = folder.key.strip_prefix("folder:").map(str::to_owned);
        if let Some(row) = user_folder
            .as_ref()
            .and_then(|id| self.rename_row(&RenameTarget::Folder(id.clone()), cx))
        {
            return row;
        }
        if let Some(anchor) = user_folder
            .as_ref()
            .and_then(|id| self.folder_bounds.get(id))
        {
            self.sidebar_list.track_anchor(anchor.clone());
        }
        let probe = user_folder
            .as_ref()
            .and_then(|id| self.folder_bounds.get(id))
            .map(|anchor| anchor_cover(anchor.probe()));
        let drop_target = user_folder.as_ref().is_some_and(|id| {
            self.sidebar_drop == Some(session_groups::ListDrop::Folder(id.clone()))
                || self
                    .folder_drag
                    .as_ref()
                    .is_some_and(|drag| drag.active() && drag.over() == Some(id))
        });
        let key = folder.key.clone();
        let menu_key = key.clone();
        let toggle = folder.clone();
        let open = folder.open;
        let selector = format!("rail-folder-{key}");
        let menu = key.starts_with("folder:") || key.starts_with(session_groups::FIXED_PREFIX);
        div()
            .id(SharedString::from(format!("rail-folder-{key}")))
            .debug_selector(move || selector.clone())
            .role(Role::Button)
            .aria_label(format!(
                "{}, {} {}{}",
                folder.label,
                folder.count,
                if folder.count == 1 {
                    "session"
                } else {
                    "sessions"
                },
                match folder.live {
                    Some(Live::NeedsYou) => ", needs you",
                    Some(Live::Running) => ", running",
                    None => "",
                }
            ))
            .aria_expanded(open)
            .cursor_pointer()
            .relative()
            .w_full()
            .h(px(FOLD_HEIGHT))
            .mt(px(2.))
            .px(px(10.))
            .rounded(px(7.))
            .flex()
            .items_center()
            .gap(px(7.))
            .text_size(px(12.))
            .text_color(if open { content(1.) } else { muted_ink() })
            .hover(|s| s.bg(content(0.06)).text_color(content(1.)))
            .when(drop_target, |d| d.bg(content(0.14)))
            .child(
                icon(
                    if open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    11.,
                )
                .flex_shrink_0(),
            )
            .child(self.folder_mark(&key, &folder.kind))
            .child(
                div()
                    .debug_selector({
                        let key = key.clone();
                        move || format!("rail-folder-label-{key}")
                    })
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(folder.label.clone()),
            )
            .child(
                div()
                    .debug_selector({
                        let key = key.clone();
                        move || format!("rail-folder-count-{key}")
                    })
                    .ml_auto()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.))
                    .text_color(faint_ink())
                    .when_some(folder.live, |d, live| {
                        d.child(live_dot(Some(live)).debug_selector({
                            let key = key.clone();
                            move || format!("rail-folder-dot-{key}")
                        }))
                    })
                    .child(folder.count.to_string()),
            )
            .children(probe)
            .when(
                user_folder.as_ref().is_some_and(|id| {
                    self.folder_drag
                        .as_ref()
                        .is_some_and(|drag| drag.active() && drag.id() == id)
                }),
                |d| d.opacity(0.4),
            )
            // A user folder drags to a new place among the folders.
            .when_some(user_folder.clone(), |d, id| {
                d.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, _| {
                        this.skip_folder_click = false;
                        this.folder_drag =
                            Some(session_groups::FolderDrag::new(id.clone(), event.position));
                    }),
                )
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if std::mem::take(&mut this.skip_folder_click) {
                    return;
                }
                // F2 then renames the folder last clicked.
                this.sidebar_target = toggle
                    .key
                    .strip_prefix("folder:")
                    .map(|id| RenameTarget::Folder(id.into()));
                window.focus(&this.sidebar_focus, cx);
                this.toggle_rail_folder(&toggle, cx);
            }))
            .when(menu, |d| {
                d.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        let target = match menu_key.strip_prefix("folder:") {
                            Some(folder) => MenuTarget::Folder(folder.to_owned()),
                            None => MenuTarget::FixedFolder(menu_key.clone()),
                        };
                        this.open_menu(target, Anchor::Point(event.position), window, cx);
                    }),
                )
            })
            .into_any()
    }

    /// "N more" under Now, Sessions or an open folder: lists the rest.
    pub(super) fn rail_more_row(
        &self,
        key: &str,
        label: String,
        indent: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expand = key.to_owned();
        let selector = format!("rail-folder-more-{key}");
        div()
            .id(SharedString::from(format!("rail-folder-more-{key}")))
            .debug_selector(move || selector.clone())
            .role(Role::Button)
            .aria_label(format!("Show {label}"))
            .cursor_pointer()
            .w_full()
            .h(px(MORE_HEIGHT))
            .pl(px(indent))
            .pr(px(10.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .text_size(px(11.5))
            .text_color(faint_ink())
            .hover(|s| s.bg(content(0.06)).text_color(content(1.)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.expand_rail_list(&expand);
                this.notify_sidebar(cx);
            }))
            .into_any()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Something new in Git shows as a dot on its icon, never a count, and
    /// the icon's name says how many.
    #[gpui::test]
    fn git_marks_unread_pull_requests_with_a_dot(cx: &mut gpui::TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(
            visual
                .debug_bounds("sidebar-navigation-unread-Git")
                .is_none()
        );
        window
            .update(visual, |shell, window, cx| {
                let store = shell.store.clone();
                let inbox = cx.new(|cx| crate::inbox::Inbox::offline(store, cx));
                inbox.update(cx, |inbox, _| {
                    inbox.receive_unread(vec![gespan_core::inbox::Item {
                        provider: "github".into(),
                        kind: "pr".into(),
                        repo: "acme/web".into(),
                        number: 7,
                        state: "open".into(),
                        updated_at: "2026-11-26T10:00:00Z".into(),
                        ..Default::default()
                    }])
                });
                shell.adopt_inbox(inbox, window, cx);
                shell.invalidate_chrome(cx);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        let dot = visual
            .debug_bounds("sidebar-navigation-unread-Git")
            .expect("the dot shows");
        let icon = visual.debug_bounds("sidebar-navigation-Git").unwrap();
        assert!(icon.contains(&dot.origin), "the dot sits on the icon");
        assert!(
            visual
                .debug_bounds("sidebar-navigation-count-Git")
                .is_none()
        );
    }

    #[test]
    fn switching_moves_a_project_to_the_front_of_recents_once() {
        let mut recent = vec!["/a".to_owned(), "/b".to_owned()];
        note_recent(&mut recent, "/b");
        assert_eq!(recent, ["/b", "/a"]);
        note_recent(&mut recent, "/c");
        assert_eq!(recent, ["/c", "/b", "/a"]);
        // Bounded, however many projects come and go.
        for ix in 0..100 {
            note_recent(&mut recent, &format!("/p{ix}"));
        }
        assert_eq!(recent.len(), 32);
        assert_eq!(recent[0], "/p99");
    }

    #[test]
    fn pinning_twice_unpins_and_keeps_pin_order() {
        let mut pinned = Vec::new();
        toggle_pin(&mut pinned, "/a");
        toggle_pin(&mut pinned, "/b");
        assert_eq!(pinned, ["/a", "/b"]);
        toggle_pin(&mut pinned, "/a");
        assert_eq!(pinned, ["/b"]);
    }

    #[test]
    fn the_board_tints_pages_in_the_dark_theme() {
        // The rail's own colours from the board, not the old list's.
        crate::theme::configure(
            &crate::settings::Preferences {
                theme: "dark".into(),
                ..Default::default()
            },
            false,
        );
        assert_eq!(rail_tint("Git"), Some(rgb(0x2dd4bf).into()));
        assert_eq!(rail_tint("Automations"), Some(rgb(0xfbbf24).into()));
        assert_eq!(rail_tint("New thread"), Some(rgb(0x6ee7b7).into()));
        // Home and Search take the row's ink.
        assert_eq!(rail_tint("Home"), None);
        assert_eq!(rail_tint("Search"), None);
    }
}
