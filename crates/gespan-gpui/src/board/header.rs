//! The board's header row: title, search, filter and New task, like gr8r's.
use super::{BoardView, menu_row};
use crate::runtime::locked;
use crate::{
    popover::{Anchor, popover},
    theme::*,
};
use gespan_core::board::extras::{Dir, FilterRow, Op, PRIORITIES, SortKey, SortSpec};
use gpui::{
    AnyElement, App, Context, Focusable, FontWeight, MouseButton, Role, SharedString, Toggled, div,
    prelude::*, px,
};

/// The Native setting behind "Open cards in": `drawer` or `page`.
pub(super) const CARD_OPEN_SETTING: &str = "board-card-open";

/// The fields a filter row can be on, as (field, label).
const FILTER_FIELDS: [(&str, &str); 8] = [
    ("status", "Status"),
    ("priority", "Priority"),
    ("labels", "Labels"),
    ("projects", "Projects"),
    ("due", "Due"),
    ("created", "Created"),
    ("updated", "Updated"),
    ("linked", "Linked work"),
];
const DUE_CHOICES: [(&str, &str); 5] = [
    ("overdue", "Overdue"),
    ("today", "Today"),
    ("next7", "Next 7 days"),
    ("later", "Later"),
    ("none", "No due date"),
];
const WITHIN_CHOICES: [(&str, &str); 3] = [
    ("24h", "Last 24 hours"),
    ("7d", "Last 7 days"),
    ("30d", "Last 30 days"),
];
const LINKED_CHOICES: [(&str, &str); 4] = [
    ("running", "Running"),
    ("failed", "Failed"),
    ("needs_review", "Needs review"),
    ("none", "No linked work"),
];
const SORT_KEYS: [SortKey; 7] = [
    SortKey::Manual,
    SortKey::Priority,
    SortKey::Due,
    SortKey::Start,
    SortKey::Title,
    SortKey::Created,
    SortKey::Updated,
];

pub(crate) fn field_label(field: &str) -> String {
    match FILTER_FIELDS.iter().find(|(known, _)| *known == field) {
        Some((_, label)) => label.to_string(),
        None if field.starts_with("field:") => "Custom field".into(),
        None => field.to_string(),
    }
}

pub(crate) fn sort_label(key: SortKey) -> &'static str {
    match key {
        SortKey::Manual => "Manual",
        SortKey::Priority => "Priority",
        SortKey::Due => "Due date",
        SortKey::Start => "Start date",
        SortKey::Title => "Title",
        SortKey::Created => "Created",
        SortKey::Updated => "Updated",
    }
}

pub(crate) fn sort_id(key: SortKey) -> &'static str {
    match key {
        SortKey::Manual => "manual",
        SortKey::Priority => "priority",
        SortKey::Due => "due",
        SortKey::Start => "start",
        SortKey::Title => "title",
        SortKey::Created => "created",
        SortKey::Updated => "updated",
    }
}

/// A filter row that says whether it is on, to assistive tech as well.
pub(super) fn checked_row(
    id: SharedString,
    label: impl Into<SharedString>,
    on: bool,
) -> gpui::Stateful<gpui::Div> {
    menu_row(id, label)
        .role(Role::MenuItemCheckBox)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
}

/// Enter or Space on a focused header button presses it.
pub(super) fn presses(event: &gpui::KeyDownEvent) -> bool {
    matches!(event.keystroke.key.as_str(), "enter" | "space")
}

impl BoardView {
    /// The workspace projects a task can be tagged with, as (path, name).
    pub(crate) fn set_projects(&mut self, projects: Vec<(String, String)>, cx: &mut Context<Self>) {
        if self.projects != projects {
            self.projects = projects;
            cx.notify();
        }
    }

