//! Only construct and lay out sidebar cards near the viewport.
use super::*;
use crate::machines::sidebar as machine_rows;
use gespan_core::session_groups::Entry;
use gpui::{ListAlignment, ListState, list};
use std::cell::{Cell, RefCell};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Row {
    Navigation,
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
    GroupedSession {
        id: i64,
        first: bool,
        last: bool,
    },
    ClassicGroupMember {
        entry: Entry,
        id: i64,
        first: bool,
        last: bool,
    },
    ClassicFolderNew(String),
    ClassicGap(usize),
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

fn age_label(updated_at: u64, now: u64) -> &'static str {
    let today = chrono::DateTime::from_timestamp_millis(now.min(i64::MAX as u64) as i64)
        .map(|t| t.with_timezone(&chrono::Local).date_naive());
    let day = chrono::DateTime::from_timestamp_millis(updated_at.min(i64::MAX as u64) as i64)
        .map(|t| t.with_timezone(&chrono::Local).date_naive());
    match today.zip(day).map(|(today, day)| (today - day).num_days()) {
        Some(days) if days <= 0 => "Today",
        Some(1) => "Yesterday",
        Some(2..=6) => "Last 7 days",
        _ => "Older",
    }
}

// Match sidebar-session-scroll: 40px below, and 28px above scrolled Focused rows.
// Mask only the edge bands so the virtual list never needs a full-pane copy.
fn sidebar_edge_mask(
    state: ListState,
    top_fade: bool,
    #[cfg_attr(target_os = "macos", allow(unused_variables))] inset: bool,
    begin: bool,
) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let offset = state.logical_scroll_top();
            let bottom_height = px(40.).min(bounds.size.height);
            let top_height = if top_fade && (offset.item_ix > 0 || offset.offset_in_item > px(0.)) {
                px(28.).min((bounds.size.height - bottom_height).max(px(0.)))
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
            "{}\0{}\0{}\0{:?}\0{}\0{}\0{}",
            self.workspace.project,
            self.working_copy(),
            self.query,
            self.workspace.session_filters,
            self.workspace.preferences.show_archived,
            self.workspace.boxy_layout(),
            self.workspace.classic_layout
        )
    }

    pub(super) fn prepare_sidebar_list(&mut self, entries: &[Entry], cx: &App) {
        if self.workspace.classic_layout {
            self.prepare_classic_sidebar_list(entries, cx);
            return;
        }
        let groups = self.project_groups();
        let now = gespan_core::time::now_ms();
        let mut rows = vec![Row::Navigation, Row::Pilots];
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
                Entry::Session(_) => continue,
            };
            if !folders {
                rows.push(Row::Folders);
                folders = true;
            }
            rows.push(Row::Group(entry.clone()));
            if !collapsed || !self.query.is_empty() {
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
        let mut previous = None;
        let mut first = true;
        for entry in entries {
            let Entry::Session(id) = entry else {
                continue;
            };
            if self.rails.children.contains(id) {
                continue;
            }
            let label = age_label(self.session_metadata(*id, cx).summary.updated_at, now);
            if previous != Some(label) {
                rows.push(Row::Heading { label, first });
                previous = Some(label);
                first = false;
            }
            rows.push(Row::Session(*id));
        }
        if first {
            rows.push(Row::Heading {
                label: "Sessions",
                first: true,
            });
            if entries.is_empty() {
                rows.push(Row::Empty);
            }
        }
        self.sidebar_list.update(self.sidebar_scope(), rows);
    }

    fn prepare_classic_sidebar_list(&mut self, entries: &[Entry], cx: &App) {
        let groups = self.project_groups();
        // The original sidebar: the list has p-1.5, gap-0.5 and pb-10.
        // Its project navigation and Pilot roster belong to the first rail.
        let mut rows = vec![Row::ClassicGap(6)];
        for (ix, entry) in entries.iter().enumerate() {
            if ix > 0 {
                rows.push(Row::ClassicGap(2));
            }
            let (members, collapsed) = match entry {
                Entry::Session(id) => {
                    rows.push(Row::Session(*id));
                    continue;
                }
                Entry::Pinned { sessions } => (sessions, groups.pinned_collapsed),
                Entry::Reminders {
                    sessions,
                    collapsed,
                } => (sessions, *collapsed),
                Entry::Folder { id, sessions } => (
                    sessions,
                    groups.folder(id).is_some_and(|folder| folder.collapsed),
                ),
            };
            rows.push(Row::Group(entry.clone()));
            let expanded = !collapsed || !self.query.is_empty();
            if expanded {
                let members: Vec<_> = members.to_vec();
                rows.extend(
                    members
                        .iter()
                        .enumerate()
                        .map(|(ix, id)| Row::ClassicGroupMember {
                            entry: entry.clone(),
                            id: *id,
                            first: ix == 0,
                            last: ix + 1 == members.len(),
                        }),
                );
                if let Entry::Folder { id, .. } = entry {
                    rows.push(Row::ClassicFolderNew(id.clone()));
                }
            }
            if expanded || matches!(entries.get(ix + 1), Some(Entry::Session(_))) {
                rows.push(Row::ClassicGap(6));
            }
        }
        if entries.is_empty() {
            rows.push(Row::Empty);
        }
        push_machine_rows(&mut rows, cx);
        rows.push(Row::ClassicGap(40));
        self.sidebar_list.update(self.sidebar_scope(), rows);
    }

    fn sync_sidebar_list_visibility(&mut self) {
        // Open rails have variable height. Remeasure only materialized rows,
        // rather than scanning every session in the project every frame.
        for ix in self.sidebar_list.visible_rows.borrow_mut().drain(..) {
            if let Some(
                Row::Session(id)
                | Row::GroupedSession { id, .. }
                | Row::ClassicGroupMember { id, .. },
            ) = self.sidebar_list.rows.get(ix)
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
                !self.workspace.classic_layout,
                !self.workspace.boxy_layout(),
                true,
            ))
            .child(
                list(
                    self.sidebar_list.state.clone(),
                    cx.processor(|this, ix, _, cx| this.sidebar_list_row(ix, cx)),
                )
                .size_full(),
            )
            .child(sidebar_edge_mask(
                self.sidebar_list.state.clone(),
                !self.workspace.classic_layout,
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
                .when(first, |d| {
                    d.child(self.sidebar_new_session_button(cx))
                        .child(self.sidebar_filter_button(cx))
                })
                .into_any(),
            Row::Session(id) => self.session_card(id, cx),
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
            Row::ClassicGroupMember {
                entry,
                id,
                first,
                last,
            } => {
                return div()
                    .px(px(6.))
                    .child(self.classic_group_member(&entry, id, first, last, cx))
                    .into_any();
            }
            Row::ClassicFolderNew(id) => {
                return div()
                    .px(px(6.))
                    .child(self.classic_folder_new_session(&id, cx))
                    .into_any();
            }
            Row::ClassicGap(height) => return div().h(px(height as f32)).into_any(),
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
                .px_2()
                .py_2()
                .text_size(px(12.))
                .text_color(rgb(MUTED))
                .child(if self.query.is_empty() {
                    "No sessions yet"
                } else {
                    "No matching sessions"
                })
                .into_any(),
        };
        div()
            .when(self.workspace.classic_layout, |d| d.px(px(6.)))
            .when(!self.workspace.classic_layout, |d| d.px_2().pb_px())
            .child(content)
            .into_any()
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
    pub(super) fn sidebar_options_menu_items(&self) -> Vec<MenuItem> {
        vec![
            MenuAction::new("sidebar-search", "Search")
                .icon("search")
                .toggle(self.workspace.sidebar_search_visible)
                .into(),
            MenuAction::new("sidebar-notes", "Notes")
                .icon("note")
                .toggle(self.workspace.sidebar_notes_visible)
                .disabled(!self.workspace.preferences.notes)
                .into(),
            MenuAction::new("sidebar-automations", "Automations")
                .icon("zap")
                .toggle(self.workspace.sidebar_automations_visible)
                .into(),
            MenuAction::new("sidebar-inbox", "Inbox")
                .icon("inbox")
                .toggle(self.workspace.sidebar_inbox_visible)
                .into(),
            MenuAction::new("sidebar-pilots", "Pilots")
                .icon("ghost")
                .toggle(self.workspace.preferences.pilots)
                .into(),
            MenuAction::new("sidebar-activity", "Activity")
                .icon("response-activity")
                .toggle(self.workspace.sidebar_activity_visible)
                .disabled(!self.workspace.preferences.working_agents)
                .into(),
            MenuItem::Separator,
            MenuAction::new("compact-session-cards", "Compact view")
                .icon("list-bullet")
                .toggle(self.workspace.compact_session_cards)
                .into(),
        ]
    }

    pub(super) fn change_sidebar_option(&mut self, action: &str, cx: &mut Context<Self>) {
        let value = match action {
            "sidebar-search" => &mut self.workspace.sidebar_search_visible,
            "sidebar-notes" => &mut self.workspace.sidebar_notes_visible,
            "sidebar-automations" => &mut self.workspace.sidebar_automations_visible,
            "sidebar-inbox" => &mut self.workspace.sidebar_inbox_visible,
            "sidebar-pilots" => &mut self.workspace.preferences.pilots,
            "sidebar-activity" => &mut self.workspace.sidebar_activity_visible,
            "compact-session-cards" => &mut self.workspace.compact_session_cards,
            _ => return,
        };
        *value = !*value;
        if action == "sidebar-pilots" {
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
        // The Focused layout shows Search as an icon beside Sidebar options.
        let search_field = self.workspace.sidebar_search_visible && self.workspace.classic_layout;
        let focused = !self.workspace.classic_layout;
        let blank_session = focused && self.active_session_is_blank(cx);
        div()
            .w_full()
            .px_2()
            .pt(px(if self.workspace.classic_layout {
                2.
            } else {
                4.
            }))
            .pb(px(if self.workspace.classic_layout {
                8.
            } else {
                12.
            }))
            .flex()
            .flex_col()
            .gap_px()
            .when(search_field, |d| {
                d.child(
                    div()
                        .id("rail-search")
                        .debug_selector(|| "sidebar-search-field".into())
                        .group("rail-search")
                        .w_full()
                        .h(px(32.))
                        .mb(px(if self.workspace.classic_layout {
                            2.
                        } else {
                            4.
                        }))
                        .px_1p5()
                        .rounded_md()
                        .border_1()
                        .border_color(content(0.08))
                        // RailSearch's Tailwind shadow-sm, including
                        // its tighter second layer below the border.
                        // The vendored shader keeps it outside the
                        // box, as CSS does.
                        .shadow(vec![
                            BoxShadow::new(px(0.), px(1.), rgba(0x0000001a).into())
                                .blur_radius(px(3.)),
                            BoxShadow::new(px(0.), px(1.), rgba(0x0000001a).into())
                                .blur_radius(px(2.))
                                .spread_radius(px(-1.)),
                        ])
                        .text_color(content(if self.sidebar_hover == Some("Search") {
                            1.
                        } else {
                            0.5
                        }))
                        .font_weight(FontWeight::MEDIUM)
                        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                        .on_hover(cx.listener(|this, hovered, _, cx| {
                            this.sidebar_hover = if *hovered { Some("Search") } else { None };
                            this.notify_navigation(cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.focus_search(window, cx);
                            }),
                        )
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(14.))
                        .child(icon("search", 16.).text_color(nav_icon_color("Search", false)))
                        .child(div().flex_1().min_w_0().h(px(20.)).child("Search"))
                        .child(
                            // The shortcut span sits outside the label's `font-medium`.
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(content(0.4))
                                .child(if cfg!(target_os = "macos") {
                                    "⌘K"
                                } else {
                                    "Ctrl K"
                                }),
                        ),
                )
            })
            .children(
                [
                    ("New", "square-plus"),
                    ("Inbox", "inbox"),
                    ("Notes", "note"),
                    ("Automations", "zap"),
                    ("Flows", "git-merge"),
                    ("Board", "kanban"),
                ]
                .into_iter()
                .filter(|(label, _)| match *label {
                    // Only the Focused layout lists New here.
                    "New" => focused,
                    "Notes" => {
                        self.workspace.preferences.notes && self.workspace.sidebar_notes_visible
                    }
                    "Automations" => self.workspace.sidebar_automations_visible,
                    "Flows" => flows_visible,
                    "Board" => self.workspace.preferences.board,
                    "Inbox" => self.workspace.sidebar_inbox_visible,
                    _ => true,
                })
                .map(|(label, glyph)| {
                    let glyph = if self.workspace.classic_layout && label == "Notes" {
                        "file"
                    } else {
                        glyph
                    };
                    let selected = if label == "New" {
                        blank_session
                    } else {
                        self.app_view_label() == Some(label)
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
                        .aria_label(if unread { "Inbox, new items" } else { label })
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
                                "New" => Some(format!("{MOD}T").into_any_element()),
                                "Notes" => self
                                    .note_count
                                    .map(|count| count.to_string().into_any_element()),
                                "Inbox" if unread => Some(
                                    div()
                                        .flex_shrink_0()
                                        .size(px(if self.workspace.classic_layout {
                                            8.
                                        } else {
                                            6.
                                        }))
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
                            "New" => this.new_chat(cx),
                            "Inbox" => this.open_inbox(window, cx),
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
    fn notify_navigation(&self, cx: &mut Context<Self>) {
        if self.workspace.classic_layout {
            self.projects_frame.update(cx, |_, cx| cx.notify());
        } else {
            self.sidebar_frame.update(cx, |_, cx| cx.notify());
        }
    }
    /// New session in this project, just before the filter.
    pub(super) fn sidebar_new_session_button(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let classic = self.workspace.classic_layout;
        let new = if classic {
            div()
                .id("new-session")
                .role(Role::Button)
                .aria_label("New session")
                .cursor_pointer()
                .group("classic-new-session")
                .size(px(24.))
                .flex_shrink_0()
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .hover(|s| s.bg(content(0.1)))
                .child(
                    icon("plus", 12.)
                        .text_color(content(0.5))
                        .group_hover("classic-new-session", |s| s.text_color(content(1.))),
                )
        } else {
            icon_button("new-session", "New session", "plus")
        };
        new.debug_selector(|| "sidebar-new-session".into())
            .on_click(cx.listener(|this, _, _, cx| {
                let kind = this.default_kind(cx);
                this.create(kind, cx)
            }))
    }
    pub(super) fn sidebar_filter_button(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self
            .workspace
            .session_filters
            .active(self.workspace.preferences.show_archived)
            || self.menu_open(&MenuTarget::SessionFilter);
        let button = if self.workspace.classic_layout {
            div()
                .id("filter-sessions")
                .role(Role::Button)
                .aria_label("Filter sessions")
                .aria_expanded(self.menu_open(&MenuTarget::SessionFilter))
                .cursor_pointer()
                .group("classic-filter")
                .size(px(24.))
                .flex_shrink_0()
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .hover(|s| s.bg(content(0.1)))
                .child(
                    icon("filter", 12.)
                        .text_color(content(if active { 1. } else { 0.5 }))
                        .group_hover("classic-filter", |s| s.text_color(content(1.))),
                )
        } else {
            icon_button("filter-sessions", "Filter sessions", "filter")
        };
        button
            .debug_selector(|| "sidebar-filter-button".into())
            .mr(px(if self.workspace.classic_layout {
                0.
            } else {
                -5.
            }))
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
    use super::age_label;
    use chrono::TimeZone;
    use gpui::TestAppContext;

    #[gpui::test]
    fn focused_navigation_trailing_items_share_the_filter_axis(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        for count in [0, 128] {
            window
                .update(visual, |shell, _, cx| {
                    shell.note_count = Some(count);
                    shell.notify_sidebar(cx);
                })
                .unwrap();
            visual.run_until_parked();
            let axis = visual
                .debug_bounds("sidebar-filter-button")
                .unwrap()
                .center()
                .x;
            for selector in [
                "sidebar-navigation-trailing-New",
                "sidebar-navigation-trailing-Notes",
            ] {
                let trailing = visual.debug_bounds(selector).unwrap();
                assert_eq!(trailing.center().x, axis, "{selector} with {count} notes");
            }
        }
    }
    #[gpui::test]
    fn focused_search_sits_beside_sidebar_options(cx: &mut TestAppContext) {
        let (window, _, _) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("sidebar-search-field").is_none());
        let search = visual.debug_bounds("sidebar-search-button").unwrap();
        let options = visual.debug_bounds("sidebar-options").unwrap();
        assert!(search.right() <= options.left());
        assert!((search.center().y - options.center().y).abs() <= px(1.));
        visual.simulate_click(search.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| assert!(shell.app_search.is_some()))
            .unwrap();
    }

    #[gpui::test]
    fn focused_new_session_leads_navigation_and_marks_blank_sessions(cx: &mut TestAppContext) {
        let (window, _, active) = super::super::tests::window(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let new = visual.debug_bounds("sidebar-navigation-New").unwrap();
        let inbox = visual.debug_bounds("sidebar-navigation-Inbox").unwrap();
        assert!(new.bottom() <= inbox.top());
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
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.classic_layout = true;
                shell.invalidate_chrome(cx);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("sidebar-navigation-New").is_none());
    }

    #[gpui::test]
    fn sidebar_options_switch_card_heights_and_persist_without_loading_history(
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
            let options = visual.debug_bounds("sidebar-options").unwrap();
            visual.simulate_click(options.center(), Default::default());
            visual.run_until_parked();
            let compact = visual
                .debug_bounds("menu-action-compact-session-cards")
                .unwrap();
            visual.simulate_click(compact.center(), Default::default());
            visual.run_until_parked();
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
            // Switches keep the popover open for more choices.
            let compact = visual
                .debug_bounds("menu-action-compact-session-cards")
                .unwrap();
            visual.simulate_click(compact.center(), Default::default());
            visual.run_until_parked();
            assert_eq!(visual.debug_bounds(selector).unwrap().size.height, px(32.));
            assert!(visual.debug_bounds(model_selector).is_none());
            visual.simulate_click(options.center(), Default::default());
            visual.run_until_parked();
        }
    }

    #[gpui::test]
    fn sidebar_visibility_switches_stay_open_preserve_notes_and_survive_restart(
        cx: &mut TestAppContext,
    ) {
        let (window, _, _) = super::super::tests::window(cx);
        window
            .update(cx, |shell, w, cx| shell.open_notes(w, cx))
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let options = visual.debug_bounds("sidebar-options").unwrap();
        visual.simulate_click(options.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                let menu = shell.menu.as_ref().unwrap().menu.read(cx);
                let bounds = menu.surface_region().get().unwrap();
                assert_eq!(bounds.left(), options.left());
                assert_eq!(bounds.size.width, px(184.));
            })
            .unwrap();
        let search = visual.debug_bounds("menu-action-sidebar-search").unwrap();
        let notes = visual.debug_bounds("menu-action-sidebar-notes").unwrap();
        assert!(search.bottom() <= notes.top());
        let automations = visual
            .debug_bounds("menu-action-sidebar-automations")
            .unwrap();
        let inbox = visual.debug_bounds("menu-action-sidebar-inbox").unwrap();
        let pilots = visual.debug_bounds("menu-action-sidebar-pilots").unwrap();
        let activity = visual.debug_bounds("menu-action-sidebar-activity").unwrap();
        let compact = visual
            .debug_bounds("menu-action-compact-session-cards")
            .unwrap();
        assert!(notes.bottom() <= automations.top() && automations.bottom() <= inbox.top());
        assert!(inbox.bottom() <= pilots.top() && pilots.bottom() <= activity.top());
        assert!(
            activity.bottom() < compact.top(),
            "a divider separates Compact view"
        );
        for (action, navigation) in [
            ("menu-action-sidebar-search", "sidebar-search-button"),
            ("menu-action-sidebar-notes", "sidebar-navigation-Notes"),
            (
                "menu-action-sidebar-automations",
                "sidebar-navigation-Automations",
            ),
            ("menu-action-sidebar-inbox", "sidebar-navigation-Inbox"),
            ("menu-action-sidebar-pilots", "pilot-intro"),
        ] {
            assert!(visual.debug_bounds(navigation).is_some());
            let row = visual.debug_bounds(action).unwrap();
            visual.simulate_click(row.center(), Default::default());
            visual.run_until_parked();
            assert!(visual.debug_bounds(navigation).is_none());
            assert!(
                visual
                    .debug_bounds("menu-action-compact-session-cards")
                    .is_some()
            );
        }
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(shell.active_app_view, Some(app_views::AppView::Notes));
                assert!(shell.notes.is_some());
                assert!(
                    shell.workspace.preferences.notes,
                    "hiding the shortcut leaves Notes enabled"
                );
                let menu = shell.menu.as_ref().unwrap().menu.read(cx);
                for id in [
                    "sidebar-search",
                    "sidebar-notes",
                    "sidebar-automations",
                    "sidebar-inbox",
                    "sidebar-pilots",
                ] {
                    assert_eq!(menu.toggle_state(id), Some(false));
                }
                assert!(!cx.global::<crate::settings::Preferences>().pilots);
                let saved = shell.latest_state.lock().unwrap().clone();
                let restored = Workspace::restore(Some(&saved), &[], "/work/gespan");
                assert!(!restored.sidebar_search_visible);
                assert!(!restored.sidebar_notes_visible);
                assert!(!restored.sidebar_automations_visible);
                assert!(!restored.sidebar_inbox_visible);
                assert!(!restored.preferences.pilots);
            })
            .unwrap();
        for (action, navigation) in [
            ("menu-action-sidebar-search", "sidebar-search-button"),
            ("menu-action-sidebar-notes", "sidebar-navigation-Notes"),
            (
                "menu-action-sidebar-automations",
                "sidebar-navigation-Automations",
            ),
            ("menu-action-sidebar-inbox", "sidebar-navigation-Inbox"),
            ("menu-action-sidebar-pilots", "pilot-intro"),
        ] {
            let row = visual.debug_bounds(action).unwrap();
            visual.simulate_click(row.center(), Default::default());
            visual.run_until_parked();
            assert!(visual.debug_bounds(navigation).is_some());
        }
        visual.simulate_click(options.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, w, cx| {
                shell.open_settings(crate::settings::Section::Pilots, w, cx)
            })
            .unwrap();
        visual.run_until_parked();
        let toggle = visual
            .debug_bounds("settings-toggle-pilots-enabled")
            .unwrap();
        visual.simulate_click(toggle.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, w, cx| {
                assert!(!shell.workspace.preferences.pilots);
                shell.close_settings(w, cx);
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("pilot-intro").is_none());
        let options = visual.debug_bounds("sidebar-options").unwrap();
        visual.simulate_click(options.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                let menu = shell.menu.as_ref().unwrap().menu.read(cx);
                assert_eq!(menu.toggle_state("sidebar-pilots"), Some(false));
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
            assert!(visual.debug_bounds("sidebar-activity-card").is_some());
            let options = visual.debug_bounds("sidebar-options").unwrap();
            visual.simulate_click(options.center(), Default::default());
            visual.run_until_parked();
            let activity = visual.debug_bounds("menu-action-sidebar-activity").unwrap();
            visual.simulate_click(activity.center(), Default::default());
            visual.run_until_parked();
            assert!(visual.debug_bounds("sidebar-activity-card").is_none());
            window
                .update(visual, |shell, _, cx| {
                    assert!(shell.workspace.preferences.working_agents);
                    for id in [a, b] {
                        assert!(shell.sessions[&id].read(cx).responding());
                    }
                    let menu = shell.menu.as_ref().unwrap().menu.read(cx);
                    assert_eq!(menu.toggle_state("sidebar-activity"), Some(false));
                    let saved = shell.latest_state.lock().unwrap().clone();
                    let restored = Workspace::restore(Some(&saved), &[], "/work/gespan");
                    assert!(!restored.sidebar_activity_visible);
                })
                .unwrap();
            let activity = visual.debug_bounds("menu-action-sidebar-activity").unwrap();
            visual.simulate_click(activity.center(), Default::default());
            visual.run_until_parked();
            assert!(visual.debug_bounds("sidebar-activity-card").is_some());
            window
                .update(visual, |shell, _, _| {
                    let saved = shell.latest_state.lock().unwrap().clone();
                    let restored = Workspace::restore(Some(&saved), &[], "/work/gespan");
                    assert!(restored.sidebar_activity_visible);
                })
                .unwrap();
            visual.simulate_click(options.center(), Default::default());
            visual.run_until_parked();
        }
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.preferences.working_agents = false;
                shell.invalidate_chrome(cx);
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("sidebar-activity-card").is_none());
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
                shell.query.push_str("unmatched-session-query");
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
        let (first, last) = window
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
                let last = summaries[0].id;
                for summary in summaries {
                    shell.order.push(summary.id);
                    shell.session_summaries.insert(summary.id, summary);
                }
                shell.workspace.open(first);
                shell.select_project("/work/many-sessions".into(), window, cx);
                (first, last)
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
                shell.open(first - 1, window, cx);
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
                assert!(shell.session_row_bounds[&(first - 1)].get().is_none());
            })
            .unwrap();
    }

    #[test]
    fn sidebar_dates_use_local_calendar_days_and_match_the_tauri_sections() {
        let ms = |day, hour, minute| {
            chrono::Local
                .with_ymd_and_hms(2026, 10, day, hour, minute, 0)
                .single()
                .unwrap()
                .timestamp_millis() as u64
        };
        let now = ms(20, 0, 5);
        assert_eq!(age_label(ms(20, 0, 1), now), "Today");
        assert_eq!(age_label(ms(19, 23, 59), now), "Yesterday");
        assert_eq!(age_label(ms(14, 0, 0), now), "Last 7 days");
        assert_eq!(age_label(ms(13, 23, 59), now), "Older");
        assert_eq!(age_label(0, now), "Older");
        // A local calendar day remains Yesterday across daylight-saving changes.
        assert_eq!(age_label(ms(25, 0, 1), ms(26, 0, 5)), "Yesterday");
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
                rows[..6],
                [
                    Row::Navigation,
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
        fn machines_show_in_the_classic_sidebar_after_the_sessions(cx: &mut TestAppContext) {
            let (window, _) = window(vec![two_agents(Online::Online)], true, cx);
            window
                .update(cx, |shell, _, cx| {
                    shell.workspace.classic_layout = true;
                    shell.sidebar_list.invalidate_structure();
                    shell.refresh_sidebar_structure(cx);
                })
                .unwrap();
            let rows = rows(&window, cx);
            let at = rows
                .iter()
                .position(|r| *r == Row::MachinesHeading)
                .unwrap();
            assert_eq!(
                rows[at..],
                [
                    Row::MachinesHeading,
                    Row::Machine("m1".into()),
                    agent_row("a"),
                    agent_row("b"),
                    Row::ClassicGap(40),
                ]
            );
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
}
