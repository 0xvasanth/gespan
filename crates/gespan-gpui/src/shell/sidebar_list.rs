//! Only construct and lay out sidebar cards near the viewport.
use super::*;
use crate::machines::sidebar as machine_rows;
use gespan_core::session_groups::Entry;
use gpui::{ListAlignment, ListState, list};
use std::cell::{Cell, RefCell};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Row {
    Navigation,
    /// The Focused layout's project dropdown and gear, under All projects.
    Project,
    /// The Focused layout's standalone New thread button.
    NewThread,
    Pilots,
    Pinned,
    Folders,
    Group(Entry),
    Gap,
    Heading {
        label: &'static str,
        first: bool,
    },
    Session(i64),
    /// A worktree's group under Sessions, by its working copy's path.
    Worktree(String),
    GroupedSession {
        id: i64,
        first: bool,
        last: bool,
    },
    Empty,
    MachinesHeading,
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
        // Navigation and the Pilot overview can change height independently.
        self.state.remeasure_items(0..self.rows.len().min(2));
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
    rows.push(Row::MachinesHeading);
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

/// A session in the sidebar's worktree grouping.
struct Listed {
    id: i64,
    /// When the session started, or zero where the store does not say.
    created_at: u64,
    path: String,
}

/// Sessions by the worktree they run in, each group keeping the list's order.
/// The worktree where the newest session started comes first. Stores that do
/// not record a start time hand out growing ids, so the id breaks ties.
fn worktree_groups(sessions: &[Listed]) -> Vec<(String, Vec<i64>)> {
    use gespan_core::files::path_key;
    let mut groups: Vec<(String, Vec<i64>, (u64, i64))> = Vec::new();
    for session in sessions {
        let started = (session.created_at, session.id);
        match groups
            .iter_mut()
            .find(|(path, ..)| path_key(path) == path_key(&session.path))
        {
            Some((_, ids, newest)) => {
                ids.push(session.id);
                *newest = (*newest).max(started);
            }
            None => groups.push((session.path.clone(), vec![session.id], started)),
        }
    }
    groups.sort_by_key(|(.., newest)| std::cmp::Reverse(*newest));
    groups
        .into_iter()
        .map(|(path, ids, _)| (path, ids))
        .collect()
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
        if self.sidebar_list.structure_dirty.replace(false)
            || self.sidebar_list.scope != scope
            || self.sidebar_list.indexed_minute != minute
        {
            // Date/time expiry refreshes membership without resetting the
            // project scope or scroll anchor.
            self.sidebar_list.indexed_minute = minute;
            #[cfg(test)]
            {
                self.sidebar_list.structural_rebuilds += 1;
            }
            let ids = self.sidebar_session_ids(cx);
            self.sync_rails(&ids, cx);
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
            "{}\0{}\0{:?}\0{}\0{}",
            self.workspace.project,
            self.working_copy(),
            self.workspace.session_filters,
            self.workspace.preferences.show_archived,
            self.workspace.boxy_layout(),
        )
    }

    pub(super) fn prepare_sidebar_list(&mut self, entries: &[Entry], cx: &App) {
        let groups = self.project_groups();
        let mut rows = vec![Row::Navigation, Row::Project, Row::NewThread, Row::Pilots];
        if push_machine_rows(&mut rows, cx) {
            rows.push(Row::Gap);
        }
        let mut folders = false;
        for entry in entries {
            let (members, collapsed) = match entry {
                Entry::Pinned { sessions } => {
                    // Pins sit between the folders and the dated sessions,
                    // as ordinary cards under a plain label.
                    if folders {
                        rows.push(Row::Gap);
                        folders = false;
                    }
                    rows.push(Row::Pinned);
                    rows.extend(
                        sessions
                            .iter()
                            .filter(|id| !self.rails.children.contains(id))
                            .map(|id| Row::Session(*id)),
                    );
                    rows.push(Row::Gap);
                    continue;
                }
                Entry::Reminders {
                    sessions,
                    collapsed,
                } => (sessions, *collapsed),
                Entry::Folder { id, sessions } => {
                    (sessions, groups.folder(id).is_some_and(|g| g.collapsed))
                }
                Entry::Fixed { id, sessions, .. } => (sessions, groups.fixed_is_collapsed(id)),
                Entry::Session(_) => continue,
            };
            if !folders {
                rows.push(Row::Folders);
                folders = true;
            }
            rows.push(Row::Group(entry.clone()));
            if !collapsed {
                let members: Vec<_> = members
                    .iter()
                    .copied()
                    .filter(|id| !self.rails.children.contains(id))
                    .collect();
                rows.extend(
                    members
                        .iter()
                        .enumerate()
                        .map(|(ix, id)| Row::GroupedSession {
                            id: *id,
                            first: ix == 0,
                            last: ix + 1 == members.len(),
                        }),
                );
            }
        }
        if folders {
            rows.push(Row::Gap);
        }
        // One list under one heading; each row shows its own last-use time.
        rows.push(Row::Heading {
            label: "Sessions",
            first: true,
        });
        let sessions: Vec<_> = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Session(id) if !self.rails.children.contains(id) => Some(Listed {
                    id: *id,
                    created_at: self.session_metadata(*id, cx).summary.created_at,
                    path: self.session_working_copy(*id, cx).to_owned(),
                }),
                _ => None,
            })
            .collect();
        let worktrees = worktree_groups(&sessions);
        // A project that only works in its own folder needs no group headings.
        let grouped = worktrees.len() > 1
            || worktrees.first().is_some_and(|(path, _)| {
                gespan_core::files::path_key(path)
                    != gespan_core::files::path_key(&self.workspace.project)
            });
        for (path, ids) in worktrees {
            if grouped {
                rows.push(Row::Worktree(path));
            }
            rows.extend(ids.into_iter().map(Row::Session));
        }
        if entries.is_empty() {
            rows.push(Row::Empty);
        }
        self.sidebar_list.update(self.sidebar_scope(), rows);
    }

    fn sync_sidebar_list_visibility(&mut self) {
        // Open rails have variable height. Remeasure only materialized rows,
        // rather than scanning every session in the project every frame.
        for ix in self.sidebar_list.visible_rows.borrow_mut().drain(..) {
            if let Some(Row::Session(id) | Row::GroupedSession { id, .. }) =
                self.sidebar_list.rows.get(ix)
                && (self.rails.started.contains_key(id)
                    || self.orchestration_summaries.contains_key(id))
            {
                self.sidebar_list.state.remeasure_items(ix..ix + 1);
            }
        }
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
            Row::Navigation => return self.sidebar_navigation(cx),
            Row::Project => return self.sidebar_project_row(cx),
            Row::NewThread => return self.sidebar_new_thread(cx),
            Row::Pilots => {
                return self
                    .pilots
                    .clone()
                    .map(|pilots| {
                        pilots.update(cx, |pilots, cx| {
                            pilots.rail(Some(self.workspace.project.clone()), cx)
                        })
                    })
                    .unwrap_or_else(|| div().into_any());
            }
            Row::Pinned => section_label("Pinned").into_any(),
            Row::Folders => section_label("Folders").into_any(),
            Row::Group(entry) => {
                if let Entry::Folder { id, .. } = &entry
                    && let Some(anchor) = self.folder_bounds.get(id)
                {
                    self.sidebar_list.track_anchor(anchor.clone());
                }
                self.sidebar_group_header(&entry, cx)
            }
            Row::Gap => return div().h(px(12.)).into_any(),
            Row::Heading { label, first } => section_label(label)
                .when(!first, |d| d.mt_3())
                // The New row above already starts a session.
                .when(first, |d| d.child(self.sidebar_filter_button(cx)))
                .into_any(),
            Row::Session(id) => self.session_card(id, cx),
            Row::Worktree(path) => {
                let label = self.worktree_label(&path);
                div()
                    .debug_selector(|| "sidebar-worktree-group".into())
                    .h(px(24.))
                    .mt_1()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child(icon("git-branch", 12.))
                    .child(div().min_w_0().truncate().child(label))
                    .into_any()
            }
            Row::GroupedSession { id, first, last } => {
                return div()
                    .px_2()
                    .child(
                        div()
                            .ml(px(10.))
                            .when(first, |d| d.pt_1())
                            .child(rails::step(last, self.nested_session_card(id, cx))),
                    )
                    .into_any();
            }
            Row::MachinesHeading => section_label("Machines").into_any(),
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
                .mx_1()
                .mt_1()
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
        div().px_2().pb_px().child(content).into_any()
    }
}

