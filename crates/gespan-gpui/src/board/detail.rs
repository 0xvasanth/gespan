//! The task drawer, laid out like gr8r's: a top bar of actions, the title, a
//! grid of properties, the description, subtasks, then Comments and Activity.
//! It docks beside the board instead of covering it.
mod more;
mod view;
#[cfg(test)]
pub(crate) use view::{SEND_HINT, comment_author};

use super::{BoardView, Event, quick, style, turns};
use crate::{
    input::{self, Input},
    popover::{AnchorSlot, Menu, MenuAction, MenuEvent, MenuItem},
    runtime::locked,
    theme::*,
};
use gespan_core::board::{
    Activity,
    extras::{CardLink, Comment, Field, Relation, Subtask},
};
use gpui::{ClipboardItem, Context, Entity, Focusable, Subscription, Window, prelude::*};
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
    StartDate,
    Estimate,
    /// Search the board's other cards to relate this one to.
    Relation(RelationKind),
    /// A text, number or typed-date field, by its index in the board's fields.
    Field(usize),
    /// Add a session or Pilot to the links list.
    AddLink(&'static str),
    PrUrl,
    WorktreePath,
}

/// How a picked card relates to the open one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RelationKind {
    Blocks,
    BlockedBy,
    RelatesTo,
}

/// Which keyboard menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Status,
    Priority,
    Due,
    Start,
    /// A subtask's menu, by its index in the list.
    Subtask(usize),
    /// A date field's quick picks, by field index.
    FieldDate(usize),
    FieldSelect(usize),
    LinkAdd,
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
    /// Enter in the subtask field, replaced each time the field opens.
    subtask_submit: Option<Subscription>,
    comment: Entity<Input>,
    choice: Option<(Choice, Entity<Menu>, Subscription)>,
    anchors: HashMap<String, AnchorSlot>,
    /// Start date, estimate (minutes), relations, links and custom fields,
    /// read with the other lists.
    start_at: Option<i64>,
    estimate: Option<i64>,
    relations: Vec<Relation>,
    links: Vec<CardLink>,
    fields: Vec<Field>,
    field_values: HashMap<String, String>,
    /// Why the open pick list's last entry was refused, shown under it.
    inline_error: Option<String>,
    /// The detail fills the board area instead of docking beside it.
    full_page: bool,
    _subscriptions: Vec<Subscription>,
}

impl CardDetail {
    pub(crate) fn full_page(&self) -> bool {
        self.full_page
    }

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

    /// The description and subtask fields' current text.
    #[cfg(test)]
    pub(crate) fn drafts(&self, cx: &gpui::App) -> (String, Option<String>) {
        (
            self.body.read(cx).content.to_string(),
            self.subtask_input
                .as_ref()
                .map(|input| input.read(cx).content.to_string()),
        )
    }

