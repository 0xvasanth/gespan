//! Only construct and lay out sidebar cards near the viewport.
use super::rail::{RAIL_PAD, Trailing, section, section_more};
use super::rail_plan::{self, Live};
use super::*;
use crate::machines::sidebar as machine_rows;
use gespan_core::session_groups::Entry;
use gpui::{Div, Stateful};
use gpui::{ListAlignment, ListState, list};
use std::cell::{Cell, RefCell};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Row {
    /// The Pilots section, drawn by the Pilots view.
    Pilots,
    /// "Now": the live sessions follow.
    NowHeading,
    /// A live session in Now, with its age.
    Now {
        id: i64,
        live: Live,
    },
    /// "Sessions", then the user's folders and the flat list.
    SessionsHeading,
    /// One of the user's own folders, when Settings groups by folders.
    Folder(rail_plan::Folder),
    /// An idle session under an open user folder.
    FolderSession(i64),
    /// An idle session in the flat Sessions list, with its branch.
    Session(i64),
    /// A session another one started, right under the row of the session
    /// that started it, `depth` levels in.
    Child {
        id: i64,
        depth: u8,
    },
    /// "N more" under Now, Sessions or an open folder, by its key.
    More {
        key: String,
        more: usize,
    },
    Empty,
    /// A herdr machine, by id.
    Machine(String),
    MachineAgent {
        machine: String,
        terminal_id: String,
    },
}

pub(super) struct SidebarList {
    pub(super) state: ListState,
    rows: Vec<Row>,
    scope: String,
    structure_dirty: Cell<bool>,
    indexed_minute: u64,
    #[cfg(test)]
    structural_rebuilds: usize,
    /// Only anchors constructed for the last viewport, including nested rails.
    visible_anchors: RefCell<Vec<AnchorSlot>>,
    visible_rows: RefCell<Vec<usize>>,
    /// Lists whose "N more" was clicked, as `project\0key`, until the app
    /// quits.
    pub(super) expanded: HashSet<String>,
    /// The switcher's counts: folders, then sessions.
    pub(super) summary: (usize, usize),
    /// The live sessions the rows were built for; a change rebuilds them.
    live: Vec<(i64, Live)>,
    /// Whether Now's "N more" holds only running sessions.
    now_more_running: bool,
}

impl SidebarList {
    pub(super) fn new(cx: &mut Context<Shell>) -> Self {
        let state =
            ListState::new(0, ListAlignment::Top, px(96.)).with_uniform_item_height(px(33.));
        let owner = cx.weak_entity();
        state.set_scroll_handler(move |_, _, cx| {
            let _ = owner.update(cx, |shell, cx| {
                shell.sidebar_scrolled(cx);
                // Historical rows are retained as cheap IDs, but hit targets
                // only belong to the viewport that was actually laid out.
                shell.sidebar_list.clear_bounds();
                shell.sidebar_frame.update(cx, |_, cx| cx.notify());
            });
        });
        Self {
            state,
            rows: Vec::new(),
            scope: String::new(),
            structure_dirty: Cell::new(true),
            indexed_minute: 0,
            #[cfg(test)]
            structural_rebuilds: 0,
            visible_anchors: RefCell::new(Vec::new()),
            visible_rows: RefCell::new(Vec::new()),
            expanded: HashSet::new(),
            summary: (0, 0),
            live: Vec::new(),
            now_more_running: false,
        }
    }

    pub(super) fn invalidate_structure(&self) {
        self.structure_dirty.set(true);
    }

    pub(super) fn remeasure(&self) {
        self.state.remeasure_items(0..self.rows.len());
        self.clear_bounds();
    }

    pub(super) fn track_anchor(&self, anchor: AnchorSlot) {
        self.visible_anchors.borrow_mut().push(anchor);
    }

    fn clear_bounds(&self) {
        for anchor in self.visible_anchors.borrow_mut().drain(..) {
            anchor.set(None);
        }
    }

    /// The rail's session rows in order, each with whether it is live:
    /// Now's first, then Sessions'.
    #[cfg(test)]
    pub(super) fn session_rows(&self) -> Vec<(i64, bool)> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                Row::Now { id, .. } => Some((*id, true)),
                Row::Session(id) => Some((*id, false)),
                _ => None,
            })
            .collect()
    }

    /// The started sessions shown under their parent, with their depth.
    #[cfg(test)]
    pub(super) fn child_rows(&self) -> Vec<(i64, u8)> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                Row::Child { id, depth, .. } => Some((*id, *depth)),
                _ => None,
            })
            .collect()
    }
    fn update(&mut self, scope: String, rows: Vec<Row>) {
        if self.scope != scope {
            self.state.reset_with_uniform_height(rows.len(), px(33.));
            self.scope = scope;
        } else if self.rows != rows {
            // Keep the scroll anchor when sessions are added, regrouped or
            // reordered. Only replace the changed run of list items.
            let prefix = self
                .rows
                .iter()
                .zip(&rows)
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = self.rows[prefix..]
                .iter()
                .rev()
                .zip(rows[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            self.state.splice(
                prefix..self.rows.len() - suffix,
                rows.len() - prefix - suffix,
            );
        }
        self.rows = rows;
        // The Pilots section changes height on its own.
        if let Some(ix) = self.rows.iter().position(|row| *row == Row::Pilots) {
            self.state.remeasure_items(ix..ix + 1);
        }
    }
}

/// The Machines section, when it is shown and has a machine to list.
/// Whether any rows were added.
fn push_machine_rows(rows: &mut Vec<Row>, cx: &App) -> bool {
    if !crate::machines::visible(cx) {
        return false;
    }
    let Some(machines) = crate::machines::get(cx) else {
        return false;
    };
    let mut listed = machines
        .read(cx)
        .views()
        .iter()
        .filter(|view| machine_rows::listed(view))
        .peekable();
    if listed.peek().is_none() {
        return false;
    }
    for view in listed {
        let machine = view.machine.id.clone();
        rows.push(Row::Machine(machine.clone()));
        rows.extend(
            machine_rows::agents(view)
                .into_iter()
                .map(|agent| Row::MachineAgent {
                    machine: machine.clone(),
                    terminal_id: agent.terminal_id.clone(),
                }),
        );
    }
    true
}

/// Height of the fade the sidebar list paints over its bottom edge, where it
/// meets the project spaces bar. Rows under it are blurred into the shell.
const SIDEBAR_BOTTOM_FADE_PX: f32 = 40.;
pub(super) const SIDEBAR_BOTTOM_FADE: Pixels = px(SIDEBAR_BOTTOM_FADE_PX);
/// Height of the fade over the top edge once the list has scrolled.
const SIDEBAR_TOP_FADE: Pixels = px(28.);
/// Space after the last row, so at the end of the list the last row clears
/// the bottom fade instead of staying under it. Mid-list, rows still pass
/// beneath the fade.
pub(super) const SIDEBAR_TRAILING_INSET: Pixels = px(SIDEBAR_BOTTOM_FADE_PX + 4.);

// Match sidebar-session-scroll: a fade below, and above scrolled Focused rows.
// Mask only the edge bands so the virtual list never needs a full-pane copy.
fn sidebar_edge_mask(
    state: ListState,
    #[cfg_attr(target_os = "macos", allow(unused_variables))] inset: bool,
    begin: bool,
) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let offset = state.logical_scroll_top();
            let bottom_height = SIDEBAR_BOTTOM_FADE.min(bounds.size.height);
            let top_height = if offset.item_ix > 0 || offset.offset_in_item > px(0.) {
                SIDEBAR_TOP_FADE.min((bounds.size.height - bottom_height).max(px(0.)))
            } else {
                px(0.)
            };
            for (at_top, height) in [(true, top_height), (false, bottom_height)] {
                if height <= px(0.) {
                    continue;
                }
                let band = gpui::Bounds::new(
                    gpui::point(
                        bounds.left(),
                        if at_top {
                            bounds.top()
                        } else {
                            bounds.bottom() - height
                        },
                    ),
                    gpui::size(bounds.size.width, height),
                );
                #[cfg(target_os = "macos")]
                window.paint_quad(gpui::fill(band, gpui::black()).border_color(
                    gpui_apple::metal_renderer::edge_fade_marker(
                        begin,
                        if at_top {
                            height.as_f32() * window.scale_factor()
                        } else {
                            0.
                        },
                        if at_top {
                            0.
                        } else {
                            height.as_f32() * window.scale_factor()
                        },
                    ),
                ));
                #[cfg(not(target_os = "macos"))]
                if !begin {
                    let background = if inset {
                        shell_fill().blend(inset_chrome_wash())
                    } else {
                        shell_fill()
                    };
                    // Without a mask the fade paints the background over the
                    // rows. Over Windows' glass that becomes an opaque band,
                    // so a translucent background keeps the list's hard edge.
                    if cfg!(target_os = "windows") && background.a < 1. {
                        continue;
                    }
                    for y in 0..height.as_f32().ceil() as usize {
                        let mut color = background;
                        color.a = (1. - (y as f32 + 0.5) / height.as_f32()).clamp(0., 1.);
                        let y = if at_top {
                            band.top() + px(y as f32)
                        } else {
                            band.bottom() - px(y as f32 + 1.)
                        };
                        window.paint_quad(gpui::fill(
                            gpui::Bounds::new(
                                gpui::point(band.left(), y),
                                gpui::size(band.size.width, px(1.)),
                            ),
                            color,
                        ));
                    }
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl Shell {
    /// Session membership/order changes independently of scroll position and
    /// hover state. Keep the expensive project index until a data notification
    /// invalidates it; individual cards still read live metadata each render.
    pub(super) fn refresh_sidebar_structure(&mut self, cx: &mut Context<Self>) {
        let scope = self.sidebar_scope();
        let minute = gespan_core::time::now_ms() / 60_000;
        // A session that starts or stops waiting or running changes section.
        let mut live: Vec<_> = self.rail_live(cx).into_iter().collect();
        live.sort_unstable();
        let live_changed = self.sidebar_list.live != live;
        if self.sidebar_list.structure_dirty.replace(false)
            || live_changed
            || self.sidebar_list.scope != scope
            || self.sidebar_list.indexed_minute != minute
        {
            self.sidebar_list.live = live;
            // Date/time expiry refreshes membership without resetting the
            // project scope or scroll anchor.
            self.sidebar_list.indexed_minute = minute;
            #[cfg(test)]
            {
                self.sidebar_list.structural_rebuilds += 1;
            }
            let ids = self.sidebar_session_ids(cx);
            let entries = self.session_entries(&ids, cx);
            let listed: HashSet<_> = ids.iter().copied().collect();
            self.session_row_bounds.retain(|id, _| listed.contains(id));
            for id in ids {
                self.session_row_bounds.entry(id).or_default();
            }
            let folders: HashSet<_> = entries
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Folder { id, .. } => Some(id.clone()),
                    _ => None,
                })
                .collect();
            self.folder_bounds.retain(|id, _| folders.contains(id));
            for id in folders {
                self.folder_bounds.entry(id).or_default();
            }
            self.prepare_sidebar_list(&entries, cx);
        }
        self.sync_sidebar_list_visibility();
        self.sidebar_list.clear_bounds();
    }

    fn sidebar_scope(&self) -> String {
        format!(
            "{}\0{}\0{:?}\0{}\0{}\0{}",
            self.workspace.project,
            self.working_copy(),
            self.workspace.session_filters,
            self.workspace.preferences.show_archived,
            self.workspace.boxy_layout(),
            self.workspace.preferences.sidebar_group_by,
        )
    }

    /// The "This project" card's list: Pilots, Now, Sessions (after the
    /// user's folders, when Settings groups by folders), then any machines.
    /// Every session shows once.
    pub(super) fn prepare_sidebar_list(&mut self, entries: &[Entry], cx: &App) {
        let live = self.rail_live(cx);
        // Started sessions sit under the session that started them, unless
        // they are live: live work is always in Now.
        let listable: HashSet<i64> = entries.iter().flat_map(entry_sessions).collect();
        let nested: HashSet<i64> = listable
            .iter()
            .copied()
            .filter(|id| {
                !live.contains_key(id)
                    && self
                        .session_tree
                        .parent(*id)
                        .is_some_and(|parent| listable.contains(&parent))
            })
            .collect();
        let entries = &without(entries, &nested);
        let (folders, loose) = self.rail_folders(entries, cx);
        let updated: HashMap<i64, u64> = entries
            .iter()
            .flat_map(entry_sessions)
            .map(|id| (id, self.session_metadata(id, cx).summary.updated_at))
            .collect();
        let plan = rail_plan::plan(
            folders,
            loose,
            rail_plan::Facts {
                live: Some(&live),
                updated: Some(&updated),
            },
            &self.expanded_here(),
        );
        let worktrees: HashSet<String> = if self.workspace.preferences.groups_by_worktree() {
            updated
                .keys()
                .map(|id| gespan_core::files::path_key(self.session_working_copy(*id, cx)))
                .collect()
        } else {
            plan.folders.iter().map(|f| f.key.clone()).collect()
        };
        self.sidebar_list.summary = (worktrees.len(), plan.total + nested.len());
        self.sidebar_list.now_more_running = plan.now_more_running;
        let mut body = Vec::new();
        if !plan.now.is_empty() {
            body.push(Row::NowHeading);
            body.extend(plan.now.iter().map(|(id, live)| Row::Now {
                id: *id,
                live: *live,
            }));
            if plan.now_more > 0 {
                body.push(Row::More {
                    key: rail_plan::NOW_KEY.into(),
                    more: plan.now_more,
                });
            }
        }
        // A session filter that hides everything keeps its button, on the
        // Sessions heading, reachable.
        let session_filtered = self
            .workspace
            .session_filters
            .active(self.workspace.preferences.show_archived);
        let idle = !plan.folders.is_empty() || !plan.sessions.is_empty();
        if idle || session_filtered {
            body.push(Row::SessionsHeading);
        }
        for folder in plan.folders {
            let (shown, more, key) = (folder.shown.clone(), folder.more, folder.key.clone());
            body.push(Row::Folder(folder));
            body.extend(shown.into_iter().map(Row::FolderSession));
            if more > 0 {
                body.push(Row::More { key, more });
            }
        }
        body.extend(plan.sessions.iter().copied().map(Row::Session));
        if plan.more > 0 {
            body.push(Row::More {
                key: rail_plan::SESSIONS_KEY.into(),
                more: plan.more,
            });
        }
        let mut rows = vec![Row::Pilots];
        rows.extend(nest_children(body, &self.session_tree, &nested));
        if plan.total == 0 {
            rows.push(Row::Empty);
        }
        let mut machines = Vec::new();
        if push_machine_rows(&mut machines, cx) {
            rows.extend(machines);
        }
        self.sidebar_list.update(self.sidebar_scope(), rows);
    }

    /// The lists whose "N more" was clicked in this project: a list opened
    /// in one project stays closed in the others.
    fn expanded_here(&self) -> HashSet<String> {
        let prefix = format!("{}\0", self.workspace.project);
        self.sidebar_list
            .expanded
            .iter()
            .filter_map(|key| key.strip_prefix(&prefix).map(str::to_owned))
            .collect()
    }

    /// Opens the list behind an "N more", in this project only.
    pub(super) fn expand_rail_list(&mut self, key: &str) {
        self.sidebar_list
            .expanded
            .insert(format!("{}\0{key}", self.workspace.project));
    }

    /// The Pilots that launched sessions in this project, so a Pilot that
    /// does not list the project still shows in its card.
    pub(super) fn pilots_with_sessions(&self, cx: &App) -> HashSet<String> {
        self.pilots
            .as_ref()
            .map(|pilots| {
                pilots
                    .read(cx)
                    .pilot_folders(&self.workspace.project)
                    .into_iter()
                    .filter(|(_, _, sessions)| !sessions.is_empty())
                    .map(|(id, ..)| id)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn sync_sidebar_list_visibility(&mut self) {
        // Rail rows have fixed heights; only the laid-out set is tracked.
        self.sidebar_list.visible_rows.borrow_mut().clear();
    }

    pub(super) fn virtual_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("sidebar-scroll")
            .debug_selector(|| "sidebar-sessions".into())
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(|this, _, _, cx| this.sidebar_scrolled(cx)))
            .child(sidebar_edge_mask(
                self.sidebar_list.state.clone(),
                !self.workspace.boxy_layout(),
                true,
            ))
            .child(
                list(
                    self.sidebar_list.state.clone(),
                    cx.processor(|this, ix, _, cx| this.sidebar_list_row(ix, cx)),
                )
                .size_full()
                .pb(SIDEBAR_TRAILING_INSET),
            )
            .child(sidebar_edge_mask(
                self.sidebar_list.state.clone(),
                !self.workspace.boxy_layout(),
                false,
            ))
            .into_any()
    }

    fn sidebar_list_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.sidebar_list.rows.get(ix).cloned() else {
            return div().into_any();
        };
        self.sidebar_list.visible_rows.borrow_mut().push(ix);
        let content = match row {
            Row::NowHeading => section("Now").into_any(),
            Row::Now { id, live } => {
                self.rail_session_row(id, Some(live), Trailing::Age, super::rail::ROW_INSET, cx)
            }
            Row::SessionsHeading => section("Sessions")
                .child(div().flex_1())
                .child(
                    // Filters stay one click away without taking room on
                    // the board's line: shown while one is on, or on hover.
                    // A slot the line's height, so the 24px button never
                    // makes the heading taller.
                    div()
                        .flex_shrink_0()
                        .w(px(16.))
                        .h(px(13.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(
                            !self
                                .workspace
                                .session_filters
                                .active(self.workspace.preferences.show_archived)
                                && !self.menu_open(&MenuTarget::SessionFilter),
                            |d| d.invisible().group_hover("rail-sessions", |s| s.visible()),
                        )
                        .child(self.sidebar_filter_button(cx)),
                )
                .child(
                    section_more("All →")
                        .id("rail-all-sessions")
                        .debug_selector(|| "rail-all-sessions".into())
                        .role(Role::Button)
                        .aria_label("Search all sessions")
                        .cursor_pointer()
                        .hover(|s| s.text_color(content(1.)))
                        .on_click(cx.listener(|this, _, window, cx| this.focus_search(window, cx))),
                )
                .group("rail-sessions")
                .into_any(),
            Row::Session(id) => {
                self.rail_session_row(id, None, Trailing::Branch, super::rail::ROW_INSET, cx)
            }
            Row::Folder(folder) => self.rail_folder_row(&folder, cx),
            Row::FolderSession(id) => {
                self.rail_session_row(id, None, Trailing::Branch, super::rail::KID_INDENT, cx)
            }
            Row::Child { id, depth } => self.rail_session_row(
                id,
                None,
                Trailing::Branch,
                super::rail::ROW_INSET + super::rail::CHILD_STEP * f32::from(depth),
                cx,
            ),
            Row::More { key, more } => {
                let (indent, label) = match key.as_str() {
                    rail_plan::NOW_KEY if self.sidebar_list.now_more_running => {
                        (super::rail::ROW_INSET, format!("{more} more running"))
                    }
                    rail_plan::NOW_KEY => (super::rail::ROW_INSET, format!("{more} more")),
                    rail_plan::SESSIONS_KEY => (super::rail::ROW_INSET, format!("{more} more")),
                    _ => (super::rail::KID_INDENT, format!("{more} more")),
                };
                self.rail_more_row(&key, label, indent, cx)
            }
            Row::Pilots => {
                // When each session last changed, for an idle Pilot's age:
                // the section cannot read the Shell while it draws.
                let updated: HashMap<i64, u64> = self
                    .session_summaries
                    .iter()
                    .map(|(id, s)| (*id, s.updated_at))
                    .collect();
                let also = self.pilots_with_sessions(cx);
                return self
                    .pilots
                    .clone()
                    .map(|pilots| {
                        pilots.update(cx, |pilots, cx| {
                            pilots.rail(Some(self.workspace.project.clone()), &updated, &also, cx)
                        })
                    })
                    .unwrap_or_else(|| div().into_any());
            }
            Row::Machine(id) => {
                let Some(view) = crate::machines::get(cx).and_then(|machines| {
                    let machines = machines.read(cx);
                    machines
                        .views()
                        .iter()
                        .find(|v| v.machine.id == id)
                        .cloned()
                }) else {
                    return div().into_any();
                };
                machine_rows::machine_row(&view).into_any_element()
            }
            Row::MachineAgent {
                machine,
                terminal_id,
            } => self.machine_agent_row(&machine, &terminal_id, cx),
            Row::Empty => div()
                .debug_selector(|| "sidebar-sessions-empty".into())
                .mt(px(12.))
                .px_3()
                .py(px(10.))
                .rounded_lg()
                .border_1()
                .border_dashed()
                .border_color(content(0.1))
                .flex()
                .items_center()
                .gap_2p5()
                .child(icon("chat", 14.).text_color(content(0.35)))
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(content(0.6))
                                .child("No sessions yet"),
                        )
                        .child(
                            div()
                                .debug_selector(|| "sidebar-sessions-empty-hint".into())
                                .text_size(px(11.))
                                .text_color(content(0.35))
                                .child(format!("Start one with {MOD}T")),
                        ),
                )
                .into_any(),
        };
        // The nav's 10px side padding, and its 1px gap between rows.
        // The project card pads its rows; the rail's 1px gap between them.
        div().pb_px().child(content).into_any()
    }
}

impl Shell {
    /// The machine list or a turn seen changed: marks follow, and the
    /// sidebar index is rebuilt only when machines or agents come or go.
    pub(super) fn machines_changed(&mut self, cx: &mut Context<Self>) {
        self.sync_machine_marks(cx);
        let mut rows = Vec::new();
        push_machine_rows(&mut rows, cx);
        let shown = self
            .sidebar_list
            .rows
            .iter()
            .filter(|row| matches!(row, Row::Machine(_) | Row::MachineAgent { .. }));
        if !shown.eq(rows.iter()) {
            self.sidebar_list.invalidate_structure();
        }
        self.sidebar_frame.update(cx, |_, cx| cx.notify());
    }

    /// One cached mark per listed agent with a known status; the rest are
    /// dropped.
    pub(super) fn sync_machine_marks(&mut self, cx: &mut Context<Self>) {
        let mut marks = HashMap::new();
        if crate::machines::visible(cx)
            && let Some(machines) = crate::machines::get(cx)
        {
            let machines = machines.read(cx);
            for view in machines.views().iter().filter(|v| machine_rows::listed(v)) {
                let id = &view.machine.id;
                for agent in &view.snapshot.agents {
                    if let Some(mark) = machines.mark(id, agent) {
                        let provider = agent.agent.clone().unwrap_or_default();
                        let key = (id.clone(), agent.terminal_id.clone());
                        marks.insert(key, (provider, machine_rows::state_for(mark)));
                    }
                }
            }
        }
        self.machine_marks.retain(|key, _| marks.contains_key(key));
        for (key, (provider, state)) in marks {
            let mark = self
                .machine_marks
                .entry(key)
                .or_insert_with(|| cx.new(|_| SessionMark::new(provider.clone())))
                .clone();
            mark.update(cx, |mark, cx| mark.set(&provider, state, cx));
        }
    }

    fn machine_agent_row(
        &mut self,
        machine_id: &str,
        terminal_id: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(machines) = crate::machines::get(cx) else {
            return div().into_any();
        };
        let machines = machines.read(cx);
        let Some((view, agent)) = machine_rows::find(machines.views(), machine_id, terminal_id)
        else {
            return div().into_any();
        };
        let (greyed, _) = machine_rows::machine_status(view);
        let mark = self
            .machine_marks
            .get(&(machine_id.to_owned(), terminal_id.to_owned()))
            .map(|mark| mark.clone().into_any_element());
        // Without a herdr there is no client to open.
        let on_click = (machines.herdr_found() == Some(true)).then(|| {
            let (machine, terminal) = (machine_id.to_owned(), terminal_id.to_owned());
            cx.listener(move |this, _, window, cx| {
                this.open_machine_agent(&machine, &terminal, window, cx)
            })
        });
        machine_rows::agent_row(machine_id, agent, greyed, mark, on_click).into_any_element()
    }

    /// Opens herdr's own client for the agent in a terminal tab, or returns
    /// to the one already open, and counts its finished turn as seen.
    fn open_machine_agent(
        &mut self,
        machine_id: &str,
        terminal_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(machines) = crate::machines::get(cx) else {
            return;
        };
        let m = machines.read(cx);
        let Some((view, agent)) = machine_rows::find(m.views(), machine_id, terminal_id) else {
            return;
        };
        let (title, target, pane, seq) = (
            machine_rows::attach_title(view, agent),
            view.machine.clone(),
            agent.pane_id.clone(),
            agent.completion_seq,
        );
        // A tab whose attach has exited shows nothing live: open a fresh
        // one, and close the dead one once it is up.
        let (open_tab, dead_tab) = match m
            .attach_tab(machine_id, terminal_id)
            .and_then(|tab| tab.upgrade())
        {
            Some(tab) if tab.read(cx).exited => (None, Some(tab)),
            tab => (tab, None),
        };
        if let Some(ix) = open_tab.and_then(|tab| {
            self.terminals
                .get(&self.workspace.project)?
                .iter()
                .position(|t| *t == tab)
        }) {
            self.show_terminal_tab(ix, window, cx);
            if let Some(seq) = seq {
                crate::machines::mark_seen(machine_id, terminal_id, seq, cx);
            }
            return;
        }
        let Some(attach) = machines.update(cx, |m, _| m.begin_attach(machine_id, terminal_id))
        else {
            return;
        };
        let (machine, terminal) = (machine_id.to_owned(), terminal_id.to_owned());
        let lookup = cx
            .background_executor()
            .spawn(async move { attach(&target, &pane) });
        cx.spawn_in(window, async move |this, cx| {
            let command = lookup.await;
            // Outside the window update, which fails once the window is
            // gone; the lookup must always be released.
            machines.update(cx, |m, cx| {
                m.end_attach(&machine, &terminal);
                if command.is_none() {
                    m.set_herdr_found(false, cx);
                }
            });
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(command) = command else {
                    return;
                };
                let tab = this.open_command_terminal(title.into(), command, window, cx);
                machines.update(cx, |m, _| {
                    m.set_attach_tab(&machine, &terminal, tab.downgrade())
                });
                let project = this.workspace.project.clone();
                if let Some(ix) = dead_tab.and_then(|dead| {
                    this.terminals
                        .get(&project)?
                        .iter()
                        .position(|t| *t == dead)
                }) {
                    this.remove_terminal(&project, ix, window, cx);
                }
                if let Some(seq) = seq {
                    crate::machines::mark_seen(&machine, &terminal, seq, cx);
                }
            });
        })
        .detach();
    }

    /// Brings the dock's tab `ix` into view, in the panel or the dock, and
    /// focuses it.
    fn show_terminal_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let in_panel = self.terminal_in_panel(cx);
        let was_open = self.dock_open(cx);
        let Some(dock) = self.workspace.docks.get_mut(&self.workspace.project) else {
            return;
        };
        dock.active = ix.min(dock.terminals.len().saturating_sub(1));
        let opening = !in_panel && !was_open;
        if opening {
            dock.open = true;
        }
        self.dock_tab_scroll.scroll_to_item(dock.active);
        if in_panel {
            self.show_panel_terminal(window, cx);
        } else {
            if opening {
                self.slide_dock(true, cx);
                self.fit_sidebar(cx);
            }
            self.focus_terminal(window, cx);
        }
        self.persist(cx);
        cx.notify();
    }
}