    pub(crate) fn project_name(&self, path: &str) -> String {
        self.projects
            .iter()
            .find(|(project, _)| project == path)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| {
                std::path::Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string())
            })
    }

    /// Projects a task can be tagged or filtered by: the workspace's, then any
    /// a task still carries after leaving the workspace, so it can be untagged,
    /// or the filter still selects, so it can be unchecked.
    pub(crate) fn project_choices(&self) -> Vec<(String, String)> {
        let mut choices = self.projects.clone();
        let mut extra: Vec<(String, String)> = self
            .extras
            .values()
            .flat_map(|extras| extras.projects.iter())
            .chain(
                self.spec
                    .rows
                    .iter()
                    .filter(|row| row.field == "projects")
                    .flat_map(|row| row.values.iter()),
            )
            .filter(|path| !self.projects.iter().any(|(known, _)| known == *path))
            .map(|path| (path.clone(), self.project_name(path)))
            .collect();
        extra.sort();
        extra.dedup();
        choices.extend(extra);
        choices
    }

    /// The search text as the query compares it: trimmed, lower case.
    pub(crate) fn query(&self, cx: &App) -> String {
        self.search.read(cx).content.trim().to_lowercase()
    }

    fn narrowing(&self) -> bool {
        self.active_rows() > 0 || !self.last_query.is_empty()
    }

    /// A column's count: `total/limit` with a WIP limit; while searching or
    /// filtering, `shown/total` (the limit still compares against the total).
    pub(crate) fn count_label(&self, column_id: &str) -> String {
        let Some(detail) = &self.detail else {
            return "0".into();
        };
        let total = detail
            .cards
            .iter()
            .filter(|card| card.column_id == column_id)
            .count();
        if self.narrowing() {
            return format!("{}/{total}", self.column_cards(detail, column_id).len());
        }
        match self.column(column_id).and_then(|column| column.wip_limit) {
            Some(limit) => format!("{total}/{limit}"),
            None => total.to_string(),
        }
    }

    /// Whether the column holds more cards than its WIP limit (the total, not
    /// the cards a filter leaves showing).
    pub(crate) fn over_wip(&self, column: &gespan_core::board::Column) -> bool {
        let Some(limit) = column.wip_limit else {
            return false;
        };
        self.detail.as_ref().is_some_and(|detail| {
            detail
                .cards
                .iter()
                .filter(|card| card.column_id == column.id)
                .count() as i64
                > limit
        })
    }

    /// Open or close the filter. The board takes focus so Escape closes it.
    fn toggle_filter(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.filter_open = !self.filter_open;
        cx.notify();
    }

    fn new_task(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let first = self
            .detail
            .as_ref()
            .and_then(|detail| detail.columns.iter().min_by_key(|c| c.position))
            .map(|column| column.id.clone());
        if let Some(first) = first {
            self.open_composer(first, window, cx);
        }
    }

    pub(super) fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active_rows();
        let search_focus = self.search.focus_handle(cx);
        div()
            .id("board-header")
            .debug_selector(|| "board-header".into())
            .h(px(PANE_HEADER_HEIGHT))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .border_b_1()
            .border_color(content(0.07))
            .child(self.render_switcher_button(cx))
            .child(
                div()
                    .id("board-search")
                    .debug_selector(|| "board-search".into())
                    .w(px(220.))
                    .h(px(26.))
                    .px_2()
                    .rounded_md()
                    .border_1()
                    .border_color(content(0.08))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_text()
                    .child(icon("search", 12.).text_color(content(0.4)))
                    .child(div().flex_1().min_w_0().child(self.search.clone()))
                    .on_click(move |_, window, cx| window.focus(&search_focus, cx)),
            )
            .child(
                div()
                    .id("board-filter")
                    .debug_selector(|| "board-filter".into())
                    .relative()
                    .role(Role::Button)
                    .aria_label("Filter")
                    .h(px(26.))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(content(if active > 0 { 0.9 } else { 0.6 }))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.06)))
                    .when(self.filter_open || active > 0, |d| d.bg(content(0.05)))
                    .track_focus(&self.filter_focus)
                    .tab_index(0)
                    .focus_visible(|s| s.border_1().border_color(accent_color()))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if presses(event) {
                            cx.stop_propagation();
                            this.toggle_filter(window, cx);
                        }
                    }))
                    .child(self.filter_anchor.probe())
                    .child(icon("filter", 12.))
                    .child("Filter")
                    .when(active > 0, |d| {
                        d.child(
                            div()
                                .px(px(5.))
                                .rounded_full()
                                .bg(accent_color().opacity(0.2))
                                .text_color(accent_color())
                                .text_size(px(11.))
                                .child(active.to_string()),
                        )
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_filter(window, cx))),
            )
            .child(
                div()
                    .id("board-sort")
                    .debug_selector(|| "board-sort".into())
                    .relative()
                    .role(Role::Button)
                    .aria_label("Sort")
                    .h(px(26.))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(content(if self.sorted() { 0.9 } else { 0.6 }))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.06)))
                    .when(self.sort_open || self.sorted(), |d| d.bg(content(0.05)))
                    .track_focus(&self.sort_focus)
                    .tab_index(0)
                    .focus_visible(|s| s.border_1().border_color(accent_color()))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if presses(event) {
                            cx.stop_propagation();
                            this.toggle_sort(window, cx);
                        }
                    }))
                    .child(self.sort_anchor.probe())
                    .child("Sort")
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_sort(window, cx))),
            )
            .when(self.sorted(), |d| {
                d.child(
                    div()
                        .id("board-sorted-by")
                        .debug_selector(|| "board-sorted-by".into())
                        .text_size(px(12.))
                        .text_color(content(0.55))
                        .child(format!("Sorted by {}", sort_label(self.spec.sort.key))),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .id("board-open-in")
                    .debug_selector(|| "board-open-in".into())
                    .relative()
                    .role(Role::Button)
                    .aria_label("Open cards in")
                    .size(px(26.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.06)))
                    .when(self.open_in_open, |d| d.bg(content(0.05)))
                    .tab_index(0)
                    .focus_visible(|s| s.border_1().border_color(accent_color()))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                        if presses(event) {
                            cx.stop_propagation();
                            this.open_in_open = !this.open_in_open;
                            cx.notify();
                        }
                    }))
                    .child(self.open_in_anchor.probe())
                    .child(icon("sliders", 13.).text_color(content(0.6)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_in_open = !this.open_in_open;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("board-archive")
                    .debug_selector(|| "board-archive".into())
                    .role(Role::Button)
                    .aria_label("Archive")
                    .aria_toggled(if self.archive_open {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
                    .h(px(26.))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(content(if self.archive_open { 0.9 } else { 0.6 }))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.06)))
                    .when(self.archive_open, |d| d.bg(content(0.05)))
                    .tab_index(0)
                    .focus_visible(|s| s.border_1().border_color(accent_color()))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                        if presses(event) {
                            cx.stop_propagation();
                            this.toggle_archive(cx);
                        }
                    }))
                    .child("Archive")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_archive(cx))),
            )
            .child(
                div()
                    .id("board-new-task")
                    .debug_selector(|| "board-new-task".into())
                    .role(Role::Button)
                    .aria_label("New task")
                    .h(px(26.))
                    .px(px(10.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .bg(accent_color())
                    .text_color(white(0.95))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.9))
                    .track_focus(&self.new_task_focus)
                    .tab_index(0)
                    .focus_visible(|s| s.border_1().border_color(white(0.9)))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if presses(event) {
                            cx.stop_propagation();
                            this.new_task(window, cx);
                        }
                    }))
                    .child(icon("plus", 12.))
                    .child("New task")
                    .on_click(cx.listener(|this, _, window, cx| this.new_task(window, cx))),
            )
    }

    /// "Where" on the first filter row, "and" on the rest.
    pub(crate) fn lead_label(row: usize) -> &'static str {
        if row == 0 { "Where" } else { "and" }
    }

    /// The values a filter row on `field` can pick, as (value, label).
    /// `held` is what the row already holds: the only choices a free-form
    /// custom field (text, number, date) can offer.
    pub(crate) fn field_choices(&self, field: &str, held: &[String]) -> Vec<(String, String)> {
        let fixed = |table: &[(&str, &str)]| {
            table
                .iter()
                .map(|(value, label)| (value.to_string(), label.to_string()))
                .collect()
        };
        match field {
            "status" => self.detail.as_ref().map_or_else(Vec::new, |detail| {
                let mut columns: Vec<_> = detail.columns.iter().collect();
                columns.sort_by_key(|column| column.position);
                columns
                    .into_iter()
                    .map(|column| (column.id.clone(), column.name.clone()))
                    .collect()
            }),
            "priority" => (0..PRIORITIES.len())
                .rev()
                .filter(|&priority| !self.priority.hidden.contains(&(priority as u8)))
                .map(|priority| (priority.to_string(), self.priority_name(priority as u8)))
                .collect(),
            "labels" => self
                .labels
                .iter()
                .map(|label| (label.id.clone(), label.name.clone()))
                .collect(),
            "projects" => self.project_choices(),
            "due" => fixed(&DUE_CHOICES),
            "created" | "updated" => fixed(&WITHIN_CHOICES),
            "linked" => fixed(&LINKED_CHOICES),
            custom => self.custom_choices(custom, held),
        }
    }

    /// A custom field's choices. Values are JSON-encoded as the engine reads
    /// them. A deleted field has none.
    fn custom_choices(&self, field: &str, held: &[String]) -> Vec<(String, String)> {
        let Some(found) = field
            .strip_prefix("field:")
            .and_then(|id| self.fields.iter().find(|f| f.id == id))
        else {
            return Vec::new();
        };
        match found.kind.as_str() {
            "select" => super::quick::field_options(found.options_json.as_deref())
                .into_iter()
                .map(|option| {
                    (
                        serde_json::Value::String(option.clone()).to_string(),
                        option,
                    )
                })
                .collect(),
            "checkbox" => vec![
                ("true".to_string(), "Yes".to_string()),
                ("false".to_string(), "No".to_string()),
            ],
            _ => held
                .iter()
                .map(|raw| {
                    let label = match serde_json::from_str::<serde_json::Value>(raw) {
                        Ok(serde_json::Value::String(text)) => text,
                        _ => raw.clone(),
                    };
                    (raw.clone(), label)
                })
                .collect(),
        }
    }

    /// A filter row's field as the user reads it: its name.
    pub(crate) fn field_title(&self, field: &str) -> String {
        match field.strip_prefix("field:") {
            Some(id) => self
                .fields
                .iter()
                .find(|f| f.id == id)
                .map_or_else(|| "Deleted field".to_string(), |f| f.name.clone()),
            None => field_label(field),
        }
    }

    /// Rows that filter something: those holding at least one value.
    pub(crate) fn active_rows(&self) -> usize {
        self.spec
            .rows
            .iter()
            .filter(|row| !row.values.is_empty())
            .count()
    }

    fn spec_changed(&mut self, cx: &mut Context<Self>) {
        self.requery(cx);
    }

    fn add_filter_row(&mut self, field: &str, cx: &mut Context<Self>) {
        self.spec.rows.push(FilterRow {
            field: field.to_string(),
            op: Op::Is,
            values: Vec::new(),
        });
        self.filter_row = Some(self.spec.rows.len() - 1);
        self.spec_changed(cx);
    }

    fn remove_filter_row(&mut self, row: usize, cx: &mut Context<Self>) {
        if row < self.spec.rows.len() {
            self.spec.rows.remove(row);
            self.filter_row = match self.filter_row {
                Some(open) if open == row => None,
                Some(open) if open > row => Some(open - 1),
                other => other,
            };
            self.spec_changed(cx);
        }
    }

    fn toggle_filter_op(&mut self, row: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.spec.rows.get_mut(row) {
            row.op = match row.op {
                Op::Is => Op::IsNot,
                Op::IsNot => Op::Is,
            };
            self.spec_changed(cx);
        }
    }

    fn toggle_filter_value(&mut self, row: usize, value: &str, cx: &mut Context<Self>) {
        if let Some(row) = self.spec.rows.get_mut(row) {
            match row.values.iter().position(|v| v == value) {
                Some(at) => {
                    row.values.remove(at);
                }
                None => row.values.push(value.to_string()),
            }
            self.spec_changed(cx);
        }
    }

    pub(crate) fn clear_filter_rows(&mut self, cx: &mut Context<Self>) {
        if !self.spec.rows.is_empty() {
            self.spec.rows.clear();
            self.filter_row = None;
            self.spec_changed(cx);
        }
    }

    pub(crate) fn set_sort(&mut self, key: SortKey, dir: Dir, cx: &mut Context<Self>) {
        self.spec.sort = SortSpec { key, dir };
        self.spec_changed(cx);
    }

    fn toggle_sort(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.sort_open = !self.sort_open;
        cx.notify();
    }

    /// What a filter row says, for its chip: "Priority is Urgent, High". Values
    /// that no longer exist on the board are left out.
    pub(crate) fn chip_text(&self, row: &FilterRow) -> String {
        let mut choices = self.field_choices(&row.field, &row.values);
        if row.field == "priority" {
            // Only the picker hides a level: a chip still names it.
            choices = (0..PRIORITIES.len())
                .map(|level| (level.to_string(), self.priority_name(level as u8)))
                .collect();
        }
        let names: Vec<&str> = row
            .values
            .iter()
            .filter_map(|value| {
                choices
                    .iter()
                    .find(|(known, _)| known == value)
                    .map(|(_, label)| label.as_str())
            })
            .collect();
        let op = match row.op {
            Op::Is => "is",
            Op::IsNot => "is not",
        };
        let names = if names.is_empty() {
            "…".to_string()
        } else {
            names.join(", ")
        };
        format!("{} {op} {names}", self.field_title(&row.field))
    }

    /// One chip per filter row under the tabs, with Clear all.
    pub(super) fn render_chips(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.spec.rows.is_empty() {
            return None;
        }
        let mut chips: Vec<AnyElement> = Vec::new();
        for (index, row) in self.spec.rows.iter().enumerate() {
            chips.push(
                div()
                    .id(("board-chip", index))
                    .debug_selector(move || format!("board-chip-{index}"))
                    .h(px(22.))
                    .pl(px(8.))
                    .pr(px(4.))
                    .rounded_full()
                    .bg(accent_color().opacity(0.12))
                    .text_color(content(0.85))
                    .text_size(px(11.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .child(self.chip_text(row))
                    .child(
                        div()
                            .id(("board-chip-remove", index))
                            .debug_selector(move || format!("board-chip-{index}-remove"))
                            .role(Role::Button)
                            .aria_label("Remove filter")
                            .size(px(14.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(content(0.12)))
                            .child(icon("x", 9.).text_color(content(0.6)))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.remove_filter_row(index, cx)
                                }),
                            ),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filter_open = true;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        Some(
            div()
                .id("board-chips")
                .debug_selector(|| "board-chips".into())
                .flex_shrink_0()
                .px_4()
                .py(px(6.))
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .border_b_1()
                .border_color(content(0.07))
                .children(chips)
                .child(
                    div()
                        .id("board-chips-clear")
                        .debug_selector(|| "board-chips-clear".into())
                        .role(Role::Button)
                        .aria_label("Clear all")
                        .px(px(6.))
                        .text_size(px(11.))
                        .text_color(content(0.55))
                        .cursor_pointer()
                        .hover(|s| s.text_color(content(0.9)))
                        .child("Clear all")
                        .on_click(cx.listener(|this, _, _, cx| this.clear_filter_rows(cx))),
                )
                .into_any_element(),
        )
    }

    /// The filter builder: a "Where" row, then "and" rows, each with its
    /// field, "is" / "is not" and the values to pick; then Add filter.
    pub(super) fn render_filter(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.filter_open {
            return None;
        }
        let weak = cx.weak_entity();
        let section = |label: &'static str| {
            div()
                .px(px(8.))
                .pt(px(6.))
                .pb(px(2.))
                .text_size(px(11.))
                .text_color(content(0.45))
                .child(label)
                .into_any_element()
        };
        let check = |on: bool| {
            div()
                .size(px(12.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .when(on, |d| {
                    d.child(icon("check", 12.).text_color(accent_color()))
                })
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, row) in self.spec.rows.iter().enumerate() {
            let open = self.filter_row == Some(index);
            let (op_text, field) = (
                match row.op {
                    Op::Is => "is",
                    Op::IsNot => "is not",
                },
                self.field_title(&row.field),
            );
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .pt(px(8.))
                    .pb(px(2.))
                    .text_size(px(12.))
                    .child(
                        div()
                            .id(("board-filter-row-lead", index))
                            .debug_selector(move || format!("board-filter-row-{index}-lead"))
                            .w(px(40.))
                            .text_color(content(0.45))
                            .child(Self::lead_label(index)),
                    )
                    .child(
                        div()
                            .id(("board-filter-row-toggle", index))
                            .debug_selector(move || format!("board-filter-row-{index}-toggle"))
                            .role(Role::Button)
                            .aria_label(format!("Pick {field} values"))
                            .aria_expanded(open)
                            .px(px(4.))
                            .rounded_md()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(0.85))
                            .cursor_pointer()
                            .hover(|s| s.bg(content(0.07)))
                            .child(field.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.filter_row = (this.filter_row != Some(index)).then_some(index);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id(("board-filter-row-op", index))
                            .debug_selector(move || format!("board-filter-row-{index}-op"))
                            .role(Role::Button)
                            .aria_label("Match or exclude")
                            .px(px(6.))
                            .rounded_md()
                            .text_color(accent_color())
                            .cursor_pointer()
                            .hover(|s| s.bg(content(0.07)))
                            .child(op_text)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.toggle_filter_op(index, cx)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id(("board-filter-row-remove", index))
                            .debug_selector(move || format!("board-filter-row-{index}-remove"))
                            .role(Role::Button)
                            .aria_label("Remove filter")
                            .size(px(18.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(content(0.08)))
                            .child(icon("x", 10.).text_color(content(0.55)))
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.remove_filter_row(index, cx)
                                }),
                            ),
                    )
                    .into_any_element(),
            );
            if !open {
                rows.push(
                    div()
                        .px(px(8.))
                        .pb(px(2.))
                        .pl(px(54.))
                        .text_size(px(11.))
                        .text_color(content(0.45))
                        .truncate()
                        .child(self.chip_text(row))
                        .into_any_element(),
                );
                continue;
            }
            let choices = self.field_choices(&row.field, &row.values);
            if choices.is_empty() {
                rows.push(
                    div()
                        .px(px(8.))
                        .py(px(4.))
                        .text_size(px(12.))
                        .text_color(content(0.4))
                        .child("Nothing to pick")
                        .into_any_element(),
                );
            }
            for (at, (value, label)) in choices.into_iter().enumerate() {
                let on = row.values.contains(&value);
                rows.push(
                    checked_row(
                        format!("board-filter-row-{index}-value-{at}").into(),
                        label,
                        on,
                    )
                    .flex()
                    .gap_2()
                    .child(check(on))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_filter_value(index, &value, cx)
                    }))
                    .into_any_element(),
                );
            }
        }
        if !self.spec.rows.is_empty() {
            rows.push(div().h(px(1.)).my_1().bg(content(0.07)).into_any_element());
        }
        rows.push(section(if self.spec.rows.is_empty() {
            "Where"
        } else {
            "Add filter"
        }));
        let mut fields: Vec<AnyElement> = Vec::new();
        let mut entries: Vec<(String, String, String)> = FILTER_FIELDS
            .iter()
            .map(|(field, label)| {
                (
                    format!("board-filter-add-{field}"),
                    field.to_string(),
                    label.to_string(),
                )
            })
            .collect();
        entries.extend(self.fields.iter().enumerate().map(|(index, field)| {
            (
                format!("board-filter-add-field-{index}"),
                format!("field:{}", field.id),
                field.name.clone(),
            )
        }));
        for (id, field, label) in entries {
            fields.push(
                menu_row(id.into(), label)
                    .text_size(px(12.))
                    .px(px(8.))
                    .py(px(3.))
                    .border_1()
                    .border_color(content(0.1))
                    .on_click(cx.listener(move |this, _, _, cx| this.add_filter_row(&field, cx)))
                    .into_any_element(),
            );
        }
        rows.push(
            div()
                .flex()
                .flex_wrap()
                .gap(px(4.))
                .px(px(4.))
                .pb(px(4.))
                .children(fields)
                .into_any_element(),
        );
        if !self.spec.rows.is_empty() {
            rows.push(div().h(px(1.)).my_1().bg(content(0.07)).into_any_element());
            rows.push(
                menu_row("board-filter-clear".into(), "Clear all")
                    .on_click(cx.listener(|this, _, _, cx| this.clear_filter_rows(cx)))
                    .into_any_element(),
            );
        }
        Some(
            popover(
                "board-filter-menu",
                Anchor::Slot(self.filter_anchor.clone()),
            )
            .width(280.)
            .on_dismiss(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.filter_open = false;
                    cx.notify();
                });
            })
            .child(
                div()
                    .id("board-filter-list")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .p(px(4.))
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .into_any_element(),
        )
    }

    /// The sort menu: a key, then ascending or descending.
    pub(super) fn render_sort(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.sort_open {
            return None;
        }
        let weak = cx.weak_entity();
        let current = self.spec.sort;
        let check = |on: bool| {
            div().size(px(12.)).flex_shrink_0().when(on, |d| {
                d.child(icon("check", 12.).text_color(accent_color()))
            })
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        for key in SORT_KEYS {
            let on = current.key == key;
            let dir = current.dir;
            rows.push(
                checked_row(
                    format!("board-sort-{}", sort_id(key)).into(),
                    sort_label(key),
                    on,
                )
                .flex()
                .gap_2()
                .child(check(on))
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(key, dir, cx)))
                .into_any_element(),
            );
        }
        if current.key != SortKey::Manual {
            rows.push(div().h(px(1.)).my_1().bg(content(0.07)).into_any_element());
            for (id, label, dir) in [
                ("board-sort-dir-asc", "Ascending", Dir::Asc),
                ("board-sort-dir-desc", "Descending", Dir::Desc),
            ] {
                let on = current.dir == dir;
                rows.push(
                    checked_row(id.into(), label, on)
                        .flex()
                        .gap_2()
                        .child(check(on))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.set_sort(current.key, dir, cx)),
                        )
                        .into_any_element(),
                );
            }
        }
        Some(
            popover("board-sort-menu", Anchor::Slot(self.sort_anchor.clone()))
                .width(200.)
                .on_dismiss(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.sort_open = false;
                        cx.notify();
                    });
                })
                .child(div().p(px(4.)).flex().flex_col().children(rows))
                .into_any_element(),
        )
    }

    /// Save where cards open. A failed save keeps the choice for this run and
    /// shows the reason.
    fn set_open_in_page(&mut self, page: bool, cx: &mut Context<Self>) {
        self.open_in_open = false;
        self.open_in_page = page;
        let value = if page { "page" } else { "drawer" };
        if let Err(error) =
            locked(&self.store).and_then(|db| db.save_setting(CARD_OPEN_SETTING, value))
        {
            self.error = Some(error);
        }
        cx.notify();
    }

    pub(super) fn render_open_in(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.open_in_open {
            return None;
        }
        let weak = cx.weak_entity();
        let check = |on: bool| {
            div().size(px(12.)).flex_shrink_0().when(on, |d| {
                d.child(icon("check", 12.).text_color(accent_color()))
            })
        };
        let option = |id: &'static str, label: &'static str, page: bool| {
            let on = self.open_in_page == page;
            checked_row(id.into(), label, on)
                .flex()
                .gap_2()
                .child(check(on))
                .on_click(cx.listener(move |this, _, _, cx| this.set_open_in_page(page, cx)))
        };
        Some(
            popover(
                "board-open-in-menu",
                Anchor::Slot(self.open_in_anchor.clone()),
            )
            .width(200.)
            .on_dismiss(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.open_in_open = false;
                    cx.notify();
                });
            })
            .child(
                div()
                    .p(px(4.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px(px(8.))
                            .pt(px(6.))
                            .pb(px(2.))
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child("Open cards in"),
                    )
                    .child(option("board-open-in-drawer", "Drawer", false))
                    .child(option("board-open-in-page", "Full page", true)),
            )
            .into_any_element(),
        )
    }
}