    /// The description's text and selection.
    #[cfg(test)]
    pub(crate) fn description_state(&self, cx: &gpui::App) -> (String, std::ops::Range<usize>) {
        let input = self.body.read(cx);
        (input.content.to_string(), input.selected_range())
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
        self.cancel_keyboard_drag(cx);
        self.close_card_detail(cx);
        if self.card_detail.is_some() {
            // The open task's edits could not be saved; it stays open.
            return;
        }
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
                this.save_card_details(cx);
            }),
            cx.on_blur(&title.focus_handle(cx), window, |this, _, cx| {
                this.save_card_details(cx);
            }),
            cx.on_blur(&body.focus_handle(cx), window, |this, _, cx| {
                this.save_card_details(cx);
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
            subtask_submit: None,
            comment,
            choice: None,
            anchors: [
                "status",
                "priority",
                "due",
                "start",
                "estimate",
                "projects",
                "labels",
                "linked",
                "ticket",
                "more",
                "blocks",
                "blocked-by",
                "relates",
                "links-add",
            ]
            .into_iter()
            .map(|key| (key.to_string(), AnchorSlot::default()))
            .collect(),
            start_at: None,
            estimate: None,
            relations: Vec::new(),
            links: Vec::new(),
            fields: Vec::new(),
            field_values: HashMap::new(),
            inline_error: None,
            full_page: self.open_in_page,
            _subscriptions: subscriptions,
        });
        self.refresh_detail_lists();
        self.menu = None;
        if self.card_tickets.contains_key(&card.id) {
            self.load_connected_trackers(cx);
            self.refresh_card_ticket(&card.id, false, false, cx);
        }
        cx.notify();
    }

    /// Close the panel, keeping its edits. True when one was open.
    pub(crate) fn close_card_detail(&mut self, cx: &mut Context<Self>) -> bool {
        if self.card_detail.is_none() {
            return false;
        }
        // A save that fails keeps the drawer, and its text, open with the error.
        if self.save_card_details(cx) {
            self.card_detail = None;
        }
        cx.notify();
        true
    }

    /// Escape inside the drawer closes its innermost open thing first.
    pub(crate) fn close_detail_layer(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(detail) = &mut self.card_detail else {
            return false;
        };
        if detail.picker.take().is_some() || detail.subtask_input.take().is_some() {
            detail.inline_error = None;
            cx.notify();
            return true;
        }
        false
    }

    /// Escape in the full page goes back to the drawer. True when it did.
    pub(crate) fn leave_full_page(&mut self, cx: &mut Context<Self>) -> bool {
        match &mut self.card_detail {
            Some(detail) if detail.full_page => {
                detail.full_page = false;
                cx.notify();
                true
            }
            _ => false,
        }
    }

    fn toggle_full_page(&mut self, cx: &mut Context<Self>) {
        if let Some(detail) = &mut self.card_detail {
            detail.full_page = !detail.full_page;
            cx.notify();
        }
    }

    pub(super) fn card(&self, card_id: &str) -> Option<&gespan_core::board::Card> {
        self.detail
            .as_ref()?
            .cards
            .iter()
            .find(|card| card.id == card_id)
    }

    fn detail_card(&self) -> Option<gespan_core::board::Card> {
        let id = &self.card_detail.as_ref()?.card_id;
        self.card(id).cloned()
    }

    /// Re-read the drawer's subtasks, comments and activity. Each list keeps
    /// what it showed when its own read fails, and the failure is reported.
    pub(super) fn refresh_detail_lists(&mut self) {
        let Some(card) = self.detail_card() else {
            return;
        };
        let (activity, subtasks, comments) = match self.service() {
            Ok(service) => (
                // One past the limit says whether there is more than shown.
                service.activity(&card.board_id, Some(&card.id), ACTIVITY_LIMIT + 1),
                service.subtasks(&card.id),
                service.comments(&card.id),
            ),
            Err(error) => (Err(error.clone()), Err(error.clone()), Err(error)),
        };
        let extra = match self.service() {
            Ok(service) => (|| {
                Ok::<_, String>((
                    service.card_meta(&card.id)?,
                    service.card_relations(&card.id)?,
                    service.card_links(&card.id)?,
                    service.list_fields(&card.board_id)?,
                    service.field_values(&card.id)?,
                ))
            })(),
            Err(error) => Err(error),
        };
        let Some(detail) = &mut self.card_detail else {
            return;
        };
        let mut failed = None;
        match extra {
            Ok((meta, relations, links, fields, values)) => {
                detail.start_at = meta.start_at;
                detail.estimate = meta.estimate_minutes;
                detail.relations = relations;
                detail.links = links;
                for index in 0..fields.len() {
                    detail.anchors.entry(format!("field-{index}")).or_default();
                }
                detail.fields = fields;
                detail.field_values = values
                    .into_iter()
                    .map(|value| (value.field_id, value.value_json))
                    .collect();
            }
            Err(error) => failed = Some(error),
        }
        detail.activity_more = activity
            .as_ref()
            .is_ok_and(|rows| rows.len() > ACTIVITY_LIMIT);
        detail.activity = Some(activity.map(|mut rows| {
            rows.truncate(ACTIVITY_LIMIT);
            rows
        }));
        match subtasks {
            Ok(subtasks) => {
                for index in 0..subtasks.len() {
                    detail
                        .anchors
                        .entry(format!("subtask-{index}"))
                        .or_default();
                }
                detail.subtasks = subtasks
            }
            Err(error) => failed = Some(error),
        }
        match comments {
            Ok(comments) => detail.comments = comments,
            Err(error) => failed = Some(error),
        }
        if let Some(error) = failed {
            self.error = Some(format!("Couldn't load this task: {error}"));
        }
    }

    /// Run a write for the open task, then refresh the board and the drawer.
    /// True when the write succeeded.
    fn write_task(
        &mut self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut gespan_core::board::Service) -> Result<(), String>,
    ) -> bool {
        let ok = self.write(cx, action);
        self.refresh_detail_lists();
        cx.notify();
        ok
    }

    /// Save typed title and description edits. False when a save failed.
    fn save_card_details(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(detail) = &mut self.card_detail else {
            return true;
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
            return true;
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
        saved
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

    /// Select a range of the description, as dragging over it would.
    #[cfg(test)]
    pub(crate) fn select_description(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) {
        if let Some(detail) = &self.card_detail {
            detail
                .body
                .update(cx, |input, cx| input.select_range(range, cx));
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
                        .map(|column| {
                            MenuAction::new(format!("status-{}", column.id), column.name.clone())
                                .into()
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Choice::Priority => quick::priority_items(&self.priority),
            Choice::Due => quick::due_items(),
            Choice::Start => quick::date_items("start"),
            Choice::FieldDate(_) => quick::date_items("field"),
            Choice::FieldSelect(index) => {
                let options = self
                    .card_detail
                    .as_ref()
                    .and_then(|detail| detail.fields.get(index))
                    .map(|field| quick::field_options(field.options_json.as_deref()))
                    .unwrap_or_default();
                let mut items: Vec<MenuItem> = options
                    .into_iter()
                    .enumerate()
                    .map(|(n, option)| MenuAction::new(format!("field-option-{n}"), option).into())
                    .collect();
                items.push(MenuItem::Separator);
                items.push(MenuAction::new("field-clear", "Clear").into());
                items
            }
            Choice::LinkAdd => vec![
                MenuAction::new("link-session", "Session…").into(),
                MenuAction::new("link-pilot", "Pilot…").into(),
                MenuAction::new("link-pr", "PR URL…").into(),
                MenuAction::new("link-worktree", "Worktree path…").into(),
            ],
            Choice::Subtask(index) => {
                let Some(detail) = &self.card_detail else {
                    return;
                };
                let Some(subtask) = detail.subtasks.get(index) else {
                    return;
                };
                let last = detail.subtasks.len() - 1;
                // Named by id, so a reload that reorders the list while the
                // menu is open cannot change which subtask is picked.
                let id = &subtask.id;
                let mut items: Vec<MenuItem> = Vec::new();
                if index > 0 {
                    items.push(MenuAction::new(format!("subtask-up-{id}"), "Move up").into());
                }
                if index < last {
                    items.push(MenuAction::new(format!("subtask-down-{id}"), "Move down").into());
                }
                items.push(
                    MenuAction::new(format!("subtask-convert-{id}"), "Convert to card").into(),
                );
                items.push(MenuItem::Separator);
                items.push(
                    MenuAction::new(format!("subtask-delete-{id}"), "Delete subtask")
                        .danger()
                        .into(),
                );
                items
            }
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
            detail.inline_error = None;
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
                // The step is named by id, so a reload that reorders steps
                // while the menu is open cannot change what was picked.
                let column = id
                    .strip_prefix("status-")
                    .filter(|column| self.column(column).is_some())
                    .map(str::to_string);
                let current = self.card(&card_id).map(|card| card.column_id.clone());
                if let Some(column) = column
                    && current.as_ref() != Some(&column)
                {
                    self.write_task(cx, |service| {
                        service.card_move(&card_id, &column, usize::MAX, "user")
                    });
                }
            }
            Choice::Priority => {
                if let Some(priority) = quick::parse_priority(id) {
                    self.write_task(cx, |service| service.set_priority(&card_id, priority));
                }
            }
            Choice::Due => match quick::parse_due(id, chrono::Local::now().date_naive()) {
                Some(quick::DueChoice::Clear) => {
                    self.write_task(cx, |service| service.set_due(&card_id, None));
                }
                Some(quick::DueChoice::Day(day)) => self.set_due_day(&card_id, day, cx),
                Some(quick::DueChoice::Pick) => self.open_picker(Picker::DueDate, window, cx),
                None => {}
            },
            Choice::Start => {
                match quick::parse_date("start", id, chrono::Local::now().date_naive()) {
                    Some(quick::DueChoice::Clear) => {
                        self.write_task(cx, |service| service.set_start(&card_id, None));
                    }
                    Some(quick::DueChoice::Day(day)) => self.set_start_day(&card_id, day, cx),
                    Some(quick::DueChoice::Pick) => self.open_picker(Picker::StartDate, window, cx),
                    None => {}
                }
            }
            Choice::FieldDate(index) => {
                match quick::parse_date("field", id, chrono::Local::now().date_naive()) {
                    Some(quick::DueChoice::Clear) => self.set_field(index, None, cx),
                    Some(quick::DueChoice::Day(day)) => self.set_field_day(index, day, cx),
                    Some(quick::DueChoice::Pick) => {
                        self.open_picker(Picker::Field(index), window, cx)
                    }
                    None => {}
                }
            }
            Choice::FieldSelect(index) => {
                if id == "field-clear" {
                    self.set_field(index, None, cx);
                } else if let Some(option) = id
                    .strip_prefix("field-option-")
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|n| {
                        let field = self.card_detail.as_ref()?.fields.get(index)?;
                        quick::field_options(field.options_json.as_deref())
                            .into_iter()
                            .nth(n)
                    })
                {
                    let value = serde_json::Value::String(option).to_string();
                    self.set_field(index, Some(value), cx);
                }
            }
            Choice::LinkAdd => match id {
                "link-session" => self.open_picker(Picker::AddLink("session"), window, cx),
                "link-pilot" => self.open_picker(Picker::AddLink("pilot"), window, cx),
                "link-pr" => self.open_picker(Picker::PrUrl, window, cx),
                "link-worktree" => self.open_picker(Picker::WorktreePath, window, cx),
                _ => {}
            },
            Choice::Subtask(_) => {
                if let Some(subtask) = id.strip_prefix("subtask-up-") {
                    self.shift_subtask(subtask, -1, cx);
                } else if let Some(subtask) = id.strip_prefix("subtask-down-") {
                    self.shift_subtask(subtask, 1, cx);
                } else if let Some(subtask) = id.strip_prefix("subtask-convert-") {
                    self.convert_subtask(subtask.to_string(), cx);
                } else if let Some(subtask) = id.strip_prefix("subtask-delete-") {
                    self.delete_subtask(subtask.to_string(), cx);
                }
            }
        }
    }

    // ---- pick lists: projects, labels, linked work, a typed date ----

    pub(super) fn open_picker(
        &mut self,
        picker: Picker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        detail.inline_error = None;
        detail.link_targets = Vec::new();
        if matches!(picker, Picker::Link | Picker::AddLink(_)) {
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
            Picker::AddLink("pilot") => "Find a Pilot…",
            Picker::AddLink(_) => "Find a session…",
            Picker::DueDate | Picker::StartDate => "YYYY-MM-DD, then Enter",
            Picker::Estimate => "90m or 1h 30m, then Enter",
            Picker::Relation(_) => "Find a card by #number or title…",
            Picker::Field(index) => match detail.fields.get(index).map(|f| f.kind.as_str()) {
                Some("number") => "A number, then Enter",
                Some("date") => "YYYY-MM-DD, then Enter",
                Some("url") => "https://…, then Enter",
                Some("multi-select") => "Pick options below",
                _ => "Type a value, then Enter",
            },
            Picker::PrUrl => "https://…, then Enter",
            Picker::WorktreePath => "/path/to/worktree, then Enter",
        };
        // Editing a text or number field starts from its saved value.
        let start = match picker {
            Picker::Field(index) => detail
                .fields
                .get(index)
                .filter(|field| matches!(field.kind.as_str(), "text" | "number" | "url"))
                .and_then(|field| {
                    quick::field_text(&field.kind, detail.field_values.get(&field.id)?)
                })
                .unwrap_or_default(),
            _ => String::new(),
        };
        let query = detail.query.clone();
        query.update(cx, |input, cx| {
            input.set_text(start, cx);
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
    pub(super) fn set_due_day(
        &mut self,
        card_id: &str,
        day: chrono::NaiveDate,
        cx: &mut Context<Self>,
    ) {
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
            Picker::StartDate => {
                let Some(card) = self.detail_card() else {
                    return;
                };
                match chrono::NaiveDate::parse_from_str(&query, "%Y-%m-%d") {
                    Ok(day) => {
                        if let Some(detail) = &mut self.card_detail {
                            detail.picker = None;
                        }
                        self.set_start_day(&card.id, day, cx);
                    }
                    Err(_) => {
                        self.inline_error(format!("“{query}” is not a date like 2026-11-02"), cx)
                    }
                }
            }
            Picker::Estimate => self.submit_estimate(&query, cx),
            Picker::Field(index) => self.submit_field(index, &query, cx),
            Picker::PrUrl => {
                if quick::valid_pr_url(&query) {
                    self.add_link_ref("pr", query, cx);
                } else {
                    self.inline_error("A PR link must start with http:// or https://".into(), cx);
                }
            }
            Picker::WorktreePath => {
                if query.is_empty() {
                    self.inline_error("Enter a folder path".into(), cx);
                } else {
                    self.add_link_ref("worktree", query, cx);
                }
            }
            Picker::Labels if !query.is_empty() => self.create_label(cx),
            _ => {}
        }
    }

    /// Show a refusal under the open pick list; nothing is written.
    fn inline_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(detail) = &mut self.card_detail {
            detail.inline_error = Some(message);
        }
        cx.notify();
    }

    /// Like `write_task`, but a refusal shows under the open pick list and
    /// the list stays open. A write that lands closes it. True on success.
    fn write_inline(
        &mut self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut gespan_core::board::Service) -> Result<(), String>,
    ) -> bool {
        match self.service().and_then(action) {
            Ok(()) => {
                self.error = None;
                if let Some(detail) = &mut self.card_detail {
                    detail.inline_error = None;
                    detail.picker = None;
                }
                self.reload(cx);
                self.refresh_detail_lists();
                cx.notify();
                true
            }
            Err(error) => {
                self.inline_error(error, cx);
                false
            }
        }
    }

    pub(super) fn set_start_day(
        &mut self,
        card_id: &str,
        day: chrono::NaiveDate,
        cx: &mut Context<Self>,
    ) {
        match style::day_start(day) {
            Some(start) => {
                self.write_task(cx, |service| service.set_start(card_id, Some(start)));
            }
            None => {
                self.error = Some(format!("{day} does not exist in this time zone"));
                cx.notify();
            }
        }
    }

    fn submit_estimate(&mut self, query: &str, cx: &mut Context<Self>) {
        let Some(card_id) = self.card_detail.as_ref().map(|d| d.card_id.clone()) else {
            return;
        };
        // Empty clears the estimate.
        let minutes = if query.is_empty() {
            None
        } else {
            match quick::parse_estimate(query) {
                Ok(minutes) => Some(minutes),
                Err(error) => return self.inline_error(error, cx),
            }
        };
        self.write_inline(cx, |service| service.set_estimate(&card_id, minutes));
    }

    // ---- relations, custom fields, links ----

    fn relate(&mut self, kind: RelationKind, other_id: String, cx: &mut Context<Self>) {
        let Some(card_id) = self.card_detail.as_ref().map(|d| d.card_id.clone()) else {
            return;
        };
        // "Blocked by" is the other card blocking this one.
        let (from, to, name) = match kind {
            RelationKind::Blocks => (card_id, other_id, "blocks"),
            RelationKind::BlockedBy => (other_id, card_id, "blocks"),
            RelationKind::RelatesTo => (card_id, other_id, "relates"),
        };
        self.write_inline(cx, |service| service.add_relation(&from, &to, name));
    }

    fn unrelate(&mut self, relation: Relation, cx: &mut Context<Self>) {
        self.write_task(cx, |service| {
            service.remove_relation(&relation.card_id, &relation.related_card_id, &relation.kind)
        });
    }

    /// Write a field value (`None` clears it) for the field at `index`.
    fn set_field(&mut self, index: usize, value: Option<String>, cx: &mut Context<Self>) {
        let Some(detail) = &self.card_detail else {
            return;
        };
        let Some(field) = detail.fields.get(index) else {
            return;
        };
        let (card_id, field_id) = (detail.card_id.clone(), field.id.clone());
        self.write_inline(cx, |service| {
            service.set_field_value(&card_id, &field_id, value.as_deref())
        });
    }

    fn set_field_day(&mut self, index: usize, day: chrono::NaiveDate, cx: &mut Context<Self>) {
        match style::day_start(day) {
            Some(at) => self.set_field(index, Some(at.to_string()), cx),
            None => self.inline_error(format!("{day} does not exist in this time zone"), cx),
        }
    }

    fn submit_field(&mut self, index: usize, query: &str, cx: &mut Context<Self>) {
        let Some(kind) = self
            .card_detail
            .as_ref()
            .and_then(|d| d.fields.get(index))
            .map(|field| field.kind.clone())
        else {
            return;
        };
        if kind == "date" {
            if query.is_empty() {
                return self.set_field(index, None, cx);
            }
            return match chrono::NaiveDate::parse_from_str(query, "%Y-%m-%d") {
                Ok(day) => self.set_field_day(index, day, cx),
                Err(_) => self.inline_error(format!("“{query}” is not a date like 2026-11-02"), cx),
            };
        }
        match quick::encode_field(&kind, query) {
            Ok(value) => self.set_field(index, value, cx),
            Err(error) => self.inline_error(error, cx),
        }
    }

    /// Add or drop one option of a multi-select field; the pick list stays
    /// open for the next one. Built on the value stored now.
    fn toggle_multi_option(&mut self, index: usize, option: &str, cx: &mut Context<Self>) {
        let Some(detail) = &self.card_detail else {
            return;
        };
        let Some(field) = detail.fields.get(index) else {
            return;
        };
        let (card_id, field_id) = (detail.card_id.clone(), field.id.clone());
        let option = option.to_string();
        let result = self.service().and_then(|service| {
            let current = service
                .field_values(&card_id)?
                .into_iter()
                .find(|value| value.field_id == field_id)
                .map(|value| value.value_json);
            let mut picks = quick::field_picks(current.as_deref());
            match picks.iter().position(|pick| *pick == option) {
                Some(at) => {
                    picks.remove(at);
                }
                None => picks.push(option),
            }
            let value = (!picks.is_empty()).then(|| serde_json::json!(picks).to_string());
            service.set_field_value(&card_id, &field_id, value.as_deref())
        });
        match result {
            Ok(()) => {
                self.error = None;
                if let Some(detail) = &mut self.card_detail {
                    detail.inline_error = None;
                }
                self.reload(cx);
                self.refresh_detail_lists();
                cx.notify();
            }
            Err(error) => self.inline_error(error, cx),
        }
    }

    fn toggle_checkbox_field(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(detail) = &self.card_detail else {
            return;
        };
        let Some(field) = detail.fields.get(index) else {
            return;
        };
        let on = quick::field_checked(detail.field_values.get(&field.id).map(String::as_str));
        self.set_field(index, Some((!on).to_string()), cx);
    }

    fn add_link_ref(&mut self, kind: &'static str, reference: String, cx: &mut Context<Self>) {
        let Some(card_id) = self.card_detail.as_ref().map(|d| d.card_id.clone()) else {
            return;
        };
        self.write_inline(cx, |service| {
            service.add_link(&card_id, kind, &reference, None)
        });
    }

    fn remove_link_row(&mut self, link: CardLink, cx: &mut Context<Self>) {
        self.write_task(cx, |service| {
            service.remove_link(&link.card_id, &link.kind, &link.reference)
        });
    }

    /// Open what a links-list row points at.
    fn open_link_row(&mut self, link: CardLink, cx: &mut Context<Self>) {
        match link.kind.as_str() {
            "pr" => cx.open_url(&link.reference),
            "worktree" => cx.reveal_path(std::path::Path::new(&link.reference)),
            "session" => self.open_session(&link.reference, cx),
            "pilot" => cx.emit(Event::OpenPilot(link.reference)),
            _ => {}
        }
    }

    fn open_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let native = locked(&self.store)
            .ok()
            .and_then(|store| store.resolve_existing_session_id(id));
        match native {
            Some(native) => cx.emit(Event::OpenSession(native)),
            None => {
                self.error = Some("That session no longer exists".into());
                cx.notify();
            }
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
        let labels = quick::toggled(&self.card_labels(&card.id), &label_id);
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
            (Some("session"), Some(id)) => self.open_session(&id, cx),
            (Some("pilot"), Some(id)) => cx.emit(Event::OpenPilot(id)),
            _ => {}
        }
    }

    // ---- subtasks and comments ----

    fn start_subtask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input =
            cx.new(|cx| Input::field("New subtask", "Subtask, then Enter", content(0.4), cx));
        let submit = cx.subscribe(&input, |this, _, _: &input::Submitted, cx| {
            this.commit_subtask(cx)
        });
        window.focus(&input.focus_handle(cx), cx);
        if let Some(detail) = &mut self.card_detail {
            detail.subtask_input = Some(input);
            detail.subtask_submit = Some(submit);
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
        // Cleared only once saved, so a rejected subtask can be corrected.
        if self.write_task(cx, |service| {
            service.add_subtask(&card.id, &title).map(drop)
        }) {
            input.update(cx, |input, cx| input.set_text(String::new(), cx));
        }
    }

    fn toggle_subtask(&mut self, subtask: Subtask, cx: &mut Context<Self>) {
        self.write_task(cx, |service| {
            service.toggle_subtask(&subtask.id, !subtask.done)
        });
    }

    fn delete_subtask(&mut self, subtask_id: String, cx: &mut Context<Self>) {
        self.write_task(cx, |service| service.delete_subtask(&subtask_id));
    }

    /// Move a subtask one place up (`-1`) or down (`1`). A subtask that is
    /// already at that end, or is gone, is left alone.
    fn shift_subtask(&mut self, subtask_id: &str, by: isize, cx: &mut Context<Self>) {
        let Some(detail) = &self.card_detail else {
            return;
        };
        let Some(index) = detail.subtasks.iter().position(|s| s.id == subtask_id) else {
            return;
        };
        let Some(target) = index
            .checked_add_signed(by)
            .filter(|target| *target < detail.subtasks.len())
        else {
            return;
        };
        let subtask_id = subtask_id.to_string();
        self.write_task(cx, |service| service.move_subtask(&subtask_id, target));
    }

    /// Turn a subtask into a card of its own, right after this one.
    fn convert_subtask(&mut self, subtask_id: String, cx: &mut Context<Self>) {
        self.write_task(cx, |service| service.convert_subtask(&subtask_id).map(drop));
    }

    // ---- description toolbar ----

    /// Apply a Markdown toolbar button to the description's selection. The
    /// text saves on the same blur and close path as any other edit.
    fn format_description(
        &mut self,
        format: super::markdown_toolbar::Format,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = &self.card_detail else {
            return;
        };
        let body = detail.body.clone();
        let (text, selection) = {
            let input = body.read(cx);
            (input.content.to_string(), input.selected_range())
        };
        let (text, selection) = super::markdown_toolbar::apply(format, &text, selection);
        body.update(cx, |input, cx| {
            input.set_text(text, cx);
            input.select_range(selection, cx);
        });
        window.focus(&body.focus_handle(cx), cx);
        cx.notify();
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
        // Cleared only once saved, so a failed comment can be sent again.
        if self.write_task(cx, |service| service.add_comment(&card.id, &body).map(drop)) {
            input.update(cx, |input, cx| input.set_text(String::new(), cx));
        }
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
    fn task_done(&self, card: &gespan_core::board::Card) -> bool {
        self.done_column().as_deref() == Some(card.column_id.as_str())
    }

    fn mark_complete(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.detail_card() else {
            return;
        };
        match self.done_column() {
            Some(done) if done != card.column_id => {
                self.move_with_undo(&card.id, &done, usize::MAX, cx);
                self.refresh_detail_lists();
                cx.notify();
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
            cx.write_to_clipboard(ClipboardItem::new_string(more::card_reference(
                card.number,
                &card.title,
            )));
        }
    }

    /// Close the drawer when it shows `card_id`, saving its edits. False when
    /// that save failed and the drawer stays open with its error.
    fn close_detail_for(&mut self, card_id: &str, cx: &mut Context<Self>) -> bool {
        if self
            .card_detail
            .as_ref()
            .is_some_and(|detail| detail.card_id == card_id)
        {
            self.close_card_detail(cx);
            return self.card_detail.is_none();
        }
        true
    }

    /// A held delete: the card is archived and the drawer closes at once; the
    /// undo toast deletes it for good when it goes, or Undo puts it back.
    pub(super) fn delete_card(&mut self, card_id: &str, cx: &mut Context<Self>) {
        let Some(card) = self.card(card_id).cloned() else {
            return;
        };
        let Some(((_, column_id, index), number)) = self.place_of(&card.id) else {
            return;
        };
        // Typed edits are saved first so Undo brings them back too; a failed
        // save keeps the drawer open with its error and deletes nothing.
        if !self.close_detail_for(&card.id, cx) {
            return;
        }
        self.commit_pending(cx);
        if self.write(cx, |service| service.archive(&card.id)) {
            let undo = super::undo::Undo::Delete {
                card_id: card.id,
                column_id,
                index,
            };
            self.push_undo(format!("Deleted #{number}"), undo, cx);
        }
    }
}
