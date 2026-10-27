//! Swimlanes: the board drawn as rows. A lane groups cards by priority, label,
//! project or a select field; each lane has a cell per column. A drop into a
//! cell moves the card to that column (appended: cells have no order of their
//! own) and sets the lane's value only for priority and select-field lanes;
//! label and project lanes change the column alone.
//!
//! The lanes draw as one windowed list of rows (C6-7): each lane's header,
//! then bands of `LANE_BAND` cards per cell, so only the rows in or near
//! view mount. Headers carry one drop target per column; arrow focus,
//! keyboard drags, Shift ranges and announcements count from the data.
use super::{
    BoardView, CARD_LIST_OVERDRAW, DraggedCard,
    column::{COLUMN_WIDTH, SETTLE_FRAMES},
    keyboard::Arrow,
};
use crate::theme::*;
use gespan_core::board::Card;
use gpui::{
    AnyElement, Context, FocusHandle, FontWeight, ListAlignment, ListOffset, ListState, Pixels,
    Role, SharedString, div, list, prelude::*, px,
};
use std::collections::{HashMap, HashSet};

/// Cards per cell in one band: band `b` holds cards `[4b, 4b + 4)`.
pub(crate) const LANE_BAND: usize = 4;
/// A collapsed column's strip width in every row.
const STRIP_WIDTH: f32 = 36.;
/// The space between cells (`gap_2`).
const LANE_GAP: f32 = 8.;

/// Shown card ids per `(lane, column)` cell, in the column's order.
pub(crate) type LaneCells = HashMap<(String, String), Vec<String>>;

/// A row of the windowed swimlanes: a lane's header, or one band of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LaneRow {
    Header { lane: String },
    Band { lane: String, band: usize },
}

impl LaneRow {
    pub(crate) fn lane(&self) -> &str {
        match self {
            LaneRow::Header { lane } | LaneRow::Band { lane, .. } => lane,
        }
    }
}

/// Who takes focus when the focused lane card is no longer shown.
enum LaneHeir {
    Card(String),
    Header(String),
}

/// A cell's shown card ids (none for a cell with no cards).
fn cell<'a>(cells: &'a LaneCells, lane: &str, column: &str) -> &'a [String] {
    cells
        .get(&(lane.to_string(), column.to_string()))
        .map_or(&[], Vec::as_slice)
}

