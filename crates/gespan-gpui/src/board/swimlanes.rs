//! Swimlanes: the board drawn as rows. A lane groups cards by priority, label,
//! project or a select field; each lane has a cell per column. A drop into a
//! cell moves the card to that column (appended: cells have no order of their
//! own) and sets the lane's value only for priority and select-field lanes;
//! label and project lanes change the column alone.
use super::{BoardView, DraggedCard, column::COLUMN_WIDTH, keyboard::Arrow};
use crate::theme::*;
use gespan_core::board::Card;
use gpui::{AnyElement, Context, FontWeight, Role, SharedString, div, prelude::*, px};

/// One row of the board.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lane {
    pub key: String,
    pub label: String,
}

impl BoardView {
    /// The lane grouping in force, or `None` for the plain board. A swimlane
    /// on a field deleted since (or no longer a select) is ignored.
    pub(crate) fn lane_mode(&self) -> Option<String> {
        let mode = self.display.swimlane.as_str();
        match mode {
            "priority" | "label" | "project" => Some(mode.to_string()),
            _ => {
                let id = mode.strip_prefix("field:")?;
                self.fields
                    .iter()
                    .any(|field| field.id == id && field.kind == "select")
                    .then(|| mode.to_string())
            }
        }
    }

    fn select_field(&self, mode: &str) -> Option<&gespan_core::board::extras::Field> {
        let id = mode.strip_prefix("field:")?;
        self.fields.iter().find(|field| field.id == id)
    }

    /// Every lane the board has, top to bottom. A hidden priority level has a
    /// lane only while a card sits in it.
    pub(crate) fn lanes(&self) -> Vec<Lane> {
        let Some(mode) = self.lane_mode() else {
            return Vec::new();
        };
        let lane = |key: String, label: String| Lane { key, label };
        match mode.as_str() {
            "priority" => (0..self.priority.names.len() as u8)
                .rev()
                .filter(|level| {
                    !self.priority.hidden.contains(level)
                        || self.extras.values().any(|extras| extras.priority == *level)
                })
                .map(|level| lane(format!("p{level}"), self.priority_name(level)))
                .collect(),
            "label" => self
                .labels
                .iter()
                .map(|label| lane(label.id.clone(), label.name.clone()))
                .chain([lane("none".into(), "No label".into())])
                .collect(),
            "project" => self
                .project_choices()
                .into_iter()
                .map(|(path, name)| lane(path, name))
                .chain([lane("none".into(), "No project".into())])
                .collect(),
            _ => {
                let options = self
                    .select_field(&mode)
                    .map(|field| super::quick::field_options(field.options_json.as_deref()))
                    .unwrap_or_default();
                options
                    .into_iter()
                    .map(|option| lane(format!("opt:{option}"), option))
                    .chain([lane("none".into(), "No value".into())])
                    .collect()
            }
        }
    }

    /// The lanes a card sits in. Nothing the board still lists puts it in
    /// "none" (or its own priority lane), so no card is ever lost.
    fn card_lane_keys(&self, card: &Card, lanes: &[Lane]) -> Vec<String> {
        let Some(mode) = self.lane_mode() else {
            return Vec::new();
        };
        let extras = self.extras.get(&card.id);
        let raw: Vec<String> = match mode.as_str() {
            "priority" => vec![format!("p{}", extras.map_or(0, |e| e.priority))],
            "label" => extras.map(|e| e.labels.clone()).unwrap_or_default(),
            "project" => extras.map(|e| e.projects.clone()).unwrap_or_default(),
            _ => {
                let id = mode.strip_prefix("field:").unwrap_or_default();
                self.index
                    .get(&card.id)
                    .and_then(|row| row.fields.get(id))
                    .and_then(|raw| serde_json::from_str::<String>(raw).ok())
                    .map(|option| vec![format!("opt:{option}")])
                    .unwrap_or_default()
            }
        };
        let mut keys: Vec<String> = raw
            .into_iter()
            .filter(|key| lanes.iter().any(|lane| &lane.key == key))
            .collect();
        if keys.is_empty() {
            keys.push("none".into());
        }
        keys
    }

