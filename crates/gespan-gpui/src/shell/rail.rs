//! Rail F1, the left nav scoped to one project: the project switcher, one
//! row of page icons, New thread, then (in the virtual list) Needs you,
//! Running, the project's folders and its Pilots. Measured from the design
//! board `ExpNavTopProject.dc.html`.
//!
//! What goes in which section is decided in [`super::rail_plan`]; this
//! module gathers the inputs and draws the rows.
// A panic while drawing takes the whole app down, so render code here may
// not index, slice, unwrap or expect.
#![deny(clippy::indexing_slicing, clippy::unwrap_used, clippy::expect_used)]
use super::rail_plan::{self, Folder, FolderInput, FolderKind, Live};
use super::session_groups::{PILOT_PREFIX, RenameTarget, toggle_group};
use super::*;
use gespan_core::files::path_key;
use gespan_core::session_groups::Entry;
use gpui::{BoxShadow, Hsla, Stateful};

/// The rail's side padding: rows sit 10px in from its edges.
pub(super) const RAIL_PAD: f32 = 10.;
/// `.rl-row`: sessions, New thread and Settings.
pub(super) const ROW_HEIGHT: f32 = 28.;
const ROW_FONT: f32 = 12.5;
/// `.fold`: a folder header.
const FOLD_HEIGHT: f32 = 26.;
/// `.kid`: a session under an open folder.
const KID_INDENT: f32 = 22.;
const MORE_HEIGHT: f32 = 24.;
/// The time column, right-aligned.
const TIME_WIDTH: f32 = 26.;
/// A live row's folder label, before it truncates.
const FOLDER_LABEL_MAX: f32 = 74.;
const SWITCHER_HEIGHT: f32 = 44.;
const PAGE_WIDTH: f32 = 42.;
const PAGE_HEIGHT: f32 = 32.;

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
        "Inbox" => 0xa5b4fc,
        "Runs" => 0x5eead4,
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