impl Shell {
    pub(super) fn change_sidebar_option(&mut self, action: &str, cx: &mut Context<Self>) {
        let value = match action {
            "sidebar-notes" => &mut self.workspace.preferences.notes,
            "sidebar-board" => &mut self.workspace.preferences.board,
            "sidebar-automations" => &mut self.workspace.sidebar_automations_visible,
            "sidebar-inbox" => &mut self.workspace.sidebar_inbox_visible,
            "sidebar-pilots" => &mut self.workspace.preferences.pilots,
            "compact-session-cards" => &mut self.workspace.compact_session_cards,
            _ => return,
        };
        *value = !*value;
        if matches!(action, "sidebar-notes" | "sidebar-board" | "sidebar-pilots") {
            self.apply_preferences(cx);
            self.sync_settings(cx);
        }
        self.close_preview(cx);
        self.sidebar_list.remeasure();
        self.persist(cx);
        cx.notify();
    }

    /// A `.rl-row` with a tinted icon, a label and a shortcut: New thread
    /// and Settings.
    fn rail_action_row(
        &self,
        id: &'static str,
        label: &'static str,
        glyph: &'static str,
        ink: gpui::Hsla,
        shortcut: String,
        selected: bool,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::Button)
            .aria_label(label)
            .aria_selected(selected)
            .cursor_pointer()
            .w_full()
            .h(px(super::rail::ROW_HEIGHT))
            .px(px(10.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap(px(9.))
            .text_size(px(12.5))
            .text_color(if selected || label == "New thread" {
                content(1.)
            } else {
                super::rail::row_ink()
            })
            .when(selected, |d| {
                d.bg(content(0.11)).shadow(vec![
                    gpui::BoxShadow::new(px(0.), px(1.), white(0.08)).inset(),
                ])
            })
            .hover(|s| s.bg(content(0.06)).text_color(content(1.)))
            .child(icon(glyph, 15.).text_color(ink))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child(
                super::rail::kbd(shortcut)
                    .debug_selector(move || format!("sidebar-navigation-trailing-{label}")),
            )
    }
    /// Settings, the sidebar's last row, below the agents footer.
    pub(super) fn sidebar_settings_row(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_shrink_0()
            .w_full()
            // On the cards' row gutter: their 1px edge and 4px padding.
            .px(px(RAIL_PAD + 1. + super::rail::CARD_PAD))
            .pt(px(4.))
            .pb(px(10.))
            .child(
                self.rail_action_row(
                    "sidebar-settings",
                    "Settings",
                    "settings",
                    super::rail::muted_ink(),
                    format!("{MOD},"),
                    false,
                )
                .debug_selector(|| "sidebar-navigation-Settings".into())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_settings(crate::settings::Section::General, window, cx)
                })),
            )
            .into_any()
    }
    /// New thread in this project, ⌘T. Lit while the open session is blank.
    pub(super) fn sidebar_new_thread(&mut self, cx: &mut Context<Self>) -> AnyElement {
        const LABEL: &str = "New thread";
        let selected = self.active_session_is_blank(cx);
        self.rail_action_row(
            "sidebar-new-thread",
            LABEL,
            "pencil-edit-02",
            super::rail::rail_tint(LABEL).unwrap_or_else(|| content(1.)),
            format!("{MOD}T"),
            selected,
        )
        .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx)))
        .into_any()
    }
    /// The workspace shows an open session that has no messages yet.
    fn active_session_is_blank(&self, cx: &App) -> bool {
        !self.app_view_open()
            && self.app_search.is_none()
            && !self.pilot_active(cx)
            && self.workspace.active.is_some_and(|id| {
                self.session_summaries.contains_key(&id)
                    && !self.session_metadata(id, cx).summary.has_messages
            })
    }
    pub(super) fn notify_navigation(&self, cx: &mut Context<Self>) {
        self.sidebar_frame.update(cx, |_, cx| cx.notify());
    }
    pub(super) fn sidebar_filter_button(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self
            .workspace
            .session_filters
            .active(self.workspace.preferences.show_archived)
            || self.menu_open(&MenuTarget::SessionFilter);
        icon_button("filter-sessions", "Filter sessions", "filter")
            .debug_selector(|| "sidebar-filter-button".into())
            .mr(px(-5.))
            .when(active, |d| d.bg(content(0.1)))
            .relative()
            .child(self.filter_anchor.probe())
            .on_click(cx.listener(|this, _, window, cx| {
                if this.menu_open(&MenuTarget::SessionFilter) {
                    this.close_menu(cx);
                } else {
                    this.open_menu(
                        MenuTarget::SessionFilter,
                        Anchor::Slot(this.filter_anchor.clone()),
                        window,
                        cx,
                    );
                }
            }))
    }
}

fn entry_sessions(entry: &Entry) -> Vec<i64> {
    match entry {
        Entry::Session(id) => vec![*id],
        Entry::Reminders { sessions, .. }
        | Entry::Folder { sessions, .. }
        | Entry::Fixed { sessions, .. }
        | Entry::Pinned { sessions } => sessions.clone(),
    }
}

/// `entries` with the `nested` sessions taken out, empty entries dropped.
fn without(entries: &[Entry], nested: &HashSet<i64>) -> Vec<Entry> {
    let keep = |sessions: &[i64]| -> Vec<i64> {
        sessions
            .iter()
            .copied()
            .filter(|id| !nested.contains(id))
            .collect()
    };
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Session(id) => (!nested.contains(id)).then(|| entry.clone()),
            Entry::Reminders {
                collapsed,
                sessions,
            } => Some(Entry::Reminders {
                collapsed: *collapsed,
                sessions: keep(sessions),
            }),
            Entry::Folder { id, sessions } => Some(Entry::Folder {
                id: id.clone(),
                sessions: keep(sessions),
            }),
            Entry::Fixed { id, name, sessions } => {
                let sessions = keep(sessions);
                (!sessions.is_empty()).then(|| Entry::Fixed {
                    id: id.clone(),
                    name: name.clone(),
                    sessions,
                })
            }
            Entry::Pinned { sessions } => Some(Entry::Pinned {
                sessions: keep(sessions),
            }),
        })
        .collect()
}