/// The length of the shared start and end of two lists, so a change splices
/// only the run between them.
fn common_ends<T: PartialEq>(old: &[T], new: &[T]) -> (usize, usize) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (prefix, suffix)
}

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

    // ---- the row model ----

    /// Each cell's shown cards (search and filter applied), in the column's
    /// order, keyed by `(lane, column)`. Every column has its cells, a
    /// collapsed one too: counts, announcements and drops use them.
    pub(crate) fn lane_cells(&self, lanes: &[Lane]) -> HashMap<(String, String), Vec<&Card>> {
        let mut cells: HashMap<(String, String), Vec<&Card>> = HashMap::new();
        let Some(detail) = &self.detail else {
            return cells;
        };
        for column in &detail.columns {
            for card in self.column_cards(detail, &column.id) {
                for key in self.card_lane_keys(card, lanes) {
                    cells
                        .entry((key, column.id.clone()))
                        .or_default()
                        .push(card);
                }
            }
        }
        cells
    }

    fn lane_cell_id_map(&self, lanes: &[Lane]) -> LaneCells {
        self.lane_cells(lanes)
            .into_iter()
            .map(|(key, cards)| (key, cards.into_iter().map(|card| card.id.clone()).collect()))
            .collect()
    }

    /// The columns whose cells show cards (a collapsed column shows none).
    fn shown_column_ids(&self) -> Vec<String> {
        self.detail.as_ref().map_or_else(Vec::new, |detail| {
            detail
                .columns
                .iter()
                .filter(|column| !column.collapsed)
                .map(|column| column.id.clone())
                .collect()
        })
    }

    /// Each lane's header, then as many bands as its longest shown cell
    /// needs, and at least one, so an empty lane still takes drops.
    fn rows_for(&self, lanes: &[Lane], cells: &LaneCells) -> Vec<LaneRow> {
        let columns = self.shown_column_ids();
        let mut rows = Vec::new();
        for lane in lanes {
            rows.push(LaneRow::Header {
                lane: lane.key.clone(),
            });
            let longest = columns
                .iter()
                .map(|column| cell(cells, &lane.key, column).len())
                .max()
                .unwrap_or(0);
            for band in 0..longest.div_ceil(LANE_BAND).max(1) {
                rows.push(LaneRow::Band {
                    lane: lane.key.clone(),
                    band,
                });
            }
        }
        rows
    }

    /// The rows of the lanes, as the lane list holds them (the list itself
    /// builds them from the cells it already computed).
    #[cfg(test)]
    pub(crate) fn lane_rows(&self, lanes: &[Lane]) -> Vec<LaneRow> {
        let cells = self.lane_cell_id_map(lanes);
        self.rows_for(lanes, &cells)
    }

    /// The lanes' cards in the order they read on screen: lane by lane,
    /// column by column (collapsed columns left out), then as each cell
    /// orders them. A card in two lanes counts once, at its first place.
    pub(crate) fn lane_order(&self) -> Vec<String> {
        let lanes = self.lanes();
        let cells = self.lane_cell_id_map(&lanes);
        let columns = self.shown_column_ids();
        let mut seen = HashSet::new();
        let mut order = Vec::new();
        for lane in &lanes {
            for column in &columns {
                for id in cell(&cells, &lane.key, column) {
                    if seen.insert(id.clone()) {
                        order.push(id.clone());
                    }
                }
            }
        }
        order
    }

    /// A first guess at a band's height: four card rows.
    fn lane_band_estimate(&self) -> Pixels {
        self.item_estimate(&self.display) * LANE_BAND as f32
    }

    // ---- the windowed list ----

    /// Before each frame: bring the lane list in line with the lanes and
    /// cells. Only the run of rows that changed is spliced, and the row at
    /// the top of the view stays there (else its lane's last row: band-level
    /// anchoring). A focused card a reload or filter hid hands focus on.
    pub(crate) fn sync_lane_list(&mut self, cx: &mut Context<Self>) {
        if self.detail.is_none() || self.lane_mode().is_none() {
            if self.lane_list.take().is_some() {
                self.lane_rows_cache.clear();
                self.lanes_cache.clear();
                self.lane_cell_ids.clear();
                self.lane_row_handles.clear();
                self.lane_settling = None;
                self.lane_header_refocus = None;
                self.lane_reveal_pending = None;
            }
            return;
        }
        let lanes = self.lanes();
        let cells = self.lane_cell_id_map(&lanes);
        let rows = self.rows_for(&lanes, &cells);
        self.lane_header_focus
            .retain(|key, _| lanes.iter().any(|lane| &lane.key == key));
        for lane in &lanes {
            self.lane_header_focus
                .entry(lane.key.clone())
                .or_insert_with(|| cx.focus_handle());
        }
        let estimate = self.lane_band_estimate();
        match self.lane_list.clone() {
            None => {
                let state = ListState::new(0, ListAlignment::Top, CARD_LIST_OVERDRAW);
                state.splice_focusable(0..0, rows.iter().map(|_| None));
                self.lane_list = Some(state.with_uniform_item_height(estimate));
                self.lane_row_handles = vec![None; rows.len()];
            }
            Some(state) => {
                if cells != self.lane_cell_ids || lanes != self.lanes_cache {
                    self.hand_on_lane_focus(&lanes, &cells);
                }
                if rows != self.lane_rows_cache {
                    let old = &self.lane_rows_cache;
                    let top = state.logical_scroll_top();
                    let at_start = top.item_ix == 0 && top.offset_in_item <= px(0.);
                    let top_row = old.get(top.item_ix).cloned().filter(|_| !at_start);
                    let (prefix, suffix) = common_ends(old, &rows);
                    let (old_end, new_end) = (old.len() - suffix, rows.len() - suffix);
                    state.splice_focusable(prefix..old_end, (prefix..new_end).map(|_| None));
                    self.lane_row_handles
                        .splice(prefix..old_end, (prefix..new_end).map(|_| None));
                    let _ = state.clone().with_uniform_item_height(estimate);
                    let goal = if at_start {
                        Some((0, px(0.)))
                    } else {
                        top_row.and_then(|row| {
                            rows.iter()
                                .position(|new| *new == row)
                                .map(|ix| (ix, top.offset_in_item))
                                .or_else(|| {
                                    rows.iter()
                                        .rposition(|new| new.lane() == row.lane())
                                        .map(|ix| (ix, px(0.)))
                                })
                        })
                    };
                    if let Some((item_ix, offset_in_item)) = goal {
                        state.scroll_to(ListOffset {
                            item_ix,
                            offset_in_item,
                        });
                    }
                }
            }
        }
        self.lane_rows_cache = rows;
        self.lanes_cache = lanes;
        self.lane_cell_ids = cells;
        if let Some(card_id) = self.lane_reveal_pending.take() {
            self.reveal_lane_card(&card_id);
        }
    }

    /// The focused card left the cells: pick who takes focus on the next
    /// render, replacing what the column lists picked.
    fn hand_on_lane_focus(&mut self, lanes: &[Lane], cells: &LaneCells) {
        let Some(focused) = self.focused_card_id.clone() else {
            return;
        };
        match self.lane_heir(&focused, lanes, cells) {
            Some(LaneHeir::Card(next)) => {
                self.refocus = Some((focused, next));
                self.lane_header_refocus = None;
            }
            Some(LaneHeir::Header(lane)) => {
                self.refocus = None;
                self.lane_header_refocus = Some((focused, lane));
            }
            None => self.refocus = None,
        }
    }

    /// Where focus goes when `focused` is no longer shown in any lane: the
    /// nearest card still shown in its old cell (next, then previous,
    /// widening), else that cell's first card, else its lane's header.
    /// `None` while the card is still shown (a reload that moved it keeps
    /// its focus), or its lane is gone.
    fn lane_heir(&self, focused: &str, lanes: &[Lane], cells: &LaneCells) -> Option<LaneHeir> {
        let columns = self.shown_column_ids();
        let shown_now = lanes.iter().any(|lane| {
            columns.iter().any(|column| {
                cell(cells, &lane.key, column)
                    .iter()
                    .any(|id| id == focused)
            })
        });
        if shown_now {
            return None;
        }
        let (lane, column, at) = self.lanes_cache.iter().find_map(|lane| {
            columns.iter().find_map(|column| {
                cell(&self.lane_cell_ids, &lane.key, column)
                    .iter()
                    .position(|id| id == focused)
                    .map(|at| (lane.key.clone(), column.clone(), at))
            })
        })?;
        if !lanes.iter().any(|candidate| candidate.key == lane) {
            return None;
        }
        let old = cell(&self.lane_cell_ids, &lane, &column);
        let now = cell(cells, &lane, &column);
        let shown: HashSet<&String> = now.iter().collect();
        for distance in 1..old.len() {
            let next = old.get(at + distance);
            let previous = at.checked_sub(distance).and_then(|ix| old.get(ix));
            if let Some(id) = [next, previous]
                .into_iter()
                .flatten()
                .find(|id| shown.contains(id))
            {
                return Some(LaneHeir::Card(id.clone()));
            }
        }
        Some(match now.first() {
            Some(id) => LaneHeir::Card(id.clone()),
            None => LaneHeir::Header(lane),
        })
    }

    /// Before each frame: move focus to the lane header a reload or filter
    /// picked, unless the user has put focus somewhere else meanwhile.
    pub(crate) fn follow_lane_header_refocus(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some((hidden, lane)) = self.lane_header_refocus.take() else {
            return;
        };
        let still_there = self
            .card_focus
            .get(&hidden)
            .is_some_and(|handle| handle.is_focused(window));
        if !(still_there || window.focused(cx).is_none()) {
            return;
        }
        let Some(handle) = self.lane_header_focus.get(&lane).cloned() else {
            return;
        };
        let header = LaneRow::Header { lane };
        if let Some(ix) = self.lane_rows_cache.iter().position(|row| *row == header) {
            self.lane_scroll_into_view(ix);
        }
        window.focus(&handle, cx);
    }

    /// The band row a card is drawn in: its first appearance, lane by lane
    /// and column by column. `None` when no shown cell holds it.
    fn lane_row_of(&self, card_id: &str) -> Option<usize> {
        let columns = self.shown_column_ids();
        let (lane, band) = self.lanes_cache.iter().find_map(|lane| {
            columns.iter().find_map(|column| {
                cell(&self.lane_cell_ids, &lane.key, column)
                    .iter()
                    .position(|id| id == card_id)
                    .map(|at| (lane.key.clone(), at / LANE_BAND))
            })
        })?;
        let row = LaneRow::Band { lane, band };
        self.lane_rows_cache
            .iter()
            .position(|candidate| *candidate == row)
    }

    /// Before each frame, once focus is known: the focus handle each row
    /// keeps itself mounted by. GPUI's list mounts only one focused row off
    /// screen, and only one of these can hold focus at a time: every lane
    /// header its own, the focused card's band the card's, and the drawer
    /// card's band the drawer's (a focused card in the same band wins).
    pub(crate) fn sync_lane_handles(&mut self) {
        let Some(state) = self.lane_list.clone() else {
            return;
        };
        let focused = self
            .focused_card_id
            .clone()
            .and_then(|id| self.lane_row_of(&id).map(|ix| (ix, id)));
        let drawer = self
            .drawer_pinned
            .as_deref()
            .and_then(|id| self.lane_row_of(id));
        let desired: Vec<Option<FocusHandle>> = self
            .lane_rows_cache
            .iter()
            .enumerate()
            .map(|(ix, row)| match row {
                LaneRow::Header { lane } => self.lane_header_focus.get(lane).cloned(),
                LaneRow::Band { .. } => match &focused {
                    Some((at, id)) if *at == ix => self.card_focus.get(id).cloned(),
                    _ if drawer == Some(ix) => Some(self.drawer_focus.clone()),
                    _ => None,
                },
            })
            .collect();
        if desired == self.lane_row_handles {
            return;
        }
        let top = state.logical_scroll_top();
        for (ix, handle) in desired.iter().enumerate() {
            if self.lane_row_handles.get(ix) != Some(handle) {
                state.splice_focusable(ix..ix + 1, [handle.clone()]);
            }
        }
        let _ = state
            .clone()
            .with_uniform_item_height(self.lane_band_estimate());
        state.scroll_to(top);
        self.lane_row_handles = desired;
    }

    /// After a density or card-face change: every row height is stale.
    pub(crate) fn remeasure_lane_list(&self) {
        let Some(state) = &self.lane_list else {
            return;
        };
        let top = state.logical_scroll_top();
        state.splice_focusable(0..state.item_count(), self.lane_row_handles.clone());
        let _ = state
            .clone()
            .with_uniform_item_height(self.lane_band_estimate());
        state.scroll_to(ListOffset {
            item_ix: top.item_ix,
            offset_in_item: px(0.),
        });
    }

    /// Scroll row `ix` into view unless it is already fully shown. True when
    /// it was shown.
    fn lane_scroll_into_view(&self, ix: usize) -> bool {
        let Some(state) = &self.lane_list else {
            return true;
        };
        let viewport = state.viewport_bounds();
        let shown = state
            .bounds_for_item(ix)
            .is_some_and(|row| row.top() >= viewport.top() && row.bottom() <= viewport.bottom());
        if !shown {
            state.scroll_to_reveal_item(ix.min(state.item_count().saturating_sub(1)));
        }
        shown
    }

    /// Scroll the lanes so a card's band is in view; the next frames check
    /// the card itself is (`settle_lane_reveal`).
    pub(crate) fn reveal_lane_card(&mut self, card_id: &str) {
        if let Some(ix) = self.lane_row_of(card_id) {
            self.lane_scroll_into_view(ix);
            self.lane_settling = Some((card_id.to_string(), SETTLE_FRAMES));
        }
    }

    /// Before each frame: finish a lane reveal from where the last frame
    /// painted the card, as `settle_reveal` does for board columns.
    pub(crate) fn settle_lane_reveal(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some((card_id, left)) = self.lane_settling.take() else {
            return;
        };
        let (Some(state), Some(ix)) = (self.lane_list.clone(), self.lane_row_of(&card_id)) else {
            return;
        };
        let painted = self
            .card_anchors
            .get(&card_id)
            .and_then(|anchor| anchor.get());
        let view = state.viewport_bounds();
        let gap = px(6.);
        let shown = match painted {
            Some(card) if card.top() >= view.top() && card.bottom() <= view.bottom() => true,
            Some(card) if card.bottom() > view.top() && card.top() < view.bottom() => {
                let by = if card.top() < view.top() {
                    card.top() - view.top() - gap
                } else {
                    card.bottom() - view.bottom() + gap
                };
                state.scroll_by(by);
                false
            }
            Some(_) => {
                if self.lane_scroll_into_view(ix) {
                    state.scroll_to(ListOffset {
                        item_ix: ix,
                        offset_in_item: px(0.),
                    });
                }
                false
            }
            None => {
                self.lane_scroll_into_view(ix);
                false
            }
        };
        if !shown && left > 1 {
            self.lane_settling = Some((card_id, left - 1));
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        }
    }

    // ---- keyboard ----

    /// The lane a lifted card is in when it is picked up: its first lane in
    /// the order on screen, where focus, the pin and Shift ranges place it.
    pub(super) fn first_lane_of(&self, card_id: &str) -> Option<String> {
        let card = self.card(card_id)?;
        let lanes = self.lanes();
        let keys = self.card_lane_keys(card, &lanes);
        lanes
            .into_iter()
            .find(|lane| keys.contains(&lane.key))
            .map(|lane| lane.key)
    }

    /// Arrows while a card is lifted: Left and Right pick the column, Up and
    /// Down the lane, one lane a step. No slot inside a cell. The target
    /// lane's header scrolls into view.
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
                    let header = LaneRow::Header {
                        lane: lanes[next].key.clone(),
                    };
                    if let Some(ix) = self.lane_rows_cache.iter().position(|row| *row == header) {
                        self.lane_scroll_into_view(ix);
                    }
                }
            }
        }
        self.lane_drop = drag.lane.clone().map(|lane| (lane, drag.column_id.clone()));
        if let Some(lane) = &drag.lane {
            self.drag_announcement = self.card(&drag.card_id).map(|card| {
                format!(
                    "Card #{}, {} is over {}.",
                    card.number,
                    card.title,
                    self.lane_place(&card.id, lane, &drag.column_id)
                )
            });
        }
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
        // Said before the write, from where the card lands.
        self.drag_announcement = Some(format!(
            "Dropped card #{} in {}.",
            dragged.number,
            self.lane_place(&dragged.card_id, &lane, &drag.column_id)
        ));
        self.drop_card_in_lane(&dragged, &lane, &drag.column_id, cx);
        // The card keeps focus: once the lanes hold it in its new cell,
        // scroll it into view (`sync_lane_list`).
        self.lane_reveal_pending = Some(dragged.card_id);
    }

    /// Plain arrows on a focused card in a lane board: Left and Right to the
    /// same index in the next column with cards in the same lane (clamped),
    /// Up and Down within the cell and then into the nearest lane with cards
    /// in that column. Counts come from the data, not the mounted rows.
    pub(super) fn lane_focus_target(&self, card_id: &str, direction: Arrow) -> Option<String> {
        let detail = self.detail.as_ref()?;
        let card = self.card(card_id)?;
        let lanes = self.lanes();
        let cells = self.lane_cell_id_map(&lanes);
        let keys = self.card_lane_keys(card, &lanes);
        let lane_at = lanes.iter().position(|lane| keys.contains(&lane.key))?;
        let column_at = detail
            .columns
            .iter()
            .position(|column| column.id == card.column_id)?;
        let cell_at = |lane: usize, column: usize| -> &[String] {
            if detail.columns[column].collapsed {
                return &[];
            }
            cell(&cells, &lanes[lane].key, &detail.columns[column].id)
        };
        let here = cell_at(lane_at, column_at);
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
                // A card with several labels sits in several lanes of this
                // column: moving on never lands back on the card itself.
                let order: Vec<usize> = if down {
                    (lane_at + 1..lanes.len()).collect()
                } else {
                    (0..lane_at).rev().collect()
                };
                order.into_iter().find_map(|lane| {
                    let cards: Vec<&String> = cell_at(lane, column_at)
                        .iter()
                        .filter(|id| *id != card_id)
                        .collect();
                    if down { cards.first() } else { cards.last() }.map(|id| (*id).clone())
                })
            }
            Arrow::Left | Arrow::Right => {
                let order: Vec<usize> = if direction == Arrow::Right {
                    (column_at + 1..detail.columns.len()).collect()
                } else {
                    (0..column_at).rev().collect()
                };
                order.into_iter().find_map(|column| {
                    let cards = cell_at(lane_at, column);
                    cards.get(index.min(cards.len().saturating_sub(1))).cloned()
                })
            }
        }
    }

    /// "lane …, column …, position i of n", counted over the whole cell (a
    /// card not in it would land at its end), in Tauri's words.
    fn lane_place(&self, card_id: &str, lane: &str, column_id: &str) -> String {
        let lanes = self.lanes();
        let cells = self.lane_cell_id_map(&lanes);
        let ids = cell(&cells, lane, column_id);
        let (index, total) = match ids.iter().position(|id| id == card_id) {
            Some(at) => (at + 1, ids.len()),
            None => (ids.len() + 1, ids.len() + 1),
        };
        format!(
            "lane {}, column {}, position {index} of {total}",
            self.lane_label(lane),
            self.column_name(column_id)
        )
    }

    pub(super) fn announce_lane_pick_up(&mut self, card_id: &str, lane: &str, column_id: &str) {
        let place = self.lane_place(card_id, lane, column_id);
        // "lane …" opens a sentence here: "Lane …".
        let place = format!("L{}", &place[1..]);
        self.drag_announcement = self
            .card(card_id)
            .map(|card| format!("Picked up card #{}, {}. {place}.", card.number, card.title));
    }

    /// Escape or a lost lift: back in the lane it was picked up from.
    pub(super) fn announce_lane_cancel(&mut self, card_id: &str) {
        let Some(card) = self.card(card_id) else {
            return;
        };
        let (number, column) = (card.number, card.column_id.clone());
        let Some(lane) = self.first_lane_of(card_id) else {
            return;
        };
        self.drag_announcement = Some(format!(
            "Moving card #{number} was cancelled. It is back in {}.",
            self.lane_place(card_id, &lane, &column)
        ));
    }

    /// What the last swimlane keyboard drag step said: picked up, over,
    /// dropped or cancelled, in Tauri's words, with full counts.
    pub(crate) fn lane_announcement(&self) -> Option<String> {
        self.lane_mode()?;
        self.drag_announcement.clone()
    }

    // ---- drawing ----

    fn lane_column_width(column: &gespan_core::board::Column) -> f32 {
        if column.collapsed {
            STRIP_WIDTH
        } else {
            COLUMN_WIDTH
        }
    }

    /// A drop target for a swimlane cell: a drop anywhere in it moves the
    /// card to the cell and appends (cells have no order of their own).
    fn lane_drop_target(
        &self,
        target: gpui::Stateful<gpui::Div>,
        lane: &str,
        column_id: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let (move_lane, move_column) = (lane.to_string(), column_id.to_string());
        let (drop_lane, drop_column) = (lane.to_string(), column_id.to_string());
        let over = self
            .lane_drop
            .as_ref()
            .is_some_and(|(key, id)| key == lane && id == column_id);
        target
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
            .on_drop::<DraggedCard>(cx.listener(move |this, card: &DraggedCard, _, cx| {
                this.drop_card_in_lane(card, &drop_lane, &drop_column, cx)
            }))
    }

    /// Row `ix` of the lane list: a lane's header or one of its bands.
    fn render_lane_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let (Some(detail), Some(row)) = (self.detail.as_ref(), self.lane_rows_cache.get(ix)) else {
            return div().into_any_element();
        };
        let Some(li) = self
            .lanes_cache
            .iter()
            .position(|lane| lane.key == row.lane())
        else {
            return div().into_any_element();
        };
        match row {
            LaneRow::Header { .. } => self.render_lane_header(li, detail, cx),
            LaneRow::Band { band, .. } => {
                let last = !matches!(
                    self.lane_rows_cache.get(ix + 1),
                    Some(LaneRow::Band { lane, .. }) if lane == row.lane()
                );
                self.render_lane_band(li, *band, last, detail, cx)
            }
        }
    }

    /// A lane's header: its name and card count, then one drop target per
    /// column (the top of each cell), which keyboard drags land on.
    fn render_lane_header(
        &self,
        li: usize,
        detail: &gespan_core::board::BoardDetail,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let lane = &self.lanes_cache[li];
        let count: usize = detail
            .columns
            .iter()
            .map(|column| cell(&self.lane_cell_ids, &lane.key, &column.id).len())
            .sum();
        let caps: Vec<AnyElement> = detail
            .columns
            .iter()
            .enumerate()
            .map(|(ci, column)| {
                let cap = div()
                    .id(SharedString::from(format!(
                        "board-lane-head-cell-{li}-{ci}"
                    )))
                    .debug_selector(move || format!("board-lane-head-cell-{li}-{ci}"))
                    .w(px(Self::lane_column_width(column)))
                    .flex_shrink_0()
                    .h(px(14.))
                    .rounded_t(px(10.))
                    .border_t_1()
                    .border_l_1()
                    .border_r_1()
                    .border_color(content(0.06))
                    .bg(content(0.02));
                self.lane_drop_target(cap, &lane.key, &column.id, cx)
                    .into_any_element()
            })
            .collect();
        div()
            .id(("board-lane", li))
            .debug_selector(move || format!("board-lane-{li}"))
            .role(Role::Group)
            .aria_label(lane.label.clone())
            .flex()
            .flex_col()
            .gap(px(4.))
            .when(li > 0, |d| d.pt(px(12.)))
            .child(
                div()
                    .id(("board-lane-header", li))
                    .debug_selector(move || format!("board-lane-header-{li}"))
                    .when_some(self.lane_header_focus.get(&lane.key), |d, focus| {
                        d.track_focus(focus)
                    })
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(4.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(0.7))
                    .focus_visible(|s| s.text_color(accent_color()))
                    .child(lane.label.clone())
                    .child(div().text_color(content(0.4)).child(count.to_string())),
            )
            .child(div().flex().gap_2().children(caps))
            .into_any_element()
    }

    /// Band `band` of a lane: cards `[4b, 4b + 4)` of each of its cells.
    /// Band 0's cells are `board-lane-cell-{lane}-{column}`; later bands'
    /// are `board-lane-band-{lane}-{band}-{column}`.
    fn render_lane_band(
        &self,
        li: usize,
        band: usize,
        last: bool,
        detail: &gespan_core::board::BoardDetail,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let lane = &self.lanes_cache[li];
        let start = band * LANE_BAND;
        let cells: Vec<AnyElement> = detail
            .columns
            .iter()
            .enumerate()
            .map(|(ci, column)| {
                let ids = cell(&self.lane_cell_ids, &lane.key, &column.id);
                let done = column.role.as_deref() == Some("done");
                let body: Vec<AnyElement> = if column.collapsed {
                    (band == 0)
                        .then(|| {
                            div()
                                .text_size(px(12.))
                                .text_color(content(0.4))
                                .child(ids.len().to_string())
                                .into_any_element()
                        })
                        .into_iter()
                        .collect()
                } else {
                    ids.iter()
                        .skip(start)
                        .take(LANE_BAND)
                        .filter_map(|id| self.card_slots.get(id))
                        .filter_map(|&slot| detail.cards.get(slot))
                        .map(|card| self.render_card(card, done, cx))
                        .collect()
                };
                let name = if band == 0 {
                    format!("board-lane-cell-{li}-{ci}")
                } else {
                    format!("board-lane-band-{li}-{band}-{ci}")
                };
                let selector = name.clone();
                let piece = div()
                    .id(SharedString::from(name))
                    .debug_selector(move || selector.clone())
                    .role(Role::Group)
                    .aria_label(format!("{}, {}", column.name, lane.label))
                    .w(px(Self::lane_column_width(column)))
                    .flex_shrink_0()
                    .when(band == 0, |d| d.min_h(px(40.)))
                    .when(!column.collapsed, |d| d.px(px(6.)))
                    .when(column.collapsed, |d| d.items_center())
                    .pt(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .border_l_1()
                    .border_r_1()
                    .border_color(content(0.06))
                    .bg(content(0.02))
                    .when(last, |d| d.pb(px(6.)).border_b_1().rounded_b(px(10.)))
                    .children(body);
                self.lane_drop_target(piece, &lane.key, &column.id, cx)
                    .into_any_element()
            })
            .collect();
        div()
            .id(SharedString::from(format!("board-lane-row-{li}-{band}")))
            .flex()
            .items_stretch()
            .gap_2()
            .children(cells)
            .into_any_element()
    }

    pub(super) fn render_swimlanes(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let detail = self.detail.as_ref()?;
        self.lane_mode()?;
        let state = self.lane_list.clone()?;
        let header: Vec<AnyElement> = detail
            .columns
            .iter()
            .enumerate()
            .map(|(ci, column)| {
                let over = self.over_wip(column);
                div()
                    .id(("board-lane-column", ci))
                    .debug_selector(move || format!("board-lane-column-{ci}"))
                    .w(px(Self::lane_column_width(column)))
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
        let gaps = detail.columns.len().saturating_sub(1) as f32 * LANE_GAP;
        let width: f32 = detail
            .columns
            .iter()
            .map(Self::lane_column_width)
            .sum::<f32>()
            + gaps;
        let rows = list(
            state,
            cx.processor(|this, ix: usize, _, cx| this.render_lane_row(ix, cx)),
        )
        .size_full();
        // The status line reads keyboard drags aloud; it shows while a card
        // is lifted and is there, hidden, the rest of the time.
        let lifted = self
            .kb_drag
            .as_ref()
            .is_some_and(|drag| drag.lane.is_some());
        let text = self.lane_announcement().unwrap_or_default();
        let announce = div()
            .id("board-lane-announce")
            .debug_selector(|| "board-lane-announce".into())
            .role(Role::Status)
            .aria_label(SharedString::from(text.clone()))
            .text_size(px(12.))
            .text_color(accent_color())
            .when(!lifted, |d| {
                d.absolute()
                    .top_0()
                    .left_0()
                    .size(px(1.))
                    .overflow_hidden()
                    .opacity(0.)
            })
            .child(text);
        Some(
            div()
                .id("board-lanes")
                .debug_selector(|| "board-lanes".into())
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_x_scroll()
                .px_4()
                .py_3()
                .flex()
                .flex_col()
                .gap_3()
                .child(announce)
                .child(div().flex().flex_shrink_0().gap_2().children(header))
                .child(div().w(px(width)).flex_1().min_h_0().child(rows))
                .into_any_element(),
        )
    }
}

enum LaneValue {
    Priority(u8),
    /// A select field and its new JSON value (`None` clears it).
    Field(String, Option<String>),
}