/// `.count`, and `.count.hot` for what waits on you.
pub(super) fn count_pill(n: usize, hot: bool) -> gpui::Div {
    let (ink, fill) = if hot {
        (wait_ink(), Hsla::from(rgb(WAIT)).opacity(0.22))
    } else {
        (content(1.), content(0.10))
    };
    div()
        .flex_shrink_0()
        .h(px(16.))
        .min_w(px(18.))
        .px(px(5.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(fill)
        .text_size(px(11.))
        .font_weight(FontWeight::NORMAL)
        .text_color(ink)
        .child(n.to_string())
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

/// The pages in the icon row, in order, with their glyphs.
const PAGES: [(&str, &str); 9] = [
    ("Home", "home"),
    ("My tasks", "circle-alert"),
    ("Inbox", "inbox"),
    ("Runs", "clock"),
    ("Automations", "zap"),
    ("Flows", "git-merge"),
    ("Board", "kanban"),
    ("Notes", "note"),
    ("Search", "search"),
];

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

    /// The rail's folders for this project, by the Settings choice: the
    /// worktrees sessions run in, or the user's own folders.
    pub(super) fn rail_folders(
        &self,
        entries: &[Entry],
        ids: &[i64],
        cx: &App,
    ) -> Vec<FolderInput> {
        let recent_first = |sessions: &[i64]| {
            let mut sessions = sessions.to_vec();
            sessions.sort_by_key(|id| {
                std::cmp::Reverse((self.session_metadata(*id, cx).summary.updated_at, *id))
            });
            sessions
        };
        let stored = |key: &str| self.workspace.rail_folds.get(key).copied();
        if !self.workspace.preferences.groups_by_worktree() {
            let groups = self.project_groups();
            let mut inputs = Vec::new();
            let mut loose = Vec::new();
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
                            recent_first(sessions),
                            !folder.collapsed,
                        )
                    }
                    Entry::Fixed { id, name, sessions } => (
                        id.clone(),
                        name.clone(),
                        match id.strip_prefix(PILOT_PREFIX) {
                            Some(pilot) => FolderKind::Pilot(pilot.to_owned()),
                            None => FolderKind::Automation,
                        },
                        recent_first(sessions),
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
                    stored_open: Some(open),
                    open_by_default: true,
                });
            }
            let key = format!("loose:{}", path_key(&self.workspace.project));
            inputs.push(FolderInput {
                stored_open: stored(&key),
                key,
                label: "Sessions".into(),
                kind: FolderKind::Loose,
                sessions: recent_first(&loose),
                open_by_default: true,
            });
            return inputs;
        }
        // By worktree: a worktree whose sessions a Pilot launched shows the
        // Pilot; one holding only automation runs, a bolt.
        let (_, fixed) = self.sidebar_groups(cx);
        let mut pilot_of: HashMap<i64, String> = HashMap::new();
        for folder in &fixed {
            if let Some(pilot) = folder.id.strip_prefix(PILOT_PREFIX) {
                for id in &folder.session_ids {
                    pilot_of.entry(*id).or_insert_with(|| pilot.to_owned());
                }
            }
        }
        let automation = |id: &i64| self.automation_state.session_automations.contains_key(id);
        let listed: Vec<_> = ids
            .iter()
            .map(|id| super::sidebar_list::Listed {
                id: *id,
                created_at: self.session_metadata(*id, cx).summary.created_at,
                path: self.session_working_copy(*id, cx).to_owned(),
            })
            .collect();
        let here = path_key(self.working_copy());
        let mut inputs: Vec<FolderInput> = super::sidebar_list::worktree_groups(&listed)
            .into_iter()
            .map(|(path, sessions)| {
                let key = format!("wt:{}", path_key(&path));
                let kind = match sessions.iter().find_map(|id| pilot_of.get(id)) {
                    Some(pilot) => FolderKind::Pilot(pilot.clone()),
                    None if sessions.iter().all(automation) => FolderKind::Automation,
                    None => FolderKind::Branch,
                };
                FolderInput {
                    stored_open: stored(&key),
                    key,
                    label: self.worktree_label(&path),
                    kind,
                    sessions: recent_first(&sessions),
                    open_by_default: path_key(&path) == here,
                }
            })
            .collect();
        // The folder the project works in leads, as on the board.
        if let Some(ix) = inputs.iter().position(|f| f.open_by_default) {
            let first = inputs.remove(ix);
            inputs.insert(0, first);
        }
        inputs
    }

    /// Opens or closes a rail folder, remembering the choice. The user's
    /// folders keep their own stored state, as before.
    pub(super) fn toggle_rail_folder(&mut self, folder: &Folder, cx: &mut Context<Self>) {
        let key = folder.key.clone();
        if key.starts_with("wt:") || key.starts_with("loose:") {
            self.workspace.rail_folds.insert(key, !folder.open);
            self.persist(cx);
            self.notify_sidebar(cx);
        } else {
            self.update_groups(cx, |groups| toggle_group(groups, &key));
        }
    }

    /// The pinned top of the rail: switcher, page icons and New thread.
    pub(super) fn rail_top(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_shrink_0()
            .w_full()
            .px(px(RAIL_PAD))
            .pt(px(4.))
            .flex()
            .flex_col()
            .gap_px()
            .child(self.rail_switcher(cx))
            .child(self.rail_pages(cx))
            .child(self.sidebar_new_thread(cx))
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
            .child(kbd(format!("{MOD}O")).debug_selector(|| "sidebar-project-shortcut".into()))
            .child(self.project_anchor.probe())
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

    /// ⌘O and the switcher: opens the project picker, or closes it.
    pub(super) fn toggle_project_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu_open(&MenuTarget::Header) {
            self.close_menu(cx);
        } else {
            let anchor = Anchor::Slot(self.project_anchor.clone());
            self.open_menu(MenuTarget::Header, anchor, window, cx);
        }
    }

    /// The pages as one row of icons. Flows, Board and Notes join it when
    /// Settings turns them on, and the icons share the width.
    fn rail_pages(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let flows_visible = crate::flows::visible(cx);
        let waiting = self.my_task_count(cx);
        let unread = self
            .inbox
            .as_ref()
            .map_or(0, |inbox| inbox.read(cx).unread_count());
        let searching = self.app_search.is_some();
        div()
            .debug_selector(|| "sidebar-pages".into())
            .w_full()
            .pt(px(8.))
            .pb(px(4.))
            .flex()
            .justify_between()
            .children(
                PAGES
                    .into_iter()
                    .filter(|(label, _)| match *label {
                        "Notes" => self.workspace.preferences.notes,
                        "Automations" => self.workspace.sidebar_automations_visible,
                        "Flows" => flows_visible,
                        "Board" => self.workspace.preferences.board,
                        "Inbox" => self.workspace.sidebar_inbox_visible,
                        _ => true,
                    })
                    .map(|(label, glyph)| {
                        let on = if label == "Search" {
                            searching
                        } else {
                            self.app_view_label() == Some(label)
                        };
                        let badge = match label {
                            "My tasks" if waiting > 0 => Some((waiting, true)),
                            "Inbox" if unread > 0 => Some((unread, false)),
                            _ => None,
                        };
                        let aria: SharedString = match (label, badge) {
                            ("My tasks", Some((n, _))) => format!("My tasks, {n} waiting").into(),
                            ("Inbox", Some((n, _))) => format!("Inbox, {n} new").into(),
                            ("Search", _) => "Search sessions".into(),
                            _ => label.into(),
                        };
                        let ink =
                            rail_tint(label).unwrap_or_else(|| content(if on { 1. } else { 0.74 }));
                        div()
                            .id(label)
                            .debug_selector(move || format!("sidebar-navigation-{label}"))
                            .role(Role::Button)
                            .aria_label(aria)
                            .aria_selected(on)
                            .tooltip(move |_, cx| {
                                cx.new(|_| crate::conversation::HintTooltip(label)).into()
                            })
                            .cursor_pointer()
                            .relative()
                            .w(px(PAGE_WIDTH))
                            .min_w_0()
                            .flex_shrink(1.)
                            .h(px(PAGE_HEIGHT))
                            .rounded(px(8.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(on, |d| d.bg(content(0.11)))
                            .hover(|s| s.bg(content(0.06)))
                            .child(icon(glyph, 16.).text_color(ink))
                            .when_some(badge, |d, (n, hot)| {
                                d.child(
                                    div()
                                        .debug_selector(move || {
                                            format!("sidebar-navigation-count-{label}")
                                        })
                                        .absolute()
                                        .top(px(2.))
                                        .right(px(3.))
                                        .h(px(14.))
                                        .min_w(px(14.))
                                        .px(px(4.))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .bg(if hot {
                                            Hsla::from(rgb(WAIT))
                                        } else {
                                            content(0.18)
                                        })
                                        .text_size(px(9.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(if hot { black(0.93) } else { content(1.) })
                                        .child(n.to_string()),
                                )
                            })
                            .when(label == "Inbox", |d| {
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
                                "Inbox" => this.open_inbox(window, cx),
                                "Runs" => this.show_runs(window, cx),
                                "Notes" => this.open_notes(window, cx),
                                "Automations" => this.open_automations(window, cx),
                                "Flows" => this.open_flows(window, cx),
                                "Board" => this.open_board(window, cx),
                                "Search" => this.focus_search(window, cx),
                                _ => (),
                            }))
                    }),
            )
            .into_any()
    }

    /// A session as the rail lists it: dot, title, its folder when it is
    /// listed outside one, and its age. Hovering swaps the age for Archive.
    pub(super) fn rail_session_row(
        &self,
        id: i64,
        live: Option<Live>,
        folder: Option<SharedString>,
        kid: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
            .pl(px(if kid { KID_INDENT } else { 10. }))
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
            .when_some(folder, |d, folder| {
                d.child(
                    div()
                        .debug_selector(move || format!("rail-session-folder-{id}"))
                        .flex_shrink_0()
                        .max_w(px(FOLDER_LABEL_MAX))
                        .truncate()
                        .text_size(px(11.))
                        .text_color(faint_ink())
                        .child(folder),
                )
            })
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
    fn folder_mark(&self, key: &str, kind: &FolderKind, cx: &App) -> AnyElement {
        let glyph = |name: &'static str, color: Option<Hsla>| {
            icon(name, 12.)
                .flex_shrink_0()
                .text_color(color.unwrap_or_else(muted_ink))
                .into_any_element()
        };
        match kind {
            FolderKind::Branch => glyph("git-branch", None), // drawn as text
            FolderKind::Automation => glyph("zap", rail_tint("Automations")),
            FolderKind::Pilot(pilot) => self
                .pilots
                .as_ref()
                .and_then(|pilots| {
                    let pilots = pilots.read(cx);
                    let pilot = pilots.roster.get(pilot)?;
                    Some(crate::pilots::avatar::avatar(
                        &pilot.mascot,
                        pilot.color,
                        "idle",
                        12.,
                        true,
                    ))
                })
                .unwrap_or_else(|| glyph("bot", None)),
            FolderKind::Folder => glyph(
                "folder",
                key.strip_prefix("folder:")
                    .and_then(|id| self.project_groups().folder(id).cloned())
                    .and_then(|folder| session_groups::folder_tint(&folder)),
            ),
            FolderKind::Reminders => glyph("clock", Some(rgb(0xf59e0b).into())),
            FolderKind::Pinned => glyph("pin", None),
            FolderKind::Loose => glyph("chat", None),
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
            .px(px(8.))
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
            // A worktree is marked as on the board: "⑂ main", in its text.
            .when(folder.kind != FolderKind::Branch, |d| {
                d.child(self.folder_mark(&key, &folder.kind, cx))
            })
            .child(
                div()
                    .debug_selector({
                        let key = key.clone();
                        move || format!("rail-folder-label-{key}")
                    })
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(if folder.kind == FolderKind::Branch {
                        format!("⑂ {}", folder.label)
                    } else {
                        folder.label.clone()
                    }),
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

    /// "N more…" under an open folder: lists the rest of it.
    pub(super) fn rail_more_row(
        &self,
        key: &str,
        more: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expand = key.to_owned();
        let selector = format!("rail-folder-more-{key}");
        div()
            .id(SharedString::from(format!("rail-folder-more-{key}")))
            .debug_selector(move || selector.clone())
            .role(Role::Button)
            .aria_label(format!("Show {more} more"))
            .cursor_pointer()
            .w_full()
            .h(px(MORE_HEIGHT))
            .pl(px(KID_INDENT))
            .pr(px(10.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .text_size(px(11.5))
            .text_color(faint_ink())
            .hover(|s| s.bg(content(0.06)).text_color(content(1.)))
            .child(format!("{more} more…"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sidebar_list.expanded.insert(expand.clone());
                this.notify_sidebar(cx);
            }))
            .into_any()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(rail_tint("Inbox"), Some(rgb(0xa5b4fc).into()));
        assert_eq!(rail_tint("Automations"), Some(rgb(0xfbbf24).into()));
        assert_eq!(rail_tint("New thread"), Some(rgb(0x6ee7b7).into()));
        // Home and Search take the row's ink.
        assert_eq!(rail_tint("Home"), None);
        assert_eq!(rail_tint("Search"), None);
    }
}