/// Puts each `nested` session (one whose parent the sidebar lists) right
/// under the row of the session that started it, as a [`Row::Child`].
/// Where no session above it has a row (a collapsed folder, or past "N
/// more"), it has no row either. Live sessions are never nested: they are
/// in Now.
fn nest_children(
    rows: Vec<Row>,
    tree: &gespan_core::orchestrator::tree::Tree,
    nested: &HashSet<i64>,
) -> Vec<Row> {
    fn under(
        parent: i64,
        depth: u8,
        tree: &gespan_core::orchestrator::tree::Tree,
        nested: &HashSet<i64>,
        out: &mut Vec<Row>,
    ) {
        // Deeper than the tree allows: a loop in a corrupt store.
        if usize::from(depth) > tree.links().count() {
            return;
        }
        for child in tree.children(parent) {
            if !nested.contains(&child) {
                continue;
            }
            out.push(Row::Child { id: child, depth });
            under(child, depth + 1, tree, nested, out);
        }
    }
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let parent = match &row {
            Row::Now { id, .. } | Row::Session(id) | Row::FolderSession(id) => Some(*id),
            _ => None,
        };
        out.push(row);
        if let Some(id) = parent {
            under(id, 1, tree, nested, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod nesting_tests {
    use super::{Entry, Live, Row, nest_children, without};
    use gespan_core::orchestrator::tree::{Link, Origin, Tree, Wake};
    use std::collections::HashSet;

    fn tree(links: &[(i64, i64, u8)]) -> Tree {
        Tree::new(links.iter().map(|&(child, parent, depth)| Link {
            child,
            parent,
            root: parent,
            depth,
            origin: Origin::Chat,
            wake: Wake::Settled,
        }))
    }

    fn child(id: i64, depth: u8) -> Row {
        Row::Child { id, depth }
    }

    fn set(ids: &[i64]) -> HashSet<i64> {
        ids.iter().copied().collect()
    }

    #[test]
    fn children_follow_their_parent_indented_by_depth() {
        // 2, 3 and 5 were taken out of the list: only 1 has a row.
        let rows = vec![Row::Session(1), Row::FolderSession(9)];
        let tree = tree(&[(2, 1, 1), (3, 2, 2), (5, 1, 1)]);
        let nested = nest_children(rows, &tree, &set(&[2, 3, 5]));
        assert_eq!(
            nested,
            [
                Row::Session(1),
                child(2, 1),
                child(3, 2),
                child(5, 1),
                Row::FolderSession(9),
            ]
        );
    }

    #[test]
    fn a_live_parent_in_now_keeps_its_started_sessions_under_it() {
        let rows = vec![Row::Now {
            id: 1,
            live: Live::Running,
        }];
        let nested = nest_children(rows, &tree(&[(2, 1, 1)]), &set(&[2]));
        assert_eq!(nested.last(), Some(&child(2, 1)));
    }

    #[test]
    fn a_child_whose_parent_has_no_row_has_none_either() {
        // The parent sits past "N more": its child is hidden with it.
        let rows = vec![
            Row::Session(7),
            Row::More {
                key: "sessions".into(),
                more: 3,
            },
        ];
        assert_eq!(
            nest_children(rows.clone(), &tree(&[(2, 1, 1)]), &set(&[2])),
            rows
        );
    }

    #[test]
    fn unrelated_rows_keep_their_order() {
        let rows = vec![Row::Empty, Row::Session(4), Row::Session(3)];
        assert_eq!(
            nest_children(rows.clone(), &Tree::default(), &set(&[])),
            rows
        );
    }

    #[test]
    fn a_cycle_in_the_links_never_hangs_the_sidebar() {
        // A corrupt store: 2 under 3 and 3 under 2, and 4 under itself.
        let looped = tree(&[(2, 3, 1), (3, 2, 1), (4, 4, 1), (5, 1, 1)]);
        let nested = nest_children(vec![Row::Session(1)], &looped, &set(&[2, 3, 4, 5]));
        assert_eq!(nested, [Row::Session(1), child(5, 1)]);
    }

    #[test]
    fn nested_sessions_leave_their_folders() {
        let entries = vec![
            Entry::Folder {
                id: "f".into(),
                sessions: vec![1, 2],
            },
            Entry::Session(3),
            Entry::Fixed {
                id: "wt".into(),
                name: "worktree".into(),
                sessions: vec![3],
            },
            Entry::Pinned { sessions: vec![2] },
        ];
        let kept = without(&entries, &set(&[2, 3]));
        assert_eq!(
            kept,
            [
                Entry::Folder {
                    id: "f".into(),
                    sessions: vec![1],
                },
                Entry::Pinned { sessions: vec![] },
            ]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use gpui::TestAppContext;

    /// The Everywhere card's rows that are showing, top to bottom: Search,
    /// then the pages.
    fn page_order(visual: &mut gpui::VisualTestContext) -> Vec<&'static str> {
        let mut found: Vec<_> = [
            "Search",
            "Home",
            "My tasks",
            "Pull requests",
            "Runs",
            "Automations",
            "Flows",
            "Board",
            "Notes",
        ]
        .into_iter()
        .filter_map(|label| {
            let selector: &'static str =
                Box::leak(format!("sidebar-navigation-{label}").into_boxed_str());
            visual.debug_bounds(selector).map(|bounds| (bounds, label))
        })
        .collect();
        found.sort_by(|a, b| a.0.top().partial_cmp(&b.0.top()).unwrap());
        // One column: the pages share the card's row edges.
        let pages: Vec<_> = found.iter().filter(|(_, l)| *l != "Search").collect();
        if let Some((first, _)) = pages.first() {
            assert!(
                pages
                    .iter()
                    .all(|(b, _)| (b.left(), b.right()) == (first.left(), first.right())),
                "{found:?}"
            );
        }
        found.into_iter().map(|(_, label)| label).collect()
    }

    #[gpui::test]
    fn everywhere_lists_search_then_labelled_pages_in_the_boards_order(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, _, cx| {
                // The board's four; Board and Notes are on by default.
                shell.workspace.preferences.board = false;
                shell.workspace.preferences.notes = false;
                shell.invalidate_chrome(cx);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        let flows = window
            .update(visual, |_, _, cx| crate::flows::visible(cx))
            .unwrap();
        // Runs is a pane of Automations, and the Inbox is now Pull requests.
        let mut expected = vec!["Search", "Home", "My tasks", "Pull requests", "Automations"];
        if flows {
            expected.push("Flows");
        }
        assert_eq!(page_order(visual), expected);
        // `.rl-row`s, 28px, inside the Everywhere card; Search a 30px field.
        let card = visual.debug_bounds("rail-everywhere").unwrap();
        let home = visual.debug_bounds("sidebar-navigation-Home").unwrap();
        let search = visual.debug_bounds("sidebar-navigation-Search").unwrap();
        assert_eq!(home.size.height, px(28.));
        assert_eq!(search.size.height, px(30.));
        assert!(card.contains(&home.center()) && card.contains(&search.center()));
        let label = visual.debug_bounds("rail-zone-Everywhere").unwrap();
        assert!(label.bottom() <= search.top());
        // No "All projects" list, and no icon row, is left.
        assert!(visual.debug_bounds("sidebar-all-projects").is_none());
        assert!(visual.debug_bounds("sidebar-pages").is_none());
    }

    #[gpui::test]
    fn extra_pages_join_the_card_and_hidden_ones_stay_out(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let order = page_order(visual);
        // Board and Notes follow the board's pages, Notes last.
        assert_eq!(order.first(), Some(&"Search"));
        assert_eq!(order.last(), Some(&"Notes"));
        let board = order.iter().position(|l| *l == "Board").unwrap();
        assert!(order[board + 1] == "Notes");
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.sidebar_inbox_visible = false;
                shell.workspace.sidebar_automations_visible = false;
                shell.workspace.preferences.notes = false;
                shell.invalidate_chrome(cx);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        let order = page_order(visual);
        for gone in ["Pull requests", "Automations", "Notes"] {
            assert!(!order.contains(&gone), "{gone} hidden: {order:?}");
        }
        // Settings stays the last row of the sidebar either way.
        let settings = visual.debug_bounds("sidebar-navigation-Settings").unwrap();
        let new = visual.debug_bounds("sidebar-new-thread").unwrap();
        assert!(new.bottom() <= settings.top());
    }

    #[gpui::test]
    fn my_tasks_wears_its_count_at_the_rows_end(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let waiting = window
            .update(visual, |shell, _, cx| shell.my_task_count(cx))
            .unwrap();
        let badge = visual.debug_bounds("sidebar-navigation-count-My tasks");
        assert_eq!(badge.is_some(), waiting > 0);
        if let Some(badge) = badge {
            let row = visual.debug_bounds("sidebar-navigation-My tasks").unwrap();
            // At the row's right end, 10px in, inside it.
            assert!(row.contains(&badge.center()));
            assert_eq!(row.right() - badge.right(), px(10.));
            assert_eq!(badge.size.height, px(16.));
        }
    }

    #[gpui::test]
    fn new_thread_sits_below_the_switcher_with_the_search_fields_spacing(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let switcher = visual.debug_bounds("sidebar-project-button").unwrap();
        let more = visual.debug_bounds("sidebar-project-actions").unwrap();
        let new = visual.debug_bounds("sidebar-new-thread").unwrap();
        let search = visual.debug_bounds("sidebar-navigation-Search").unwrap();
        let home = visual.debug_bounds("sidebar-navigation-Home").unwrap();
        // No overlap: New thread starts 5px under the switcher (4px of air
        // plus the rows' 1px gap), the same step as Search to Home.
        assert_eq!(new.top() - switcher.bottom(), px(5.));
        assert_eq!(new.top() - switcher.bottom(), home.top() - search.bottom());
        assert_eq!((switcher.size.height, new.size.height), (px(44.), px(28.)));
        assert!(more.bottom() <= switcher.bottom(), "\"…\" stays inside");
    }

    #[gpui::test]
    fn the_project_card_leads_with_the_switcher_then_new_thread(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let card = visual.debug_bounds("rail-project-card").unwrap();
        let switcher = visual.debug_bounds("sidebar-project-button").unwrap();
        let home = visual.debug_bounds("sidebar-navigation-Home").unwrap();
        let new = visual.debug_bounds("sidebar-new-thread").unwrap();
        // The board: the switcher heads the project card, on its row edges
        // (1px edge, 4px padding), 44px tall, under the Everywhere pages.
        assert_eq!(switcher.top() - card.top(), px(5.));
        assert_eq!(switcher.left() - card.left(), px(5.));
        assert_eq!(card.right() - switcher.right(), px(5.));
        assert_eq!(switcher.size.height, px(44.));
        assert!(home.bottom() <= switcher.top());
        assert!(switcher.bottom() <= new.top());
        // New thread is a 28px `.rl-row` the switcher's width.
        assert_eq!(new.size.height, px(28.));
        assert_eq!(
            (new.left(), new.right()),
            (switcher.left(), switcher.right())
        );
        // No gear or divider beside the switcher any more.
        assert!(visual.debug_bounds("sidebar-project-settings").is_none());
        assert!(visual.debug_bounds("sidebar-project-divider").is_none());
        let summary = visual.debug_bounds("sidebar-project-summary").unwrap();
        let name = visual.debug_bounds("sidebar-project-name").unwrap();
        assert!(name.bottom() <= summary.top());
        // The ⌘O hint gave its place to "…"; the shortcut is in the tooltip.
        assert!(visual.debug_bounds("sidebar-project-shortcut").is_none());
        let more = visual.debug_bounds("sidebar-project-actions").unwrap();
        assert_eq!(more.size, gpui::size(px(28.), px(28.)));
        assert_eq!(
            switcher.right() - more.right(),
            px(11.),
            "10px padding, 1px edge"
        );
        assert_eq!(
            more.top() - switcher.top(),
            switcher.bottom() - more.bottom()
        );
        assert!(
            summary.right() <= more.left(),
            "the name truncates before \"…\""
        );
    }

    /// The switcher's machine chip beside the name: "This Mac" for a project
    /// here, "Other machine" for one on a connected machine, so two projects
    /// with one name differ. The switcher keeps its two lines.
    #[gpui::test]
    fn the_switcher_names_the_machine_beside_the_project(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let place = |visual: &mut gpui::VisualTestContext| {
            window
                .update(visual, |shell, _, _| {
                    let (_, sessions) = shell.sidebar_list.summary;
                    let place = crate::pickers::project_place(&shell.workspace.project);
                    let line = rail_plan::switcher_line(&place.path, sessions);
                    (place, line)
                })
                .unwrap()
        };
        let check = |visual: &mut gpui::VisualTestContext| {
            let switcher = visual.debug_bounds("sidebar-project-button").unwrap();
            let name = visual.debug_bounds("sidebar-project-name").unwrap();
            let chip = visual
                .debug_bounds("sidebar-project-machine")
                .expect("the machine chip");
            let summary = visual.debug_bounds("sidebar-project-summary").unwrap();
            let more = visual.debug_bounds("sidebar-project-actions").unwrap();
            assert_eq!(switcher.size.height, px(44.), "still two lines");
            assert_eq!(chip.size.height, px(18.));
            assert_eq!(chip.left() - name.right(), px(6.), "beside the name");
            assert!(chip.right() <= more.left(), "clear of \"…\"");
            assert!(chip.bottom() <= summary.top(), "on the name's line");
            assert_eq!(
                chip.center().y,
                name.center().y,
                "centred on the name"
            );
            chip.size.width
        };

        let (here, line) = place(visual);
        assert!(!here.other_machine);
        assert_eq!(here.machine(), crate::pickers::THIS_MACHINE);
        assert!(line.starts_with(&here.path), "{line}");
        assert!(!line.contains(crate::pickers::THIS_MACHINE), "the chip says it");
        let this_mac = check(visual);

        let remote = gespan_core::remote::remote_path("box", "/home/dev/gespan");
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.projects.push(remote.clone());
                shell.workspace.project = remote.clone();
                shell.invalidate_chrome(cx);
                cx.notify();
            })
            .unwrap();
        visual.refresh().unwrap();
        visual.run_until_parked();
        let (there, line) = place(visual);
        assert!(there.other_machine);
        assert!(line.starts_with("~/gespan · "), "{line}");
        let other = check(visual);
        assert!(other > this_mac, "\"Other machine\" is the longer chip");
    }

    /// The picker's rows carry the same chip, with the folder on a second
    /// line: every row a chip, and only the remote one says "Other machine".
    #[gpui::test]
    fn the_picker_rows_name_each_projects_machine_and_folder(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        let remote = gespan_core::remote::remote_path("box", "/home/dev/gespan");
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.projects.push(remote.clone());
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        click(visual, "sidebar-project-button");
        let bounds = |visual: &mut gpui::VisualTestContext, what: &str, ix: usize| {
            visual.debug_bounds(Box::leak(
                format!("project-picker-{what}-{ix}").into_boxed_str(),
            ))
        };
        let mut widths = Vec::new();
        for ix in 0..3 {
            let row = bounds(visual, "project", ix).unwrap_or_else(|| panic!("row {ix}"));
            let place = bounds(visual, "place", ix).unwrap();
            let chip = bounds(visual, "machine", ix).expect("every row names its machine");
            assert_eq!(row.size.height, px(crate::pickers::PICKER_ROW_HEIGHT));
            assert!(row.top() < place.top() && place.bottom() <= row.bottom());
            assert!(chip.bottom() <= place.top(), "the chip is on the name's line");
            widths.push(chip.size.width);
        }
        assert!(bounds(visual, "project", 3).is_none(), "three projects");
        let widest = widths.iter().copied().fold(px(0.), |a, b| if b > a { b } else { a });
        assert_eq!(
            widths.iter().filter(|w| **w == widest).count(),
            1,
            "only the remote project says \"Other machine\": {widths:?}"
        );
    }

    fn click(visual: &mut gpui::VisualTestContext, selector: &'static str) {
        visual.refresh().unwrap();
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
    }

    fn open_target(
        window: gpui::WindowHandle<Shell>,
        cx: &mut gpui::VisualTestContext,
    ) -> Option<MenuTarget> {
        window
            .update(cx, |shell, _, _| {
                shell.menu.as_ref().map(|m| m.target.clone())
            })
            .unwrap()
    }

    #[gpui::test]
    fn the_switchers_more_button_opens_and_closes_the_project_menu(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let project = window
            .update(visual, |shell, _, _| shell.workspace.project.clone())
            .unwrap();
        click(visual, "sidebar-project-actions");
        assert!(
            open_target(window, visual)
                == Some(MenuTarget::Project {
                    path: project.clone()
                }),
            "\"…\" opens this project's menu, not the picker"
        );
        let items = window
            .update(visual, |shell, _, cx| {
                shell.menu.as_ref().unwrap().menu.read(cx).item_shape()
            })
            .unwrap();
        for item in [
            "Theme colors",
            "Reveal in Finder",
            "Import sessions…",
            "Archive",
            "Delete",
        ] {
            let item = if item == "Reveal in Finder" {
                super::REVEAL_LABEL
            } else {
                item
            };
            assert!(items.iter().any(|i| i == item), "{item} in {items:?}");
        }
        // It drops from the button, not from where the pointer was.
        let more = visual.debug_bounds("sidebar-project-actions").unwrap();
        let card = window
            .update(visual, |shell, _, cx| {
                shell
                    .menu
                    .as_ref()
                    .unwrap()
                    .menu
                    .read(cx)
                    .surface_region()
                    .get()
                    .unwrap()
            })
            .unwrap();
        assert!(card.top() >= more.top(), "{card:?} {more:?}");
        click(visual, "sidebar-project-actions");
        assert!(
            open_target(window, visual).is_none(),
            "a second click closes it"
        );
    }

    #[gpui::test]
    fn the_switcher_still_opens_the_picker_and_more_swaps_to_the_menu(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        click(visual, "sidebar-project-name");
        assert!(open_target(window, visual) == Some(MenuTarget::Header));
        click(visual, "sidebar-project-actions");
        assert!(
            matches!(
                open_target(window, visual),
                Some(MenuTarget::Project { .. })
            ),
            "\"…\" never toggles the picker it sits on"
        );
    }

    #[gpui::test]
    fn an_action_picked_from_the_more_menu_runs_on_this_project(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let project = window
            .update(visual, |shell, _, _| {
                shell.workspace.pinned_projects.clear();
                shell.workspace.project.clone()
            })
            .unwrap();
        click(visual, "sidebar-project-actions");
        click(visual, "menu-action-pin-project");
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.pinned_projects, [project]);
                assert!(shell.menu.is_none(), "picking an item closes the menu");
            })
            .unwrap();
    }

    #[gpui::test]
    fn new_thread_and_settings_share_the_rows_shape_and_shortcut_column(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let new = visual.debug_bounds("sidebar-new-thread").unwrap();
        let settings = visual.debug_bounds("sidebar-navigation-Settings").unwrap();
        assert_eq!(new.left(), settings.left());
        assert_eq!(new.size, settings.size);
        let a = visual
            .debug_bounds("sidebar-navigation-trailing-New thread")
            .unwrap();
        let b = visual
            .debug_bounds("sidebar-navigation-trailing-Settings")
            .unwrap();
        assert_eq!(a.right(), b.right(), "shortcuts end on one edge");
    }

    #[gpui::test]
    fn right_clicking_the_switcher_opens_the_projects_menu(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let switcher = visual.debug_bounds("sidebar-project-button").unwrap();
        visual.simulate_event(gpui::MouseDownEvent {
            position: switcher.center(),
            button: gpui::MouseButton::Right,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                let open = shell.menu.as_ref().expect("the project menu opened");
                assert!(
                    open.target
                        == MenuTarget::Project {
                            path: shell.workspace.project.clone(),
                        }
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn cmd_o_opens_the_switcher_and_cmd_p_still_finds_files(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::input::bind(cx);
            super::super::bind_keys(cx);
        });
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        visual.simulate_keystrokes(&format!("{modifier}-o"));
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(shell.menu_open(&MenuTarget::Header), "the switcher opened");
            })
            .unwrap();
        assert!(visual.debug_bounds("project-picker").is_some());
        // Pressed again, it closes.
        window
            .update(visual, |shell, window, cx| {
                shell.toggle_project_switcher(window, cx)
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(!shell.menu_open(&MenuTarget::Header))
            })
            .unwrap();
        // ⌘P keeps its own job.
        let keys = window
            .update(visual, |_, window, _| {
                window
                    .bindings_for_action(&super::super::GoToFile)
                    .into_iter()
                    .map(|b| b.keystrokes().first().unwrap().unparse())
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert!(keys.contains(&format!("{modifier}-p")), "{keys:?}");
    }

    /// The rows that can end the sidebar, by selector.
    fn bottom_rows(
        visual: &mut gpui::VisualTestContext,
    ) -> Vec<(&'static str, gpui::Bounds<Pixels>)> {
        [
            "sidebar-navigation-Notes",
            "sidebar-new-thread",
            "agents-footer",
            "sidebar-navigation-Settings",
        ]
        .into_iter()
        .filter_map(|selector| visual.debug_bounds(selector).map(|b| (selector, b)))
        .collect()
    }

    #[gpui::test]
    fn settings_is_the_last_row_below_the_agents_footer(cx: &mut TestAppContext) {
        let (window, a, _) = super::super::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.workspace.preferences.working_agents = true;
                shell.sessions[&a].update(cx, |session, _| session.busy = true);
                shell.invalidate_chrome(cx);
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let sidebar = visual.debug_bounds("sidebar-content").unwrap();
        let footer = visual.debug_bounds("agents-footer").unwrap();
        let settings = visual.debug_bounds("sidebar-navigation-Settings").unwrap();
        // Directly below the pill, with only a small gap, and lowest of all.
        assert!(footer.bottom() <= settings.top());
        assert!(settings.top() - footer.bottom() <= px(8.));
        for (selector, bounds) in bottom_rows(visual) {
            if selector != "sidebar-navigation-Settings" {
                assert!(
                    bounds.bottom() <= settings.top(),
                    "{selector} above Settings"
                );
            }
        }
        assert!(settings.bottom() <= sidebar.bottom());
        assert!(
            sidebar.bottom() - settings.bottom() <= px(12.),
            "pinned to the bottom"
        );
        // On the cards' row gutter, 5px inside the pill's edges as on the
        // board, and its hint in the shared trailing column.
        let new = visual.debug_bounds("sidebar-new-thread").unwrap();
        assert_eq!(
            (settings.left(), settings.right()),
            (new.left(), new.right())
        );
        assert_eq!(settings.left() - footer.left(), px(5.));
        assert!(
            visual
                .debug_bounds("sidebar-navigation-trailing-Settings")
                .is_some()
        );
        // A saved Activity-off preference from before no longer hides the
        // card: it is always shown, Settings still last.
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.preferences.working_agents = false;
                shell.invalidate_chrome(cx);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("agents-footer").is_some());
        let still = visual.debug_bounds("sidebar-navigation-Settings").unwrap();
        assert_eq!(still.bottom(), settings.bottom());
    }

    #[gpui::test]
    fn the_bottom_settings_row_opens_settings(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let settings = visual.debug_bounds("sidebar-navigation-Settings").unwrap();
        visual.simulate_click(settings.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                let view = shell.settings.clone().expect("Settings opened");
                assert_eq!(view.read(cx).section, crate::settings::Section::General);
            })
            .unwrap();
        // The settings nav replaces the sidebar, so the row is gone until
        // Settings closes, then back in its place.
        assert!(visual.debug_bounds("sidebar-navigation-Settings").is_none());
        window
            .update(visual, |shell, w, cx| shell.close_settings(w, cx))
            .unwrap();
        visual.run_until_parked();
        assert_eq!(
            visual.debug_bounds("sidebar-navigation-Settings").unwrap(),
            settings
        );
    }

    #[gpui::test]
    fn the_project_spaces_bar_is_gone_and_the_chip_still_switches(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.workspace.projects = ["/work/gespan", "/work/a", "/work/b", "/work/c"]
                    .map(str::to_owned)
                    .to_vec();
                shell.invalidate_chrome(cx);
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        // Several projects, and still no strip of project spaces.
        for ix in 0..4 {
            let selector = Box::leak(format!("project-space-{ix}").into_boxed_str());
            assert!(visual.debug_bounds(selector).is_none(), "{selector}");
        }
        // The chip under the divider is the way to another project.
        let chip = visual.debug_bounds("sidebar-project-button").unwrap();
        visual.simulate_click(chip.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(shell.menu_open(&MenuTarget::Header), "the picker opens");
            })
            .unwrap();
    }

    #[gpui::test]
    fn the_project_menu_and_shortcut_still_reorder_projects(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, window, cx| {
                shell.workspace.projects = ["/work/a", "/work/b", "/work/c", "/work/d"]
                    .map(str::to_owned)
                    .to_vec();
                let target = MenuTarget::Project {
                    path: "/work/c".into(),
                };
                shell.run_menu(target.clone(), "move:start", None, window, cx);
                assert_eq!(shell.workspace.projects[0], "/work/c");
                shell.run_menu(target, "move:end", None, window, cx);
                assert_eq!(shell.workspace.projects[3], "/work/c");
                shell.select_project("/work/c".into(), window, cx);
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| shell.move_current_project(-1, cx))
            .unwrap();
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.projects[2], "/work/c");
                let saved = locked(&shell.store).unwrap().ui_state().unwrap().unwrap();
                let restored = Workspace::restore(Some(&saved), &[], "/work/c");
                assert_eq!(restored.projects, shell.workspace.projects);
            })
            .unwrap();
    }

    #[gpui::test]
    fn settings_entry_opens_settings(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| assert!(shell.settings.is_none()))
            .unwrap();
        let settings = visual.debug_bounds("sidebar-navigation-Settings").unwrap();
        visual.simulate_click(settings.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| assert!(shell.settings.is_some()))
            .unwrap();
    }

    #[gpui::test]
    fn new_thread_button_starts_a_draft(cx: &mut TestAppContext) {
        let (window, _, active) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        // Give the active session a message so New thread must make another.
        window
            .update(visual, |shell, _, cx| {
                shell.sessions[&active]
                    .update(cx, |session, _| session.summary.has_messages = true);
            })
            .unwrap();
        let before = window
            .update(visual, |shell, _, _| shell.sessions.len())
            .unwrap();
        let button = visual.debug_bounds("sidebar-new-thread").unwrap();
        visual.simulate_click(button.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.sessions.len(), before + 1);
                assert_ne!(shell.workspace.active, Some(active));
            })
            .unwrap();
    }

    #[gpui::test]
    fn focused_search_opens_from_the_keyboard(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::input::bind(cx);
            super::super::bind_keys(cx);
        });
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("sidebar-search-button").is_none());
        window
            .update(visual, |shell, _, _| assert!(shell.app_search.is_none()))
            .unwrap();
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        visual.simulate_keystrokes(&format!("{modifier}-k"));
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| assert!(shell.app_search.is_some()))
            .unwrap();
    }

    #[gpui::test]
    fn the_project_button_opens_the_project_picker_with_no_worktree_rows(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let button = visual.debug_bounds("sidebar-project-button").unwrap();
        visual.simulate_click(button.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                let open = shell.menu.as_ref().expect("the project picker opened");
                assert!(open.target == MenuTarget::Header);
                // No Switch worktree, Switch project or editor rows: the
                // picker itself, with its search field focused.
                assert!(open.menu.read(cx).action_ids().is_empty());
                assert_eq!(open.menu.read(cx).item_shape(), Vec::<String>::new());
            })
            .unwrap();
        assert!(visual.debug_bounds("project-picker").is_some());
    }

    #[gpui::test]
    fn new_thread_sits_below_navigation_and_marks_blank_sessions(cx: &mut TestAppContext) {
        let (window, _, active) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let new = visual.debug_bounds("sidebar-new-thread").unwrap();
        let inbox = visual
            .debug_bounds("sidebar-navigation-Pull requests")
            .unwrap();
        assert!(new.top() >= inbox.bottom());
        let selected = |visual: &mut gpui::VisualTestContext| {
            window
                .update(visual, |shell, _, cx| shell.active_session_is_blank(cx))
                .unwrap()
        };
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(shell.workspace.active, Some(active));
                assert!(!shell.session_metadata(active, cx).summary.has_messages);
            })
            .unwrap();
        assert!(selected(visual));
        visual.simulate_click(inbox.center(), Default::default());
        visual.run_until_parked();
        assert!(!selected(visual), "Pull requests takes the highlight");
        window
            .update(visual, |shell, w, cx| {
                shell.close_app_view(w, cx);
                shell.sessions[&active]
                    .update(cx, |session, _| session.summary.has_messages = true);
            })
            .unwrap();
        visual.run_until_parked();
        assert!(!selected(visual), "a conversation is not a new session");
    }

    /// Flip a sidebar switch the way a person does: on the Appearance page.
    fn flip_in_settings(
        window: gpui::WindowHandle<Shell>,
        visual: &mut gpui::VisualTestContext,
        id: &str,
    ) {
        window
            .update(visual, |shell, w, cx| {
                shell.open_settings(crate::settings::Section::Appearance, w, cx)
            })
            .unwrap();
        visual.simulate_resize(gpui::size(px(1200.), px(2600.)));
        visual.run_until_parked();
        let toggle = visual
            .debug_bounds(Box::leak(format!("settings-toggle-{id}").into_boxed_str()))
            .unwrap_or_else(|| panic!("Settings has a {id} switch"));
        visual.simulate_click(toggle.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, w, cx| shell.close_settings(w, cx))
            .unwrap();
        visual.run_until_parked();
    }

    #[gpui::test]
    fn each_sidebar_row_turns_its_feature_off(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let field = |shell: &Shell, id: &str| match id {
            "sidebar-notes" => shell.workspace.preferences.notes,
            "sidebar-board" => shell.workspace.preferences.board,
            _ => shell.workspace.preferences.pilots,
        };
        for (id, selector) in [
            ("sidebar-notes", "sidebar-navigation-Notes"),
            ("sidebar-board", "sidebar-navigation-Board"),
            ("sidebar-pilots", "pilot-intro"),
        ] {
            assert!(visual.debug_bounds(selector).is_some(), "{selector}");
            flip_in_settings(window, visual, id);
            assert!(!window.update(visual, |s, _, _| field(s, id)).unwrap());
            assert!(visual.debug_bounds(selector).is_none(), "{selector}");
            flip_in_settings(window, visual, id);
            assert!(window.update(visual, |s, _, _| field(s, id)).unwrap());
            assert!(visual.debug_bounds(selector).is_some(), "{selector}");
        }
        // The running card has no switch: it is always shown.
        window
            .update(visual, |shell, w, cx| {
                shell.open_settings(crate::settings::Section::Appearance, w, cx)
            })
            .unwrap();
        visual.run_until_parked();
        assert!(
            visual
                .debug_bounds("settings-toggle-sidebar-activity")
                .is_none()
        );
        window
            .update(visual, |shell, w, cx| shell.close_settings(w, cx))
            .unwrap();
    }

    #[gpui::test]
    fn turning_board_off_with_its_tab_open_closes_it(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, w, cx| {
                shell.open_board(w, cx);
                assert_eq!(shell.active_app_view, Some(app_views::AppView::Board));
            })
            .unwrap();
        visual.run_until_parked();
        flip_in_settings(window, visual, "sidebar-board");
        window
            .update(visual, |shell, _, cx| {
                assert!(!shell.workspace.preferences.board);
                assert_eq!(shell.active_app_view, None);
                // The tab stays in the saved order but drops out of the strip.
                assert!(
                    !shell
                        .visible_tabs(cx)
                        .iter()
                        .any(|t| matches!(t, TitleTab::App(app_views::AppView::Board)))
                );
                let saved = shell.latest_state.lock().unwrap().clone();
                let restored = Workspace::restore(Some(&saved), &[], "/work/gespan");
                assert!(!restored.preferences.board);
                cx.notify();
            })
            .unwrap();
    }

    #[gpui::test]
    fn turning_notes_off_with_notes_open_closes_it(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        window
            .update(cx, |shell, w, cx| shell.open_notes(w, cx))
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        flip_in_settings(window, visual, "sidebar-notes");
        window
            .update(visual, |shell, _, _| {
                assert!(!shell.workspace.preferences.notes);
                assert_ne!(shell.active_app_view, Some(app_views::AppView::Notes));
            })
            .unwrap();
    }

    #[gpui::test]
    fn sidebar_group_lists_its_rows_in_order(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, w, cx| {
                shell.open_settings(crate::settings::Section::Appearance, w, cx)
            })
            .unwrap();
        visual.simulate_resize(gpui::size(px(1200.), px(2600.)));
        visual.run_until_parked();
        // Appearance lists the sections in the order they appear in the
        // sidebar.
        let ids = [
            "sidebar-inbox",
            "sidebar-automations",
            "sidebar-pilots",
            "sidebar-board",
            "sidebar-notes",
        ];
        // The sidebar sections are chips that wrap across rows, so each one
        // comes after the last in reading order: on a later row, or further
        // right on the same row.
        let origins: Vec<_> = ids
            .iter()
            .map(|id| {
                visual
                    .debug_bounds(Box::leak(format!("settings-toggle-{id}").into_boxed_str()))
                    .unwrap_or_else(|| panic!("{id} has a switch"))
                    .origin
            })
            .collect();
        assert!(
            origins
                .windows(2)
                .all(|w| w[0].y < w[1].y || (w[0].y == w[1].y && w[0].x < w[1].x)),
            "{origins:?}"
        );
        assert!(
            visual
                .debug_bounds("settings-toggle-sidebar-search")
                .is_none()
        );
        // The rail's grouping follows, as one choice; Compact view is gone.
        let chips = origins.last().unwrap();
        let group_by = visual
            .debug_bounds("settings-segmented-sidebar-group-by")
            .expect("Group sessions by has a control");
        assert!(group_by.origin.y > chips.y);
        assert!(
            visual
                .debug_bounds("settings-toggle-compact-session-cards")
                .is_none()
        );
        // The General page no longer repeats these features.
        window
            .update(visual, |shell, w, cx| {
                shell.open_settings(crate::settings::Section::General, w, cx)
            })
            .unwrap();
        visual.run_until_parked();
        for gone in ["notes", "board", "working-agents"] {
            assert!(
                visual
                    .debug_bounds(Box::leak(
                        format!("settings-toggle-{gone}").into_boxed_str()
                    ))
                    .is_none(),
                "{gone}"
            );
        }
        window
            .update(visual, |shell, w, cx| shell.close_settings(w, cx))
            .unwrap();
    }

    #[gpui::test]
    fn sidebar_switches_hide_entries_and_survive_restart(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let rows = [
            ("sidebar-automations", "sidebar-navigation-Automations"),
            ("sidebar-inbox", "sidebar-navigation-Pull requests"),
        ];
        for (id, navigation) in rows {
            assert!(visual.debug_bounds(navigation).is_some(), "{navigation}");
            flip_in_settings(window, visual, id);
            assert!(visual.debug_bounds(navigation).is_none(), "{navigation}");
        }
        flip_in_settings(window, visual, "sidebar-notes");
        flip_in_settings(window, visual, "sidebar-pilots");
        window
            .update(visual, |shell, w, cx| {
                assert!(!shell.workspace.sidebar_automations_visible);
                assert!(!shell.workspace.sidebar_inbox_visible);
                assert!(!cx.global::<crate::settings::Preferences>().pilots);
                let saved = shell.latest_state.lock().unwrap().clone();
                let restored = Workspace::restore(Some(&saved), &[], "/work/gespan");
                assert!(!restored.sidebar_automations_visible);
                assert!(!restored.sidebar_inbox_visible);
                assert!(!restored.preferences.notes);
                assert!(!restored.preferences.pilots);
                // The switches read the saved state back, not a stale copy.
                shell.open_settings(crate::settings::Section::Appearance, w, cx);
                let view = shell.settings.clone().unwrap();
                assert_eq!(
                    view.read(cx).sidebar_switches(),
                    [false, true, false, false, false, true]
                );
                shell.close_settings(w, cx);
            })
            .unwrap();
    }

    #[gpui::test]
    fn the_running_card_ignores_a_saved_activity_off_preference(cx: &mut TestAppContext) {
        let (window, a, b) = super::super::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                for id in [a, b] {
                    shell.sessions[&id].update(cx, |session, _| session.busy = true);
                }
                // Saved before the card was always shown.
                shell.workspace.preferences.working_agents = false;
                shell.invalidate_chrome(cx);
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        for boxy in [false, true] {
            window
                .update(visual, |shell, _, cx| {
                    shell.workspace.new_layout_boxy = boxy;
                    shell.layout_changed(cx);
                })
                .unwrap();
            visual.run_until_parked();
            assert!(
                visual.debug_bounds("agents-footer").is_some(),
                "boxy {boxy}"
            );
            window
                .update(visual, |shell, _, cx| {
                    for id in [a, b] {
                        assert!(shell.sessions[&id].read(cx).responding());
                    }
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn scroll_and_hover_redraws_reuse_the_sidebar_index(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.refresh_sidebar_structure(cx);
                let rebuilds = shell.sidebar_list.structural_rebuilds;
                for _ in 0..100 {
                    // This is the path used by scroll/hover redraws: visible
                    // cards refresh, while membership/order stays cached.
                    shell.refresh_sidebar_structure(cx);
                }
                assert_eq!(shell.sidebar_list.structural_rebuilds, rebuilds);
                shell.sidebar_list.invalidate_structure();
                shell.refresh_sidebar_structure(cx);
                assert_eq!(shell.sidebar_list.structural_rebuilds, rebuilds + 1);
                // Time expiry is a content refresh, not navigation: retain
                // the user's position instead of snapping back to the active row.
                shell.sidebar_list.state.scroll_to(gpui::ListOffset {
                    item_ix: 1,
                    offset_in_item: px(5.),
                });
                let offset = shell.sidebar_list.state.logical_scroll_top();
                shell.sidebar_list.indexed_minute = 0;
                shell.refresh_sidebar_structure(cx);
                let refreshed = shell.sidebar_list.state.logical_scroll_top();
                assert_eq!(refreshed.item_ix, offset.item_ix);
                assert_eq!(refreshed.offset_in_item, offset.offset_in_item);
                assert_eq!(shell.sidebar_list.structural_rebuilds, rebuilds + 2);
                // Scope changes are also detected without a notification hook.
                shell.workspace.session_filters.status.working = true;
                shell.refresh_sidebar_structure(cx);
                assert_eq!(shell.sidebar_list.structural_rebuilds, rebuilds + 3);
                assert!(shell.sidebar_list.rows.contains(&super::Row::Empty));
            })
            .unwrap();
    }

    /// Opens `project`'s folder all the way and drops the Pilots section,
    /// so the list ends with that folder's last session.
    /// Lists every session: Sessions' "N more" clicked, Pilots hidden.
    fn expand_everything(shell: &mut Shell, _project: &str, cx: &mut Context<Shell>) {
        shell.expand_rail_list(super::rail_plan::SESSIONS_KEY);
        shell.workspace.preferences.pilots = false;
        shell.apply_preferences(cx);
        shell.notify_sidebar(cx);
    }

    #[gpui::test]
    fn large_project_only_lays_out_nearby_session_cards(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let started = std::time::Instant::now();
        let (first, next, last) = window
            .update(cx, |shell, window, cx| {
                let mut summaries = Vec::new();
                {
                    let store = locked(&shell.store).unwrap();
                    for i in 0..1000 {
                        let mut summary = store.create("codex", "/work/many-sessions").unwrap();
                        summary.title =
                            format!("Session {i}: {}", "A long conversation title 🙂 ".repeat(4));
                        summary.has_messages = true;
                        // Session 999 is the newest; ids alone are random.
                        summary.updated_at += i;
                        summaries.push(summary);
                    }
                }
                let first = summaries.last().unwrap().id;
                // The newest but one: the next card down from `first`.
                let next = summaries[summaries.len() - 2].id;
                let last = summaries[0].id;
                for summary in summaries {
                    shell.order.push(summary.id);
                    shell.session_summaries.insert(summary.id, summary);
                }
                shell.workspace.open(first);
                shell.select_project("/work/many-sessions".into(), window, cx);
                // The rail lists two rows of a folder until "N more" opens
                // it all; a long list is only possible past that, and with
                // no Pilots section after it.
                expand_everything(shell, "/work/many-sessions", cx);
                (first, next, last)
            })
            .unwrap();
        cx.refresh().unwrap();
        window
            .update(cx, |shell, _, cx| {
                let laid_out = shell
                    .session_row_bounds
                    .values()
                    .filter(|slot| slot.get().is_some())
                    .count();
                eprintln!(
                    "1,000-session project: {laid_out} laid-out cards, {:?} setup + first frame",
                    started.elapsed()
                );
                assert!(laid_out > 0);
                assert!(
                    shell.sessions.len() < 10,
                    "scrolling the sidebar must not construct off-screen conversations"
                );
                assert!(
                    laid_out < 80,
                    "off-screen session cards should not be laid out: {laid_out}"
                );
                assert!(shell.session_row_bounds[&first].get().is_some());
                assert!(shell.session_row_bounds[&last].get().is_none());
                shell.sidebar_list.state.scroll_to_end();
                shell.sidebar_frame.update(cx, |_, cx| cx.notify());
            })
            .unwrap();
        cx.refresh().unwrap();
        window
            .update(cx, |shell, _, cx| {
                assert!(
                    shell.session_row_bounds[&first].get().is_none(),
                    "scrolled-away cards cannot remain hit targets"
                );
                assert!(
                    shell.session_row_bounds[&last].get().is_some(),
                    "the oldest conversation is reachable"
                );
                let members = shell.sidebar_session_ids(cx);
                shell.groups.insert(
                    "/work/many-sessions".into(),
                    gespan_core::session_groups::Groups {
                        folders: vec![gespan_core::session_groups::Folder {
                            id: "large-folder".into(),
                            name: "All history".into(),
                            session_ids: members,
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                );
                shell.notify_sidebar(cx);
            })
            .unwrap();
        cx.refresh().unwrap();
        window
            .update(cx, |shell, _, _| {
                let laid_out = shell
                    .session_row_bounds
                    .values()
                    .filter(|slot| slot.get().is_some())
                    .count();
                assert!(
                    laid_out > 0 && laid_out < 80,
                    "expanded folders must also be virtualized: {laid_out}"
                );
                shell.sidebar_list.state.scroll_to_end();
            })
            .unwrap();
        cx.refresh().unwrap();
        let offset = window
            .update(cx, |shell, window, cx| {
                // Resolve scroll_to_end's end sentinel before checking selection.
                let offset = shell.sidebar_list.state.logical_scroll_top();
                shell.open(next, window, cx);
                offset
            })
            .unwrap();
        cx.refresh().unwrap();
        window
            .update(cx, |shell, _, _| {
                // Selecting a session (e.g. from a title tab) leaves the
                // user's scroll position alone.
                let after = shell.sidebar_list.state.logical_scroll_top();
                assert_eq!(
                    (after.item_ix, after.offset_in_item),
                    (offset.item_ix, offset.offset_in_item)
                );
                assert!(shell.session_row_bounds[&next].get().is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn the_last_session_clears_the_bottom_fade_at_the_end_of_the_list(cx: &mut TestAppContext) {
        use super::{SIDEBAR_BOTTOM_FADE, SIDEBAR_TRAILING_INSET};
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        visual.refresh().unwrap();
        // A list that fits doesn't scroll: the inset adds no reachable space.
        window
            .update(visual, |shell, _, _| {
                shell.sidebar_list.state.scroll_to_end()
            })
            .unwrap();
        visual.refresh().unwrap();
        window
            .update(visual, |shell, _, _| {
                let top = shell.sidebar_list.state.logical_scroll_top();
                assert_eq!((top.item_ix, top.offset_in_item), (0, px(0.)));
            })
            .unwrap();

        let (newest, oldest) = window
            .update(visual, |shell, window, cx| {
                let mut summaries = Vec::new();
                {
                    let store = locked(&shell.store).unwrap();
                    for i in 0..60 {
                        let mut summary = store.create("codex", "/work/long-list").unwrap();
                        summary.title = format!("Session {i}");
                        summary.has_messages = true;
                        // The list sorts by (updated_at, id), newest first.
                        // These are made within one millisecond and ids are
                        // random, so space the times: session 0 is the oldest.
                        summary.updated_at += i;
                        summaries.push(summary);
                    }
                }
                let newest = summaries.last().unwrap().id;
                let oldest = summaries[0].id;
                for summary in summaries {
                    shell.order.push(summary.id);
                    shell.session_summaries.insert(summary.id, summary);
                }
                shell.workspace.open(newest);
                shell.select_project("/work/long-list".into(), window, cx);
                // The rail lists two rows of a folder until "N more" opens
                // it all; a long list is only possible past that, and with
                // no Pilots section after it.
                expand_everything(shell, "/work/long-list", cx);
                (newest, oldest)
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        let list = visual.debug_bounds("sidebar-sessions").unwrap();
        let fade_top = list.bottom() - SIDEBAR_BOTTOM_FADE;
        window
            .update(visual, |shell, _, cx| {
                // Mid-list, rows still run on under the fade.
                let under = shell
                    .session_row_bounds
                    .values()
                    .filter_map(|slot| slot.get())
                    .any(|row| row.bottom() > fade_top);
                assert!(under, "rows pass beneath the fade while scrolling");
                assert!(shell.session_row_bounds[&newest].get().is_some());
                shell.sidebar_list.state.scroll_to_end();
                shell.sidebar_frame.update(cx, |_, cx| cx.notify());
            })
            .unwrap();
        visual.refresh().unwrap();
        window
            .update(visual, |shell, _, _| {
                let last = shell.session_row_bounds[&oldest]
                    .get()
                    .expect("the oldest session is on screen at the end");
                assert!(
                    last.bottom() <= fade_top,
                    "the last row {last:?} must sit above the fade from {fade_top:?}"
                );
                // And no further: the gap under it is the inset, not more.
                assert!(
                    list.bottom() - last.bottom() <= SIDEBAR_TRAILING_INSET + px(1.),
                    "{last:?} in {list:?}"
                );
            })
            .unwrap();
    }

    mod machines {
        use super::super::super::*;
        use super::super::Row;
        use crate::machines::Machines;
        use crate::session_mark::State;
        use gespan_core::machines::{
            Agent, AgentStatus, Machine, MachineView, Online, Snapshot, Workspace,
        };
        use gpui::{TestAppContext, VisualTestContext, WindowHandle};
        use std::time::Duration;

        fn agent(terminal: &str, workspace: &str, status: AgentStatus, seq: Option<u64>) -> Agent {
            Agent {
                terminal_id: terminal.into(),
                pane_id: format!("{workspace}:p-{terminal}"),
                workspace_id: workspace.into(),
                tab_id: format!("{workspace}:t1"),
                name: Some(format!("{terminal} agent")),
                agent: Some("claude".into()),
                title: None,
                agent_status: status,
                completion_seq: seq,
                cwd: None,
            }
        }

        fn view(online: Online, agents: Vec<Agent>) -> MachineView {
            MachineView {
                machine: Machine {
                    id: "m1".into(),
                    label: "box".into(),
                    target: "me@box".into(),
                    session: "default".into(),
                    enabled: true,
                },
                snapshot: Snapshot {
                    online,
                    version: None,
                    workspaces: vec![
                        Workspace {
                            workspace_id: "w1".into(),
                            label: "one".into(),
                        },
                        Workspace {
                            workspace_id: "w2".into(),
                            label: "two".into(),
                        },
                    ],
                    agents,
                },
                version_mismatch: None,
                needs_install: false,
            }
        }

        /// One machine: `a` finished a turn not yet seen, `b` is working.
        /// herdr lists `b` first, but its workspace comes second.
        fn two_agents(online: Online) -> MachineView {
            view(
                online,
                vec![
                    agent("b", "w2", AgentStatus::Working, None),
                    agent("a", "w1", AgentStatus::Done, Some(3)),
                ],
            )
        }

        fn window(
            views: Vec<MachineView>,
            shown: bool,
            cx: &mut TestAppContext,
        ) -> (WindowHandle<Shell>, Entity<Machines>) {
            let machines = cx.update(|cx| {
                if shown {
                    crate::machines::show_for_test(cx);
                }
                crate::machines::install_for_test(views, cx)
            });
            let (window, _, _) = super::super::super::tests::window(cx);
            (window, machines)
        }

        fn rows(window: &WindowHandle<Shell>, cx: &mut TestAppContext) -> Vec<Row> {
            window
                .update(cx, |shell, _, cx| {
                    shell.refresh_sidebar_structure(cx);
                    shell.sidebar_list.rows.clone()
                })
                .unwrap()
        }

        fn machine_rows(rows: &[Row]) -> Vec<Row> {
            rows.iter()
                .filter(|row| matches!(row, Row::Machine(_) | Row::MachineAgent { .. }))
                .cloned()
                .collect()
        }

        fn agent_row(terminal: &str) -> Row {
            Row::MachineAgent {
                machine: "m1".into(),
                terminal_id: terminal.into(),
            }
        }

        fn mark(
            window: &WindowHandle<Shell>,
            terminal: &str,
            cx: &mut TestAppContext,
        ) -> Option<State> {
            window
                .update(cx, |shell, _, cx| {
                    shell
                        .machine_marks
                        .get(&("m1".to_owned(), terminal.to_owned()))
                        .map(|mark| mark.read(cx).state())
                })
                .unwrap()
        }

        fn click_agent(window: &WindowHandle<Shell>, terminal: &str, cx: &mut TestAppContext) {
            let visual = &mut VisualTestContext::from_window((*window).into(), cx);
            visual.run_until_parked();
            let selector = format!("machine-agent-m1-{terminal}");
            let bounds = visual
                .debug_bounds(Box::leak(selector.into_boxed_str()))
                .unwrap_or_else(|| panic!("agent {terminal} is not in the sidebar"));
            visual.simulate_click(bounds.center(), Default::default());
            visual.run_until_parked();
        }

        fn terminal_count(window: &WindowHandle<Shell>, cx: &mut TestAppContext) -> usize {
            window
                .update(cx, |shell, _, _| {
                    shell
                        .terminals
                        .get(&shell.workspace.project)
                        .map_or(0, Vec::len)
                })
                .unwrap()
        }

        #[gpui::test]
        fn machines_close_the_list_with_agents_in_workspace_order(cx: &mut TestAppContext) {
            let (window, _) = window(vec![two_agents(Online::Online)], true, cx);
            let rows = rows(&window, cx);
            // No Machines heading: the machine row names itself.
            assert_eq!(
                rows[rows.len() - 3..],
                [Row::Machine("m1".into()), agent_row("a"), agent_row("b"),]
            );
            assert_eq!(mark(&window, "a", cx), Some(State::Done));
            assert_eq!(mark(&window, "b", cx), Some(State::Working));
        }

        #[gpui::test]
        fn machines_offline_machine_keeps_its_agents(cx: &mut TestAppContext) {
            let offline = Online::Offline {
                reason: "no route".into(),
                retry_in: Duration::from_secs(8),
            };
            let (window, _) = window(vec![two_agents(offline)], true, cx);
            assert_eq!(
                machine_rows(&rows(&window, cx)),
                [Row::Machine("m1".into()), agent_row("a"), agent_row("b"),]
            );
            let visual = &mut VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            assert!(visual.debug_bounds("machine-m1-offline").is_some());
            assert!(visual.debug_bounds("machine-agent-m1-a").is_some());
        }

        #[gpui::test]
        fn machines_rows_are_absent_without_machines(cx: &mut TestAppContext) {
            let (window, _) = window(Vec::new(), true, cx);
            assert!(machine_rows(&rows(&window, cx)).is_empty());
        }

        #[gpui::test]
        fn machines_rows_are_absent_when_hidden(cx: &mut TestAppContext) {
            let (window, _) = window(vec![two_agents(Online::Online)], false, cx);
            assert!(machine_rows(&rows(&window, cx)).is_empty());
            let marks = window
                .update(cx, |shell, _, _| shell.machine_marks.len())
                .unwrap();
            assert_eq!(marks, 0);
        }

        #[gpui::test]
        fn machines_rows_and_marks_follow_the_list(cx: &mut TestAppContext) {
            let (window, machines) = window(vec![two_agents(Online::Online)], true, cx);
            assert_eq!(machine_rows(&rows(&window, cx)).len(), 3);
            // `b` closed and `a` started working.
            let next = view(
                Online::Online,
                vec![agent("a", "w1", AgentStatus::Working, Some(3))],
            );
            cx.update(|cx| crate::machines::set_for_test(&machines, vec![next], None, cx));
            cx.run_until_parked();
            assert_eq!(
                machine_rows(&rows(&window, cx)),
                [Row::Machine("m1".into()), agent_row("a"),]
            );
            assert_eq!(mark(&window, "a", cx), Some(State::Working));
            assert_eq!(mark(&window, "b", cx), None);
            // An unknown status shows no mark.
            let unknown = view(
                Online::Online,
                vec![agent("a", "w1", AgentStatus::Unknown, Some(3))],
            );
            cx.update(|cx| crate::machines::set_for_test(&machines, vec![unknown], None, cx));
            cx.run_until_parked();
            assert_eq!(mark(&window, "a", cx), None);
            // A status change repaints without rebuilding the sidebar index.
            let rebuilds = window
                .update(cx, |shell, _, _| shell.sidebar_list.structural_rebuilds)
                .unwrap();
            let blocked = view(
                Online::Online,
                vec![agent("a", "w1", AgentStatus::Blocked, Some(3))],
            );
            cx.update(|cx| crate::machines::set_for_test(&machines, vec![blocked], None, cx));
            cx.run_until_parked();
            assert_eq!(mark(&window, "a", cx), Some(State::Attention));
            rows(&window, cx);
            assert_eq!(
                window
                    .update(cx, |shell, _, _| shell.sidebar_list.structural_rebuilds)
                    .unwrap(),
                rebuilds
            );
            // The last machine going away takes the section with it.
            cx.update(|cx| crate::machines::set_for_test(&machines, Vec::new(), None, cx));
            cx.run_until_parked();
            assert!(machine_rows(&rows(&window, cx)).is_empty());
        }

        #[gpui::test]
        fn machines_clicking_an_agent_attaches_and_marks_it_seen(cx: &mut TestAppContext) {
            let (window, machines) = window(vec![two_agents(Online::Online)], true, cx);
            window
                .update(cx, |_, _, cx| cx.set_reduce_motion(true))
                .unwrap();
            let before = terminal_count(&window, cx);
            click_agent(&window, "a", cx);
            assert_eq!(terminal_count(&window, cx), before + 1);
            let terminal = window
                .update(cx, |shell, window, cx| {
                    let terminal = shell.terminals[&shell.workspace.project]
                        .last()
                        .unwrap()
                        .clone();
                    let t = terminal.read(cx);
                    assert_eq!(t.title, "a agent on box");
                    #[cfg(unix)]
                    assert_eq!(
                        t.command().map(|(_, args)| args[1].clone()),
                        Some("ssh -t -- 'me@box' 'attach w1:p-a'".to_owned())
                    );
                    assert!(t.focus_handle(cx).is_focused(window));
                    terminal
                })
                .unwrap();
            // Seen at once, without waiting for the registry.
            assert_eq!(
                cx.update(|cx| machines.read(cx).seen_seq("m1", "a")),
                Some(3)
            );
            assert_eq!(mark(&window, "a", cx), Some(State::Idle));
            assert_eq!(
                cx.update(|cx| machines
                    .read(cx)
                    .attach_tab("m1", "a")
                    .and_then(|t| t.upgrade())),
                Some(terminal.clone())
            );

            // A second click returns to that tab instead of opening another.
            window
                .update(cx, |shell, window, cx| {
                    shell.select_terminal(0, window, cx);
                })
                .unwrap();
            click_agent(&window, "a", cx);
            assert_eq!(terminal_count(&window, cx), before + 1);
            window
                .update(cx, |shell, window, cx| {
                    let project = shell.workspace.project.clone();
                    let dock = &shell.workspace.docks[&project];
                    assert_eq!(shell.terminals[&project][dock.active], terminal);
                    assert!(terminal.focus_handle(cx).is_focused(window));
                })
                .unwrap();

            // Another agent gets a tab of its own.
            click_agent(&window, "b", cx);
            assert_eq!(terminal_count(&window, cx), before + 2);
        }

        #[gpui::test]
        fn machines_clicking_an_agent_whose_attach_exited_opens_a_fresh_tab(
            cx: &mut TestAppContext,
        ) {
            let (window, machines) = window(vec![two_agents(Online::Online)], true, cx);
            window
                .update(cx, |_, _, cx| cx.set_reduce_motion(true))
                .unwrap();
            let before = terminal_count(&window, cx);
            click_agent(&window, "a", cx);
            let attached = |cx: &mut TestAppContext| {
                cx.update(|cx| {
                    machines
                        .read(cx)
                        .attach_tab("m1", "a")
                        .and_then(|t| t.upgrade())
                })
                .unwrap()
            };
            let dead = attached(cx);
            // herdr's attach ended, say the connection dropped.
            cx.update(|cx| dead.update(cx, |t, _| t.exited = true));
            click_agent(&window, "a", cx);
            let fresh = attached(cx);
            assert_ne!(fresh, dead, "a new attach tab, not the dead one");
            assert!(!fresh.read_with(cx, |t, _| t.exited));
            window
                .update(cx, |shell, window, cx| {
                    let project = shell.workspace.project.clone();
                    let tabs = &shell.terminals[&project];
                    assert!(!tabs.contains(&dead), "the dead tab is closed");
                    assert_eq!(tabs.len(), before + 1);
                    let dock = &shell.workspace.docks[&project];
                    assert_eq!(tabs[dock.active], fresh);
                    assert!(fresh.focus_handle(cx).is_focused(window));
                })
                .unwrap();
        }

        #[gpui::test]
        fn machines_clicking_without_herdr_does_nothing(cx: &mut TestAppContext) {
            let (window, machines) = window(vec![two_agents(Online::Online)], true, cx);
            cx.update(|cx| crate::machines::set_attach_for_test(&machines, |_, _| None, cx));
            let before = terminal_count(&window, cx);
            click_agent(&window, "a", cx);
            assert_eq!(terminal_count(&window, cx), before);
            // Nothing was opened, so nothing counts as seen.
            assert_eq!(mark(&window, "a", cx), Some(State::Done));
            assert_eq!(cx.update(|cx| machines.read(cx).herdr_found()), Some(false));
            // Known missing, the row stops offering the click at all.
            click_agent(&window, "a", cx);
            assert_eq!(terminal_count(&window, cx), before);
        }

        #[gpui::test]
        fn machines_lookup_is_released_when_the_window_closes_meanwhile(cx: &mut TestAppContext) {
            let (window, machines) = window(vec![two_agents(Online::Online)], true, cx);
            window
                .update(cx, |shell, window, cx| {
                    shell.open_machine_agent("m1", "a", window, cx);
                    // The lookup finishes only after the window is gone.
                    window.remove_window();
                })
                .unwrap();
            // While under way a second lookup for the agent is refused.
            assert!(
                cx.update(|cx| machines.update(cx, |m, _| m.begin_attach("m1", "a")))
                    .is_none()
            );
            cx.run_until_parked();
            assert!(
                cx.update(|cx| machines.update(cx, |m, _| m.begin_attach("m1", "a")))
                    .is_some(),
                "a closed window left the agent stuck as attaching"
            );
        }

        fn notice(terminal: &str, kind: crate::machines::MachineNotice) -> crate::machines::Notice {
            crate::machines::Notice {
                machine_id: "m1".into(),
                machine_label: "box".into(),
                terminal_id: terminal.into(),
                agent_label: format!("{terminal} agent"),
                notice: kind,
            }
        }

        fn deliver(
            window: &WindowHandle<Shell>,
            notice: &crate::machines::Notice,
            cx: &mut impl gpui::AppContext,
        ) -> Vec<String> {
            window
                .update(cx, |shell, window, cx| {
                    shell.deliver_machine_notice(notice, window, cx);
                    shell.notification_state.machine_notices.clone()
                })
                .unwrap()
        }

        #[gpui::test]
        fn machines_notices_skip_the_agent_being_watched_only(cx: &mut TestAppContext) {
            use crate::machines::MachineNotice::{Finished, NeedsAttention};
            let (window, _) = window(
                vec![view(
                    Online::Online,
                    vec![
                        agent("a", "w1", AgentStatus::Working, None),
                        agent("b", "w1", AgentStatus::Working, None),
                    ],
                )],
                true,
                cx,
            );
            window
                .update(cx, |shell, _, _| shell.allow_notices_for_test(true))
                .unwrap();
            click_agent(&window, "a", cx);
            let visual = &mut VisualTestContext::from_window(*window, cx);
            visual.update(|window, _| window.activate_window());
            visual.run_until_parked();
            assert!(visual.update(|window, _| window.is_window_active()));

            // Its tab is focused in the active window, so the user sees it.
            assert!(deliver(&window, &notice("a", NeedsAttention), visual).is_empty());
            // Another agent has no tab to look at.
            assert_eq!(
                deliver(&window, &notice("b", NeedsAttention), visual),
                ["b agent needs your input"]
            );
            assert_eq!(
                deliver(&window, &notice("b", Finished), visual),
                ["b agent needs your input", "b agent finished"]
            );
            // With the window in the background the focused tab goes unseen.
            visual.deactivate_window();
            visual.run_until_parked();
            assert_eq!(deliver(&window, &notice("a", Finished), visual).len(), 3);
        }

        #[gpui::test]
        fn machines_notices_follow_the_global_switch(cx: &mut TestAppContext) {
            let (window, _) = window(vec![two_agents(Online::Online)], true, cx);
            let blocked = notice("b", crate::machines::MachineNotice::NeedsAttention);
            window
                .update(cx, |shell, _, _| shell.allow_notices_for_test(false))
                .unwrap();
            assert!(deliver(&window, &blocked, cx).is_empty());
            window
                .update(cx, |shell, _, _| shell.allow_notices_for_test(true))
                .unwrap();
            assert_eq!(deliver(&window, &blocked, cx).len(), 1);
        }

        #[gpui::test]
        fn machines_blocked_agent_reaches_the_window_that_tells_the_user(cx: &mut TestAppContext) {
            let working = view(
                Online::Online,
                vec![agent("b", "w1", AgentStatus::Working, None)],
            );
            let (window, machines) = window(vec![working], true, cx);
            window
                .update(cx, |shell, _, _| shell.allow_notices_for_test(true))
                .unwrap();
            let blocked = view(
                Online::Online,
                vec![agent("b", "w1", AgentStatus::Blocked, None)],
            );
            cx.update(|cx| crate::machines::set_for_test(&machines, vec![blocked], None, cx));
            cx.run_until_parked();
            let delivered = window
                .update(cx, |shell, _, _| {
                    shell.notification_state.machine_notices.clone()
                })
                .unwrap();
            assert_eq!(delivered, ["b agent needs your input"]);
        }
    }

    /// Rail F1's list: what goes in Needs you, Running and each folder.
    mod rail {
        use super::super::super::*;
        use super::super::Row;
        use super::super::rail_plan::{FolderKind, Live};
        use crate::conversation::SessionChanged;
        use gpui::{TestAppContext, VisualTestContext, WindowHandle};

        const PROJECT: &str = "/work/gespan";

        /// A window whose project lists `worktrees.len()` talked-to sessions,
        /// session `i` in worktree `worktrees[i]` (`None`: the project
        /// folder), the first the newest.
        fn rail_window(
            worktrees: &[Option<&str>],
            cx: &mut TestAppContext,
        ) -> (WindowHandle<Shell>, Vec<i64>) {
            let (window, a, b) = super::super::super::tests::window(cx);
            let ids = window
                .update(cx, |shell, _, cx| {
                    for id in [a, b] {
                        shell.sidebar_hidden_sessions.insert(id);
                    }
                    let now = gespan_core::time::now_ms();
                    let mut ids = Vec::new();
                    for (ix, worktree) in worktrees.iter().enumerate() {
                        let mut summary = locked(&shell.store)
                            .unwrap()
                            .create("codex", PROJECT)
                            .unwrap();
                        summary.has_messages = true;
                        summary.title = format!("Session {ix}");
                        summary.updated_at = now - (ix as u64 + 1) * 60 * 60_000;
                        summary.created_at = summary.updated_at;
                        let id = summary.id;
                        shell.add(summary, cx);
                        shell
                            .session_worktrees
                            .insert(id, worktree.map(str::to_owned));
                        ids.push(id);
                    }
                    shell.notify_sidebar(cx);
                    ids
                })
                .unwrap();
            cx.run_until_parked();
            (window, ids)
        }

        fn rows(window: &WindowHandle<Shell>, cx: &mut TestAppContext) -> Vec<Row> {
            window
                .update(cx, |shell, _, cx| {
                    shell.sidebar_list.invalidate_structure();
                    shell.refresh_sidebar_structure(cx);
                    shell.sidebar_list.rows.clone()
                })
                .unwrap()
        }

        fn folder<'a>(rows: &'a [Row], label: &str) -> &'a super::super::rail_plan::Folder {
            rows.iter()
                .find_map(|row| match row {
                    Row::Folder(f) if f.label == label => Some(f),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no folder {label} in {rows:?}"))
        }

        /// Every session id the rail lists: in Now, in folders and in
        /// Sessions alike.
        fn listed(rows: &[Row]) -> Vec<i64> {
            rows.iter()
                .filter_map(|row| match row {
                    Row::Now { id, .. }
                    | Row::Session(id)
                    | Row::FolderSession(id)
                    | Row::Child { id, .. } => Some(*id),
                    _ => None,
                })
                .collect()
        }

        /// The flat Sessions list, in order.
        fn sessions(rows: &[Row]) -> Vec<i64> {
            rows.iter()
                .filter_map(|row| match row {
                    Row::Session(id) => Some(*id),
                    _ => None,
                })
                .collect()
        }

        /// Now's rows, in order.
        fn now(rows: &[Row]) -> Vec<(i64, Live)> {
            rows.iter()
                .filter_map(|row| match row {
                    Row::Now { id, live } => Some((*id, *live)),
                    _ => None,
                })
                .collect()
        }

        /// Clicks Sessions' "N more", so every idle session has a row.
        fn expand_sessions(window: &WindowHandle<Shell>, cx: &mut TestAppContext) {
            window
                .update(cx, |shell, _, cx| {
                    shell.expand_rail_list("sessions");
                    shell.notify_sidebar(cx);
                })
                .unwrap();
        }

        fn leak(text: String) -> &'static str {
            Box::leak(text.into_boxed_str())
        }

        fn click(visual: &mut VisualTestContext, selector: &'static str) {
            let bounds = visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is drawn"));
            visual.simulate_click(bounds.center(), Default::default());
            visual.run_until_parked();
        }

        fn each_once(rows: &[Row], ids: &[i64]) {
            let mut seen = listed(rows);
            seen.sort_unstable();
            let mut all = ids.to_vec();
            all.sort_unstable();
            assert_eq!(seen, all, "every session exactly once in {rows:?}");
        }

        fn set_live(
            window: &WindowHandle<Shell>,
            id: i64,
            live: Option<Live>,
            cx: &mut TestAppContext,
        ) {
            window
                .update(cx, |shell, _, cx| {
                    // A real chat carries its worktree in its settings.
                    let worktree = shell.session_worktrees.get(&id).cloned().flatten();
                    shell.sessions[&id].update(cx, |view, cx| {
                        view.settings.worktree_cwd = worktree;
                        view.busy = live.is_some();
                        view.set_approvals_for_test(match live {
                            Some(Live::NeedsYou) => vec![gespan_core::agent::Approval {
                                id: "a1".into(),
                                title: "Run gh pr create?".into(),
                                detail: "gh pr create".into(),
                                can_remember: false,
                                tool: None,
                            }],
                            _ => Vec::new(),
                        });
                        cx.emit(SessionChanged);
                    })
                })
                .unwrap();
            cx.run_until_parked();
        }

        #[gpui::test]
        fn sessions_are_one_flat_list_with_no_folders(cx: &mut TestAppContext) {
            let tree = "/work/gespan-worktrees/glass-home";
            let (window, ids) = rail_window(&[Some(tree), None, None, None, Some(tree)], cx);
            let rows = rows(&window, cx);
            // No per-branch folders, and nothing live: no Now either.
            assert!(!rows.iter().any(|r| matches!(r, Row::Folder(_))));
            assert!(!rows.contains(&Row::NowHeading));
            assert!(rows.contains(&Row::SessionsHeading));
            // Newest first: three, then "2 more".
            assert_eq!(sessions(&rows), ids[..3]);
            assert!(rows.contains(&Row::More {
                key: "sessions".into(),
                more: 2
            }));
            expand_sessions(&window, cx);
            let all = self::rows(&window, cx);
            assert_eq!(sessions(&all), ids);
            each_once(&all, &ids);
            // The switcher still counts worktrees and sessions.
            let summary = window
                .update(cx, |shell, _, _| shell.sidebar_list.summary)
                .unwrap();
            assert_eq!(summary, (2, 5));
        }

        #[gpui::test]
        fn a_session_moves_to_now_when_it_goes_live_and_back_when_idle(cx: &mut TestAppContext) {
            let (window, ids) = rail_window(&[None, None, None, None], cx);
            expand_sessions(&window, cx);
            set_live(&window, ids[3], Some(Live::Running), cx);
            set_live(&window, ids[2], Some(Live::NeedsYou), cx);
            // No manual rebuild: the chat's own change moves it.
            let live = window
                .update(cx, |shell, _, cx| {
                    shell.refresh_sidebar_structure(cx);
                    shell.sidebar_list.rows.clone()
                })
                .unwrap();
            // Needs you leads Now, then running; Now sits above Sessions.
            assert_eq!(
                now(&live),
                [(ids[2], Live::NeedsYou), (ids[3], Live::Running)]
            );
            assert_eq!(sessions(&live), [ids[0], ids[1]]);
            let at = |row: &Row| live.iter().position(|r| r == row).unwrap();
            assert!(at(&Row::NowHeading) < at(&Row::SessionsHeading));
            each_once(&live, &ids);
            // Idle again: back in Sessions by age, and Now is gone.
            set_live(&window, ids[3], None, cx);
            set_live(&window, ids[2], None, cx);
            let idle = rows(&window, cx);
            assert!(now(&idle).is_empty());
            assert!(!idle.contains(&Row::NowHeading));
            assert_eq!(sessions(&idle), ids);
        }

        #[gpui::test]
        fn sessions_show_three_then_n_more_lists_the_rest(cx: &mut TestAppContext) {
            let (window, ids) = rail_window(&[None; 6], cx);
            let visual = &mut VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            assert_eq!(sessions(&rows(&window, visual)), ids[..3]);
            click(visual, "rail-folder-more-sessions");
            let all = rows(&window, visual);
            assert_eq!(sessions(&all), ids);
            assert!(!all.iter().any(|r| matches!(r, Row::More { .. })));
        }

        #[gpui::test]
        fn n_more_opens_the_list_in_its_own_project_only(cx: &mut TestAppContext) {
            let (window, ids) = rail_window(&[None; 5], cx);
            expand_sessions(&window, cx);
            assert_eq!(sessions(&rows(&window, cx)), ids);
            let (here, elsewhere) = window
                .update(cx, |shell, _, _| {
                    let here = shell.expanded_here();
                    shell.workspace.project = "/work/elsewhere".into();
                    let elsewhere = shell.expanded_here();
                    shell.workspace.project = PROJECT.into();
                    (here, elsewhere)
                })
                .unwrap();
            assert!(here.contains("sessions"));
            assert!(elsewhere.is_empty(), "{elsewhere:?}");
        }

        #[gpui::test]
        fn now_shows_five_then_n_more_running(cx: &mut TestAppContext) {
            let (window, ids) = rail_window(&[None; 7], cx);
            for id in &ids {
                set_live(&window, *id, Some(Live::Running), cx);
            }
            let rows = rows(&window, cx);
            assert_eq!(
                now(&rows).iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                ids[..5]
            );
            assert!(rows.contains(&Row::More {
                key: "now".into(),
                more: 2
            }));
            assert!(sessions(&rows).is_empty());
        }

        #[gpui::test]
        fn sessions_show_their_branch_and_now_shows_age(cx: &mut TestAppContext) {
            let tree = "/work/gespan-worktrees/glass-home";
            let long = "/work/gespan-worktrees/long";
            crate::git_cache::remember(
                PROJECT,
                &gespan_core::git::Snapshot {
                    worktrees: vec![
                        gespan_core::git::Worktree {
                            path: tree.into(),
                            branch: Some("claude/glass-home-1b4fa9".into()),
                            ..Default::default()
                        },
                        gespan_core::git::Worktree {
                            path: long.into(),
                            branch: Some("feature/very-long-branch-name-for-the-rail".into()),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
            );
            // The third runs where no branch is known yet; the fourth, on a
            // branch, is live.
            let (window, ids) = rail_window(
                &[Some(tree), Some(long), Some("/work/nowhere"), Some(tree)],
                cx,
            );
            set_live(&window, ids[3], Some(Live::Running), cx);
            let visual = &mut VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            let labels = window
                .update(visual, |shell, _, cx| {
                    ids[..3]
                        .iter()
                        .map(|id| shell.rail_branch_label(*id, cx).map(|l| l.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap();
            assert_eq!(
                labels,
                [
                    Some("glass-home".to_owned()),
                    Some("feature/…he-rail".to_owned()),
                    None
                ]
            );
            let branch = |id: i64, v: &mut VisualTestContext| {
                v.debug_bounds(leak(format!("rail-session-branch-{id}")))
            };
            let age = |id: i64, v: &mut VisualTestContext| {
                v.debug_bounds(leak(format!("rail-session-age-{id}")))
            };
            for id in &ids[..2] {
                let label = branch(*id, visual).expect("a labelled row shows its branch");
                assert!(label.size.width <= px(84.), "{label:?}");
                assert!(age(*id, visual).is_none(), "a labelled row shows no age");
            }
            assert!(branch(ids[2], visual).is_none());
            assert!(age(ids[2], visual).is_some(), "no branch: its age instead");
            // In Now: its age, never a label.
            assert!(branch(ids[3], visual).is_none());
            assert!(age(ids[3], visual).is_some());
        }

        #[gpui::test]
        fn the_two_cards_share_edges_and_every_row_one_gutter(cx: &mut TestAppContext) {
            let (window, ids) = rail_window(&[None, None], cx);
            set_live(&window, ids[1], Some(Live::Running), cx);
            add_pilots(&window, PROJECT, &["Release"], cx);
            let visual = &mut VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            let at = |selector: String, v: &mut VisualTestContext| {
                v.debug_bounds(leak(selector.clone()))
                    .unwrap_or_else(|| panic!("{selector} is drawn"))
            };
            let everywhere = at("rail-everywhere".into(), visual);
            let project = at("rail-project-card".into(), visual);
            assert_eq!(everywhere.left(), project.left());
            assert_eq!(everywhere.right(), project.right());
            assert_eq!(project.top() - everywhere.bottom(), px(10.));
            // Everywhere: Search, then the labelled pages in order.
            let tops: Vec<_> = ["Search", "Home", "My tasks", "Pull requests", "Automations"]
                .map(|label| at(format!("sidebar-navigation-{label}"), visual))
                .to_vec();
            assert!(tops.windows(2).all(|w| w[0].bottom() <= w[1].top()));
            assert!(tops.iter().all(|r| everywhere.contains(&r.center())));
            assert!(visual.debug_bounds("rail-zone-Everywhere").is_some());
            // This project: switcher, New thread, Pilots, Now, Sessions.
            let switcher = at("sidebar-project-button".into(), visual);
            let new_thread = at("sidebar-new-thread".into(), visual);
            let pilot = at("rail-section-Pilots".into(), visual);
            let now = at("rail-section-Now".into(), visual);
            let sessions = at("rail-section-Sessions".into(), visual);
            for (a, b) in [
                (switcher, new_thread),
                (new_thread, pilot),
                (pilot, now),
                (now, sessions),
            ] {
                assert!(a.bottom() <= b.top(), "{a:?} above {b:?}");
                assert!(project.contains(&b.center()));
            }
            // One gutter: rows in both cards start and end on the same
            // edges, their dots and icons 10px in.
            let home = at("sidebar-navigation-Home".into(), visual);
            let live = at(format!("sidebar-session-{}", ids[1]), visual);
            let idle = at(format!("sidebar-session-{}", ids[0]), visual);
            for row in [new_thread, live, idle] {
                assert_eq!((row.left(), row.right()), (home.left(), home.right()));
            }
            let dot = at(format!("rail-session-dot-{}", ids[0]), visual);
            assert_eq!(dot.left() - idle.left(), px(10.));
            assert_eq!(home.left() - everywhere.left(), px(5.), "1px edge, 4px pad");
            assert_eq!(idle.size.height, px(28.));
            // The old sections are gone.
            for gone in ["Needs you", "Running", "Folders", "Machines"] {
                assert!(
                    visual
                        .debug_bounds(leak(format!("rail-section-{gone}")))
                        .is_none(),
                    "{gone}"
                );
            }
        }

        /// Adds Pilot `name` on this project with `sessions` launched here.
        fn add_pilot_with(
            window: &WindowHandle<Shell>,
            name: &str,
            sessions: &[i64],
            cx: &mut TestAppContext,
        ) -> String {
            let id = add_pilots(window, PROJECT, &[name], cx).remove(0);
            window
                .update(cx, |shell, _, cx| {
                    shell.pilot_manager().update(cx, |m, cx| {
                        let pilot = m.roster.get_mut(&id).unwrap();
                        for session in sessions {
                            pilot.launches.push(gespan_core::pilot::Launch {
                                session_id: *session,
                                cwd: PROJECT.into(),
                                title: "Child".into(),
                                harness: "codex".into(),
                                model: None,
                                origin_turn: 0,
                                notify_on_complete: false,
                                completed: false,
                                dismissed: false,
                            });
                        }
                        cx.notify();
                    });
                    shell.notify_sidebar(cx);
                })
                .unwrap();
            cx.run_until_parked();
            id
        }

        #[gpui::test]
        fn a_pilots_sessions_list_with_the_rest_not_under_it(cx: &mut TestAppContext) {
            let (window, ids) = rail_window(&[None, None, None], cx);
            let pilot = add_pilot_with(&window, "Release", &[ids[0], ids[2]], cx);
            let visual = &mut VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            let rows = rows(&window, visual);
            assert_eq!(sessions(&rows), ids, "in Sessions by age, once each");
            each_once(&rows, &ids);
            // The Pilot row opens its chat; its sessions are not under it.
            let pilot_row = visual
                .debug_bounds(leak(format!("pilot-rail-{pilot}")))
                .expect("the Pilot row");
            let heading = visual.debug_bounds("rail-section-Sessions").unwrap();
            assert!(pilot_row.bottom() <= heading.top());
            let first = visual
                .debug_bounds(leak(format!("sidebar-session-{}", ids[0])))
                .unwrap();
            assert!(heading.bottom() <= first.top());
        }

        #[gpui::test]
        fn the_running_card_is_always_at_the_bottom(cx: &mut TestAppContext) {
            let (window, ids) = rail_window(&[None, None], cx);
            let visual = &mut VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            let card = |v: &mut VisualTestContext| {
                v.debug_bounds("agents-footer")
                    .expect("the running card is drawn")
            };
            let settings = visual.debug_bounds("sidebar-navigation-Settings").unwrap();
            let project = visual.debug_bounds("rail-project-card").unwrap();
            // Nothing running: still there, under the project card and just
            // above Settings.
            let idle = card(visual);
            assert!(project.bottom() <= idle.top());
            assert!(idle.bottom() <= settings.top());
            set_live(&window, ids[0], Some(Live::Running), cx);
            visual.run_until_parked();
            card(visual);
            click(visual, "agents-footer");
            assert!(
                window
                    .update(visual, |shell, _, _| shell.agents_open)
                    .unwrap(),
                "it opens the Agents panel"
            );
        }

        #[gpui::test]
        fn a_live_dot_shows_on_a_closed_user_folder(cx: &mut TestAppContext) {
            let (window, ids) = rail_window(&[None, None, None], cx);
            window
                .update(cx, |shell, _, cx| {
                    shell.workspace.preferences.sidebar_group_by = "folders".into();
                    shell.update_groups(cx, |g| {
                        let id = g.create_folder(&[ids[1], ids[2]], "Release".into());
                        if let Some(folder) =
                            g.folders.iter_mut().find(|f| Some(&f.id) == id.as_ref())
                        {
                            folder.collapsed = true;
                        }
                    });
                })
                .unwrap();
            set_live(&window, ids[2], Some(Live::NeedsYou), cx);
            let visual = &mut VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            let rows = rows(&window, visual);
            let release = folder(&rows, "New folder");
            assert!(!release.open);
            // The waiting one is in Now, never also in Sessions.
            assert_eq!(now(&rows), [(ids[2], Live::NeedsYou)]);
            assert_eq!(sessions(&rows), [ids[0]]);
            let dot = visual
                .debug_bounds(leak(format!("rail-folder-dot-{}", release.key)))
                .expect("the closed folder shows the dot");
            assert_eq!(dot.size.width, px(7.));
            assert_eq!(release.count, 2);
        }

        #[gpui::test]
        fn the_setting_groups_by_the_users_own_folders(cx: &mut TestAppContext) {
            let (window, ids) = rail_window(&[None, None, None], cx);
            window
                .update(cx, |shell, _, cx| {
                    shell.workspace.preferences.sidebar_group_by = "folders".into();
                    shell.update_groups(cx, |g| {
                        g.create_folder(&[ids[0], ids[2]], "Release".into());
                    });
                })
                .unwrap();
            let rows = rows(&window, cx);
            let release = folder(&rows, "New folder");
            assert_eq!(release.kind, FolderKind::Folder);
            // A user folder keeps its own open state: open by default.
            assert!(release.open);
            assert_eq!(release.shown, [ids[0], ids[2]]);
            // The rest are the flat list, below the folders.
            assert_eq!(sessions(&rows), [ids[1]]);
            each_once(&rows, &ids);
            let summary = window
                .update(cx, |shell, _, _| shell.sidebar_list.summary)
                .unwrap();
            assert_eq!(summary, (1, 3));
            // Back to worktrees: no user folder in the rail, the data kept.
            window
                .update(cx, |shell, _, _| {
                    shell.workspace.preferences.sidebar_group_by = "worktree".into();
                })
                .unwrap();
            let flat = self::rows(&window, cx);
            assert!(!flat.iter().any(|r| matches!(r, Row::Folder(_))));
            assert_eq!(sessions(&flat), ids);
            assert_eq!(
                window
                    .update(cx, |shell, _, _| shell.project_groups().folders.len())
                    .unwrap(),
                1
            );
        }

        /// Adds Pilots named `names` that work on `project`.
        fn add_pilots(
            window: &WindowHandle<Shell>,
            project: &str,
            names: &[&str],
            cx: &mut TestAppContext,
        ) -> Vec<String> {
            window
                .update(cx, |shell, _, cx| {
                    let ids = shell.pilot_manager().update(cx, |m, cx| {
                        let ids: Vec<String> = names
                            .iter()
                            .map(|name| {
                                let id = m.roster.create(vec![project.to_owned()]).id.clone();
                                m.roster.get_mut(&id).unwrap().name = (*name).to_owned();
                                id
                            })
                            .collect();
                        cx.notify();
                        ids
                    });
                    shell.notify_sidebar(cx);
                    ids
                })
                .unwrap()
        }

        #[gpui::test]
        fn pilots_list_this_projects_first_three_then_all_n(cx: &mut TestAppContext) {
            let (window, _) = rail_window(&[None], cx);
            let mine = add_pilots(&window, PROJECT, &["A", "B", "C", "D", "E"], cx);
            let other = add_pilots(&window, "/work/elsewhere", &["Elsewhere"], cx);
            let visual = &mut VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            let row = |id: &str, v: &mut VisualTestContext| {
                let s: &'static str = Box::leak(format!("pilot-rail-{id}").into_boxed_str());
                v.debug_bounds(s)
            };
            let shown: Vec<bool> = mine.iter().map(|id| row(id, visual).is_some()).collect();
            assert_eq!(shown, [true, true, true, false, false]);
            assert!(row(&other[0], visual).is_none(), "another project's Pilot");
            // The board's 32px rows, under the section heading.
            let first = row(&mine[0], visual).unwrap();
            assert_eq!(first.size.height, px(32.));
            let heading = visual.debug_bounds("rail-section-Pilots").unwrap();
            assert!(heading.bottom() <= first.top());
            // The 8px status dot sits on the 16px mascot's corner.
            let dot: &'static str =
                Box::leak(format!("pilot-status-dot-{}", mine[0]).into_boxed_str());
            assert_eq!(visual.debug_bounds(dot).unwrap().size.width, px(8.));
            // "All 5 →" lists the rest, and folds them away again.
            let all = visual.debug_bounds("pilots-rail-all").expect("All 5 →");
            visual.simulate_click(all.center(), Default::default());
            visual.run_until_parked();
            assert!(mine.iter().all(|id| row(id, visual).is_some()));
            let all = visual.debug_bounds("pilots-rail-all").unwrap();
            visual.simulate_click(all.center(), Default::default());
            visual.run_until_parked();
            assert!(row(&mine[4], visual).is_none());
        }

        #[test]
        fn a_live_or_open_pilot_is_never_cut_off() {
            use crate::pilots::rail_shown;
            // Ranks: 0 needs you, 1 working, 2 open, 3 idle; sorted.
            assert_eq!(rail_shown(&[3, 3, 3, 3, 3]), 3);
            assert_eq!(rail_shown(&[0, 1, 1, 2, 3, 3]), 4);
            assert_eq!(rail_shown(&[3, 3]), 2);
            assert_eq!(rail_shown(&[]), 0);
        }

        #[gpui::test]
        fn clicking_a_rail_row_opens_its_session(cx: &mut TestAppContext) {
            let (window, ids) = rail_window(&[None, None], cx);
            let visual = &mut VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            let row: &'static str =
                Box::leak(format!("sidebar-session-{}", ids[1]).into_boxed_str());
            let row = visual.debug_bounds(row).unwrap();
            visual.simulate_click(row.center(), Default::default());
            visual.run_until_parked();
            window
                .update(visual, |shell, _, _| {
                    assert_eq!(shell.workspace.active, Some(ids[1]))
                })
                .unwrap();
        }
    }

    mod fixed_folders {
        use super::super::super::session_groups::RenameTarget;
        use super::super::super::*;
        use super::super::Row;
        use gespan_core::pilot::{Launch, Pilot};
        use gpui::{TestAppContext, VisualTestContext, WindowHandle};

        const AUTOMATIONS: &str = "fixed:automations";

        /// Both fixture sessions listed in the open project.
        fn setup(cx: &mut TestAppContext) -> (WindowHandle<Shell>, i64, i64) {
            let (window, a, b) = super::super::super::tests::window(cx);
            window
                .update(cx, |shell, _, cx| {
                    // These folders are what "Group sessions by: My folders"
                    // lists.
                    shell.workspace.preferences.sidebar_group_by = "folders".into();
                    for id in [a, b] {
                        shell.sessions[&id].update(cx, |session, _| {
                            session.summary.cwd = shell.workspace.project.clone();
                            session.summary.has_messages = true;
                        });
                    }
                    shell.invalidate_chrome(cx);
                })
                .unwrap();
            cx.run_until_parked();
            (window, a, b)
        }

        fn rows(window: &WindowHandle<Shell>, cx: &mut TestAppContext) -> Vec<Row> {
            window
                .update(cx, |shell, _, cx| {
                    shell.refresh_sidebar_structure(cx);
                    shell.sidebar_list.rows.clone()
                })
                .unwrap()
        }

        /// The fixed folder headers, as (id, name, the sessions they list).
        fn fixed(rows: &[Row]) -> Vec<(String, String, Vec<i64>)> {
            members(rows)
                .into_iter()
                .filter(|(f, _)| f.key.starts_with("fixed:"))
                .map(|(f, ids)| (f.key, f.label, ids))
                .collect()
        }

        /// Each folder with the sessions listed under it.
        fn members(rows: &[Row]) -> Vec<(super::super::rail_plan::Folder, Vec<i64>)> {
            let mut out: Vec<(super::super::rail_plan::Folder, Vec<i64>)> = Vec::new();
            for row in rows {
                match row {
                    Row::Folder(folder) => out.push((folder.clone(), Vec::new())),
                    Row::FolderSession(id) => {
                        if let Some((_, ids)) = out.last_mut() {
                            ids.push(*id);
                        }
                    }
                    _ => {}
                }
            }
            out
        }

        /// Sessions listed in a folder of their own.
        fn grouped(rows: &[Row]) -> Vec<i64> {
            members(rows).into_iter().flat_map(|(_, ids)| ids).collect()
        }

        /// Sessions in no folder: the flat Sessions list, every row shown.
        fn loose(rows: &[Row]) -> Vec<i64> {
            let mut ids: Vec<i64> = rows
                .iter()
                .filter_map(|row| match row {
                    Row::Session(id) => Some(*id),
                    _ => None,
                })
                .collect();
            ids.sort_unstable();
            ids
        }

        fn sorted(mut ids: Vec<i64>) -> Vec<i64> {
            ids.sort_unstable();
            ids
        }

        /// The Pilots the project card lists: those working on it, and
        /// those with sessions here.
        fn listed_pilots(window: &WindowHandle<Shell>, cx: &mut TestAppContext) -> Vec<String> {
            window
                .update(cx, |shell, _, cx| {
                    let also = shell.pilots_with_sessions(cx);
                    shell
                        .pilots
                        .as_ref()
                        .unwrap()
                        .read(cx)
                        .rail_roster(Some(&shell.workspace.project), &also, cx)
                        .into_iter()
                        .map(|(id, _)| id)
                        .collect()
                })
                .unwrap()
        }

        fn mark_automation(window: &WindowHandle<Shell>, id: i64, cx: &mut TestAppContext) {
            window
                .update(cx, |shell, _, cx| {
                    shell
                        .automation_state
                        .session_automations
                        .insert(id, "nightly".into());
                    shell.invalidate_chrome(cx);
                })
                .unwrap();
        }

        fn launch(session_id: i64, cwd: &str) -> Launch {
            Launch {
                session_id,
                cwd: cwd.into(),
                title: "Child".into(),
                harness: "codex".into(),
                model: None,
                origin_turn: 0,
                notify_on_complete: false,
                completed: false,
                dismissed: false,
            }
        }

        #[gpui::test]
        fn automation_sessions_land_in_the_automation_runs_folder(cx: &mut TestAppContext) {
            let (window, a, b) = setup(cx);
            mark_automation(&window, a, cx);
            let rows = rows(&window, cx);
            assert_eq!(
                fixed(&rows),
                [(AUTOMATIONS.into(), "Automation runs".into(), vec![a])]
            );
            assert_eq!(grouped(&rows), [a]);
            assert_eq!(loose(&rows), [b], "a run is not also in the dated list");
        }

        #[gpui::test]
        fn a_users_folder_or_pin_wins_over_the_automation_runs_folder(cx: &mut TestAppContext) {
            let (window, a, b) = setup(cx);
            mark_automation(&window, a, cx);
            mark_automation(&window, b, cx);
            window
                .update(cx, |shell, _, cx| {
                    shell.update_groups(cx, |g| {
                        g.create_folder(&[a], "work".into());
                    });
                })
                .unwrap();
            let rows = rows(&window, cx);
            assert_eq!(
                fixed(&rows),
                [(AUTOMATIONS.into(), "Automation runs".into(), vec![b])]
            );
            assert!(folders(&rows).contains(&("work".to_owned(), vec![a])));
        }

        #[gpui::test]
        fn automation_runs_folder_fills_in_when_the_map_loads(cx: &mut TestAppContext) {
            let (window, a, _) = setup(cx);
            assert!(fixed(&rows(&window, cx)).is_empty());
            window
                .update(cx, |shell, _, cx| {
                    locked(&shell.store)
                        .unwrap()
                        .set_automation_id(a, "nightly")
                        .unwrap();
                    // Open the automations again, as at startup.
                    shell.automation_state.service = None;
                    shell.automation_state.session_automations.clear();
                    shell.initialize_automations(cx);
                })
                .unwrap();
            cx.run_until_parked();
            assert_eq!(
                fixed(&rows(&window, cx)),
                [(AUTOMATIONS.into(), "Automation runs".into(), vec![a])]
            );
        }

        #[gpui::test]
        fn fixed_folder_collapse_is_remembered(cx: &mut TestAppContext) {
            let (window, a, b) = setup(cx);
            mark_automation(&window, a, cx);
            let visual = &mut VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            let header = visual
                .debug_bounds("rail-folder-fixed:automations")
                .expect("the Automation runs header is shown");
            visual.simulate_click(header.center(), Default::default());
            visual.run_until_parked();
            let folded = rows(&window, cx);
            assert_eq!(fixed(&folded).len(), 1, "a folded folder keeps its header");
            assert!(grouped(&folded).is_empty());
            assert_eq!(loose(&folded), [b]);

            let (store, saved) = window
                .update(cx, |shell, _, _| {
                    (
                        shell.store.clone(),
                        serde_json::to_string(&shell.workspace).unwrap(),
                    )
                })
                .unwrap();
            assert!(
                locked(&store)
                    .unwrap()
                    .groups("/work/gespan")
                    .unwrap()
                    .fixed_is_collapsed(AUTOMATIONS)
            );
            let summaries = locked(&store).unwrap().sessions().unwrap();
            let reopened = cx.update(|cx| {
                cx.open_window(Default::default(), |_, cx| {
                    cx.new(|cx| {
                        Shell::new(store, summaries, "/work/gespan".into(), Some(saved), cx)
                    })
                })
                .unwrap()
            });
            cx.run_until_parked();
            for id in [a, b] {
                reopened
                    .update(cx, |shell, _, cx| {
                        shell.ensure_session(id, cx);
                        shell.sessions[&id].update(cx, |session, _| {
                            session.summary.cwd = "/work/gespan".into();
                            session.summary.has_messages = true;
                        });
                    })
                    .unwrap();
            }
            mark_automation(&reopened, a, cx);
            let listed = rows(&reopened, cx);
            // Still folded: the header counts the run, and lists no row.
            assert_eq!(
                fixed(&listed),
                [(AUTOMATIONS.into(), "Automation runs".into(), vec![])]
            );
            let (header, _) = members(&listed).into_iter().next().unwrap();
            assert_eq!((header.count, header.open), (1, false));
            assert!(grouped(&listed).is_empty(), "still folded after reopening");
        }

        /// Adds Pilot Scout with launches of `a` here and `b` in another project.
        fn add_scout(
            window: &WindowHandle<Shell>,
            a: i64,
            b: i64,
            cx: &mut TestAppContext,
        ) -> Entity<crate::pilots::Pilots> {
            let pilots = window
                .update(cx, |shell, _, _| shell.pilots.clone().unwrap())
                .unwrap();
            pilots.update(cx, |pilots, cx| {
                pilots.roster.pilots.push(Pilot {
                    id: "scout".into(),
                    name: "Scout".into(),
                    launches: vec![
                        launch(a, "/work/gespan"),
                        launch(b, "/work/another-project"),
                    ],
                    ..Default::default()
                });
                pilots.roster.pilots.push(Pilot {
                    id: "elsewhere".into(),
                    name: "Elsewhere".into(),
                    launches: vec![launch(b, "/work/another-project")],
                    ..Default::default()
                });
                cx.notify();
            });
            cx.run_until_parked();
            pilots
        }

        /// The user folders the sidebar lists, as (id, sessions).
        fn folders(rows: &[Row]) -> Vec<(String, Vec<i64>)> {
            members(rows)
                .into_iter()
                .filter_map(|(f, ids)| Some((f.key.strip_prefix("folder:")?.to_owned(), ids)))
                .collect()
        }

        fn stored_pilot_folder(window: &WindowHandle<Shell>, cx: &mut TestAppContext) -> Vec<i64> {
            window
                .update(cx, |shell, _, _| {
                    shell
                        .project_groups()
                        .folder("pilot-scout")
                        .map(|f| f.session_ids.clone())
                        .unwrap_or_default()
                })
                .unwrap()
        }

        #[gpui::test]
        fn pilot_folder_only_holds_this_projects_launches(cx: &mut TestAppContext) {
            let (window, a, b) = setup(cx);
            let pilots = add_scout(&window, a, b, cx);
            // Filed the way a launch is: into the stored `pilot-scout` folder.
            pilots.update(cx, |pilots, cx| pilots.group_launch("scout", a, cx));
            cx.run_until_parked();
            assert_eq!(stored_pilot_folder(&window, cx), [a], "stored data is kept");
            cx.update(|cx| {
                let pilots = pilots.read(cx);
                assert_eq!(
                    pilots.pilot_folders("/work/gespan"),
                    [
                        ("scout".to_string(), "Scout".to_string(), vec![a]),
                        ("elsewhere".to_string(), "Elsewhere".to_string(), vec![]),
                    ]
                );
                assert_eq!(
                    pilots.pilot_folders("/work/another-project"),
                    [
                        ("scout".to_string(), "Scout".to_string(), vec![b]),
                        ("elsewhere".to_string(), "Elsewhere".to_string(), vec![b]),
                    ]
                );
            });
            let rows = rows(&window, cx);
            // No Pilot folder: its session is in Sessions with the rest, and
            // the Pilot heads the card. The Pilot with nothing here and no
            // project of its own is not listed.
            assert!(fixed(&rows).is_empty(), "no Pilot folder");
            assert!(
                folders(&rows).is_empty(),
                "no user folder named after the Pilot"
            );
            assert_eq!(loose(&rows), sorted(vec![a, b]));
            assert_eq!(listed_pilots(&window, cx), ["scout"]);
        }

        #[gpui::test]
        fn a_launch_cwd_with_a_trailing_slash_still_lands_in_the_pilot_folder(
            cx: &mut TestAppContext,
        ) {
            let (window, a, b) = setup(cx);
            let pilots = add_scout(&window, a, b, cx);
            pilots.update(cx, |pilots, cx| {
                pilots.roster.pilots[0].launches = vec![launch(a, "/work/gespan/")];
                cx.notify();
            });
            cx.update(|cx| {
                assert_eq!(
                    pilots.read(cx).pilot_folders("/work/gespan")[0].2,
                    vec![a],
                    "a trailing slash is the same project"
                );
            });
        }

        #[gpui::test]
        fn stored_pilot_folder_members_join_the_fixed_pilot_folder(cx: &mut TestAppContext) {
            let (window, a, b) = setup(cx);
            let pilots = add_scout(&window, a, b, cx);
            // A Flow session is filed in the Pilot's folder but is not a launch.
            pilots.update(cx, |pilots, cx| {
                pilots.group_launch("scout", a, cx);
                pilots.file_flow_session("scout", "/work/gespan".into(), b, cx);
            });
            cx.run_until_parked();
            let rows = rows(&window, cx);
            // The stored Pilot folder's members list once, in Sessions.
            assert!(fixed(&rows).is_empty());
            assert!(folders(&rows).is_empty());
            assert_eq!(loose(&rows), sorted(vec![a, b]));
        }

        #[gpui::test]
        fn a_pilot_session_moved_to_a_user_folder_stays_there(cx: &mut TestAppContext) {
            let (window, a, b) = setup(cx);
            let pilots = add_scout(&window, a, b, cx);
            pilots.update(cx, |pilots, cx| {
                pilots.group_launch("scout", a, cx);
                pilots.file_flow_session("scout", "/work/gespan".into(), b, cx);
            });
            cx.run_until_parked();
            window
                .update(cx, |shell, _, cx| {
                    shell.update_groups(cx, |g| {
                        g.create_folder(&[b], "folder:mine".into());
                    });
                    shell.invalidate_chrome(cx);
                })
                .unwrap();
            let rows = rows(&window, cx);
            assert_eq!(folders(&rows), [("folder:mine".to_string(), vec![b])]);
            assert_eq!(loose(&rows), [a]);
        }

        #[gpui::test]
        fn fixed_folder_has_no_rename_or_delete(cx: &mut TestAppContext) {
            let (window, a, _) = setup(cx);
            mark_automation(&window, a, cx);
            let ids = |shell: &Shell| -> Vec<String> {
                shell
                    .fixed_folder_menu_items(AUTOMATIONS)
                    .into_iter()
                    .filter_map(|item| match item {
                        MenuItem::Action(action) => Some(action.id.to_string()),
                        _ => None,
                    })
                    .collect()
            };
            window
                .update(cx, |shell, w, cx| {
                    let before = ids(shell);
                    assert_eq!(before, ["fixed-collapse"]);
                    for id in &before {
                        for word in ["rename", "delete", "dissolve", "ungroup"] {
                            assert!(!id.contains(word), "{id} offers {word}");
                        }
                    }
                    // Folder paths keyed by id stay inert for a fixed id.
                    let groups = shell.project_groups();
                    for action in ["rename", "ungroup"] {
                        shell.run_menu(MenuTarget::Folder(AUTOMATIONS.into()), action, None, w, cx);
                    }
                    shell.start_rename(RenameTarget::Folder(AUTOMATIONS.into()), w, cx);
                    assert!(shell.renaming.is_none());
                    assert_eq!(shell.project_groups(), groups);

                    shell.run_menu(
                        MenuTarget::FixedFolder(AUTOMATIONS.into()),
                        "fixed-collapse",
                        None,
                        w,
                        cx,
                    );
                    assert!(shell.project_groups().fixed_is_collapsed(AUTOMATIONS));
                    assert_eq!(ids(shell), ["fixed-expand"]);
                    shell.run_menu(
                        MenuTarget::FixedFolder(AUTOMATIONS.into()),
                        "fixed-expand",
                        None,
                        w,
                        cx,
                    );
                    assert!(!shell.project_groups().fixed_is_collapsed(AUTOMATIONS));
                })
                .unwrap();
        }
    }
}
