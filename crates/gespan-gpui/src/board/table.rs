//! The Table layout: one row per card, the columns the view chose, resizable
//! (60 px at least). Title, status, priority, due and labels edit in place.
//! A title being typed lives in `TableEdit` on the view, not in the rows, so a
//! refresh from the other app never takes it back; Enter commits it and
//! Escape drops it.
use super::{
    BoardView,
    list::{LayoutSpec, ROW_HEIGHT},
    quick::{Face, field_text, format_estimate},
    selection::MenuKind,
    style::{due_label, label_chip, local_day, priority_glyph},
};
use crate::{
    input::{self, Input},
    popover::{Anchor, MenuAction},
    theme::*,
};
use gespan_core::board::Card;
use gpui::{
    AnyElement, ClickEvent, Context, Entity, Focusable, FontWeight, MouseButton, Role,
    SharedString, Subscription, Window, div, prelude::*, px, uniform_list,
};
use std::rc::Rc;

/// The narrowest a column can be dragged.
pub(crate) const MIN_COLUMN: i64 = 60;

/// The built-in columns as (id, label, default width).
const COLUMNS: [(&str, &str, i64); 10] = [
    ("title", "Title", 320),
    ("status", "Status", 130),
    ("priority", "Priority", 110),
    ("due", "Due", 100),
    ("start", "Start", 100),
    ("labels", "Labels", 190),
    ("projects", "Projects", 150),
    ("estimate", "Estimate", 90),
    ("created", "Created", 100),
    ("updated", "Updated", 100),
];
const DEFAULT_COLUMNS: [&str; 5] = ["title", "status", "priority", "due", "labels"];
const FIELD_WIDTH: i64 = 140;

/// A title being typed in a row.
pub(super) struct TableEdit {
    pub card_id: String,
    pub input: Entity<Input>,
    _submit: Subscription,
}

/// A column drag: the column, where the pointer went down and the width then.
pub(super) type Resize = (String, f32, i64);

/// A column as the table draws it.
struct Column {
    id: String,
    label: String,
    width: i64,
}

impl BoardView {
    /// The columns the view shows, left to right. Title always leads, and a
    /// custom field deleted since the view was saved is dropped.
    fn table_columns(&self, spec: &LayoutSpec) -> Vec<Column> {
        let chosen: Vec<String> = spec
            .columns
            .clone()
            .unwrap_or_else(|| DEFAULT_COLUMNS.iter().map(|id| id.to_string()).collect());
        let mut ids: Vec<String> = vec!["title".into()];
        ids.extend(chosen.into_iter().filter(|id| id != "title"));
        ids.into_iter()
            .filter_map(|id| {
                let (label, default) = match COLUMNS.iter().find(|(known, ..)| *known == id) {
                    Some((_, label, width)) => (label.to_string(), *width),
                    None => {
                        let field = id.strip_prefix("field:")?;
                        let field = self.fields.iter().find(|f| f.id == field)?;
                        (field.name.clone(), FIELD_WIDTH)
                    }
                };
                let width = spec
                    .widths
                    .get(&id)
                    .copied()
                    .unwrap_or(default)
                    .max(MIN_COLUMN);
                Some(Column { id, label, width })
            })
            .collect()
    }

    /// Show or hide a column. Title can't be hidden.
    pub(super) fn toggle_table_column(&mut self, column: &str, cx: &mut Context<Self>) {
        if column == "title" {
            return;
        }
        let spec = self.layout_spec();
        let mut ids: Vec<String> = self
            .table_columns(&spec)
            .into_iter()
            .map(|column| column.id)
            .collect();
        match ids.iter().position(|id| id == column) {
            Some(at) => {
                ids.remove(at);
            }
            None => ids.push(column.to_string()),
        }
        self.edit_layout(cx, |spec| spec.columns = Some(ids));
    }

    /// Set a column's width, never under the minimum.
    pub(crate) fn set_column_width(&mut self, column: &str, width: i64, cx: &mut Context<Self>) {
        let (column, width) = (column.to_string(), width.max(MIN_COLUMN));
        self.edit_layout(cx, |spec| {
            spec.widths.insert(column, width);
        });
    }

    /// Follow a column drag.
    pub(super) fn drag_resize(&mut self, x: f32, cx: &mut Context<Self>) {
        if let Some((column, from, width)) = self.resizing.clone() {
            self.set_column_width(&column, width + (x - from).round() as i64, cx);
        }
    }

    fn column_menu_items(&self, spec: &LayoutSpec) -> Vec<crate::popover::MenuItem> {
        let shown: Vec<String> = self
            .table_columns(spec)
            .into_iter()
            .map(|column| column.id)
            .collect();
        let builtin = COLUMNS
            .iter()
            .skip(1)
            .map(|(id, label, _)| (id.to_string(), label.to_string()));
        let fields = self
            .fields
            .iter()
            .map(|field| (format!("field:{}", field.id), field.name.clone()));
        builtin
            .chain(fields)
            .map(|(id, label)| {
                MenuAction::new(format!("column-{id}"), label)
                    .checked(shown.contains(&id))
                    .into()
            })
            .collect()
    }