/// The navigation rows' trailing column: centred 7px in from the row's
/// padding, on the same axis as the section headers' 24px icon buttons.
const TRAILING_SLOT: f32 = 14.;

impl Shell {
    /// The machine list or a turn seen changed: marks follow, and the
    /// sidebar index is rebuilt only when machines or agents come or go.
    pub(super) fn machines_changed(&mut self, cx: &mut Context<Self>) {
        self.sync_machine_marks(cx);
        let mut rows = Vec::new();
        push_machine_rows(&mut rows, cx);
        let shown = self.sidebar_list.rows.iter().filter(|row| {
            matches!(
                row,
                Row::MachinesHeading | Row::Machine(_) | Row::MachineAgent { .. }
            )
        });
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
            "sidebar-activity" => &mut self.workspace.preferences.working_agents,
            "compact-session-cards" => &mut self.workspace.compact_session_cards,
            _ => return,
        };
        *value = !*value;
        if matches!(
            action,
            "sidebar-notes" | "sidebar-board" | "sidebar-pilots" | "sidebar-activity"
        ) {
            self.apply_preferences(cx);
            self.sync_settings(cx);
        }
        self.close_preview(cx);
        self.sidebar_list.remeasure();
        self.persist(cx);
        cx.notify();
    }

    pub(super) fn sidebar_navigation(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let flows_visible = crate::flows::visible(cx);
        div()
            .w_full()
            .px_2()
            .pt(px(4.))
            // The New thread row below supplies its own top margin.
            .pb(px(0.))
            .flex()
            .flex_col()
            .gap_px()
            .child(
                div().w_full().flex().items_center().child(
                    section_label("All projects")
                        .flex_1()
                        .debug_selector(|| "sidebar-all-projects".into()),
                ),
            )
            .children(
                [
                    ("Home", "home"),
                    ("My tasks", "circle-alert"),
                    ("Inbox", "inbox"),
                    ("Runs", "clock"),
                    ("Automations", "zap"),
                    ("Flows", "git-merge"),
                    ("Board", "kanban"),
                    ("Notes", "note"),
                    ("Settings", "settings"),
                ]
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
                    let selected = self.app_view_label() == Some(label);
                    let waiting = if label == "My tasks" {
                        self.my_task_count(cx)
                    } else {
                        0
                    };
                    let unread = label == "Inbox"
                        && self
                            .inbox
                            .as_ref()
                            .is_some_and(|inbox| inbox.read(cx).unread_count() > 0);
                    div()
                        .id(label)
                        .debug_selector(move || format!("sidebar-navigation-{label}"))
                        .role(Role::Button)
                        .aria_label(if unread {
                            SharedString::from("Inbox, new items")
                        } else if waiting > 0 {
                            SharedString::from(format!("My tasks, {waiting} waiting"))
                        } else {
                            SharedString::from(label)
                        })
                        .aria_selected(selected)
                        .cursor_pointer()
                        .group("rail-action")
                        .h(px(32.))
                        .px_2()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(content(if selected || self.sidebar_hover == Some(label) {
                            1.
                        } else {
                            0.5
                        }))
                        .when(selected, |d| d.bg(content(0.1)))
                        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                        .child(icon(glyph, 16.).text_color(nav_icon_color(label, selected)))
                        .child(div().flex_1().child(label))
                        .when_some(
                            match label {
                                "Settings" => Some(format!("{MOD},").into_any_element()),
                                "My tasks" if waiting > 0 => Some(
                                    div()
                                        .debug_selector(|| {
                                            "sidebar-navigation-count-My tasks".into()
                                        })
                                        .flex_shrink_0()
                                        .min_w(px(18.))
                                        .h(px(16.))
                                        .px(px(5.))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .bg(gpui::rgb(0xffb900))
                                        .text_color(black(0.9))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(waiting.to_string())
                                        .into_any_element(),
                                ),
                                "Inbox" if unread => Some(
                                    div()
                                        .flex_shrink_0()
                                        .size(px(6.))
                                        .rounded_full()
                                        .bg(accent_color())
                                        .into_any_element(),
                                ),
                                _ => None,
                            },
                            // One column for hints, counts and the unread dot,
                            // centred under the section buttons' icons below.
                            |row, trailing| {
                                row.child(
                                    div()
                                        .debug_selector(move || {
                                            format!("sidebar-navigation-trailing-{label}")
                                        })
                                        .w(px(TRAILING_SLOT))
                                        .flex_shrink_0()
                                        .flex()
                                        .justify_center()
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::NORMAL)
                                        .text_color(content(0.35))
                                        .child(trailing),
                                )
                            },
                        )
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
                            "Settings" => {
                                this.open_settings(crate::settings::Section::General, window, cx)
                            }
                            "Home" => this.open_home(window, cx),
                            "My tasks" => this.open_my_tasks(window, cx),
                            "Inbox" => this.open_inbox(window, cx),
                            "Runs" => this.show_runs(window, cx),
                            "Notes" => this.open_notes(window, cx),
                            "Automations" => this.open_automations(window, cx),
                            "Flows" => this.open_flows(window, cx),
                            "Board" => this.open_board(window, cx),
                            _ => (),
                        }))
                        .on_hover(cx.listener(move |this, hovered, _, cx| {
                            if *hovered {
                                this.sidebar_hover = Some(label);
                            } else if this.sidebar_hover == Some(label) {
                                this.sidebar_hover = None;
                            }
                            this.notify_navigation(cx);
                        }))
                }),
            )
            .into_any()
    }
    /// The row that starts a thread in this project, styled like the
    /// navigation rows above it.
    fn sidebar_new_thread(&mut self, cx: &mut Context<Self>) -> AnyElement {
        const LABEL: &str = "New thread";
        let selected = self.active_session_is_blank(cx);
        div()
            .w_full()
            .px_2()
            .pt(px(8.))
            .pb(px(6.))
            .child(
                div()
                    .id("sidebar-new-thread")
                    .debug_selector(|| "sidebar-new-thread".into())
                    .role(Role::Button)
                    .aria_label(LABEL)
                    .aria_selected(selected)
                    .cursor_pointer()
                    .h(px(32.))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(if selected || self.sidebar_hover == Some(LABEL) {
                        1.
                    } else {
                        0.5
                    }))
                    .when(selected, |d| d.bg(content(0.1)))
                    .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                    .child(icon("pencil-edit-02", 16.).text_color(nav_icon_color(LABEL, selected)))
                    .child(div().flex_1().child(LABEL))
                    .child(
                        div()
                            .debug_selector(|| "sidebar-navigation-trailing-New thread".into())
                            .w(px(TRAILING_SLOT))
                            .flex_shrink_0()
                            .flex()
                            .justify_center()
                            .text_size(px(11.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(content(0.35))
                            .child(format!("{MOD}T")),
                    )
                    .on_hover(cx.listener(|this, hovered, _, cx| {
                        if *hovered {
                            this.sidebar_hover = Some(LABEL);
                        } else if this.sidebar_hover == Some(LABEL) {
                            this.sidebar_hover = None;
                        }
                        this.notify_navigation(cx);
                    }))
                    .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx))),
            )
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

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::Row;
    use gpui::TestAppContext;

    #[gpui::test]
    fn focused_navigation_keeps_the_settings_hint_on_the_filter_axis(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let axis = visual
            .debug_bounds("sidebar-filter-button")
            .unwrap()
            .center()
            .x;
        // New thread shows its shortcut in the same trailing column.
        assert!(visual.debug_bounds("sidebar-new-thread").is_some());
        let hint = visual
            .debug_bounds("sidebar-navigation-trailing-New thread")
            .expect("New thread shows its shortcut");
        assert_eq!(hint.center().x, axis);
        let settings = visual
            .debug_bounds("sidebar-navigation-trailing-Settings")
            .expect("Settings shows its shortcut");
        assert_eq!(settings.center().x, axis);
        // Notes is a plain row: no count beside it.
        assert!(visual.debug_bounds("sidebar-navigation-Notes").is_some());
        assert!(
            visual
                .debug_bounds("sidebar-navigation-trailing-Notes")
                .is_none()
        );
    }
    fn nav_order(visual: &mut gpui::VisualTestContext) -> Vec<&'static str> {
        let mut found: Vec<_> = [
            ("Inbox", "sidebar-navigation-Inbox"),
            ("Runs", "sidebar-navigation-Runs"),
            ("Automations", "sidebar-navigation-Automations"),
            ("Flows", "sidebar-navigation-Flows"),
            ("Board", "sidebar-navigation-Board"),
            ("Notes", "sidebar-navigation-Notes"),
            ("Settings", "sidebar-navigation-Settings"),
        ]
        .into_iter()
        .filter_map(|(label, selector)| {
            visual
                .debug_bounds(selector)
                .map(|bounds| (bounds.top(), label))
        })
        .collect();
        found.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        found.into_iter().map(|(_, label)| label).collect()
    }

    fn cx_flows_visible(
        window: gpui::WindowHandle<Shell>,
        visual: &mut gpui::VisualTestContext,
    ) -> bool {
        window
            .update(visual, |_, _, cx| crate::flows::visible(cx))
            .unwrap()
    }

    #[gpui::test]
    fn all_projects_block_lists_global_entries_in_order(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let label = visual
            .debug_bounds("sidebar-all-projects")
            .expect("the block is labelled");
        let mut expected = vec!["Inbox", "Runs", "Automations", "Board", "Notes"];
        if cx_flows_visible(window, visual) {
            expected.insert(3, "Flows");
        }
        expected.push("Settings");
        let order = nav_order(visual);
        assert_eq!(order, expected);
        let first = visual.debug_bounds("sidebar-navigation-Inbox").unwrap();
        assert!(label.bottom() <= first.top(), "label heads the entries");
        assert!(visual.debug_bounds("sidebar-navigation-New").is_none());
    }

    #[gpui::test]
    fn hidden_entries_stay_hidden_but_settings_is_always_shown(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.sidebar_inbox_visible = false;
                shell.workspace.sidebar_automations_visible = false;
                shell.workspace.preferences.notes = false;
                shell.sidebar_frame.update(cx, |_, cx| cx.notify());
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        let order = nav_order(visual);
        assert!(!order.contains(&"Inbox"));
        assert!(!order.contains(&"Automations"));
        assert!(!order.contains(&"Notes"));
        assert_eq!(order.last(), Some(&"Settings"));
    }

    #[gpui::test]
    fn project_row_sits_between_all_projects_and_new_thread(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let rows = window
            .update(cx, |shell, _, cx| {
                shell.refresh_sidebar_structure(cx);
                shell.sidebar_list.rows.clone()
            })
            .unwrap();
        assert_eq!(
            rows[..4],
            [Row::Navigation, Row::Project, Row::NewThread, Row::Pilots]
        );
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let all = visual.debug_bounds("sidebar-all-projects").unwrap();
        let settings = visual.debug_bounds("sidebar-navigation-Settings").unwrap();
        let divider = visual.debug_bounds("sidebar-project-divider").unwrap();
        let project = visual.debug_bounds("sidebar-project-button").unwrap();
        let gear = visual.debug_bounds("sidebar-project-settings").unwrap();
        let new = visual.debug_bounds("sidebar-new-thread").unwrap();
        assert!(all.bottom() <= settings.top());
        assert!(settings.bottom() <= divider.top());
        assert!(divider.size.height <= px(1.));
        assert!(divider.bottom() <= project.top());
        assert!(project.bottom() <= new.top());
        assert!(project.right() <= gear.left());
        assert!((gear.center().y - project.center().y).abs() <= px(1.));
        assert!(gear.bottom() <= new.top());
    }

    #[gpui::test]
    fn project_row_matches_the_canvas_controls(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let project = visual.debug_bounds("sidebar-project-button").unwrap();
        let gear = visual.debug_bounds("sidebar-project-settings").unwrap();
        let sidebar = visual.debug_bounds("sidebar-content").unwrap();
        assert!((project.size.height - gear.size.height).abs() <= px(0.5));
        assert!((project.top() - gear.top()).abs() <= px(0.5));
        assert!((gear.size.width - gear.size.height).abs() <= px(0.5));
        let gap = gear.left() - project.right();
        assert!(gap >= px(4.) && gap <= px(8.), "gap {gap:?}");
        // The dropdown takes the room the gear leaves: it is wide, not a
        // text-width chip.
        assert!(project.left() > sidebar.left());
        assert!(project.size.width > px(100.), "{project:?}");
        assert!(gear.right() <= sidebar.right());
        assert!(
            sidebar.right() - gear.right() <= px(16.),
            "{gear:?} {sidebar:?}"
        );
    }

    #[gpui::test]
    fn all_projects_line_shows_only_its_label(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.sidebar_list.invalidate_structure();
                cx.notify();
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let sidebar = visual.debug_bounds("sidebar-content").unwrap();
        let all = visual.debug_bounds("sidebar-all-projects").unwrap();
        // The list starts at the top: nothing sits above the label line.
        assert!(all.top() - sidebar.top() <= px(8.), "{all:?} {sidebar:?}");
        for gone in ["sidebar-search-button", "sidebar-options"] {
            assert!(visual.debug_bounds(gone).is_none(), "{gone} left the line");
        }
        // The label runs the line's width, with nothing at its right end.
        assert!(
            all.right() >= sidebar.right() - px(16.),
            "{all:?} {sidebar:?}"
        );
    }

    #[gpui::test]
    fn new_thread_row_matches_the_navigation_rows(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let inbox = visual.debug_bounds("sidebar-navigation-Inbox").unwrap();
        let new = visual.debug_bounds("sidebar-new-thread").unwrap();
        assert_eq!(new.left(), inbox.left());
        assert_eq!(new.size.width, inbox.size.width);
        assert_eq!(new.size.height, inbox.size.height);
        // Same hint column as Settings.
        let hint = visual
            .debug_bounds("sidebar-navigation-trailing-New thread")
            .unwrap();
        let settings = visual
            .debug_bounds("sidebar-navigation-trailing-Settings")
            .unwrap();
        assert_eq!(hint.center().x, settings.center().x);
        assert_eq!(hint.size.width, settings.size.width);
        // Its own row: space above (project row) and below (Pilots).
        let project = visual.debug_bounds("sidebar-project-button").unwrap();
        assert!(new.top() - project.bottom() >= px(4.));
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
    fn gear_opens_project_settings_beside_the_project_button(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let project = visual.debug_bounds("sidebar-project-button").unwrap();
        let gear = visual.debug_bounds("sidebar-project-settings").unwrap();
        assert!(project.right() <= gear.left());
        assert!((gear.center().y - project.center().y).abs() <= px(1.));
        assert!((gear.size.width - gear.size.height).abs() <= px(1.));
        visual.simulate_click(gear.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                let open = shell.menu.as_ref().expect("the gear opened a menu");
                assert!(
                    open.target
                        == MenuTarget::Project {
                            path: shell.workspace.project.clone(),
                        }
                );
            })
            .unwrap();
        // A second click on the gear closes it.
        visual.simulate_click(gear.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| assert!(shell.menu.is_none()))
            .unwrap();
    }

    #[gpui::test]
    fn new_thread_sits_below_navigation_and_marks_blank_sessions(cx: &mut TestAppContext) {
        let (window, _, active) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let new = visual.debug_bounds("sidebar-new-thread").unwrap();
        let inbox = visual.debug_bounds("sidebar-navigation-Inbox").unwrap();
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
        assert!(!selected(visual), "Inbox takes the highlight");
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
    fn compact_view_switch_changes_card_heights_and_persists_without_loading_history(
        cx: &mut TestAppContext,
    ) {
        let (window, _, _) = super::super::tests::window(cx);
        let (id, loaded) = window
            .update(cx, |shell, _, cx| {
                let mut summary = locked(&shell.store)
                    .unwrap()
                    .create("codex", "/work/gespan")
                    .unwrap();
                summary.has_messages = true;
                summary.title =
                    "A detailed session with a long title that must stay inside its card".into();
                let id = summary.id;
                shell.session_details.insert(
                    id,
                    gespan_core::store::MobileSessionMetadata {
                        model: Some("gpt-6.1-sol".into()),
                        branch: Some("feature/session-cards".into()),
                        ..Default::default()
                    },
                );
                shell.session_summaries.insert(id, summary);
                shell.order.insert(0, id);
                shell.notify_sidebar(cx);
                (id, shell.sessions.len())
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        let selector: &'static str = Box::leak(format!("sidebar-session-{id}").into_boxed_str());
        let model_selector: &'static str =
            Box::leak(format!("session-model-{id}").into_boxed_str());
        let workspace_selector: &'static str =
            Box::leak(format!("session-workspace-{id}").into_boxed_str());
        let title_selector: &'static str =
            Box::leak(format!("session-title-{id}").into_boxed_str());
        for boxy in [false, true] {
            window
                .update(visual, |shell, _, cx| {
                    shell.workspace.new_layout_boxy = boxy;
                    shell.layout_changed(cx);
                })
                .unwrap();
            visual.run_until_parked();
            assert_eq!(visual.debug_bounds(selector).unwrap().size.height, px(32.));
            flip_in_settings(window, visual, "compact-session-cards");
            let row = visual.debug_bounds(selector).unwrap();
            assert_eq!(row.size.height, px(60.));
            let model = visual.debug_bounds(model_selector).unwrap();
            let workspace = visual.debug_bounds(workspace_selector).unwrap();
            let title = visual.debug_bounds(title_selector).unwrap();
            assert!(row.top() <= title.top() && title.bottom() <= workspace.top());
            assert_eq!(model.center().y, workspace.center().y);
            assert!(workspace.right() <= model.left());
            assert!(workspace.bottom() <= row.bottom());
            assert!(
                model.right() <= row.right()
                    && workspace.right() <= row.right()
                    && title.right() <= row.right()
            );
            window
                .update(visual, |shell, _, _| {
                    assert_eq!(shell.sessions.len(), loaded);
                    assert!(!shell.sessions.contains_key(&id));
                    let saved = shell.latest_state.lock().unwrap().clone();
                    let restored = Workspace::restore(Some(&saved), &[], "/work/gespan");
                    assert!(!restored.compact_session_cards);
                })
                .unwrap();
            flip_in_settings(window, visual, "compact-session-cards");
            assert_eq!(visual.debug_bounds(selector).unwrap().size.height, px(32.));
            assert!(visual.debug_bounds(model_selector).is_none());
        }
    }

    #[gpui::test]
    fn each_sidebar_row_turns_its_feature_off(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let field = |shell: &Shell, id: &str| match id {
            "sidebar-notes" => shell.workspace.preferences.notes,
            "sidebar-board" => shell.workspace.preferences.board,
            "sidebar-pilots" => shell.workspace.preferences.pilots,
            _ => shell.workspace.preferences.working_agents,
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
        // Activity has no entry without running chats; the preference is the feature.
        flip_in_settings(window, visual, "sidebar-activity");
        window
            .update(visual, |s, _, _| assert!(!field(s, "sidebar-activity")))
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
        // sidebar, then the compact-view switch on its own row.
        let ids = [
            "sidebar-inbox",
            "sidebar-automations",
            "sidebar-pilots",
            "sidebar-board",
            "sidebar-notes",
            "sidebar-activity",
            "compact-session-cards",
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
            ("sidebar-inbox", "sidebar-navigation-Inbox"),
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
                    [false, true, false, false, false, true, true]
                );
                shell.close_settings(w, cx);
            })
            .unwrap();
    }

    #[gpui::test]
    fn activity_visibility_switch_preserves_agents_and_survives_restart(cx: &mut TestAppContext) {
        let (window, a, b) = super::super::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                for id in [a, b] {
                    shell.sessions[&id].update(cx, |session, _| session.busy = true);
                }
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
            assert!(visual.debug_bounds("agents-footer").is_some());
            flip_in_settings(window, visual, "sidebar-activity");
            assert!(visual.debug_bounds("agents-footer").is_none());
            window
                .update(visual, |shell, _, cx| {
                    assert!(!shell.workspace.preferences.working_agents);
                    for id in [a, b] {
                        assert!(shell.sessions[&id].read(cx).responding());
                    }
                    let saved = shell.latest_state.lock().unwrap().clone();
                    let restored = Workspace::restore(Some(&saved), &[], "/work/gespan");
                    assert!(!restored.preferences.working_agents);
                })
                .unwrap();
            flip_in_settings(window, visual, "sidebar-activity");
            assert!(visual.debug_bounds("agents-footer").is_some());
            window
                .update(visual, |shell, _, _| {
                    let saved = shell.latest_state.lock().unwrap().clone();
                    let restored = Workspace::restore(Some(&saved), &[], "/work/gespan");
                    assert!(restored.preferences.working_agents);
                })
                .unwrap();
        }
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.preferences.working_agents = false;
                shell.invalidate_chrome(cx);
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("agents-footer").is_none());
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

    #[test]
    fn worktree_groups_keep_the_list_order_and_lead_with_the_newest_start() {
        let listed = |id, created_at, path: &str| super::Listed {
            id,
            created_at,
            path: path.into(),
        };
        // The Gespan store's ids carry no order; its start times do.
        let sessions = [
            listed(900, 40, "/work/app"),
            listed(15, 90, "/work/app-worktrees/fix"),
            listed(700, 20, "/work/app"),
            listed(42, 70, "/work/app-worktrees/old"),
            listed(3, 10, "/work/app-worktrees/fix/"),
        ];
        assert_eq!(
            super::worktree_groups(&sessions),
            [
                ("/work/app-worktrees/fix".to_owned(), vec![15, 3]),
                ("/work/app-worktrees/old".to_owned(), vec![42]),
                ("/work/app".to_owned(), vec![900, 700]),
            ]
        );
        // Without start times, the newest id leads.
        let untimed = [
            listed(4, 0, "/work/app"),
            listed(9, 0, "/work/app-worktrees/fix"),
        ];
        assert_eq!(
            super::worktree_groups(&untimed)
                .into_iter()
                .map(|(_, ids)| ids)
                .collect::<Vec<_>>(),
            [vec![9], vec![4]]
        );
        assert!(super::worktree_groups(&[]).is_empty());
    }

    /// Both test sessions, with messages, in the project; `worktree` puts the
    /// first one in a linked worktree.
    fn grouped_rows(
        window: &gpui::WindowHandle<Shell>,
        cx: &mut TestAppContext,
        ids: [i64; 2],
        started: [u64; 2],
        worktree: Option<&str>,
    ) -> Vec<Row> {
        window
            .update(cx, |shell, _, cx| {
                for (id, created_at) in ids.into_iter().zip(started) {
                    shell.sessions[&id].update(cx, |session, _| {
                        session.summary.cwd = "/work/gespan".into();
                        session.summary.has_messages = true;
                        session.summary.created_at = created_at;
                    });
                    if let Some(summary) = shell.session_summaries.get_mut(&id) {
                        summary.cwd = "/work/gespan".into();
                        summary.has_messages = true;
                        summary.created_at = created_at;
                    }
                }
                shell
                    .session_worktrees
                    .insert(ids[0], worktree.map(str::to_owned));
                shell.sidebar_list.invalidate_structure();
                shell.refresh_sidebar_structure(cx);
                let start = shell
                    .sidebar_list
                    .rows
                    .iter()
                    .position(|row| {
                        matches!(
                            row,
                            Row::Heading {
                                label: "Sessions",
                                ..
                            }
                        )
                    })
                    .unwrap();
                shell.sidebar_list.rows[start + 1..].to_vec()
            })
            .unwrap()
    }

    #[gpui::test]
    fn sessions_group_by_worktree_and_a_focused_worktree_hides_none(cx: &mut TestAppContext) {
        let (window, a, b) = super::super::tests::window(cx);
        let tree = "/work/gespan-worktrees/feature";
        // The old project dropdown's worktree choice no longer narrows the list.
        window
            .update(cx, |shell, _, _| {
                shell.worktree_focus.insert(
                    "/work/gespan".into(),
                    crate::worktree_switcher::WorktreeFocus {
                        path: "/work/gespan".into(),
                        branch: Some("main".into()),
                    },
                );
            })
            .unwrap();
        let (tree_group, folder_group) = (
            vec![Row::Worktree(tree.into()), Row::Session(a)],
            vec![Row::Worktree("/work/gespan".into()), Row::Session(b)],
        );
        // The worktree where the newest session started leads, whatever the ids.
        let rows = grouped_rows(&window, cx, [a, b], [200, 100], Some(tree));
        assert_eq!(rows, [tree_group.clone(), folder_group.clone()].concat());
        let rows = grouped_rows(&window, cx, [a, b], [100, 200], Some(tree));
        assert_eq!(rows, [folder_group, tree_group].concat());
        let label = window
            .update(cx, |shell, _, _| shell.worktree_label(tree))
            .unwrap();
        assert_eq!(
            label, "feature",
            "the folder name until the branch is known"
        );
    }

    #[gpui::test]
    fn sessions_in_the_project_folder_alone_take_no_worktree_headings(cx: &mut TestAppContext) {
        let (window, a, b) = super::super::tests::window(cx);
        let rows = grouped_rows(&window, cx, [a, b], [100, 200], None);
        assert!(
            !rows.iter().any(|row| matches!(row, Row::Worktree(_))),
            "{rows:?}"
        );
        let mut sessions: Vec<_> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Session(id) => Some(*id),
                _ => None,
            })
            .collect();
        sessions.sort();
        let mut both = vec![a, b];
        both.sort();
        assert_eq!(sessions, both);
    }

    #[gpui::test]
    fn sessions_of_any_age_share_one_sessions_heading(cx: &mut TestAppContext) {
        let (window, a, b) = super::super::tests::window(cx);
        let day = 24 * 60 * 60 * 1000;
        let rows = window
            .update(cx, |shell, _, cx| {
                // One chat used just now, the other a month ago.
                let now = gespan_core::time::now_ms();
                for (id, updated_at) in [(a, now), (b, now - 30 * day)] {
                    shell.sessions[&id].update(cx, |session, _| {
                        session.summary.cwd = "/work/gespan".into();
                        session.summary.updated_at = updated_at;
                        session.summary.has_messages = true;
                    });
                    if let Some(summary) = shell.session_summaries.get_mut(&id) {
                        summary.cwd = "/work/gespan".into();
                        summary.updated_at = updated_at;
                        summary.has_messages = true;
                    }
                }
                shell.sidebar_list.invalidate_structure();
                shell.refresh_sidebar_structure(cx);
                shell.sidebar_list.rows.clone()
            })
            .unwrap();
        let headings: Vec<_> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Heading { label, .. } => Some(*label),
                _ => None,
            })
            .collect();
        assert_eq!(headings, ["Sessions"]);
        let heading = rows
            .iter()
            .position(|row| matches!(row, Row::Heading { .. }))
            .unwrap();
        for id in [a, b] {
            let at = rows
                .iter()
                .position(|row| matches!(row, Row::Session(s) if *s == id))
                .unwrap_or_else(|| panic!("session {id} is listed"));
            assert!(at > heading, "session {id} sits under Sessions");
        }
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
                .filter(|row| {
                    matches!(
                        row,
                        Row::MachinesHeading | Row::Machine(_) | Row::MachineAgent { .. }
                    )
                })
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
        fn machines_follow_pilots_with_agents_in_workspace_order(cx: &mut TestAppContext) {
            let (window, _) = window(vec![two_agents(Online::Online)], true, cx);
            let rows = rows(&window, cx);
            assert_eq!(
                rows[..8],
                [
                    Row::Navigation,
                    Row::Project,
                    Row::NewThread,
                    Row::Pilots,
                    Row::MachinesHeading,
                    Row::Machine("m1".into()),
                    agent_row("a"),
                    agent_row("b"),
                ]
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
                [
                    Row::MachinesHeading,
                    Row::Machine("m1".into()),
                    agent_row("a"),
                    agent_row("b"),
                ]
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
            assert_eq!(machine_rows(&rows(&window, cx)).len(), 4);
            // `b` closed and `a` started working.
            let next = view(
                Online::Online,
                vec![agent("a", "w1", AgentStatus::Working, Some(3))],
            );
            cx.update(|cx| crate::machines::set_for_test(&machines, vec![next], None, cx));
            cx.run_until_parked();
            assert_eq!(
                machine_rows(&rows(&window, cx)),
                [
                    Row::MachinesHeading,
                    Row::Machine("m1".into()),
                    agent_row("a"),
                ]
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

    mod fixed_folders {
        use super::super::super::session_groups::RenameTarget;
        use super::super::super::*;
        use super::super::Row;
        use gespan_core::pilot::{Launch, Pilot};
        use gespan_core::session_groups::Entry;
        use gpui::{TestAppContext, VisualTestContext, WindowHandle};

        const AUTOMATIONS: &str = "fixed:automations";

        /// Both fixture sessions listed in the open project.
        fn setup(cx: &mut TestAppContext) -> (WindowHandle<Shell>, i64, i64) {
            let (window, a, b) = super::super::super::tests::window(cx);
            window
                .update(cx, |shell, _, cx| {
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

        /// The fixed folder headers, as (id, name, sessions).
        fn fixed(rows: &[Row]) -> Vec<(String, String, Vec<i64>)> {
            rows.iter()
                .filter_map(|row| match row {
                    Row::Group(Entry::Fixed { id, name, sessions }) => {
                        Some((id.clone(), name.clone(), sessions.clone()))
                    }
                    _ => None,
                })
                .collect()
        }

        fn grouped(rows: &[Row]) -> Vec<i64> {
            rows.iter()
                .filter_map(|row| match row {
                    Row::GroupedSession { id, .. } => Some(*id),
                    _ => None,
                })
                .collect()
        }

        fn loose(rows: &[Row]) -> Vec<i64> {
            rows.iter()
                .filter_map(|row| match row {
                    Row::Session(id) => Some(*id),
                    _ => None,
                })
                .collect()
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
            assert!(rows.contains(&Row::Group(Entry::Folder {
                id: "work".into(),
                sessions: vec![a],
            })));
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
                .debug_bounds("session-group-fixed:automations")
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
            assert_eq!(
                fixed(&listed),
                [(AUTOMATIONS.into(), "Automation runs".into(), vec![a])]
            );
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
            rows.iter()
                .filter_map(|row| match row {
                    Row::Group(Entry::Folder { id, sessions }) => {
                        Some((id.clone(), sessions.clone()))
                    }
                    _ => None,
                })
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
            assert_eq!(
                fixed(&rows),
                [("fixed:pilot:scout".into(), "Scout".into(), vec![a])]
            );
            assert!(
                folders(&rows).is_empty(),
                "no user folder named after the Pilot"
            );
            assert_eq!(loose(&rows), [b]);
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
            let mut members = fixed(&rows)
                .into_iter()
                .find(|(id, ..)| id == "fixed:pilot:scout")
                .map(|(_, _, sessions)| sessions)
                .unwrap();
            members.sort();
            let mut both = vec![a, b];
            both.sort();
            assert_eq!(members, both);
            assert!(folders(&rows).is_empty());
            assert!(loose(&rows).is_empty());
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
            assert_eq!(
                fixed(&rows),
                [("fixed:pilot:scout".into(), "Scout".into(), vec![a])]
            );
            assert_eq!(folders(&rows), [("folder:mine".to_string(), vec![b])]);
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