    /// The visible cards in one cell, in the column's order.
    pub(crate) fn lane_cell(&self, lanes: &[Lane], lane: &str, column_id: &str) -> Vec<&Card> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        self.column_cards(detail, column_id)
            .into_iter()
            .filter(|card| {
                self.card_lane_keys(card, lanes)
                    .iter()
                    .any(|key| key == lane)
            })
            .collect()
    }

    fn lane_label(&self, lane: &str) -> String {
        self.lanes()
            .into_iter()
            .find(|candidate| candidate.key == lane)
            .map(|lane| lane.label)
            .unwrap_or_default()
    }

    /// The write that puts a card in `lane`: only priority and select-field
    /// lanes have a value to set; `None` when the card is already there or the
    /// lane keeps no value (label, project).
    fn lane_value_write(&self, card: &Card, lane: &str, lanes: &[Lane]) -> Option<LaneValue> {
        let mode = self.lane_mode()?;
        if self
            .card_lane_keys(card, lanes)
            .iter()
            .any(|key| key == lane)
        {
            return None;
        }
        match mode.as_str() {
            "priority" => lane
                .strip_prefix('p')
                .and_then(|level| level.parse::<u8>().ok())
                .map(LaneValue::Priority),
            "label" | "project" => None,
            _ => {
                let field = self.select_field(&mode)?.id.clone();
                let value = lane
                    .strip_prefix("opt:")
                    .map(|option| serde_json::Value::String(option.to_string()).to_string());
                (lane == "none" || value.is_some()).then_some(LaneValue::Field(field, value))
            }
        }
    }

    /// A drop into a swimlane cell.
    pub(crate) fn drop_card_in_lane(
        &mut self,
        dragged: &DraggedCard,
        lane: &str,
        column_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.dragging = false;
        self.dragged_card = None;
        self.drop_target = None;
        self.lane_drop = None;
        let Some(card) = self.card(&dragged.card_id).cloned() else {
            return self.end_drag(cx);
        };
        let lanes = self.lanes();
        let value = self.lane_value_write(&card, lane, &lanes);
        let changes_column = card.column_id != column_id;
        let lane_name = self.lane_label(lane);
        if value.is_none() {
            if changes_column {
                self.move_with_undo_labeled(&card.id, column_id, usize::MAX, Some(&lane_name), cx);
            } else {
                self.end_drag(cx);
            }
            return;
        }
        // A value changes with the move, so one write does both and the toast
        // cannot undo half of it. The WIP warning is the same as any drop.
        let label = changes_column
            .then(|| self.wip_label(&card.id, column_id))
            .flatten()
            .unwrap_or_else(|| {
                if changes_column {
                    format!(
                        "Moved #{} to {}, {lane_name}",
                        card.number,
                        self.column_name(column_id)
                    )
                } else {
                    format!("Moved #{} to {lane_name}", card.number)
                }
            });
        self.commit_pending(cx);
        let (card_id, target) = (card.id.clone(), column_id.to_string());
        let moved = self.write(cx, move |service| {
            if changes_column {
                service.card_move(&card_id, &target, usize::MAX, "user")?;
            }
            match value {
                Some(LaneValue::Priority(level)) => service.set_priority(&card_id, level),
                Some(LaneValue::Field(field, value)) => {
                    service.set_field_value(&card_id, &field, value.as_deref())
                }
                None => Ok(()),
            }
        });
        if moved {
            self.show_toast(label, None, cx);
        }
    }

    // ---- keyboard ----

    /// The lane a lifted card is in when it is picked up.
    pub(super) fn first_lane_of(&self, card_id: &str) -> Option<String> {
        let card = self.card(card_id)?;
        let lanes = self.lanes();
        self.card_lane_keys(card, &lanes).into_iter().next()
    }

    /// Arrows while a card is lifted: Left and Right pick the column, Up and
    /// Down the lane. No slot inside a cell.
    pub(super) fn move_lane_target(&mut self, direction: Arrow, cx: &mut Context<Self>) {
        let Some(mut drag) = self.kb_drag.clone() else {
            return;
        };
        let (Some(lane), Some(detail)) = (drag.lane.clone(), &self.detail) else {
            return;
        };
        let lanes = self.lanes();
        let columns: Vec<String> = detail.columns.iter().map(|c| c.id.clone()).collect();
        let step = |at: usize, len: usize, forward: bool| {
            if forward {
                (at + 1 < len).then_some(at + 1)
            } else {
                at.checked_sub(1)
            }
        };
        match direction {
            Arrow::Left | Arrow::Right => {
                let at = columns.iter().position(|id| *id == drag.column_id);
                if let Some(next) =
                    at.and_then(|at| step(at, columns.len(), direction == Arrow::Right))
                {
                    drag.column_id = columns[next].clone();
                }
            }
            Arrow::Up | Arrow::Down => {
                let at = lanes.iter().position(|candidate| candidate.key == lane);
                if let Some(next) =
                    at.and_then(|at| step(at, lanes.len(), direction == Arrow::Down))
                {
                    drag.lane = Some(lanes[next].key.clone());
                }
            }
        }
        self.lane_drop = drag.lane.clone().map(|lane| (lane, drag.column_id.clone()));
        self.kb_drag = Some(drag);
        cx.notify();
    }

    /// Enter or Space on a lifted card in a lane board.
    pub(super) fn drop_keyboard_lane_card(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.kb_drag.take() else {
            return;
        };
        self.kb_blur = None;
        self.lane_drop = None;
        let Some(lane) = drag.lane else {
            return;
        };
        let Some(card) = self.card(&drag.card_id) else {
            return;
        };
        let dragged = DraggedCard {
            card_id: card.id.clone(),
            number: card.number,
            title: card.title.clone(),
        };
        self.drop_card_in_lane(&dragged, &lane, &drag.column_id, cx);
    }

    /// Plain arrows on a focused card in a lane board: Left and Right to the
    /// next column's card in the same lane, Up and Down within the cell and
    /// then into the next lane.
    pub(super) fn lane_focus_target(&self, card_id: &str, direction: Arrow) -> Option<String> {
        let detail = self.detail.as_ref()?;
        let card = self.card(card_id)?;
        let lanes = self.lanes();
        let lane_at = lanes
            .iter()
            .position(|lane| self.card_lane_keys(card, &lanes).contains(&lane.key))?;
        let column_at = detail
            .columns
            .iter()
            .position(|column| column.id == card.column_id)?;
        let cell = |lane: usize, column: usize| -> Vec<String> {
            if detail.columns[column].collapsed {
                return Vec::new();
            }
            self.lane_cell(&lanes, &lanes[lane].key, &detail.columns[column].id)
                .into_iter()
                .map(|card| card.id.clone())
                .collect()
        };
        let here = cell(lane_at, column_at);
        let index = here.iter().position(|id| id == card_id).unwrap_or(0);
        match direction {
            Arrow::Up | Arrow::Down => {
                let down = direction == Arrow::Down;
                let near = if down {
                    here.get(index + 1)
                } else {
                    here.get(index.wrapping_sub(1))
                };
                if let Some(id) = near {
                    return Some(id.clone());
                }
                let order: Vec<usize> = if down {
                    (lane_at + 1..lanes.len()).collect()
                } else {
                    (0..lane_at).rev().collect()
                };
                order.into_iter().find_map(|lane| {
                    let cards = cell(lane, column_at);
                    if down { cards.first() } else { cards.last() }.cloned()
                })
            }
            Arrow::Left | Arrow::Right => {
                let order: Vec<usize> = if direction == Arrow::Right {
                    (column_at + 1..detail.columns.len()).collect()
                } else {
                    (0..column_at).rev().collect()
                };
                order.into_iter().find_map(|column| {
                    let cards = cell(lane_at, column);
                    cards.get(index.min(cards.len().saturating_sub(1))).cloned()
                })
            }
        }
    }

    // ---- drawing ----

    pub(super) fn render_swimlanes(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let detail = self.detail.as_ref()?;
        self.lane_mode()?;
        let lanes = self.lanes();
        let target = self.lane_drop.clone();
        let header: Vec<AnyElement> = detail
            .columns
            .iter()
            .enumerate()
            .map(|(ci, column)| {
                let over = self.over_wip(column);
                div()
                    .id(("board-lane-column", ci))
                    .debug_selector(move || format!("board-lane-column-{ci}"))
                    .w(px(if column.collapsed { 36. } else { COLUMN_WIDTH }))
                    .flex_shrink_0()
                    .h(px(28.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(0.8))
                            .child(if column.collapsed {
                                String::new()
                            } else {
                                column.name.clone()
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(if over {
                                diff_foreground(false)
                            } else {
                                content(0.4)
                            })
                            .child(self.count_label(&column.id)),
                    )
                    .into_any_element()
            })
            .collect();
        let mut rows: Vec<AnyElement> = Vec::new();
        for (li, lane) in lanes.iter().enumerate() {
            let count: usize = detail
                .columns
                .iter()
                .map(|column| self.lane_cell(&lanes, &lane.key, &column.id).len())
                .sum();
            let cells: Vec<AnyElement> = detail
                .columns
                .iter()
                .enumerate()
                .map(|(ci, column)| {
                    let cards = self.lane_cell(&lanes, &lane.key, &column.id);
                    let done = column.role.as_deref() == Some("done");
                    let over = target
                        .as_ref()
                        .is_some_and(|(key, id)| *key == lane.key && *id == column.id);
                    let (move_lane, move_column) = (lane.key.clone(), column.id.clone());
                    let (drop_lane, drop_column) = (lane.key.clone(), column.id.clone());
                    let name = format!("{}, {}", column.name, lane.label);
                    let body: Vec<AnyElement> = if column.collapsed {
                        vec![
                            div()
                                .text_size(px(12.))
                                .text_color(content(0.4))
                                .child(cards.len().to_string())
                                .into_any_element(),
                        ]
                    } else {
                        cards
                            .iter()
                            .map(|card| self.render_card(card, done, cx))
                            .collect()
                    };
                    div()
                        .id(("board-lane-cell", li * 1000 + ci))
                        .debug_selector(move || format!("board-lane-cell-{li}-{ci}"))
                        .role(Role::Group)
                        .aria_label(name)
                        .w(px(if column.collapsed { 36. } else { COLUMN_WIDTH }))
                        .flex_shrink_0()
                        .min_h(px(56.))
                        .p(px(6.))
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(content(0.06))
                        .bg(content(0.02))
                        .when(over, |d| {
                            d.bg(accent_color().opacity(0.06))
                                .border_color(accent_color().opacity(0.38))
                        })
                        .on_drag_move::<DraggedCard>(cx.listener(
                            move |this, event: &gpui::DragMoveEvent<DraggedCard>, _, cx| {
                                if event.bounds.contains(&event.event.position) {
                                    let target = Some((move_lane.clone(), move_column.clone()));
                                    if this.lane_drop != target {
                                        this.lane_drop = target;
                                        cx.notify();
                                    }
                                }
                            },
                        ))
                        .on_drop::<DraggedCard>(cx.listener(
                            move |this, card: &DraggedCard, _, cx| {
                                this.drop_card_in_lane(card, &drop_lane, &drop_column, cx)
                            },
                        ))
                        .children(body)
                        .into_any_element()
                })
                .collect();
            rows.push(
                div()
                    .id(("board-lane", li))
                    .debug_selector(move || format!("board-lane-{li}"))
                    .role(Role::Group)
                    .aria_label(lane.label.clone())
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .id(("board-lane-header", li))
                            .debug_selector(move || format!("board-lane-header-{li}"))
                            .px(px(4.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(0.7))
                            .child(lane.label.clone())
                            .child(div().text_color(content(0.4)).child(count.to_string())),
                    )
                    .child(div().flex().items_start().gap_2().children(cells))
                    .into_any_element(),
            );
        }
        let announce = self.kb_drag.as_ref().and_then(|drag| {
            let lane = drag.lane.as_ref()?;
            let number = self.card(&drag.card_id)?.number;
            Some(format!(
                "Moving #{number} to {}, {}",
                self.column_name(&drag.column_id),
                self.lane_label(lane)
            ))
        });
        Some(
            div()
                .id("board-lanes")
                .debug_selector(|| "board-lanes".into())
                .flex_1()
                .min_h_0()
                .overflow_scroll()
                .px_4()
                .py_3()
                .flex()
                .flex_col()
                .gap_3()
                .children(announce.map(|text| {
                    div()
                        .id("board-lane-announce")
                        .debug_selector(|| "board-lane-announce".into())
                        .role(Role::Status)
                        .aria_label(SharedString::from(text.clone()))
                        .text_size(px(12.))
                        .text_color(accent_color())
                        .child(text)
                }))
                .child(div().flex().gap_2().children(header))
                .children(rows)
                .into_any_element(),
        )
    }
}

enum LaneValue {
    Priority(u8),
    /// A select field and its new JSON value (`None` clears it).
    Field(String, Option<String>),
}
