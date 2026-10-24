//! The task drawer, laid out like gr8r's: a top bar of actions, the title, a
//! grid of properties, the description, subtasks, then Comments and Activity.
//! It docks beside the board instead of covering it.
mod view;
#[cfg(test)]
pub(crate) use view::SEND_HINT;

use super::{BoardView, Event, style, turns};
use crate::{
    input::{self, Input},
    popover::{AnchorSlot, Menu, MenuAction, MenuEvent, MenuItem},
    runtime::locked,
    theme::*,
};
use gpui::{ClipboardItem, Context, Entity, Focusable, Subscription, Window, prelude::*};
use rebase_core::board::{
    Activity,
    extras::{Comment, PRIORITIES, Subtask},
};
use std::collections::HashMap;

const ACTIVITY_LIMIT: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Comments,
    Activity,
}

/// The searchable pick list open under a property row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Picker {
    Projects,
    Labels,
    Link,
    DueDate,
}

/// Which keyboard menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Status,
    Priority,
    Due,
    More,
}

pub(crate) struct CardDetail {
    pub card_id: String,
    title: Entity<Input>,
    body: Entity<Input>,
    /// What the panel last loaded or saved, so only real edits are written.
    loaded_title: String,
    loaded_body: String,
    pub activity: Option<Result<Vec<Activity>, String>>,
    /// The card has more activity than the drawer loads.
    activity_more: bool,
    subtasks: Vec<Subtask>,
    comments: Vec<Comment>,
    tab: Tab,
    picker: Option<Picker>,
    /// What the Link picker offers, read once when it opens.
    link_targets: Vec<LinkTarget>,
    query: Entity<Input>,
    subtask_input: Option<Entity<Input>>,
    comment: Entity<Input>,
    choice: Option<(Choice, Entity<Menu>, Subscription)>,
    anchors: HashMap<&'static str, AnchorSlot>,
    _subscriptions: Vec<Subscription>,
}

impl CardDetail {
    /// The Activity tab's count: "50+" when the card has more than it loads.
    pub(crate) fn activity_badge(&self) -> String {
        let shown = self
            .activity
            .as_ref()
            .and_then(|rows| rows.as_ref().ok())
            .map_or(0, Vec::len);
        if self.activity_more {
            format!("{shown}+")
        } else {
            shown.to_string()
        }
    }

    /// What assistive tech calls the description and comment fields.
    #[cfg(test)]
    pub(crate) fn field_labels(&self, cx: &gpui::App) -> (String, String) {
        (
            self.body.read(cx).label().to_string(),
            self.comment.read(cx).label().to_string(),
        )
    }
}

