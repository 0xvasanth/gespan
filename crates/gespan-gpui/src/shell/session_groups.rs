//! The sidebar's session actions: pins, renames, GitHub links, reminders,
//! folders and deletion, and the grouped list they shape.
use super::*;
#[cfg(test)]
use crate::ticket_chip::{GROUP_UNKNOWN, group_tone};
use chrono::{Local, NaiveDateTime, TimeZone};
#[cfg(test)]
use gespan_core::provider_boards::StateGroup;
use gespan_core::{
    session_groups::{
        Entry, FixedFolder, Groups, WorkItem, build_list, format_reminder, navigation_ids,
        parse_github_work_item, reminder_at, reminder_presets,
    },
    store::Reminder,
    tickets::{Resolved, TicketError, TicketRef, Tracker, parse_ticket},
};
use gpui::EventEmitter;

/// Fixed folder ids start with this; they are never user folders.
pub(super) const FIXED_PREFIX: &str = "fixed:";
pub(super) const AUTOMATION_RUNS: &str = "fixed:automations";

pub(super) const PILOT_PREFIX: &str = "fixed:pilot:";

/// Folds or opens a group header, keyed as the sidebar keys it.
pub(super) fn toggle_group(groups: &mut Groups, key: &str) {
    match key {
        "pinned" => groups.pinned_collapsed = !groups.pinned_collapsed,
        "reminders" => groups.reminders_collapsed = !groups.reminders_collapsed,
        fixed if fixed.starts_with(FIXED_PREFIX) => {
            let folded = groups.fixed_is_collapsed(fixed);
            groups.set_fixed_collapsed(fixed, !folded);
        }
        folder => groups.toggle_folder(folder.trim_start_matches("folder:")),
    }
}

actions!(
    session_groups,
    [CancelRename, RenameSidebarItem, DeleteSidebarSelection]
);

pub(super) fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CancelRename, Some("SessionRename")),
        KeyBinding::new("f2", RenameSidebarItem, Some("Workspace && !SessionRename")),
        KeyBinding::new(
            "backspace",
            DeleteSidebarSelection,
            Some("SessionSidebar && !Composer"),
        ),
        KeyBinding::new(
            "delete",
            DeleteSidebarSelection,
            Some("SessionSidebar && !Composer"),
        ),
    ]);
}

/// tabGroups.ts's TAB_GROUP_COLORS; 0 is the default wash.
const FOLDER_COLORS: [(f32, f32, f32); 9] = [
    (210., 0.08, 0.58),
    (211., 0.92, 0.62),
    (12., 0.80, 0.58),
    (45., 0.90, 0.55),
    (142., 0.55, 0.50),
    (330., 0.70, 0.62),
    (280., 0.55, 0.62),
    (175., 0.55, 0.48),
    (25., 0.85, 0.58),
];
fn folder_color(ix: usize) -> Option<gpui::Hsla> {
    FOLDER_COLORS
        .get(ix)
        .map(|&(h, s, l)| gpui::hsla(h / 360., s, l, 1.))
}
/// A user folder's colour: its custom hex, else its palette swatch.
pub(super) fn folder_tint(folder: &gespan_core::session_groups::Folder) -> Option<gpui::Hsla> {
    folder
        .custom_color
        .as_deref()
        .and_then(|color| color.strip_prefix('#'))
        .and_then(|color| u32::from_str_radix(color, 16).ok())
        .map(|color| rgb(color).into())
        .or_else(|| folder.color_index.and_then(folder_color))
}
fn local_ms(at: NaiveDateTime) -> Option<u64> {
    Local
        .from_local_datetime(&at)
        .earliest()
        .map(|at| at.timestamp_millis().max(0) as u64)
}
fn local_time(ms: u64) -> Option<NaiveDateTime> {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|at| at.with_timezone(&Local).naive_local())
}
pub(super) fn local_reminder_label(ms: u64) -> String {
    local_time(ms).map(format_reminder).unwrap_or_default()
}
#[derive(Clone, PartialEq)]
pub(super) enum ListDrop {
    Folder(String),
    Session(i64),
}
pub(super) struct FolderDrag {
    id: String,
    start: Point<Pixels>,
    active: bool,
    over: Option<String>,
}
impl FolderDrag {
    pub(super) fn new(id: String, start: Point<Pixels>) -> Self {
        Self {
            id,
            start,
            active: false,
            over: None,
        }
    }
    pub(super) fn id(&self) -> &str {
        &self.id
    }
    pub(super) fn active(&self) -> bool {
        self.active
    }
    pub(super) fn over(&self) -> Option<&String> {
        self.over.as_ref()
    }
}

#[derive(Clone, PartialEq)]
pub(super) enum RenameTarget {
    Session(i64),
    Folder(String),
}
pub(super) struct Renaming {
    pub(super) target: RenameTarget,
    input: Entity<input::Input>,
    _events: Vec<Subscription>,
}
/// What a ticket-only link dialog is for, when it is for a board card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CardTarget {
    /// The card with this id takes the ticket.
    Card(String),
    /// "New card from ticket": the ticket becomes a new card.
    New,
}
pub(super) struct LinkDialog {
    /// `None` is "New session from ticket" (or a card's ticket, see `card`):
    /// the dialog only takes a ticket.
    pub(super) id: Option<i64>,
    pub(super) card: Option<CardTarget>,
    pub(super) input: Entity<input::Input>,
    pub(super) error: Option<SharedString>,
    /// A bare key several connected trackers could own: ask which.
    choices: Option<Vec<Tracker>>,
    /// A key or link is being resolved off the UI thread.
    checking: bool,
    /// Bumped when the text changes, so a stale resolve is ignored.
    request: u64,
    pub(super) editing: bool,
    _events: Vec<Subscription>,
}

/// FolderColorSwatches: the folder menu's palette row.
pub(super) struct FolderSwatches {
    focus: FocusHandle,
    selected: usize,
}
pub(super) struct SwatchPicked(Option<usize>);
impl EventEmitter<SwatchPicked> for FolderSwatches {}
impl Render for FolderSwatches {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .px_2()
            .py_1p5()
            .flex()
            .items_center()
            .gap_1p5()
            .children((0..FOLDER_COLORS.len()).map(|ix| {
                let selected = ix == self.selected;
                div()
                    .id(("folder-swatch", ix))
                    .debug_selector(move || format!("folder-color-{ix}"))
                    .role(Role::Button)
                    .aria_label(if ix == 0 {
                        "Default color".to_string()
                    } else {
                        format!("Color {ix}")
                    })
                    .size(px(18.))
                    .rounded_full()
                    .border_2()
                    .border_color(if selected { content(0.9) } else { content(0.) })
                    .hover(|s| s.border_color(content(0.5)))
                    .child(
                        div()
                            .size_full()
                            .rounded_full()
                            .border_2()
                            .border_color(content(0.))
                            .bg(folder_color(ix).unwrap()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = ix;
                        cx.emit(SwatchPicked((ix > 0).then_some(ix)));
                        cx.notify();
                    }))
            }))
    }
}