    /// Start typing a new title for the card. A title already being typed
    /// elsewhere is committed first.
    pub(crate) fn start_title_edit(
        &mut self,
        card_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .table_edit
            .as_ref()
            .is_some_and(|edit| edit.card_id == card_id)
        {
            return;
        }
        if !self.commit_table_edit(cx) {
            return;
        }
        let Some(card) = self.card(card_id).cloned() else {
            return;
        };
        let input = cx.new(|cx| Input::field("Title", "Title", content(0.4), cx));
        input.update(cx, |input, cx| input.set_text(card.title.clone(), cx));
        let submit = cx.subscribe(&input, |this, _, _: &input::Submitted, cx| {
            this.commit_table_edit(cx);
        });
        window.focus(&input.focus_handle(cx), cx);
        self.table_edit = Some(TableEdit {
            card_id: card.id,
            input,
            _submit: submit,
        });
        cx.notify();
    }

    /// Save the title being typed. An unchanged one just closes; a blank or
    /// refused one stays in its field with the error. True when nothing is
    /// left open.
    pub(crate) fn commit_table_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.table_edit.take() else {
            return true;
        };
        let title = edit.input.read(cx).content.trim().to_string();
        let Some(card) = self.card(&edit.card_id).cloned() else {
            cx.notify();
            return true;
        };
        if title == card.title {
            cx.notify();
            return true;
        }
        if title.is_empty() {
            self.error = Some("A card needs a title".into());
            self.table_edit = Some(edit);
            cx.notify();
            return false;
        }
        let (id, body) = (card.id, card.body);
        if self.write(cx, |service| service.card_update(&id, &title, &body)) {
            return true;
        }
        self.table_edit = Some(edit);
        cx.notify();
        false
    }

    fn table_cell(&self, card: &Card, column: &Column, cx: &mut Context<Self>) -> AnyElement {
        let number = card.number;
        let extras = self.extras.get(&card.id).cloned().unwrap_or_default();
        let index = self.index.get(&card.id);
        let now = gespan_core::time::now_ms();
        let id = column.id.clone();
        let cell = div()
            .id(SharedString::from(format!(
                "board-table-cell-{id}-{number}"
            )))
            .debug_selector({
                let id = id.clone();
                move || format!("board-table-cell-{id}-{number}")
            })
            .w(px(column.width as f32))
            .h_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .overflow_hidden()
            .text_size(px(12.))
            .text_color(content(0.8));
        // A cell that opens a menu in place; its click never opens the card.
        let editable = |cell: gpui::Stateful<gpui::Div>, face: Face| {
            let card_id = card.id.clone();
            cell.cursor_pointer()
                .hover(|s| s.bg(content(0.06)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    let modifiers = event.modifiers();
                    if modifiers.secondary() || modifiers.shift {
                        // Selecting works from any cell of the row.
                        return this.card_click(&card_id, event, window, cx);
                    }
                    this.open_face_menu_at(
                        card_id.clone(),
                        face,
                        Some(Anchor::Point(event.position())),
                        window,
                        cx,
                    )
                }))
        };
        let text = |text: String| div().min_w_0().truncate().child(text);
        let day = |at: Option<i64>| {
            at.and_then(local_day)
                .map(|day| day.format("%b %-d, %Y").to_string())
                .unwrap_or_default()
        };
        match id.as_str() {
            "title" => {
                let editing = self
                    .table_edit
                    .as_ref()
                    .filter(|edit| edit.card_id == card.id)
                    .map(|edit| edit.input.clone());
                match editing {
                    Some(input) => cell
                        .debug_selector(move || format!("board-table-title-input-{number}"))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(div().flex_1().min_w_0().child(input))
                        .into_any_element(),
                    None => {
                        let card_id = card.id.clone();
                        cell.gap(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.))
                                    .text_color(content(0.92))
                                    .child(card.title.clone()),
                            )
                            .child(
                                div()
                                    .id(("board-table-edit", number as u64))
                                    .debug_selector(move || format!("board-table-edit-{number}"))
                                    .role(Role::Button)
                                    .aria_label("Edit title")
                                    .px(px(6.))
                                    .rounded(px(4.))
                                    .text_size(px(11.))
                                    .text_color(content(0.45))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(content(0.08)))
                                    .child("Edit")
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.start_title_edit(&card_id, window, cx)
                                    })),
                            )
                            .into_any_element()
                    }
                }
            }
            "status" => editable(cell, Face::Status)
                .child(text(self.column_name(&card.column_id)))
                .into_any_element(),
            "priority" => editable(cell, Face::Priority)
                .gap(px(6.))
                .child(priority_glyph(extras.priority))
                .child(text(
                    gespan_core::board::extras::PRIORITIES[extras.priority as usize].to_string(),
                ))
                .into_any_element(),
            "due" => editable(cell, Face::Due)
                .child(text(
                    extras
                        .due_at
                        .and_then(|at| due_label(at, now as i64))
                        .map(|(text, _)| text)
                        .unwrap_or_default(),
                ))
                .into_any_element(),
            "labels" => editable(cell, Face::Labels)
                .gap(px(4.))
                .children(extras.labels.iter().filter_map(|id| {
                    let label = self.labels.iter().find(|label| &label.id == id)?;
                    Some(label_chip(&label.name, &label.color))
                }))
                .into_any_element(),
            "start" => cell
                .child(text(day(index.and_then(|row| row.start_at))))
                .into_any_element(),
            "projects" => cell
                .child(text(
                    extras
                        .projects
                        .iter()
                        .map(|path| self.project_name(path))
                        .collect::<Vec<_>>()
                        .join(", "),
                ))
                .into_any_element(),
            "estimate" => cell
                .child(text(
                    index
                        .and_then(|row| row.estimate_minutes)
                        .map(format_estimate)
                        .unwrap_or_default(),
                ))
                .into_any_element(),
            "created" => cell
                .child(text(gespan_core::time::relative(
                    card.created_at.max(0) as u64,
                    now,
                )))
                .into_any_element(),
            "updated" => cell
                .child(text(gespan_core::time::relative(
                    card.updated_at.max(0) as u64,
                    now,
                )))
                .into_any_element(),
            custom => {
                let value = custom.strip_prefix("field:").and_then(|field_id| {
                    let field = self.fields.iter().find(|f| f.id == field_id)?;
                    let raw = index?.fields.get(field_id)?;
                    field_text(&field.kind, raw)
                });
                cell.child(text(value.unwrap_or_default()))
                    .into_any_element()
            }
        }
    }

    fn render_table_row(
        &self,
        at: usize,
        columns: &[Column],
        total: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(card) = self.detail.as_ref().and_then(|detail| detail.cards.get(at)) else {
            return div().into_any_element();
        };
        let cells: Vec<AnyElement> = columns
            .iter()
            .map(|column| self.table_cell(card, column, cx))
            .collect();
        self.row_shell(card, cx)
            .w(px(total))
            .px_0()
            .gap_0()
            .children(cells)
            .into_any_element()
    }

    pub(super) fn render_table(&self, cx: &mut Context<Self>) -> AnyElement {
        let spec = self.layout_spec();
        let columns = Rc::new(self.table_columns(&spec));
        let total: f32 = columns.iter().map(|column| column.width as f32).sum();
        let order = Rc::new(self.visible_order());
        let items = self.column_menu_items(&spec);
        let entity = cx.entity();
        let header = columns.iter().map(|column| {
            let id = column.id.clone();
            let selector = id.clone();
            let handle_id = id.clone();
            let handle_selector = id.clone();
            let start = column.width;
            div()
                .id(SharedString::from(format!("board-table-col-{id}")))
                .debug_selector(move || format!("board-table-col-{selector}"))
                .relative()
                .w(px(column.width as f32))
                .h_full()
                .flex_shrink_0()
                .px(px(8.))
                .flex()
                .items_center()
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(content(0.55))
                .child(div().min_w_0().truncate().child(column.label.clone()))
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "board-table-resize-{handle_id}"
                        )))
                        .debug_selector(move || format!("board-table-resize-{handle_selector}"))
                        .absolute()
                        .top_0()
                        .right_0()
                        .h_full()
                        .w(px(6.))
                        .cursor_col_resize()
                        .hover(|s| s.bg(content(0.15)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.resizing =
                                    Some((handle_id.clone(), f32::from(event.position.x), start));
                            }),
                        ),
                )
        });
        let count = order.len();
        div()
            .id("board-table")
            .debug_selector(|| "board-table".into())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(32.))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(content(0.07))
                    .child(
                        div()
                            .id("board-table-columns")
                            .debug_selector(|| "board-table-columns".into())
                            .relative()
                            .role(Role::Button)
                            .aria_label("Columns")
                            .h(px(22.))
                            .px(px(8.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .text_size(px(11.))
                            .text_color(content(0.7))
                            .cursor_pointer()
                            .hover(|s| s.bg(content(0.08)))
                            .child(self.pop_anchors.columns.probe())
                            .child("Columns")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_pop_menu(
                                    MenuKind::TableColumns,
                                    items.clone(),
                                    "Columns",
                                    window,
                                    cx,
                                )
                            })),
                    ),
            )
            .child(
                div()
                    .id("board-table-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .child(
                        div()
                            .w(px(total + 32.))
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .id("board-table-header")
                                    .debug_selector(|| "board-table-header".into())
                                    .h(px(ROW_HEIGHT))
                                    .w(px(total))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .border_b_1()
                                    .border_color(content(0.07))
                                    .children(header),
                            )
                            .child(
                                uniform_list("board-table-rows", count, move |range, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        range
                                            .filter_map(|at| order.get(at).copied())
                                            .map(|card| {
                                                this.render_table_row(card, &columns, total, cx)
                                            })
                                            .collect::<Vec<_>>()
                                    })
                                })
                                .flex_1()
                                .min_h_0(),
                            ),
                    ),
            )
            .into_any_element()
    }
}