/// One activity row as the timeline reads it, before its relative time.
pub(crate) fn activity_text(row: &Activity) -> String {
    let what = match row.kind.as_str() {
        "moved" => format!(
            "Moved from {} to {}",
            row.from_column.as_deref().unwrap_or("?"),
            row.to_column.as_deref().unwrap_or("?")
        ),
        "edited" if row.detail.is_some() => {
            format!("Changed {}", row.detail.as_deref().unwrap_or_default())
        }
        kind => {
            let mut chars = kind.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    };
    format!("{what} · {}", row.actor)
}

/// Something a card can link to: link kind, link id, what to call it.
pub(crate) struct LinkTarget {
    kind: &'static str,
    id: String,
    label: String,
}

impl BoardView {
    pub(crate) fn open_card_detail(
        &mut self,
        card_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dragging {
            return;
        }
        self.close_card_detail(cx);
        let Some(card) = self.card(&card_id).cloned() else {
            return;
        };
        let title = cx.new(|cx| Input::field("Task title", "Task title", content(0.4), cx));
        title.update(cx, |input, cx| {
            input.set_font_size(18.);
            input.set_text(card.title.clone(), cx)
        });
        let body = cx.new(|cx| {
            let mut input = Input::commit_message(cx);
            input.set_label("Description");
            input.set_placeholder("Add a description…", cx);
            input
        });
        body.update(cx, |input, cx| input.set_text(card.body.clone(), cx));
        let query = cx.new(|cx| Input::field("Search", "Search…", content(0.4), cx));
        let comment = cx.new(|cx| {
            let mut input = Input::commit_message(cx);
            input.set_label("New comment");
            input.set_placeholder("Leave a comment…", cx);
            input
        });
        let subscriptions = vec![
            cx.subscribe(&title, |this, _, _: &input::Submitted, cx| {
                this.save_card_details(cx)
            }),
            cx.on_blur(&title.focus_handle(cx), window, |this, _, cx| {
                this.save_card_details(cx)
            }),
            cx.on_blur(&body.focus_handle(cx), window, |this, _, cx| {
                this.save_card_details(cx)
            }),
            cx.subscribe(&query, |this, _, _: &input::Submitted, cx| {
                this.submit_query(cx)
            }),
            cx.observe(&query, |_, _, cx| cx.notify()),
            cx.subscribe(&comment, |this, _, _: &input::Submitted, cx| {
                this.send_comment(cx)
            }),
        ];
        self.card_detail = Some(CardDetail {
            card_id,
            title,
            body,
            loaded_title: card.title.clone(),
            loaded_body: card.body.clone(),
            activity: None,
            activity_more: false,
            subtasks: Vec::new(),
            comments: Vec::new(),
            tab: Tab::Comments,
            picker: None,
            link_targets: Vec::new(),
            query,
            subtask_input: None,
            comment,
            choice: None,
            anchors: [
                "status", "priority", "due", "projects", "labels", "linked", "more",
            ]
            .into_iter()
            .map(|key| (key, AnchorSlot::default()))
            .collect(),
            _subscriptions: subscriptions,
        });
        self.refresh_detail_lists();
        self.menu = None;
        cx.notify();
    }

    /// Close the panel, keeping its edits. True when one was open.
    pub(crate) fn close_card_detail(&mut self, cx: &mut Context<Self>) -> bool {
        if self.card_detail.is_none() {
            return false;
        }
        self.save_card_details(cx);
        self.card_detail = None;
        cx.notify();
        true
    }

    /// Escape inside the drawer closes its innermost open thing first.
    pub(crate) fn close_detail_layer(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(detail) = &mut self.card_detail else {
            return false;
        };
        if detail.picker.take().is_some() || detail.subtask_input.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    fn card(&self, card_id: &str) -> Option<&rebase_core::board::Card> {
        self.detail
            .as_ref()?
            .cards
            .iter()
            .find(|card| card.id == card_id)
    }

    fn detail_card(&self) -> Option<rebase_core::board::Card> {
        let id = &self.card_detail.as_ref()?.card_id;
        self.card(id).cloned()
    }

    /// Re-read the drawer's subtasks, comments and activity.
    fn refresh_detail_lists(&mut self) {
        let Some(card) = self.detail_card() else {
            return;
        };
        let lists = self.service().and_then(|service| {
            Ok((
                // One past the limit says whether there is more than shown.
                service.activity(&card.board_id, Some(&card.id), ACTIVITY_LIMIT + 1),
                service.subtasks(&card.id)?,
                service.comments(&card.id)?,
            ))
        });
        let Some(detail) = &mut self.card_detail else {
            return;
        };
        match lists {
            Ok((activity, subtasks, comments)) => {
                detail.activity_more = activity
                    .as_ref()
                    .is_ok_and(|rows| rows.len() > ACTIVITY_LIMIT);
                detail.activity = Some(activity.map(|mut rows| {
                    rows.truncate(ACTIVITY_LIMIT);
                    rows
                }));
                detail.subtasks = subtasks;
                detail.comments = comments;
            }
            // Keep what the drawer showed rather than empty lists.
            Err(error) => {
                detail.activity = Some(Err(error.clone()));
                self.error = Some(format!("Couldn't load this task: {error}"));
            }
        }
    }

    /// Run a write for the open task, then refresh the board and the drawer.
    /// True when the write succeeded.
    fn write_task(
        &mut self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut rebase_core::board::Service) -> Result<(), String>,
    ) -> bool {
        let ok = self.write(cx, action);
        self.refresh_detail_lists();
        cx.notify();
        ok
    }

    fn save_card_details(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &mut self.card_detail else {
            return;
        };
        let typed = detail.title.read(cx).content.trim().to_string();
        let body = detail.body.read(cx).content.to_string();
        // A task needs a title: a cleared one goes back to the saved title,
        // and a description edit still saves with it.
        let title = if typed.is_empty() {
            let saved = detail.loaded_title.clone();
            detail
                .title
                .update(cx, |input, cx| input.set_text(saved.clone(), cx));
            saved
        } else {
            typed
        };
        if title == detail.loaded_title && body == detail.loaded_body {
            return;
        }
        let card_id = detail.card_id.clone();
        let saved = self.write_task(cx, |service| service.card_update(&card_id, &title, &body));
        // Only a write that landed is the new baseline; a failed one keeps
        // the typed text as an unsaved edit.
        if saved
            && let Some(detail) = &mut self.card_detail
            && detail.card_id == card_id
        {
            detail.loaded_title = title;
            detail.loaded_body = body;
        }
    }

    /// After a reload, show changes made elsewhere in fields the user hasn't
    /// touched since the panel last loaded them.
    pub(super) fn sync_card_detail(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.detail_card() else {
            return;
        };
        let Some(detail) = &mut self.card_detail else {
            return;
        };
        let title_input = detail.title.read(cx).content.trim().to_string();
        if title_input == detail.loaded_title && card.title != detail.loaded_title {
            detail.loaded_title = card.title.clone();
            detail
                .title
                .update(cx, |input, cx| input.set_text(card.title, cx));
        }
        let body_input = detail.body.read(cx).content.to_string();
        if body_input == detail.loaded_body && card.body != detail.loaded_body {
            detail.loaded_body = card.body.clone();
            detail
                .body
                .update(cx, |input, cx| input.set_text(card.body, cx));
        }
    }

    #[cfg(test)]
    pub(crate) fn card_detail_title(&self, cx: &gpui::App) -> Option<String> {
        let detail = self.card_detail.as_ref()?;
        Some(detail.title.read(cx).content.to_string())
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if let Some(detail) = &mut self.card_detail {
            detail.tab = tab;
            cx.notify();
        }
    }

    // ---- keyboard menus: status, priority, due date, more ----

    fn open_choice(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        let Some(card) = self.detail_card() else {
            return;
        };
        let items: Vec<MenuItem> = match choice {
            Choice::Status => self
                .detail
                .as_ref()
                .map(|detail| {
                    let mut columns: Vec<_> = detail.columns.iter().collect();
                    columns.sort_by_key(|column| column.position);
                    columns
                        .into_iter()
                        .enumerate()
                        .map(|(index, column)| {
                            MenuAction::new(format!("status-{index}"), column.name.clone()).into()
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Choice::Priority => (0..PRIORITIES.len())
                .rev()
                .map(|p| MenuAction::new(format!("priority-{p}"), PRIORITIES[p]).into())
                .collect(),
            Choice::Due => vec![
                MenuAction::new("due-today", "Today").into(),
                MenuAction::new("due-tomorrow", "Tomorrow").into(),
                MenuAction::new("due-next-week", "Next week").into(),
                MenuAction::new("due-pick", "Pick a date…").into(),
                MenuItem::Separator,
                MenuAction::new("due-clear", "Clear").into(),
            ],
            Choice::More => vec![MenuAction::new("delete", "Delete task").danger().into()],
        };
        let menu = cx.new(|cx| Menu::new("Task", items, window, cx));
        let card_id = card.id.clone();
        let events = cx.subscribe_in(
            &menu,
            window,
            move |this, _, event: &MenuEvent, window, cx| {
                let Some(detail) = &mut this.card_detail else {
                    return;
                };
                let Some((choice, _, _)) = detail.choice.take() else {
                    return;
                };
                if let MenuEvent::Picked(id, _) = event {
                    this.apply_choice(choice, id, &card_id, window, cx);
                }
                cx.notify();
            },
        );
        if let Some(detail) = &mut self.card_detail {
            detail.picker = None;
            detail.choice = Some((choice, menu, events));
        }
        cx.notify();
    }

    fn apply_choice(
        &mut self,
        choice: Choice,
        id: &str,
        card_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let card_id = card_id.to_string();
        match choice {
            Choice::Status => {
                let index = id
                    .strip_prefix("status-")
                    .and_then(|i| i.parse::<usize>().ok());
                let column = index.and_then(|index| {
                    let detail = self.detail.as_ref()?;
                    let mut columns: Vec<_> = detail.columns.iter().collect();
                    columns.sort_by_key(|column| column.position);
                    columns.get(index).map(|column| column.id.clone())
                });
                if let Some(column) = column {
                    self.write_task(cx, |service| {
                        service.card_move(&card_id, &column, usize::MAX, "user")
                    });
                }
            }
            Choice::Priority => {
                if let Some(priority) = id.strip_prefix("priority-").and_then(|p| p.parse().ok()) {
                    self.write_task(cx, |service| service.set_priority(&card_id, priority));
                }
            }
            Choice::Due => {
                let today = chrono::Local::now().date_naive();
                let day = match id {
                    "due-today" => Some(Some(today)),
                    "due-tomorrow" => Some(today.succ_opt()),
                    "due-next-week" => Some(today.checked_add_days(chrono::Days::new(7))),
                    "due-clear" => Some(None),
                    _ => None,
                };
                match day {
                    Some(None) => {
                        self.write_task(cx, |service| service.set_due(&card_id, None));
                    }
                    Some(Some(day)) => self.set_due_day(&card_id, day, cx),
                    None if id == "due-pick" => self.open_picker(Picker::DueDate, window, cx),
                    None => {}
                }
            }
            Choice::More => {
                if id == "delete" {
                    self.delete_card(window, cx);
                }
            }
        }
    }

    // ---- pick lists: projects, labels, linked work, a typed date ----

    fn open_picker(&mut self, picker: Picker, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = &mut self.card_detail else {
            return;
        };
        if detail.picker == Some(picker) {
            detail.picker = None;
            cx.notify();
            return;
        }
        detail.picker = Some(picker);
        detail.choice = None;
        detail.link_targets = Vec::new();
        if picker == Picker::Link {
            let targets = self.link_targets(cx);
            if let Some(detail) = &mut self.card_detail {
                detail.link_targets = targets;
            }
        }
        let Some(detail) = &mut self.card_detail else {
            return;
        };
        let placeholder = match picker {
            Picker::Projects => "Find a project…",
            Picker::Labels => "Find or create a label…",
            Picker::Link => "Find a session or Pilot…",
            Picker::DueDate => "YYYY-MM-DD, then Enter",
        };
        let query = detail.query.clone();
        query.update(cx, |input, cx| {
            input.set_text(String::new(), cx);
            input.set_placeholder(placeholder, cx);
        });
        window.focus(&query.focus_handle(cx), cx);
        cx.notify();
    }

    fn picker_query(&self, cx: &gpui::App) -> String {
        self.card_detail
            .as_ref()
            .map(|detail| detail.query.read(cx).content.trim().to_string())
            .unwrap_or_default()
    }

    /// Due at the start of `day`; a day the local zone skipped is an error,
    /// never a cleared due date.
    fn set_due_day(&mut self, card_id: &str, day: chrono::NaiveDate, cx: &mut Context<Self>) {
        match style::day_start(day) {
            Some(due) => {
                self.write_task(cx, |service| service.set_due(card_id, Some(due)));
            }
            None => {
                self.error = Some(format!("{day} does not exist in this time zone"));
                cx.notify();
            }
        }
    }

    /// Enter in a pick list: a typed date sets it; a label name creates it.
    fn submit_query(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.card_detail.as_ref().and_then(|detail| detail.picker) else {
            return;
        };
        let query = self.picker_query(cx);
        match picker {
            Picker::DueDate => {
                let Some(card) = self.detail_card() else {
                    return;
                };
                match chrono::NaiveDate::parse_from_str(&query, "%Y-%m-%d") {
                    Ok(day) => {
                        if let Some(detail) = &mut self.card_detail {
                            detail.picker = None;
                        }
                        self.set_due_day(&card.id, day, cx);
                    }
                    Err(_) => {
                        self.error = Some(format!("“{query}” is not a date like 2026-11-02"));
                        cx.notify();
                    }
                }
            }
            Picker::Labels if !query.is_empty() => self.create_label(cx),
            _ => {}
        }
    }

    pub(crate) fn card_projects(&self, card_id: &str) -> Vec<String> {
        self.extras
            .get(card_id)
            .map(|extras| extras.projects.clone())
            .unwrap_or_default()
    }

    pub(crate) fn card_labels(&self, card_id: &str) -> Vec<String> {
        self.extras
            .get(card_id)
            .map(|extras| extras.labels.clone())
            .unwrap_or_default()
    }

    fn toggle_project(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(card) = self.detail_card() else {
            return;
        };
        let mut projects = self.card_projects(&card.id);
        match projects.iter().position(|p| *p == path) {
            Some(index) => {
                projects.remove(index);
            }
            None => projects.push(path),
        }
        self.write_task(cx, |service| service.set_projects(&card.id, &projects));
    }

    fn toggle_label(&mut self, label_id: String, cx: &mut Context<Self>) {
        let Some(card) = self.detail_card() else {
            return;
        };
        let mut labels = self.card_labels(&card.id);
        match labels.iter().position(|l| *l == label_id) {
            Some(index) => {
                labels.remove(index);
            }
            None => labels.push(label_id),
        }
        self.write_task(cx, |service| service.set_labels(&card.id, &labels));
    }

    fn create_label(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.detail_card() else {
            return;
        };
        let name = self.picker_query(cx);
        if name.is_empty() {
            return;
        }
        let color = style::next_label_color(self.labels.len());
        let created = self
            .service()
            .and_then(|service| service.create_label(&card.board_id, &name, &color));
        match created {
            Ok(label) => {
                let mut labels = self.card_labels(&card.id);
                labels.push(label.id);
                if let Some(detail) = &self.card_detail {
                    detail
                        .query
                        .update(cx, |input, cx| input.set_text(String::new(), cx));
                }
                self.write_task(cx, |service| service.set_labels(&card.id, &labels));
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    /// The 50 most recent sessions, then every Pilot.
    pub(crate) fn link_targets(&self, cx: &gpui::App) -> Vec<LinkTarget> {
        let mut targets = Vec::new();
        if let Ok(store) = locked(&self.store) {
            let mut sessions = store.sessions().unwrap_or_default();
            sessions.retain(|session| !session.archived);
            sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
            for session in sessions.into_iter().take(50) {
                if let Ok(id) = store.persisted_session_id(session.id) {
                    let label = if session.title.trim().is_empty() {
                        "Untitled session".to_string()
                    } else {
                        session.title
                    };
                    targets.push(LinkTarget {
                        kind: "session",
                        id,
                        label,
                    });
                }
            }
        }
        targets.extend(turns::pilots(cx).into_iter().map(|(id, name)| LinkTarget {
            kind: "pilot",
            id,
            label: name,
        }));
        targets
    }

    fn link_selected(
        &mut self,
        kind: Option<&'static str>,
        id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(card_id) = self.card_detail.as_ref().map(|d| d.card_id.clone()) else {
            return;
        };
        if let Some(detail) = &mut self.card_detail {
            detail.picker = None;
        }
        self.write_task(cx, |service| {
            service.card_link(&card_id, kind, id.as_deref())
        });
    }

    fn open_linked(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.detail_card() else {
            return;
        };
        match (card.link_kind.as_deref(), card.link_id) {
            (Some("session"), Some(id)) => {
                let native = locked(&self.store)
                    .ok()
                    .and_then(|store| store.resolve_existing_session_id(&id));
                match native {
                    Some(native) => cx.emit(Event::OpenSession(native)),
                    None => {
                        self.error = Some("That session no longer exists".into());
                        cx.notify();
                    }
                }
            }
            (Some("pilot"), Some(id)) => cx.emit(Event::OpenPilot(id)),
            _ => {}
        }
    }

    // ---- subtasks and comments ----

    fn start_subtask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input =
            cx.new(|cx| Input::field("New subtask", "Subtask, then Enter", content(0.4), cx));
        self._subscriptions
            .push(cx.subscribe(&input, |this, _, _: &input::Submitted, cx| {
                this.commit_subtask(cx)
            }));
        window.focus(&input.focus_handle(cx), cx);
        if let Some(detail) = &mut self.card_detail {
            detail.subtask_input = Some(input);
        }
        cx.notify();
    }

    /// Add the typed subtask and keep the field open for the next one.
    fn commit_subtask(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.detail_card() else {
            return;
        };
        let Some(input) = self
            .card_detail
            .as_ref()
            .and_then(|detail| detail.subtask_input.clone())
        else {
            return;
        };
        let title = input.read(cx).content.trim().to_string();
        if title.is_empty() {
            return;
        }
        input.update(cx, |input, cx| input.set_text(String::new(), cx));
        self.write_task(cx, |service| {
            service.add_subtask(&card.id, &title).map(drop)
        });
    }

    fn toggle_subtask(&mut self, subtask: Subtask, cx: &mut Context<Self>) {
        self.write_task(cx, |service| {
            service.toggle_subtask(&subtask.id, !subtask.done)
        });
    }

    fn delete_subtask(&mut self, subtask_id: String, cx: &mut Context<Self>) {
        self.write_task(cx, |service| service.delete_subtask(&subtask_id));
    }

    fn send_comment(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.detail_card() else {
            return;
        };
        let Some(input) = self
            .card_detail
            .as_ref()
            .map(|detail| detail.comment.clone())
        else {
            return;
        };
        let body = input.read(cx).content.trim().to_string();
        if body.is_empty() {
            return;
        }
        input.update(cx, |input, cx| input.set_text(String::new(), cx));
        self.write_task(cx, |service| service.add_comment(&card.id, &body).map(drop));
    }

    // ---- top bar ----

    /// The step with the Done role, which Mark complete moves a task to.
    fn done_column(&self) -> Option<String> {
        self.detail
            .as_ref()?
            .columns
            .iter()
            .find(|column| column.role.as_deref() == Some("done"))
            .map(|column| column.id.clone())
    }

    /// The task is complete when it sits in the Done step.
    fn task_done(&self, card: &rebase_core::board::Card) -> bool {
        self.done_column().as_deref() == Some(card.column_id.as_str())
    }

    fn mark_complete(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.detail_card() else {
            return;
        };
        match self.done_column() {
            Some(done) if done != card.column_id => {
                self.write_task(cx, |service| {
                    service.card_move(&card.id, &done, usize::MAX, "user")
                });
            }
            Some(_) => {}
            None => {
                self.error = Some("No step has the Done role. Set one from a step's menu.".into());
                cx.notify();
            }
        }
    }

    fn copy_reference(&mut self, cx: &mut Context<Self>) {
        if let Some(card) = self.detail_card() {
            cx.write_to_clipboard(ClipboardItem::new_string(format!(
                "#{} {}",
                card.number, card.title
            )));
        }
    }

    fn delete_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(card) = self.detail_card() else {
            return;
        };
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Delete “{}”?", card.title),
            Some("Its description, subtasks, comments and activity are removed."),
            &["Delete", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update(cx, |this, cx| {
                    this.card_detail = None;
                    this.write(cx, |service| service.card_delete(&card.id));
                });
            }
        })
        .detach();
    }
}