impl Shell {
    pub(super) fn rename_sidebar_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = if self.sidebar_focus.is_focused(window) {
            self.sidebar_target.clone()
        } else {
            self.session_hover
                .or(self.workspace.active)
                .map(RenameTarget::Session)
        };
        if let Some(target) = target {
            self.start_rename(target, window, cx);
        }
    }
    pub(super) fn delete_sidebar_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<_> = self.selected_sessions.iter().copied().collect();
        let chosen = ids.len();
        for id in ids {
            self.delete_chosen_session(id, chosen, window, cx);
        }
    }
    pub(super) fn sidebar_drop_at(
        &self,
        position: Point<Pixels>,
        source: i64,
        cx: &App,
    ) -> Option<ListDrop> {
        if !self.workspace.sidebar_visible || self.reminders.iter().any(|r| r.session_id == source)
        {
            return None;
        }
        let groups = self.project_groups();
        for folder in &groups.folders {
            if self
                .folder_bounds
                .get(&folder.id)
                .and_then(AnchorSlot::get)
                .is_some_and(|b| b.contains(&position))
            {
                return Some(ListDrop::Folder(folder.id.clone()));
            }
        }
        for id in self.sidebar_navigation_ids(cx) {
            if id == source || self.reminders.iter().any(|r| r.session_id == id) {
                continue;
            }
            if self
                .session_row_bounds
                .get(&id)
                .and_then(AnchorSlot::get)
                .is_some_and(|b| b.contains(&position))
            {
                return Some(match groups.folder_of(id) {
                    Some(folder) => ListDrop::Folder(folder.id.clone()),
                    None => ListDrop::Session(id),
                });
            }
        }
        None
    }
    pub(super) fn apply_sidebar_drop(
        &mut self,
        source: i64,
        target: ListDrop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            ListDrop::Folder(folder) => {
                self.update_groups(cx, |g| g.add_to_folder(&folder, source))
            }
            ListDrop::Session(target) if source != target => {
                let folder = format!("{:x}-{source}-{target}", gespan_core::time::now_ms());
                self.update_groups(cx, |g| {
                    g.create_folder(&[source, target], folder.clone());
                });
                self.start_rename(RenameTarget::Folder(folder), window, cx);
            }
            _ => {}
        }
    }
    pub(super) fn move_folder_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.folder_drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.folder_drag = None;
            return;
        }
        let delta = event.position - drag.start;
        if !drag.active && delta.x.as_f32().hypot(delta.y.as_f32()) < 5. {
            return;
        }
        drag.active = true;
        self.skip_folder_click = true;
        drag.over = self.folder_bounds.iter().find_map(|(id, slot)| {
            slot.get()
                .filter(|b| b.contains(&event.position))
                .map(|_| id.clone())
        });
        self.notify_sidebar(cx);
    }
    pub(super) fn finish_folder_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.folder_drag.take() else {
            return;
        };
        if !drag.active {
            return;
        }
        if let Some(over) = drag.over {
            self.update_groups(cx, |g| {
                if let Some((from, to)) = g
                    .folders
                    .iter()
                    .position(|f| f.id == drag.id)
                    .zip(g.folders.iter().position(|f| f.id == over))
                {
                    let folder = g.folders.remove(from);
                    g.folders.insert(to, folder);
                }
            });
        }
        self.notify_sidebar(cx);
    }
    pub(super) fn project_groups(&self) -> Groups {
        self.groups
            .get(&self.workspace.project)
            .cloned()
            .unwrap_or_default()
    }
    pub(super) fn update_groups(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Groups),
    ) {
        let project = self.workspace.project.clone();
        let groups = self.groups.entry(project.clone()).or_default();
        change(groups);
        let groups = groups.clone();
        self.store_write("save folders", cx, move |store| {
            store.save_groups(&project, &groups)
        });
        self.invalidate_chrome(cx);
        cx.notify();
    }
    pub(super) fn store_write(
        &mut self,
        what: &'static str,
        cx: &mut Context<Self>,
        write: impl FnOnce(&mut gespan_core::store::Store) -> Result<(), String> + Send + 'static,
    ) {
        let store = self.store.clone();
        let previous = self.group_writes.take();
        self.group_writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx
                .background_executor()
                .spawn(async move { write(&mut *locked(&store)?) });
            if let Err(error) = work.await {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(format!("Could not {what}: {error}"));
                    cx.notify();
                });
            }
        }));
    }

    /// The project's sidebar entries, reminders first.
    pub(super) fn session_entries(&self, ids: &[i64], cx: &App) -> Vec<Entry> {
        let (groups, fixed) = self.sidebar_groups(cx);
        let reminders: Vec<_> = self
            .reminders
            .iter()
            .map(|r| (r.session_id, r.due_at))
            .collect();
        build_list(
            ids,
            |id| self.session_metadata(id, cx).summary.pinned,
            &groups,
            &reminders,
            &fixed,
        )
    }
    /// The folders the sidebar lists and its automatic folders: one per
    /// Pilot with sessions in this project, then Automation runs. The stored
    /// folder a Pilot files its sessions in (`pilot-<id>`) shows as that
    /// Pilot's fixed folder, after its launches, so it is not a user folder
    /// here; the stored data is untouched.
    pub(super) fn sidebar_groups(&self, cx: &App) -> (Groups, Vec<FixedFolder>) {
        let mut groups = self.project_groups();
        let mut folders = Vec::new();
        let pilots = self
            .pilots
            .as_ref()
            .map(|pilots| pilots.read(cx).pilot_folders(&self.workspace.project))
            .unwrap_or_default();
        for (id, name, mut session_ids) in pilots {
            let stored = crate::pilots::pilot_folder_id(&id);
            if let Some(ix) = groups.folders.iter().position(|f| f.id == stored) {
                for session in groups.folders.remove(ix).session_ids {
                    if !session_ids.contains(&session) {
                        session_ids.push(session);
                    }
                }
            }
            if !session_ids.is_empty() {
                folders.push(FixedFolder {
                    id: format!("{PILOT_PREFIX}{id}"),
                    name,
                    session_ids,
                });
            }
        }
        let runs: Vec<i64> = self
            .automation_state
            .session_automations
            .keys()
            .copied()
            .collect();
        if !runs.is_empty() {
            folders.push(FixedFolder {
                id: AUTOMATION_RUNS.into(),
                name: "Automation runs".into(),
                session_ids: runs,
            });
        }
        (groups, folders)
    }
    /// The sessions in the order the sidebar shows them, for stepping.
    pub(super) fn sidebar_navigation_ids(&self, cx: &App) -> Vec<i64> {
        let ids = self.sidebar_session_ids(cx);
        let groups = self.project_groups();
        let entries: Vec<_> = self
            .session_entries(&ids, cx)
            .into_iter()
            .filter(|entry| match entry {
                Entry::Folder { id, .. } => groups.folder(id).is_none_or(|f| !f.collapsed),
                Entry::Fixed { id, .. } => !groups.fixed_is_collapsed(id),
                _ => true,
            })
            .collect();
        navigation_ids(&entries)
    }

    fn session_menu_ids(&self, id: i64, cx: &App) -> Vec<i64> {
        if !self.selected_sessions.contains(&id) {
            return vec![id];
        }
        self.sidebar_session_ids(cx)
            .into_iter()
            .filter(|id| self.selected_sessions.contains(id))
            .collect()
    }
    pub(super) fn clear_context_selection(&mut self) {
        if std::mem::take(&mut self.context_selection) {
            self.selected_sessions.clear();
        }
    }
    pub(super) fn select_sidebar_session(
        &mut self,
        id: i64,
        modifiers: gpui::Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_selection = false;
        self.close_menu(cx);
        self.sidebar_target = Some(RenameTarget::Session(id));
        let additive = modifiers.control || modifiers.platform;
        if modifiers.shift {
            let visible = self.sidebar_navigation_ids(cx);
            let anchor = self
                .selection_anchor
                .filter(|id| visible.contains(id))
                .or(self.workspace.active)
                .unwrap_or(id);
            let range = visible
                .iter()
                .position(|s| *s == anchor)
                .zip(visible.iter().position(|s| *s == id));
            if !additive {
                self.selected_sessions.clear();
            }
            if let Some((a, b)) = range {
                self.selected_sessions.extend(&visible[a.min(b)..=a.max(b)]);
                self.selection_anchor = Some(anchor);
            } else {
                self.selected_sessions.insert(id);
                self.selection_anchor = Some(id);
            }
        } else if additive {
            self.selection_anchor = Some(id);
            if !self.selected_sessions.insert(id) {
                self.selected_sessions.remove(&id);
            }
        } else {
            self.selected_sessions.clear();
            self.selection_anchor = Some(id);
            self.open(id, window, cx);
        }
        if modifiers.shift || additive {
            window.focus(&self.sidebar_focus, cx);
        }
        self.invalidate_chrome(cx);
        cx.notify();
    }
    pub(super) fn session_menu_items(&self, id: i64, cx: &App) -> Vec<MenuItem> {
        let summary = self.session_metadata(id, cx).summary;
        let ids = self.session_menu_ids(id, cx);
        let multiple = ids.len() > 1;
        let groups = self.project_groups();
        let all_pinned = ids
            .iter()
            .all(|id| self.session_metadata(*id, cx).summary.pinned);
        let all_archived = ids
            .iter()
            .all(|id| self.session_metadata(*id, cx).summary.archived);
        let mut items = Vec::new();
        if self.reminders.iter().any(|r| ids.contains(&r.session_id)) {
            items.push(MenuAction::new("reminder:cancel", "Cancel reminder").into());
            items.push(MenuItem::Separator);
        }
        items.push(MenuAction::new("pin", if all_pinned { "Unpin" } else { "Pin" }).into());
        if !multiple {
            items.extend([
                MenuAction::new("rename", "Rename").shortcut("F2").into(),
                MenuAction::new("copy-session-id", "Copy session ID")
                    .submenu(vec![
                        MenuAction::new("copy-harness-session-id", "Harness session ID")
                            .disabled(summary.provider_id.is_none()),
                        MenuAction::new("copy-gespan-session-id", "Gespan session ID"),
                    ])
                    .into(),
                MenuAction::new(
                    "link-work-item",
                    if summary.work_item.is_some() {
                        "Edit issue, PR or ticket link…"
                    } else {
                        "Link issue, PR or ticket…"
                    },
                )
                .into(),
            ]);
            if let Some(WorkItem::Ticket(ticket)) = &summary.work_item {
                items.extend([
                    MenuAction::new("ticket-open", format!("Open in {}", ticket.tracker.label()))
                        .disabled(ticket.url.is_none())
                        .into(),
                    MenuAction::new("ticket-refresh", "Refresh ticket").into(),
                    ticket_status_menu(crate::board::turns::status_setting(
                        &self.store,
                        id,
                        &self.board_service,
                    )),
                    MenuAction::new("ticket-unlink", "Unlink ticket").into(),
                ]);
            }
        }
        items.extend([
            MenuAction::new("reminder", "Remind me")
                .submenu(
                    reminder_presets(Local::now().naive_local())
                        .into_iter()
                        .map(|(id, label, enabled)| MenuAction::new(id, label).disabled(!enabled))
                        .collect(),
                )
                .into(),
            MenuItem::Separator,
            MenuAction::new("folder-new", "New folder").into(),
        ]);
        if !groups.folders.is_empty() {
            items.push(MenuItem::Separator);
            items.extend(groups.folders.iter().map(|folder| {
                MenuAction::new(
                    format!("folder-add:{}", folder.id),
                    format!("Add to {}", folder.name),
                )
                .checked(ids.iter().all(|id| folder.session_ids.contains(id)))
                .into()
            }));
        }
        if ids.iter().any(|id| groups.folder_of(*id).is_some()) {
            items.push(
                MenuAction::new(
                    "folder-remove",
                    if multiple {
                        "Remove from folders"
                    } else {
                        "Remove from folder"
                    },
                )
                .into(),
            );
        }
        items.extend([
            MenuItem::Separator,
            MenuAction::new(
                "archive",
                if all_archived { "Unarchive" } else { "Archive" },
            )
            .into(),
            MenuAction::new("delete", "Delete")
                .shortcut("⌫")
                .danger()
                .into(),
        ]);
        items
    }
    pub(super) fn run_session_menu(
        &mut self,
        id: i64,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.session_summaries.contains_key(&id) {
            return;
        }
        let summary = self.session_metadata(id, cx).summary.clone();
        let ids = self.session_menu_ids(id, cx);
        match action {
            "copy-harness-session-id" | "copy-gespan-session-id" => {
                let text = if action == "copy-harness-session-id" {
                    summary.provider_id
                } else {
                    locked(&self.store)
                        .and_then(|store| store.persisted_session_id(summary.id))
                        .ok()
                };
                if let Some(text) = text {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            "pin" => {
                let pinned = !ids
                    .iter()
                    .all(|id| self.session_metadata(*id, cx).summary.pinned);
                for id in &ids {
                    if let Some(summary) = self.session_summaries.get_mut(id) {
                        summary.pinned = pinned;
                    }
                    if let Some(view) = self.sessions.get(id) {
                        view.update(cx, |s, _| s.summary.pinned = pinned);
                    }
                }
                self.store_write("pin the session", cx, move |s| s.set_pinned(&ids, pinned));
                self.invalidate_chrome(cx);
            }
            "rename" => self.start_rename(RenameTarget::Session(id), window, cx),
            "link-work-item" => self.open_link_dialog(id, window, cx),
            "ticket-open" => {
                if let Some(WorkItem::Ticket(ticket)) =
                    self.session_metadata(id, cx).summary.work_item.clone()
                    && let Some(url) = ticket.url
                {
                    cx.open_url(&url);
                }
            }
            "ticket-refresh" => self.refresh_ticket(id, true, cx),
            "ticket-unlink" => self.set_work_item(id, None, cx),
            "ticket-status:board" | "ticket-status:on" | "ticket-status:off" => {
                let choice = match action {
                    "ticket-status:on" => Some(true),
                    "ticket-status:off" => Some(false),
                    _ => None,
                };
                self.store_write("save the ticket status setting", cx, move |s| {
                    crate::board::turns::set_status_setting(s, id, choice)
                });
            }
            "reminder:cancel" => {
                let occurrences: Vec<_> = self
                    .reminders
                    .iter()
                    .filter(|reminder| ids.contains(&reminder.session_id))
                    .map(|reminder| (reminder.session_id, reminder.due_at))
                    .collect();
                for (id, due_at) in occurrences {
                    self.clear_notification_reminder(id, due_at, cx);
                }
                self.invalidate_chrome(cx);
            }
            preset if preset.starts_with("reminder:") => {
                let Some(due_at) =
                    reminder_at(preset, Local::now().naive_local()).and_then(local_ms)
                else {
                    return;
                };
                self.reminders.retain(|r| !ids.contains(&r.session_id));
                self.reminders.extend(ids.iter().map(|id| Reminder {
                    session_id: *id,
                    due_at,
                    fired_at: None,
                }));
                let project = self.workspace.project.clone();
                if let Some(groups) = self.groups.get_mut(&project) {
                    groups.reminders_collapsed = false;
                }
                self.update_groups(cx, |_| {});
                self.store_write("set the reminder", cx, move |s| {
                    s.set_reminders(&ids, due_at)
                });
            }
            "folder-new" => {
                let folder = format!("{:x}", gespan_core::time::now_ms() ^ ((id as u64) << 40));
                self.update_groups(cx, |groups| {
                    groups.create_folder(&ids, folder.clone());
                });
                self.start_rename(RenameTarget::Folder(folder), window, cx);
            }
            "folder-remove" => self.update_groups(cx, |groups| groups.remove_sessions(&ids)),
            add if add.starts_with("folder-add:") => {
                let folder = add["folder-add:".len()..].to_string();
                self.update_groups(cx, |groups| {
                    for id in &ids {
                        groups.add_to_folder(&folder, *id);
                    }
                });
            }
            "archive" => {
                let archived = !ids
                    .iter()
                    .all(|id| self.session_metadata(*id, cx).summary.archived);
                if let ([id], true) = (ids.as_slice(), archived) {
                    return self.archive_session(*id, window, cx);
                }
                for id in ids {
                    if self.session_metadata(id, cx).summary.archived != archived {
                        self.toggle_archive(id, cx);
                    }
                }
            }
            "delete" => {
                let chosen = ids.len();
                for id in ids {
                    self.delete_chosen_session(id, chosen, window, cx);
                }
            }
            _ => {}
        }
    }
    pub(super) fn folder_menu_items(
        &self,
        folder: &str,
        events: &mut Vec<Subscription>,
        cx: &mut Context<Self>,
    ) -> Vec<MenuItem> {
        let selected = self
            .project_groups()
            .folder(folder)
            .and_then(|f| f.color_index)
            .unwrap_or(0);
        let swatches = cx.new(|cx| FolderSwatches {
            focus: cx.focus_handle(),
            selected,
        });
        let id = folder.to_string();
        events.push(
            cx.subscribe(&swatches, move |this, _, SwatchPicked(color), cx| {
                let color = *color;
                this.update_groups(cx, |groups| groups.set_color(&id, color));
            }),
        );
        let focus = swatches.read(cx).focus.clone();
        vec![
            MenuItem::Custom(swatches.into(), focus),
            MenuAction::new("rename", "Rename").shortcut("F2").into(),
            MenuItem::Separator,
            MenuAction::new("ungroup", "Ungroup").into(),
        ]
    }
    pub(super) fn run_folder_menu(
        &mut self,
        folder: String,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if folder.starts_with(FIXED_PREFIX) {
            return;
        }
        match action {
            "rename" => self.start_rename(RenameTarget::Folder(folder), window, cx),
            "ungroup" => self.update_groups(cx, |groups| groups.dissolve(&folder)),
            _ => {}
        }
    }
    /// A fixed folder cannot be renamed, recoloured or ungrouped: it only folds.
    pub(super) fn fixed_folder_menu_items(&self, folder: &str) -> Vec<MenuItem> {
        let item = if self.project_groups().fixed_is_collapsed(folder) {
            MenuAction::new("fixed-expand", "Expand")
        } else {
            MenuAction::new("fixed-collapse", "Collapse")
        };
        vec![item.into()]
    }
    pub(super) fn run_fixed_folder_menu(
        &mut self,
        folder: String,
        action: &str,
        cx: &mut Context<Self>,
    ) {
        let collapsed = match action {
            "fixed-collapse" => true,
            "fixed-expand" => false,
            _ => return,
        };
        self.update_groups(cx, |groups| groups.set_fixed_collapsed(&folder, collapsed));
    }

    /// Opening a session whose reminder is due dismisses it.
    pub(super) fn dismiss_due_reminder(&mut self, id: i64, cx: &mut Context<Self>) {
        let now = gespan_core::time::now_ms();
        let Some(due) = self
            .reminders
            .iter()
            .find(|r| r.session_id == id && r.due_at <= now)
            .map(|r| r.due_at)
        else {
            return;
        };
        self.clear_notification_reminder(id, due, cx);
    }

    pub(super) fn poll_reminders(&mut self, cx: &mut Context<Self>) {
        if self.secondary || self.notification_state.reminder_error.is_some() {
            return;
        }
        let now = gespan_core::time::now_ms();
        let due: Vec<_> = self
            .reminders
            .iter_mut()
            .filter_map(|reminder| {
                if reminder.due_at > now || reminder.fired_at.is_some() {
                    return None;
                }
                reminder.fired_at = Some(now);
                Some((reminder.session_id, reminder.due_at))
            })
            .collect();
        if due.is_empty() {
            return;
        }
        let store = self.store.clone();
        let previous = self.group_writes.take();
        self.group_writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx
                .background_executor()
                .spawn(async move { locked(&store)?.take_due_reminders(now) });
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(claimed) => {
                        this.notification_state.reminder_error = None;
                        for reminder in claimed {
                            // A cancel/reschedule while the store operation was
                            // pending must never deliver an obsolete occurrence.
                            if this.reminders.iter().any(|current| {
                                current.session_id == reminder.session_id
                                    && current.due_at == reminder.due_at
                            }) {
                                this.deliver_reminder_notice(
                                    reminder.session_id,
                                    reminder.due_at,
                                    cx,
                                );
                            }
                        }
                    }
                    Err(error) => {
                        // Re-arm only the occurrences claimed by this request;
                        // a newer schedule has its own independent fired state.
                        for reminder in &mut this.reminders {
                            if due.contains(&(reminder.session_id, reminder.due_at))
                                && reminder.fired_at == Some(now)
                            {
                                reminder.fired_at = None;
                            }
                        }
                        this.notification_state.reminder_error =
                            Some(format!("Could not deliver reminders: {error}"));
                    }
                }
                this.invalidate_chrome(cx);
                cx.notify();
            });
        }));
        self.invalidate_chrome(cx);
    }

    pub(super) fn delete_session(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        // The window running it stops and removes it.
        if crate::windows::on_owner(id, cx.entity_id(), cx, move |shell, window, cx| {
            shell.delete_session(id, window, cx)
        }) {
            return;
        }
        if self.workspace.review_owners.iter().any(|(pane, owner)| {
            *owner == id
                && (self.workspace.tab_of(*pane).is_some()
                    || self
                        .reviews
                        .get(pane)
                        .is_some_and(|r| r.read(cx).unsaved_files(cx)))
        }) {
            self.error = Some("Close this session's detached file panes before deleting it".into());
            cx.notify();
            return;
        }
        if self
            .workspace
            .terminal_panes
            .iter()
            .any(|(pane, t)| t.owner == id && self.workspace.tab_of(*pane).is_some())
        {
            self.error = Some("Close this session's terminal panes before deleting it".into());
            cx.notify();
            return;
        }
        if !self.session_summaries.contains_key(&id) || !self.deleting.insert(id) {
            return;
        }
        self.close_preview(cx);
        // A Flow visit in this session fails now, not as a user stop.
        self.flow_session_closed(id, cx);
        if let Some(view) = self.sessions.get(&id) {
            view.update(cx, |session, cx| session.stop_if_busy(cx));
        }
        if self.workspace.tab_of(id).is_some() {
            self.close_pane(id, window, cx);
        }
        if self.workspace.tab_of(id).is_some() {
            self.deleting.remove(&id);
            self.error = Some("Could not delete a session that is still open".into());
            cx.notify();
            return;
        }
        self.invalidate_chrome(cx);
        let store = self.store.clone();
        let previous = self.group_writes.take();
        self.group_writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx.background_executor().spawn(async move {
                locked(&store)?.delete(id)?;
                // Drop the session's recorded turns and their Git refs.
                if let Some(checkpoints) = gespan_core::checkpoint::store() {
                    let _ = checkpoints.exclusive(|c| c.forget(&id.to_string()));
                }
                Ok::<_, String>(())
            });
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.deleting.remove(&id);
                match result {
                    Ok(()) => {
                        this.forget_session(id, cx);
                        this.session_removed(id, true, cx);
                    }
                    Err(error) => {
                        this.session_removed(id, false, cx);
                        this.error = Some(format!("Could not delete the session: {error}"))
                    }
                }
                this.invalidate_chrome(cx);
                this.sync_settings(cx);
                cx.notify();
            });
        }));
    }
    /// One chosen chat can offer its worktree; a bulk delete just deletes.
    fn delete_chosen_session(
        &mut self,
        id: i64,
        chosen: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if chosen == 1 {
            self.remove_session(id, session_worktree::Removal::Delete, None, window, cx);
        } else {
            self.delete_session(id, window, cx);
        }
    }
    pub(super) fn forget_session(&mut self, id: i64, cx: &mut Context<Self>) {
        // A tab opened on it while the store was deleting keeps it alive.
        if self.workspace.tab_of(id).is_some() {
            return;
        }
        if self
            .session_preview
            .as_ref()
            .is_some_and(|preview| preview.id == id)
        {
            self.session_preview = None;
            self.preview_close = None;
        }
        self.flow_session_closed(id, cx);
        self.sessions.remove(&id);
        self.session_observations.remove(&id);
        self.closed_panes.remove(&id);
        crate::windows::release(id, cx.entity_id(), cx);
        self.session_summaries.remove(&id);
        self.draft_sessions.remove(&id);
        self.history_recency.remove(&id);
        self.session_marks.remove(&id);
        self.tab_marks.remove(&id);
        self.order.retain(|s| *s != id);
        self.busy_sessions.remove(&id);
        self.sync_keep_awake();
        self.unseen_finished.remove(&id);
        self.selected_sessions.remove(&id);
        self.reminders.retain(|r| r.session_id != id);
        self.workspace.project_focus.retain(|_, s| *s != id);
        self.history
            .prune(&self.workspace.open_sessions(), self.workspace.active);
        if self.session_hover == Some(id) {
            self.session_hover = None;
        }
        if self
            .renaming
            .as_ref()
            .is_some_and(|r| r.target == RenameTarget::Session(id))
        {
            self.renaming = None;
        }
        let projects: Vec<String> = self
            .groups
            .iter()
            .filter(|(_, g)| g.folder_of(id).is_some())
            .map(|(p, _)| p.clone())
            .collect();
        for project in projects {
            let groups = self.groups.get_mut(&project).unwrap();
            groups.remove_sessions(&[id]);
            let groups = groups.clone();
            self.store_write("save folders", cx, move |s| {
                s.save_groups(&project, &groups)
            });
        }
        self.persist(cx);
    }

    pub(super) fn start_rename(
        &mut self,
        target: RenameTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(&target, RenameTarget::Folder(f) if f.starts_with(FIXED_PREFIX)) {
            return;
        }
        self.close_preview(cx);
        let text = match &target {
            RenameTarget::Session(id) => self.session_metadata(*id, cx).display_title(),
            RenameTarget::Folder(folder) => self
                .project_groups()
                .folder(folder)
                .map(|f| f.name.clone())
                .unwrap_or_default(),
        };
        let input = cx.new(|cx| {
            let mut input = input::Input::field(
                "Rename. Press Enter to confirm or Escape to cancel.",
                "",
                content(0.35),
                cx,
            );
            input.set_font_size(14.);
            input
        });
        let len = text.len();
        input.update(cx, |i, cx| {
            i.set_text(text, cx);
            i.select_range(0..len, cx);
        });
        let focus = input.read(cx).focus_handle(cx);
        let events = vec![
            cx.subscribe_in(
                &input,
                window,
                |this, _, _: &input::Submitted, window, cx| this.finish_rename(true, window, cx),
            ),
            cx.on_blur(&focus, window, |this, window, cx| {
                this.finish_rename(true, window, cx)
            }),
        ];
        window.focus(&focus, cx);
        self.renaming = Some(Renaming {
            target,
            input,
            _events: events,
        });
        self.invalidate_chrome(cx);
        cx.notify();
    }
    fn finish_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.take() else {
            return;
        };
        let name = renaming.input.read(cx).content.trim().to_string();
        if commit && !name.is_empty() {
            match renaming.target {
                RenameTarget::Session(id) => {
                    if self.session_summaries.contains_key(&id)
                        && self.session_metadata(id, cx).summary.title != name
                    {
                        if let Some(summary) = self.session_summaries.get_mut(&id) {
                            summary.title = name.clone();
                        }
                        if let Some(session) = self.sessions.get(&id) {
                            session.update(cx, |s, cx| {
                                s.rename(name.clone(), cx);
                            });
                        }
                        self.store_write("rename the session", cx, move |s| s.title(id, &name));
                    }
                }
                RenameTarget::Folder(folder) => {
                    self.update_groups(cx, |groups| groups.rename_folder(&folder, &name))
                }
            }
        }
        if renaming.input.read(cx).focus_handle(cx).is_focused(window) {
            self.focus_active(window, cx);
        }
        self.invalidate_chrome(cx);
        cx.notify();
    }
    /// SessionRenameRow and FolderRenameRow: the name in a focused field.
    pub(super) fn rename_row(
        &self,
        target: &RenameTarget,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let renaming = self.renaming.as_ref().filter(|r| r.target == *target)?;
        Some(
            div()
                .key_context("SessionRename")
                .on_action(cx.listener(|this, _: &CancelRename, window, cx| {
                    this.finish_rename(false, window, cx)
                }))
                .h(px(32.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .size(px(16.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(matches!(target, RenameTarget::Folder(_)), |d| {
                            d.child(icon("chevron-down", 14.))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h(px(22.))
                        .px_2()
                        .flex()
                        .items_center()
                        .rounded_sm()
                        .bg(content(0.1))
                        .border_1()
                        .border_color(rgba(0x4da3f566))
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(content(1.))
                        .child(renaming.input.clone()),
                )
                .into_any(),
        )
    }

    fn open_link_dialog(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.open_link_target(Some(id), None, window, cx);
    }
    /// A board card's "Link ticket…" and the palette's "New card from ticket…".
    pub(super) fn open_card_ticket_dialog(
        &mut self,
        target: CardTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_link_target(None, Some(target), window, cx);
    }
    /// The palette's "Link ticket…": the session in view takes the link.
    pub(super) fn link_ticket_to_active_session(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.workspace.active {
            Some(id) => self.open_link_dialog(id, window, cx),
            None => {
                self.error = Some("Open a session to link a ticket".into());
                cx.notify();
            }
        }
    }
    /// The palette's "New card from ticket…": opens the Board, then asks for
    /// the ticket.
    pub(super) fn open_new_card_ticket_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_board(window, cx);
        if self.board.is_none() {
            self.error = Some("Turn on the Board in Settings to add cards".into());
            cx.notify();
            return;
        }
        self.open_card_ticket_dialog(CardTarget::New, window, cx);
    }
    pub(super) fn open_new_ticket_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_link_target(None, None, window, cx);
    }
    fn open_link_target(
        &mut self,
        id: Option<i64>,
        card: Option<CardTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_preview(cx);
        let initial = match &card {
            Some(CardTarget::Card(card_id)) => self.card_ticket(card_id, cx).map(Box::new),
            Some(CardTarget::New) => None,
            None => None,
        }
        .map(WorkItem::Ticket)
        .or_else(|| id.and_then(|id| self.session_metadata(id, cx).summary.work_item.clone()));
        let input = cx.new(|cx| {
            input::Input::field(
                "Issue, PR or ticket",
                if id.is_some() {
                    "https://github.com/owner/repo/pull/123 or ENG-123"
                } else {
                    "ENG-123 or a ticket link"
                },
                content(0.3),
                cx,
            )
        });
        let existing = match &initial {
            Some(WorkItem::Github(item)) => Some(item.url.clone()),
            Some(WorkItem::Ticket(ticket)) => {
                Some(ticket.url.clone().unwrap_or_else(|| ticket.key.clone()))
            }
            None => None,
        };
        if let Some(text) = existing {
            input.update(cx, |i, cx| i.set_text(text, cx));
        }
        let events = vec![
            cx.subscribe_in(
                &input,
                window,
                |this, _, _: &input::Submitted, window, cx| this.save_link(true, window, cx),
            ),
            cx.subscribe(&input, |this, _, _: &input::Changed, cx| {
                if let Some(dialog) = &mut this.link_dialog {
                    dialog.request += 1;
                    dialog.checking = false;
                    dialog.error = None;
                    dialog.choices = None;
                    cx.notify();
                }
            }),
        ];
        window.focus(&input.read(cx).focus_handle(cx), cx);
        self.link_dialog = Some(LinkDialog {
            id,
            card,
            input,
            error: None,
            choices: None,
            checking: false,
            request: 0,
            editing: initial.is_some(),
            _events: events,
        });
        cx.notify();
    }
    pub(super) fn close_link_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.link_dialog.take().is_some() {
            self.focus_active(window, cx);
            cx.notify();
        }
    }
    /// Saves the pasted GitHub link or ticket, or with `keep` false, removes the link.
    pub(super) fn save_link(&mut self, keep: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &self.link_dialog else {
            return;
        };
        let id = dialog.id;
        let card = dialog.card.clone();
        if !keep {
            if let Some(id) = id {
                self.set_work_item(id, None, cx);
            }
            if let Some(CardTarget::Card(card_id)) = card {
                self.with_board(cx, |board, cx| board.unlink_ticket(&card_id, cx));
            }
            self.close_link_dialog(window, cx);
            return;
        }
        let text = dialog.input.read(cx).content.trim().to_string();
        if let Some(id) = id
            && let Some(item) = parse_github_work_item(&text)
        {
            self.set_work_item(id, Some(WorkItem::Github(item)), cx);
            self.close_link_dialog(window, cx);
            return;
        }
        if dialog.checking {
            return;
        }
        let request = dialog.request;
        if let Some(dialog) = &mut self.link_dialog {
            dialog.checking = true;
            dialog.error = None;
            dialog.choices = None;
        }
        cx.notify();
        // Resolving reads saved credentials from the keychain, which can block.
        let backend = self.tickets.clone();
        let lookup = text.clone();
        let work = cx
            .background_executor()
            .spawn(async move { backend.resolve(&lookup) });
        cx.spawn_in(window, async move |this, cx| {
            let resolved = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.link_resolved(request, (id, card), &text, resolved, window, cx)
            });
        })
        .detach();
    }
    fn link_resolved(
        &mut self,
        request: u64,
        (id, card): (Option<i64>, Option<CardTarget>),
        text: &str,
        resolved: Resolved,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &mut self.link_dialog {
            Some(dialog) if dialog.request == request => dialog.checking = false,
            _ => return,
        }
        let error = match resolved {
            Resolved::One(mut ticket) => {
                // Saving the same ticket again keeps what was already fetched.
                let current = match &card {
                    Some(CardTarget::Card(card_id)) => self.card_ticket(card_id, cx),
                    Some(CardTarget::New) => None,
                    None => match id
                        .and_then(|id| self.session_metadata(id, cx).summary.work_item.clone())
                    {
                        Some(WorkItem::Ticket(current)) => Some(*current),
                        _ => None,
                    },
                };
                if let Some(current) = current
                    && current.tracker == ticket.tracker
                    && current.key == ticket.key
                {
                    ticket = TicketRef {
                        url: ticket.url.or(current.url.clone()),
                        ..current
                    };
                }
                self.close_link_dialog(window, cx);
                return self.finish_ticket_link(id, card, ticket, cx);
            }
            Resolved::Ambiguous(trackers) => {
                if let Some(dialog) = &mut self.link_dialog {
                    dialog.choices = Some(trackers);
                }
                cx.notify();
                return;
            }
            Resolved::Invalid if parse_ticket(text).is_some() => {
                "Connect Jira, Linear or Plane in Settings to link tickets".to_string()
            }
            Resolved::Invalid if id.is_some() => {
                "Enter a GitHub issue or PR URL, or a ticket key or link".to_string()
            }
            other => other.message().unwrap_or_default(),
        };
        if let Some(dialog) = &mut self.link_dialog {
            dialog.error = Some(error.into());
        }
        cx.notify();
    }
    /// A bare key several trackers could own, once the user picks one.
    fn choose_ticket_tracker(
        &mut self,
        tracker: Tracker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = &self.link_dialog else {
            return;
        };
        let id = dialog.id;
        let card = dialog.card.clone();
        let Some(parsed) = parse_ticket(dialog.input.read(cx).content.trim()) else {
            return;
        };
        self.close_link_dialog(window, cx);
        self.finish_ticket_link(
            id,
            card,
            TicketRef {
                tracker,
                key: parsed.key,
                url: None,
                title: None,
                status: None,
                status_group: None,
                fetched_at: None,
            },
            cx,
        );
    }
    fn finish_ticket_link(
        &mut self,
        id: Option<i64>,
        card: Option<CardTarget>,
        ticket: TicketRef,
        cx: &mut Context<Self>,
    ) {
        match (card, id) {
            (Some(CardTarget::Card(card_id)), _) => {
                self.with_board(cx, |board, cx| board.link_ticket(&card_id, ticket, cx));
            }
            (Some(CardTarget::New), _) => {
                self.with_board(cx, |board, cx| {
                    board.create_card_from_ticket(ticket, cx);
                });
            }
            (None, Some(id)) => self.link_ticket(id, ticket, cx),
            (None, None) => {
                self.create_session_target(self.default_kind(cx), false, None, Some(ticket), cx)
            }
        }
    }
    /// The Board view, handed the shell's ticket backend first so the card's
    /// chip is read through the same one the session chips use.
    fn with_board<R>(
        &mut self,
        cx: &mut Context<Self>,
        run: impl FnOnce(&mut crate::board::BoardView, &mut Context<crate::board::BoardView>) -> R,
    ) -> Option<R> {
        let board = self.board.clone()?;
        let backend = self.tickets.clone();
        Some(board.update(cx, |board, cx| {
            board.set_tickets(backend);
            run(board, cx)
        }))
    }
    fn card_ticket(&mut self, card_id: &str, cx: &mut Context<Self>) -> Option<TicketRef> {
        let board = self.board.clone()?;
        board.read(cx).card_ticket(card_id).cloned()
    }
    /// Saves the link at once, with no network; the title and status follow.
    fn link_ticket(&mut self, id: i64, ticket: TicketRef, cx: &mut Context<Self>) {
        self.stale_tickets.remove(&id);
        self.set_work_item(id, Some(WorkItem::Ticket(Box::new(ticket))), cx);
        self.refresh_ticket(id, false, cx);
    }
    /// A session made from a ticket drafts the key until the title is fetched.
    pub(super) fn start_ticket_session(
        &mut self,
        id: i64,
        ticket: TicketRef,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sessions.get(&id) {
            let key = ticket.key.clone();
            view.update(cx, |view, cx| view.set_draft_text(key.clone(), cx));
            self.ticket_drafts.insert(id, key);
        }
        self.link_ticket(id, ticket, cx);
    }
    fn set_work_item(&mut self, id: i64, item: Option<WorkItem>, cx: &mut Context<Self>) {
        if let Some(summary) = self.session_summaries.get_mut(&id) {
            summary.work_item = item.clone();
        }
        if let Some(session) = self.sessions.get(&id) {
            session.update(cx, |s, _| s.summary.work_item = item.clone());
        }
        if item.is_none() {
            self.stale_tickets.remove(&id);
            self.ticket_drafts.remove(&id);
        }
        self.store_write("link the session", cx, move |s| {
            s.set_work_item(id, item.as_ref())?;
            // A running turn's ticket tools follow the new link at once.
            crate::runtime::ticket_scope::changed(Some(id));
            Ok(())
        });
        self.invalidate_chrome(cx);
    }
    /// Reads the linked ticket's title and status. Runs on link, on open and
    /// from the menu, never on a timer. `report` shows a failure to the user.
    pub(super) fn refresh_ticket(&mut self, id: i64, report: bool, cx: &mut Context<Self>) {
        let Some(WorkItem::Ticket(ticket)) =
            self.session_metadata(id, cx).summary.work_item.clone()
        else {
            return;
        };
        // Opening the session again while the same ticket is being read adds nothing.
        if !report
            && self
                .ticket_requests
                .get(&id)
                .is_some_and(|(_, tracker, key)| *tracker == ticket.tracker && *key == ticket.key)
        {
            return;
        }
        self.ticket_sequence += 1;
        let sequence = self.ticket_sequence;
        self.ticket_requests
            .insert(id, (sequence, ticket.tracker, ticket.key.clone()));
        let backend = self.tickets.clone();
        let requested = (*ticket).clone();
        let work = cx
            .background_executor()
            .spawn(async move { backend.fetch(&requested) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.ticket_refreshed(id, sequence, &ticket, result, report, cx)
            });
        })
        .detach();
    }
    /// A move's snapshot is saved as is, with no fetch; it also supersedes a
    /// fetch already in flight, which would carry the status from before.
    pub(super) fn agent_changed_ticket(
        &mut self,
        id: i64,
        change: crate::runtime::TicketChange,
        cx: &mut Context<Self>,
    ) {
        let crate::runtime::TicketChange::Moved(moved) = change else {
            self.refresh_ticket(id, false, cx);
            return;
        };
        let Some(WorkItem::Ticket(current)) =
            self.session_metadata(id, cx).summary.work_item.clone()
        else {
            return;
        };
        if current.tracker != moved.tracker || current.key != moved.key {
            return;
        }
        self.ticket_requests.remove(&id);
        self.stale_tickets.remove(&id);
        let merged = TicketRef {
            url: moved.url.or(current.url),
            title: moved.title.or(current.title),
            ..moved
        };
        self.set_work_item(id, Some(WorkItem::Ticket(Box::new(merged))), cx);
    }
    fn ticket_refreshed(
        &mut self,
        id: i64,
        sequence: u64,
        requested: &TicketRef,
        result: Result<TicketRef, TicketError>,
        report: bool,
        cx: &mut Context<Self>,
    ) {
        // A newer request for this session (a relink, a manual refresh) owns the outcome.
        if self.ticket_requests.get(&id).map(|(latest, ..)| *latest) != Some(sequence) {
            return;
        }
        self.ticket_requests.remove(&id);
        // Unlinked or relinked while the request was out.
        let still_linked = matches!(
            self.session_metadata(id, cx).summary.work_item.clone(),
            Some(WorkItem::Ticket(current))
                if current.tracker == requested.tracker && current.key == requested.key
        );
        if !still_linked {
            return;
        }
        let placeholder = self.ticket_drafts.remove(&id);
        match result {
            Ok(fresh) => {
                self.stale_tickets.remove(&id);
                if let (Some(placeholder), Some(title), Some(view)) =
                    (placeholder, fresh.title.clone(), self.sessions.get(&id))
                {
                    view.update(cx, |view, cx| {
                        if view.draft_text(cx) == placeholder {
                            view.set_draft_text(title, cx);
                        }
                    });
                }
                self.set_work_item(id, Some(WorkItem::Ticket(Box::new(fresh))), cx);
            }
            Err(error) => {
                if matches!(error, TicketError::NotConnected(_)) {
                    self.stale_tickets.insert(id);
                }
                if report {
                    self.error = Some(error.to_string());
                } else {
                    tracing::warn!(session = id, %error, "ticket refresh failed");
                }
                self.invalidate_chrome(cx);
            }
        }
        cx.notify();
    }
    /// Which trackers have saved credentials, read off the UI thread, so chips
    /// can show a disconnected tracker as stale without touching the keychain
    /// while rendering.
    pub(super) fn refresh_connected_trackers(&mut self, cx: &mut Context<Self>) {
        let backend = self.tickets.clone();
        let work = cx
            .background_executor()
            .spawn(async move { backend.connected() });
        cx.spawn(async move |this, cx| {
            let connected = work.await;
            let _ = this.update(cx, |this, cx| {
                this.connected_trackers = Some(connected.into_iter().collect());
                this.stale_tickets.clear();
                this.invalidate_chrome(cx);
                cx.notify();
            });
        })
        .detach();
    }
    /// LinkSessionWorkItemDialog, centred over the window.
    pub(super) fn link_modal(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.link_dialog.as_ref()?;
        let heading = match (dialog.id, &dialog.card, dialog.editing) {
            (_, Some(CardTarget::New), _) => "New card from ticket",
            (_, Some(CardTarget::Card(_)), true) => "Edit ticket link",
            (_, Some(CardTarget::Card(_)), false) => "Link ticket",
            (None, ..) => "New session from ticket",
            (Some(_), _, true) => "Edit link",
            (Some(_), _, false) => "Link issue, PR or ticket",
        };
        let title = dialog
            .id
            .map(|id| self.session_metadata(id, cx).display_title())
            .unwrap_or_default();
        let viewport = window.viewport_size();
        let width = 384f32.min(viewport.width.as_f32() - 24.);
        let anchor = Anchor::Center;
        let weak = cx.weak_entity();
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .role(Role::Button)
                .cursor_pointer()
                .px_3()
                .py_1p5()
                .rounded_md()
                .text_size(px(12.))
                .child(label)
        };
        let field = div()
            .h(px(36.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded_md()
            .border_1()
            .border_color(if dialog.error.is_some() {
                rgba(0xf8717199).into()
            } else {
                content(0.1)
            })
            .bg(content(0.05))
            .text_size(px(13.))
            .text_color(content(1.))
            .child(dialog.input.clone());
        Some(
            div()
                .absolute()
                .size_full()
                .child(crate::modal::scrim("link-work-item-scrim"))
                .child(
                    popover("link-work-item-modal", anchor)
                        .modal_surface(self.workspace.boxy_layout())
                        .width(width)
                        .gap(0.)
                        .priority(60)
                        .on_dismiss(move |window, cx| {
                            let _ = weak.update(cx, |this, cx| this.close_link_dialog(window, cx));
                        })
                        .child(
                            div()
                                .id("link-work-item-dialog")
                                .role(Role::Dialog)
                                .aria_label(heading)
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .px_4()
                                        .pt_4()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(px(14.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(content(1.))
                                                .child(heading),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(content(0.5))
                                                .truncate()
                                                .child(title),
                                        ),
                                )
                                .child(
                                    div()
                                        .p_4()
                                        .flex()
                                        .flex_col()
                                        .gap_4()
                                        .text_size(px(12.))
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .gap_1p5()
                                                .child(
                                                    div()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(content(0.8))
                                                        .child(if dialog.id.is_some() {
                                                            "Issue, PR or ticket"
                                                        } else {
                                                            "Ticket key or link"
                                                        }),
                                                )
                                                .child(field)
                                                .when_some(dialog.choices.clone(), |d, trackers| {
                                                    d.child(
                                                        div()
                                                            .id("link-tracker-choice")
                                                            .flex()
                                                            .flex_col()
                                                            .gap_1p5()
                                                            .child(
                                                                div()
                                                                    .text_size(px(11.))
                                                                    .text_color(content(0.6))
                                                                    .child("Which tracker is this ticket in?"),
                                                            )
                                                            .child(
                                                                div().flex().gap_2().children(
                                                                    trackers.into_iter().map(|tracker| {
                                                                        crate::modal::ghost_button(
                                                                            SharedString::from(format!(
                                                                                "link-tracker-{}",
                                                                                tracker.as_str()
                                                                            )),
                                                                            tracker.label(),
                                                                        )
                                                                        .debug_selector(move || {
                                                                            format!("link-tracker-{}", tracker.as_str())
                                                                        })
                                                                        .cursor_pointer()
                                                                        .on_click(cx.listener(
                                                                            move |this, _, window, cx| {
                                                                                this.choose_ticket_tracker(
                                                                                    tracker, window, cx,
                                                                                )
                                                                            },
                                                                        ))
                                                                    }),
                                                                ),
                                                            ),
                                                    )
                                                })
                                                .child(match &dialog.error {
                                                    Some(error) => div()
                                                            .id("link-error")
                                                        .role(Role::Alert)
                                                        .text_size(px(11.))
                                                        .text_color(rgb(0xf87171))
                                                        .child(error.clone()),
                                                    None => div()
                                                            .id("link-help")
                                                        .text_size(px(11.))
                                                        .text_color(content(0.45))
                                                        .child(
                                                            if dialog.id.is_some() {
                                                                "Paste a github.com URL, a ticket key like ENG-123 or a Jira, Linear or Plane link."
                                                            } else {
                                                                "Paste a ticket key like ENG-123 or a Jira, Linear or Plane link."
                                                            },
                                                        ),
                                                }),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .justify_end()
                                                .gap_2()
                                                .when(dialog.editing, |d| {
                                                    d.child(
                                                        button("link-remove", "Remove link")
                                                            .mr_auto()
                                                            .text_color(rgb(0xf87171))
                                                            .hover(|s| s.bg(rgba(0xf871711a)))
                                                            .on_click(cx.listener(|this, _, window, cx| {
                                                                this.save_link(false, window, cx)
                                                            })),
                                                    )
                                                })
                                                .child(
                                                    crate::modal::ghost_button("link-cancel", "Cancel")
                                                        .cursor_pointer()
                                                        .on_click(cx.listener(|this, _, window, cx| {
                                                            this.close_link_dialog(window, cx)
                                                        })),
                                                )
                                                .child(
                                                    crate::modal::disabled(
                                                        crate::modal::primary_button(
                                                            "link-save",
                                                            if dialog.checking {
                                                                "Checking…"
                                                            } else if dialog.editing {
                                                                "Update link"
                                                            } else {
                                                                "Link"
                                                            },
                                                        ),
                                                        dialog.checking,
                                                    )
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        this.save_link(true, window, cx)
                                                    })),
                                                ),
                                        ),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

/// "Agent may change ticket status" for one session: follow its boards, or
/// on or off whatever they say. A setting that can't be read offers no
/// choice to tick, and says so.
fn ticket_status_menu(setting: Result<Option<bool>, String>) -> MenuItem {
    let label = "Agent may change ticket status";
    let choices = match setting {
        Ok(current) => vec![
            MenuAction::new("ticket-status:board", "Board default").checked(current.is_none()),
            MenuAction::new("ticket-status:on", "On").checked(current == Some(true)),
            MenuAction::new("ticket-status:off", "Off").checked(current == Some(false)),
        ],
        Err(_) => vec![
            MenuAction::new("ticket-status:error", "Couldn't read this setting").disabled(true),
        ],
    };
    MenuAction::new("ticket-status", label)
        .submenu(choices)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext, WindowHandle};

    fn setup(cx: &mut TestAppContext) -> (WindowHandle<Shell>, i64, i64) {
        setup_at(cx, std::path::Path::new(":memory:"))
    }

    /// [`setup`] on a store saved at `path`, so the board store reads back
    /// what it saved.
    fn setup_at(
        cx: &mut TestAppContext,
        path: &std::path::Path,
    ) -> (WindowHandle<Shell>, i64, i64) {
        cx.update(|cx| {
            crate::shell::bind_keys(cx);
            crate::input::bind(cx);
            crate::popover::bind(cx);
        });
        let (window, a, b) = crate::shell::tests::window_at(cx, path);
        window
            .update(cx, |shell, _, cx| {
                // These tests work with the user's own folders.
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
        cx.refresh().unwrap();
        (window, a, b)
    }

    #[gpui::test]
    fn an_empty_session_list_shows_a_card_that_says_how_to_start_one(cx: &mut TestAppContext) {
        let (window, _, _) = setup(cx);
        cx.refresh().unwrap();
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("sidebar-sessions-empty").is_none());
        window
            .update(visual, |shell, _, cx| {
                // Only working sessions, and none is working.
                shell.workspace.session_filters.status.working = true;
                shell.notify_sidebar(cx);
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        let card = visual
            .debug_bounds("sidebar-sessions-empty")
            .expect("an empty list shows its card");
        let hint = visual
            .debug_bounds("sidebar-sessions-empty-hint")
            .expect("with a hint line");
        let filter = visual.debug_bounds("sidebar-filter-button").unwrap();
        let sidebar = visual.debug_bounds("sidebar-sessions").unwrap();
        // Under the heading, inside the sidebar, the hint inside the card.
        assert!(card.top() >= filter.bottom(), "{card:?} under {filter:?}");
        assert!(sidebar.contains(&card.origin) && card.right() <= sidebar.right());
        assert!(card.contains(&hint.center()), "{hint:?} in {card:?}");
        assert!(card.size.height >= px(40.), "two lines, not a bare label");
        // Clearing the filter brings the sessions back.
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.session_filters.status.working = false;
                shell.notify_sidebar(cx);
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("sidebar-sessions-empty").is_none());
    }

    #[gpui::test]
    fn session_actions_are_enabled_and_bulk_selection_shapes_the_menu(cx: &mut TestAppContext) {
        let (window, a, b) = setup(cx);
        window
            .update(cx, |shell, w, cx| {
                let items = shell.session_menu_items(a, cx);
                for item in items {
                    if let MenuItem::Action(action) = item {
                        assert!(!action.disabled, "{} must be usable", action.label);
                    }
                }
                shell.select_sidebar_session(
                    a,
                    gpui::Modifiers {
                        platform: true,
                        ..Default::default()
                    },
                    w,
                    cx,
                );
                shell.select_sidebar_session(
                    b,
                    gpui::Modifiers {
                        shift: true,
                        ..Default::default()
                    },
                    w,
                    cx,
                );
                assert_eq!(shell.selected_sessions.len(), 2);
                let actions: Vec<_> = shell
                    .session_menu_items(a, cx)
                    .into_iter()
                    .filter_map(|item| match item {
                        MenuItem::Action(action) => Some(action.id),
                        _ => None,
                    })
                    .collect();
                assert!(
                    !actions.iter().any(|id| id == "rename"
                        || id == "copy-session-id"
                        || id == "link-work-item")
                );
                shell.run_menu(MenuTarget::Session(a), "pin", None, w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert!(shell.sessions[&a].read(cx).summary.pinned);
                assert!(shell.sessions[&b].read(cx).summary.pinned);
                let stored = locked(&shell.store).unwrap().sessions().unwrap();
                assert!(stored.iter().all(|s| s.pinned));
            })
            .unwrap();
    }

    #[gpui::test]
    fn folders_rename_move_collapse_color_and_ungroup_with_saved_state(cx: &mut TestAppContext) {
        let (window, a, b) = setup(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.run_menu(MenuTarget::Session(a), "folder-new", None, w, cx)
            })
            .unwrap();
        cx.refresh().unwrap();
        window
            .update(cx, |shell, _, cx| {
                let renaming = shell.renaming.as_ref().unwrap();
                renaming
                    .input
                    .update(cx, |input, cx| input.set_text("Work".into(), cx));
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "enter");
        cx.run_until_parked();
        let folder = window
            .update(cx, |shell, w, cx| {
                assert!(shell.renaming.is_none());
                let folder = shell.project_groups().folder_of(a).unwrap().id.clone();
                shell.run_menu(
                    MenuTarget::Session(b),
                    &format!("folder-add:{folder}"),
                    None,
                    w,
                    cx,
                );
                shell.update_groups(cx, |g| {
                    g.toggle_folder(&folder);
                    g.set_color(&folder, Some(3));
                });
                assert!(shell.sidebar_navigation_ids(cx).is_empty());
                folder
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                let saved = locked(&shell.store)
                    .unwrap()
                    .groups(&shell.workspace.project)
                    .unwrap();
                let saved = saved.folder(&folder).unwrap();
                assert_eq!(saved.name, "Work");
                assert_eq!(saved.color_index, Some(3));
                assert!(saved.collapsed);
                assert!(saved.session_ids.contains(&a) && saved.session_ids.contains(&b));
                shell.run_menu(MenuTarget::Session(b), "folder-remove", None, w, cx);
                assert_eq!(
                    shell.project_groups().folder(&folder).unwrap().session_ids,
                    [a]
                );
                shell.run_menu(MenuTarget::Folder(folder.clone()), "ungroup", None, w, cx);
                assert_eq!(shell.sidebar_navigation_ids(cx).len(), 2);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert!(
                    locked(&shell.store)
                        .unwrap()
                        .groups(&shell.workspace.project)
                        .unwrap()
                        .folders
                        .is_empty()
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn session_rename_accepts_enter_and_escape_cancels(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.run_menu(MenuTarget::Session(a), "rename", None, w, cx);
                shell
                    .renaming
                    .as_ref()
                    .unwrap()
                    .input
                    .update(cx, |i, cx| i.set_text("First name".into(), cx));
            })
            .unwrap();
        cx.refresh().unwrap();
        cx.simulate_keystrokes(window.into(), "enter");
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                assert_eq!(shell.sessions[&a].read(cx).summary.title, "First name");
                let saved = locked(&shell.store).unwrap().sessions().unwrap();
                assert_eq!(
                    saved.iter().find(|s| s.id == a).unwrap().title,
                    "First name"
                );
                shell.run_menu(MenuTarget::Session(a), "rename", None, w, cx);
                shell
                    .renaming
                    .as_ref()
                    .unwrap()
                    .input
                    .update(cx, |i, cx| i.set_text("Discard".into(), cx));
            })
            .unwrap();
        cx.refresh().unwrap();
        cx.simulate_keystrokes(window.into(), "escape");
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert!(shell.renaming.is_none());
                assert_eq!(shell.sessions[&a].read(cx).summary.title, "First name");
            })
            .unwrap();
    }

    #[gpui::test]
    fn links_and_reminders_validate_persist_and_clear_when_due_session_opens(
        cx: &mut TestAppContext,
    ) {
        let (window, a, _) = setup(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.run_menu(MenuTarget::Session(a), "link-work-item", None, w, cx);
                shell
                    .link_dialog
                    .as_ref()
                    .unwrap()
                    .input
                    .update(cx, |i, cx| i.set_text("invalid".into(), cx));
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| shell.save_link(true, w, cx))
            .unwrap();
        // Resolving runs off the UI thread now.
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                assert!(shell.link_dialog.as_ref().unwrap().error.is_some());
                shell
                    .link_dialog
                    .as_ref()
                    .unwrap()
                    .input
                    .update(cx, |i, cx| {
                        i.set_text("https://github.com/acme/app/pull/42".into(), cx)
                    });
                shell.save_link(true, w, cx);
                shell.run_menu(MenuTarget::Session(a), "reminder:tomorrow", None, w, cx);
                shell.run_menu(MenuTarget::Session(a), "reminder:1h", None, w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                let store = locked(&shell.store).unwrap();
                assert_eq!(store.reminders().unwrap().len(), 1);
                assert_eq!(
                    store
                        .sessions()
                        .unwrap()
                        .iter()
                        .find(|s| s.id == a)
                        .unwrap()
                        .work_item
                        .as_ref()
                        .and_then(|w| w.github())
                        .unwrap()
                        .number,
                    42
                );
                drop(store);
                assert!(shell.link_dialog.is_none());
                shell.run_menu(MenuTarget::Session(a), "reminder:cancel", None, w, cx);
                let due_at = gespan_core::time::now_ms().saturating_sub(1);
                shell.reminders.push(Reminder {
                    session_id: a,
                    due_at,
                    fired_at: None,
                });
                shell.store_write("set test reminder", cx, move |store| {
                    store.set_reminders(&[a], due_at)
                });
                shell.poll_reminders(cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                assert!(
                    locked(&shell.store).unwrap().reminders().unwrap()[0]
                        .fired_at
                        .is_some()
                );
                shell.open(a, w, cx);
                assert!(shell.reminders.is_empty());
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert!(
                    locked(&shell.store)
                        .unwrap()
                        .reminders()
                        .unwrap()
                        .is_empty()
                )
            })
            .unwrap();
    }

    #[gpui::test]
    fn dragging_sessions_creates_a_folder_and_moves_into_an_existing_one(cx: &mut TestAppContext) {
        let (window, a, b) = setup(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        let a_row = visual
            .debug_bounds(Box::leak(format!("sidebar-session-{a}").into_boxed_str()))
            .unwrap();
        let b_row = visual
            .debug_bounds(Box::leak(format!("sidebar-session-{b}").into_boxed_str()))
            .unwrap();
        visual.simulate_mouse_down(a_row.center(), MouseButton::Left, Default::default());
        visual.simulate_mouse_move(b_row.center(), Some(MouseButton::Left), Default::default());
        visual.simulate_mouse_up(b_row.center(), MouseButton::Left, Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, w, cx| {
                let groups = shell.project_groups();
                assert_eq!(groups.folders.len(), 1);
                assert_eq!(groups.folders[0].session_ids, [a, b]);
                assert!(shell.renaming.is_some());
                shell.finish_rename(false, w, cx);
                shell.update_groups(cx, |g| g.remove_sessions(&[a]));
            })
            .unwrap();
        visual.refresh().unwrap();
        let folder = window
            .update(visual, |shell, _, _| {
                shell.project_groups().folders[0].id.clone()
            })
            .unwrap();
        let row = visual
            .debug_bounds(Box::leak(format!("sidebar-session-{a}").into_boxed_str()))
            .unwrap();
        let header = visual
            .debug_bounds(Box::leak(
                format!("rail-folder-folder:{folder}").into_boxed_str(),
            ))
            .unwrap();
        visual.simulate_mouse_down(row.center(), MouseButton::Left, Default::default());
        visual.simulate_mouse_move(header.center(), Some(MouseButton::Left), Default::default());
        visual.simulate_mouse_up(header.center(), MouseButton::Left, Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(
                    shell
                        .project_groups()
                        .folder(&folder)
                        .unwrap()
                        .session_ids
                        .contains(&a)
                )
            })
            .unwrap();
    }

    #[gpui::test]
    fn delete_closes_the_session_and_cleans_up_folders_and_reminders(cx: &mut TestAppContext) {
        let (window, a, b) = setup(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.run_menu(MenuTarget::Session(a), "pin", None, w, cx);
                shell.run_menu(MenuTarget::Session(a), "reminder:1h", None, w, cx);
                shell.update_groups(cx, |g| {
                    g.create_folder(&[a], "remove-me".into());
                });
                shell.open(a, w, cx);
                shell.run_menu(MenuTarget::Session(a), "delete", None, w, cx);
                shell.open(a, w, cx);
                assert_ne!(
                    shell.workspace.active,
                    Some(a),
                    "deleting sessions cannot reopen"
                );
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert!(!shell.sessions.contains_key(&a));
                assert!(shell.sessions.contains_key(&b));
                assert!(shell.project_groups().folders.is_empty());
                let store = locked(&shell.store).unwrap();
                assert!(!store.sessions().unwrap().iter().any(|s| s.id == a));
                assert!(store.load(a).map_or(true, |t| t.rows.is_empty()));
                assert!(store.reminders().unwrap().is_empty());
                assert!(
                    store
                        .groups(&shell.workspace.project)
                        .unwrap()
                        .folders
                        .is_empty()
                );
            })
            .unwrap();
    }
    #[gpui::test]
    fn folder_headers_reorder_and_the_color_menu_updates_the_saved_folder(cx: &mut TestAppContext) {
        let (window, a, b) = setup(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.update_groups(cx, |g| {
                    g.create_folder(&[a], "first".into());
                    g.create_folder(&[b], "second".into());
                });
            })
            .unwrap();
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        let first = visual
            .debug_bounds("rail-folder-folder:first")
            .unwrap()
            .center();
        let second = visual
            .debug_bounds("rail-folder-folder:second")
            .unwrap()
            .center();
        visual.simulate_mouse_down(second, MouseButton::Left, Default::default());
        visual.simulate_mouse_move(first, Some(MouseButton::Left), Default::default());
        visual.simulate_mouse_up(first, MouseButton::Left, Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                let g = shell.project_groups();
                assert_eq!(g.folders[0].id, "first");
                assert_eq!(g.folders[1].id, "second");
                assert!(!g.folders[0].collapsed && !g.folders[1].collapsed);
            })
            .unwrap();
        visual.refresh().unwrap();
        let first = visual
            .debug_bounds("rail-folder-folder:first")
            .unwrap()
            .center();
        visual.simulate_mouse_down(first, MouseButton::Right, Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
        let swatch = visual.debug_bounds("folder-color-3").unwrap().center();
        visual.simulate_click(swatch, Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                let saved = locked(&shell.store)
                    .unwrap()
                    .groups(&shell.workspace.project)
                    .unwrap();
                assert_eq!(saved.folders[0].id, "first");
                assert_eq!(saved.folder("first").unwrap().color_index, Some(3));
            })
            .unwrap();
    }

    #[gpui::test]
    fn f2_renames_the_active_session_and_the_focused_folder(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        window
            .update(cx, |shell, w, cx| shell.open(a, w, cx))
            .unwrap();
        cx.simulate_keystrokes(window.into(), "f2");
        cx.run_until_parked();
        window.update(cx, |shell, w, cx| {
            assert!(matches!(shell.renaming.as_ref().unwrap().target, RenameTarget::Session(id) if id == a));
            shell.finish_rename(false, w, cx);
            shell.update_groups(cx, |g| { g.create_folder(&[a], "work".into()); });
        }).unwrap();
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        let folder = visual
            .debug_bounds("rail-folder-folder:work")
            .unwrap()
            .center();
        visual.simulate_click(folder, Default::default());
        visual.simulate_keystrokes("f2");
        visual.run_until_parked();
        window.update(visual, |shell, _, _| {
            assert!(matches!(&shell.renaming.as_ref().unwrap().target, RenameTarget::Folder(id) if id == "work"));
        }).unwrap();
    }

    use crate::runtime::TicketChange;
    use crate::tickets::fakes::FakeTickets;

    fn fake_tickets(
        window: WindowHandle<Shell>,
        cx: &mut TestAppContext,
        connected: &[Tracker],
    ) -> Arc<FakeTickets> {
        let fake = Arc::new(FakeTickets::new(connected));
        window
            .update(cx, |shell, _, _| shell.tickets = fake.clone())
            .unwrap();
        fake
    }

    /// Types into the link dialog and submits it, letting the async resolve land.
    fn submit_link(window: WindowHandle<Shell>, cx: &mut TestAppContext, id: i64, text: &str) {
        let text = text.to_string();
        window
            .update(cx, |shell, w, cx| {
                if shell.link_dialog.is_none() {
                    shell.run_menu(MenuTarget::Session(id), "link-work-item", None, w, cx);
                }
                shell
                    .link_dialog
                    .as_ref()
                    .unwrap()
                    .input
                    .update(cx, |i, cx| i.set_text(text, cx));
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| shell.save_link(true, w, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn saved_ticket(shell: &Shell, id: i64) -> Option<TicketRef> {
        match locked(&shell.store)
            .unwrap()
            .sessions()
            .unwrap()
            .into_iter()
            .find(|s| s.id == id)?
            .work_item?
        {
            WorkItem::Ticket(ticket) => Some(*ticket),
            WorkItem::Github(_) => None,
        }
    }

    fn dialog_error(window: WindowHandle<Shell>, cx: &mut TestAppContext) -> Option<String> {
        window
            .update(cx, |shell, _, _| {
                shell
                    .link_dialog
                    .as_ref()
                    .and_then(|d| d.error.as_ref().map(|e| e.to_string()))
            })
            .unwrap()
    }

    #[gpui::test]
    fn link_dialog_accepts_a_ticket_url_and_saves_it(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        fake_tickets(window, cx, &[Tracker::Linear]);
        // Hold the fetch back: the link must be saved before any fetch runs.
        window
            .update(cx, |shell, w, cx| {
                shell.run_menu(MenuTarget::Session(a), "link-work-item", None, w, cx);
                shell
                    .link_dialog
                    .as_ref()
                    .unwrap()
                    .input
                    .update(cx, |i, cx| {
                        i.set_text("https://linear.app/acme/issue/ENG-7/fix-login".into(), cx)
                    });
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                shell.save_link(true, w, cx);
                let dialog = shell.link_dialog.as_ref().unwrap();
                assert!(dialog.checking, "the dialog shows it is checking");
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert!(shell.link_dialog.is_none());
                let ticket = saved_ticket(shell, a).expect("persisted");
                assert_eq!(
                    (ticket.tracker, ticket.key.as_str()),
                    (Tracker::Linear, "ENG-7")
                );
                assert_eq!(ticket.title.as_deref(), Some("Fix login"));
                assert_eq!(ticket.status.as_deref(), Some("In Progress"));
                assert_eq!(ticket.fetched_at, Some(1_700_000_000_000));
            })
            .unwrap();
    }

    #[gpui::test]
    fn the_link_is_saved_before_the_fetch_returns(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        let fake = fake_tickets(window, cx, &[Tracker::Linear]);
        *fake.offline.lock().unwrap() = true;
        submit_link(window, cx, a, "ENG-7");
        window
            .update(cx, |shell, _, _| {
                let ticket = saved_ticket(shell, a).expect("saved although the fetch failed");
                assert_eq!(ticket.key, "ENG-7");
                assert_eq!(ticket.title, None);
            })
            .unwrap();
    }

    #[gpui::test]
    fn github_urls_still_link_and_unknown_input_saves_nothing(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        fake_tickets(window, cx, &[Tracker::Linear]);
        for (input, message) in [
            (
                "nonsense",
                "Enter a GitHub issue or PR URL, or a ticket key or link",
            ),
            (
                "https://acme.atlassian.net/browse/ENG-7",
                "Connect Jira in Settings",
            ),
        ] {
            submit_link(window, cx, a, input);
            let error = dialog_error(window, cx).unwrap_or_default();
            assert!(error.starts_with(message), "{input}: {error}");
            window
                .update(cx, |shell, _, _| {
                    assert!(shell.session_summaries[&a].work_item.is_none())
                })
                .unwrap();
        }
        submit_link(window, cx, a, "https://github.com/acme/app/issues/5");
        window
            .update(cx, |shell, _, _| {
                assert!(matches!(
                    shell.session_summaries[&a].work_item,
                    Some(WorkItem::Github(_))
                ));
            })
            .unwrap();
    }

    #[gpui::test]
    fn ticket_key_with_no_tracker_connected_points_to_settings(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        fake_tickets(window, cx, &[]);
        submit_link(window, cx, a, "ENG-7");
        let error = dialog_error(window, cx).unwrap();
        assert!(error.contains("Settings"), "{error}");
        window
            .update(cx, |shell, _, _| {
                assert!(shell.session_summaries[&a].work_item.is_none())
            })
            .unwrap();
    }

    #[gpui::test]
    fn editing_the_text_while_checking_discards_the_stale_answer(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        fake_tickets(window, cx, &[Tracker::Linear]);
        window
            .update(cx, |shell, w, cx| {
                shell.run_menu(MenuTarget::Session(a), "link-work-item", None, w, cx);
                let input = shell.link_dialog.as_ref().unwrap().input.clone();
                input.update(cx, |i, cx| i.set_text("ENG-7".into(), cx));
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                shell.save_link(true, w, cx);
                let input = shell.link_dialog.as_ref().unwrap().input.clone();
                input.update(cx, |i, cx| i.set_text("ENG-8".into(), cx));
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert!(
                    shell.link_dialog.is_some(),
                    "the old answer does not close it"
                );
                assert!(shell.session_summaries[&a].work_item.is_none());
                assert!(!shell.link_dialog.as_ref().unwrap().checking);
            })
            .unwrap();
    }

    #[gpui::test]
    fn ambiguous_key_asks_which_tracker(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        fake_tickets(window, cx, &[Tracker::Jira, Tracker::Linear]);
        submit_link(window, cx, a, "eng-7");
        window
            .update(cx, |shell, _, _| {
                let dialog = shell.link_dialog.as_ref().unwrap();
                assert_eq!(dialog.choices, Some(vec![Tracker::Jira, Tracker::Linear]));
                assert!(shell.session_summaries[&a].work_item.is_none());
            })
            .unwrap();
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        visual.refresh().unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("link-tracker-jira").is_some());
        assert!(
            visual.debug_bounds("link-tracker-linear").is_some(),
            "one button per connected tracker"
        );
        window
            .update(visual, |shell, w, cx| {
                shell.choose_ticket_tracker(Tracker::Linear, w, cx);
                assert!(shell.link_dialog.is_none());
                let Some(WorkItem::Ticket(ticket)) = shell.session_summaries[&a].work_item.clone()
                else {
                    panic!("linked")
                };
                assert_eq!(
                    (ticket.tracker, ticket.key.as_str()),
                    (Tracker::Linear, "ENG-7")
                );
            })
            .unwrap();
    }

    #[test]
    fn chip_tone_follows_the_state_group() {
        use crate::ticket_chip::TicketChip;
        assert_eq!(group_tone(StateGroup::Completed), 0x4ade80);
        assert_eq!(group_tone(StateGroup::Cancelled), 0xf87171);
        assert_eq!(group_tone(StateGroup::Backlog), GROUP_UNKNOWN);
        let ticket = TicketRef {
            tracker: Tracker::Jira,
            key: "ENG-1".into(),
            url: None,
            title: Some(String::new()),
            status: None,
            status_group: None,
            fetched_at: None,
        };
        let chip = TicketChip::new(&ticket, true);
        assert_eq!(chip.title, None, "an empty title is not shown");
        assert_eq!(chip.tone, GROUP_UNKNOWN);
    }

    #[gpui::test]
    fn ticket_refreshes_when_the_session_opens_and_never_on_a_timer(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        let fake = fake_tickets(window, cx, &[Tracker::Linear]);
        submit_link(window, cx, a, "ENG-7");
        assert_eq!(fake.fetch_count(), 1, "one fetch on link");
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(3600));
        cx.run_until_parked();
        assert_eq!(fake.fetch_count(), 1, "nothing polls");
        window
            .update(cx, |shell, w, cx| shell.open(a, w, cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(fake.fetch_count(), 2, "one fetch on open");
    }

    #[gpui::test]
    fn a_ticket_tool_call_refreshes_the_chip(cx: &mut TestAppContext) {
        let (window, a, b) = setup(cx);
        let fake = fake_tickets(window, cx, &[Tracker::Linear]);
        submit_link(window, cx, a, "ENG-7");
        assert_eq!(fake.fetch_count(), 1);
        let emit = |id: i64, change: TicketChange, cx: &mut TestAppContext| {
            window
                .update(cx, |shell, _, cx| {
                    let view = shell.sessions.get(&id).unwrap().clone();
                    view.update(cx, |_, cx| {
                        cx.emit(crate::conversation::TicketChanged(change))
                    });
                })
                .unwrap();
            cx.run_until_parked();
        };
        emit(a, TicketChange::Refresh, cx);
        assert_eq!(fake.fetch_count(), 2, "the agent's comment refreshes once");
        emit(b, TicketChange::Refresh, cx);
        assert_eq!(
            fake.fetch_count(),
            2,
            "a session with no ticket fetches nothing"
        );
    }

    #[gpui::test]
    fn an_agent_status_move_is_saved_without_a_fetch(cx: &mut TestAppContext) {
        let (window, a, b) = setup(cx);
        let fake = fake_tickets(window, cx, &[Tracker::Linear]);
        submit_link(window, cx, a, "ENG-7");
        let before = window
            .update(cx, |shell, _, _| saved_ticket(shell, a))
            .unwrap()
            .unwrap();
        assert!(before.title.is_some(), "the link fetch filled the title");
        let moved = TicketRef {
            url: None,
            title: None,
            status: Some("Done".into()),
            status_group: Some(StateGroup::Completed),
            fetched_at: Some(42),
            ..before.clone()
        };
        let emit = |id: i64, moved: TicketRef, cx: &mut TestAppContext| {
            window
                .update(cx, |shell, _, cx| {
                    let view = shell.sessions.get(&id).unwrap().clone();
                    view.update(cx, |_, cx| {
                        cx.emit(crate::conversation::TicketChanged(TicketChange::Moved(
                            moved,
                        )))
                    });
                })
                .unwrap();
            cx.run_until_parked();
        };
        emit(a, moved.clone(), cx);
        let after = window
            .update(cx, |shell, _, _| saved_ticket(shell, a))
            .unwrap()
            .unwrap();
        assert_eq!(after.status.as_deref(), Some("Done"));
        assert_eq!(after.status_group, Some(StateGroup::Completed));
        assert_eq!(after.fetched_at, Some(42));
        assert_eq!(after.title, before.title, "a missing title is kept");
        assert_eq!(after.url, before.url, "a missing url is kept");
        assert_eq!(fake.fetch_count(), 1, "no fetch after a move");
        let in_view = window
            .update(cx, |shell, _, cx| {
                shell.sessions[&a].read(cx).summary.work_item.clone()
            })
            .unwrap();
        assert_eq!(in_view, Some(WorkItem::Ticket(Box::new(after))));
        // A move for another ticket, or a session linked to none, changes nothing.
        emit(
            a,
            TicketRef {
                key: "ENG-8".into(),
                ..moved.clone()
            },
            cx,
        );
        assert_eq!(
            window
                .update(cx, |shell, _, _| saved_ticket(shell, a))
                .unwrap()
                .unwrap()
                .key,
            "ENG-7"
        );
        emit(b, moved, cx);
        assert_eq!(
            window
                .update(cx, |shell, _, _| saved_ticket(shell, b))
                .unwrap(),
            None
        );
        assert_eq!(fake.fetch_count(), 1);
    }

    #[gpui::test]
    fn relinking_while_a_fetch_is_in_flight_still_fetches_the_new_ticket(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        let fake = fake_tickets(window, cx, &[Tracker::Linear]);
        window
            .update(cx, |shell, _, cx| {
                for key in ["ENG-7", "ENG-8"] {
                    let ticket = TicketRef {
                        tracker: Tracker::Linear,
                        key: key.into(),
                        url: None,
                        title: None,
                        status: None,
                        status_group: None,
                        fetched_at: None,
                    };
                    // No parking in between: the first fetch is still out.
                    shell.link_ticket(a, ticket, cx);
                }
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                let ticket = saved_ticket(shell, a).unwrap();
                assert_eq!(ticket.key, "ENG-8");
                assert_eq!(ticket.title.as_deref(), Some("Fix login"));
                assert_eq!(fake.fetch_count(), 2);
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_manual_refresh_during_an_open_refresh_still_reports_its_error(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        let fake = fake_tickets(window, cx, &[Tracker::Linear]);
        submit_link(window, cx, a, "ENG-7");
        *fake.offline.lock().unwrap() = true;
        window
            .update(cx, |shell, _, cx| {
                shell.refresh_ticket(a, false, cx);
                shell.refresh_ticket(a, true, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.error.as_deref(), Some("Could not reach the tracker"));
                assert_eq!(
                    saved_ticket(shell, a).unwrap().title.as_deref(),
                    Some("Fix login"),
                    "the cached title survives a failed refresh"
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn saving_the_same_ticket_again_keeps_its_cached_title_and_status(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        let fake = fake_tickets(window, cx, &[Tracker::Linear]);
        submit_link(window, cx, a, "ENG-7");
        *fake.offline.lock().unwrap() = true;
        submit_link(window, cx, a, "eng-7");
        window
            .update(cx, |shell, _, _| {
                let ticket = saved_ticket(shell, a).unwrap();
                assert_eq!(ticket.title.as_deref(), Some("Fix login"));
                assert_eq!(ticket.status.as_deref(), Some("In Progress"));
            })
            .unwrap();
    }

    #[gpui::test]
    fn unlink_clears_the_ticket(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        fake_tickets(window, cx, &[Tracker::Linear]);
        submit_link(window, cx, a, "ENG-7");
        window
            .update(cx, |shell, w, cx| {
                assert!(saved_ticket(shell, a).is_some());
                shell.run_menu(MenuTarget::Session(a), "ticket-unlink", None, w, cx);
                assert!(shell.session_summaries[&a].work_item.is_none());
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert!(saved_ticket(shell, a).is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn an_in_flight_fetch_does_not_revive_an_unlinked_ticket(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        fake_tickets(window, cx, &[Tracker::Linear]);
        window
            .update(cx, |shell, w, cx| {
                let ticket = TicketRef {
                    tracker: Tracker::Linear,
                    key: "ENG-7".into(),
                    url: None,
                    title: None,
                    status: None,
                    status_group: None,
                    fetched_at: None,
                };
                shell.link_ticket(a, ticket, cx);
                shell.run_menu(MenuTarget::Session(a), "ticket-unlink", None, w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert!(shell.session_summaries[&a].work_item.is_none());
                assert!(saved_ticket(shell, a).is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn open_in_tracker_opens_the_url(cx: &mut TestAppContext) {
        let (window, a, _) = setup(cx);
        fake_tickets(window, cx, &[Tracker::Linear]);
        let labels = |shell: &Shell, cx: &App| -> Vec<String> {
            shell
                .session_menu_items(a, cx)
                .into_iter()
                .filter_map(|item| match item {
                    MenuItem::Action(action) => Some(action.label.to_string()),
                    _ => None,
                })
                .collect()
        };
        window
            .update(cx, |shell, _, cx| {
                assert!(!labels(shell, cx).iter().any(|l| l.starts_with("Open in")));
            })
            .unwrap();
        submit_link(window, cx, a, "ENG-7");
        window
            .update(cx, |shell, w, cx| {
                let labels = labels(shell, cx);
                for expected in ["Open in Linear", "Refresh ticket", "Unlink ticket"] {
                    assert!(
                        labels.iter().any(|l| l == expected),
                        "{expected} in {labels:?}"
                    );
                }
                shell.run_menu(MenuTarget::Session(a), "ticket-open", None, w, cx);
            })
            .unwrap();
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://tracker.test/ENG-7")
        );
    }

    /// The ticket items' "Agent may change ticket status" choices, as
    /// (label, checked).
    fn status_choices(shell: &Shell, id: i64, cx: &App) -> Vec<(String, bool, bool)> {
        shell
            .session_menu_items(id, cx)
            .into_iter()
            .find_map(|item| match item {
                MenuItem::Action(action) if action.label == "Agent may change ticket status" => {
                    Some(action.submenu)
                }
                _ => None,
            })
            .expect("the ticket menu offers the status setting")
            .into_iter()
            .map(|choice| (choice.label.to_string(), choice.checked, choice.disabled))
            .collect()
    }

    #[gpui::test]
    fn ticket_menu_sets_agent_status_permission_on_off_or_board_default(cx: &mut TestAppContext) {
        // A database file, so the board store reads back what it saved.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let (window, a, _) = setup_at(cx, &path);
        let key = window
            .update(cx, |shell, _, _| {
                locked(&shell.store)
                    .unwrap()
                    .persisted_session_id(a)
                    .unwrap()
            })
            .unwrap();
        fake_tickets(window, cx, &[Tracker::Linear]);
        submit_link(window, cx, a, "ENG-7");
        let read = || {
            gespan_core::board::Service::open(&path)
                .unwrap()
                .session_status_setting(&key)
        };
        let choices = |on: [bool; 3]| -> Vec<(String, bool, bool)> {
            ["Board default", "On", "Off"]
                .into_iter()
                .zip(on)
                .map(|(label, checked)| (label.to_string(), checked, false))
                .collect()
        };
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(status_choices(shell, a, cx), choices([true, false, false]))
            })
            .unwrap();
        for (action, saved, checked) in [
            ("ticket-status:off", Some(false), [false, false, true]),
            ("ticket-status:on", Some(true), [false, true, false]),
            ("ticket-status:board", None, [true, false, false]),
        ] {
            window
                .update(cx, |shell, w, cx| {
                    shell.run_menu(MenuTarget::Session(a), action, None, w, cx)
                })
                .unwrap();
            cx.run_until_parked();
            assert_eq!(read(), Ok(saved), "{action}");
            window
                .update(cx, |shell, _, cx| {
                    assert_eq!(status_choices(shell, a, cx), choices(checked), "{action}")
                })
                .unwrap();
        }
    }

    /// R93: building the session menu reads the status setting through one
    /// board store the shell keeps, not a new one per build.
    #[gpui::test]
    fn session_menu_opens_no_new_service_per_build(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let (window, a, _) = setup_at(cx, &path);
        fake_tickets(window, cx, &[Tracker::Linear]);
        submit_link(window, cx, a, "ENG-7");
        let opens = |cx: &mut TestAppContext| {
            window
                .update(cx, |shell, _, _| shell.board_service.opens())
                .unwrap()
        };
        let before = opens(cx);
        for _ in 0..3 {
            window
                .update(cx, |shell, _, cx| {
                    assert_eq!(status_choices(shell, a, cx).len(), 3);
                })
                .unwrap();
        }
        assert_eq!(opens(cx) - before, 1, "one board store for every build");
        // A setting saved from the menu shows in the next build.
        window
            .update(cx, |shell, w, cx| {
                shell.run_menu(MenuTarget::Session(a), "ticket-status:off", None, w, cx)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                let choices = status_choices(shell, a, cx);
                assert!(choices[2].0 == "Off" && choices[2].1, "{choices:?}");
            })
            .unwrap();
        assert_eq!(opens(cx) - before, 1);
        // Another database (the store was swapped) gets its own board store.
        // A copy holds the same sessions.
        let other = dir.path().join("other.db");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("VACUUM INTO ?1", [other.to_string_lossy()])
            .unwrap();
        let store = Arc::new(Mutex::new(gespan_core::store::Store::open(&other).unwrap()));
        window
            .update(cx, |shell, _, cx| {
                shell.store = store.clone();
                status_choices(shell, a, cx);
            })
            .unwrap();
        assert_eq!(opens(cx) - before, 2);
    }

    #[gpui::test]
    fn a_failed_status_setting_read_shows_the_error_not_board_default(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gespan.db");
        let (window, a, _) = setup_at(cx, &path);
        fake_tickets(window, cx, &[Tracker::Linear]);
        submit_link(window, cx, a, "ENG-7");
        window
            .update(cx, |shell, w, cx| {
                shell.run_menu(MenuTarget::Session(a), "ticket-status:off", None, w, cx)
            })
            .unwrap();
        cx.run_until_parked();

        // The board store opens, but the setting's table can't be read.
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("ALTER TABLE board_session_settings RENAME TO moved")
            .unwrap();
        let read_error = vec![("Couldn't read this setting".to_string(), false, true)];
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(status_choices(shell, a, cx), read_error)
            })
            .unwrap();

        // Readable again: the saved "Off" is back.
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("ALTER TABLE moved RENAME TO board_session_settings")
            .unwrap();
        window
            .update(cx, |shell, _, cx| {
                let choices = status_choices(shell, a, cx);
                assert_eq!(choices.len(), 3);
                assert!(choices[2].0 == "Off" && choices[2].1);
            })
            .unwrap();
    }

    #[test]
    fn an_unreadable_status_setting_offers_no_choice() {
        let MenuItem::Action(action) = ticket_status_menu(Err("locked".into())) else {
            panic!("an action")
        };
        assert_eq!(action.submenu.len(), 1);
        assert_eq!(action.submenu[0].label, "Couldn't read this setting");
        assert!(action.submenu[0].disabled && !action.submenu[0].checked);
    }
}
