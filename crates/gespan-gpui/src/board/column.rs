//! One board column: role ring, name, count, actions, cards and the composer.
use super::{BoardView, CARD_LIST_OVERDRAW, DraggedCard, DraggedColumn};
use crate::theme::*;
use gespan_core::board::{
    BoardDetail, Column,
    extras::{Density, DisplayOptions},
};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, ListAlignment, ListOffset, ListState, MouseButton,
    Pixels, Role, canvas, div, list, prelude::*, px, rgb,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

pub(in crate::board) const COLUMN_WIDTH: f32 = 280.;
/// The space above each card in a column, and around the composer.
pub(super) const ITEM_GAP: f32 = 6.;
/// How many frames a reveal keeps checking that its item is fully in view.
pub(super) const SETTLE_FRAMES: u8 = 6;
/// The composer's "Add card" row, until it has been measured.
const FOOTER_ESTIMATE: f32 = 40.;

/// A first guess at a card's height until it has been painted, for a board
/// with no cards to guess from.
fn card_estimate(display: &DisplayOptions) -> Pixels {
    match display.density {
        Density::Comfortable => px(76.),
        Density::Compact => px(44.),
    }
}

/// The card face's rows, as `render_card` lays them out: the title's line,
/// the meta row and each chip row are 18px, the label row 20px.
const FACE_LINE: f32 = 18.;
const LABEL_ROW: f32 = 20.;
/// About how many characters of a 13px title fit on one line of a card.
const TITLE_LINE_CHARS: usize = 30;

/// A first guess at one card's row (card and the gap above it), from the
/// rows its face will show under `display`.
fn card_row_estimate(rows: f32, labels: bool, title: &str, display: &DisplayOptions) -> Pixels {
    let (padding, gap) = match display.density {
        Density::Comfortable => (10., 6.),
        Density::Compact => (5., 2.),
    };
    let title_lines = if title.chars().count() > TITLE_LINE_CHARS {
        2.
    } else {
        1.
    };
    // The title and the meta row are always there.
    let lines = title_lines * FACE_LINE + FACE_LINE + rows * FACE_LINE;
    let label_row = if labels { LABEL_ROW + gap } else { 0. };
    let gaps = (1. + rows) * gap;
    px(2. + 2. * padding + lines + label_row + gaps + ITEM_GAP)
}

/// The `item_heights` key of a column's composer row.
fn footer_key(column_id: &str) -> String {
    format!("footer:{column_id}")
}

/// The column a `footer_key` names.
pub(super) fn footer_column(key: &str) -> Option<&str> {
    key.strip_prefix("footer:")
}

/// Records the painted height of a column item, and asks for another frame
/// when it changed so the column's own height catches up. A change also
/// moves `version`, so the column heights summed from these are redone.
fn height_probe(
    heights: Rc<RefCell<HashMap<String, Pixels>>>,
    version: Rc<Cell<u64>>,
    key: String,
) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            let height = bounds.size.height;
            let old = heights.borrow_mut().insert(key, height);
            if old != Some(height) {
                version.set(version.get() + 1);
                window.refresh();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

/// The length of the shared start and end of two id lists, so a reload
/// replaces only what changed between them.
fn common_ends(old: &[String], new: &[String]) -> (usize, usize) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (prefix, suffix)
}

/// The card that takes focus when `focused` leaves a column's shown cards:
/// the next card after it in the old order that is still shown, else the
/// previous one, else the column's first card. `None` when the old ids did
/// not hold it, or nothing is shown.
fn nearest_shown(old: &[String], new: &[String], focused: &str) -> Option<String> {
    let at = old.iter().position(|id| id == focused)?;
    if new.iter().any(|id| id == focused) {
        return None;
    }
    let shown: std::collections::HashSet<&String> = new.iter().collect();
    old[at + 1..]
        .iter()
        .find(|id| shown.contains(id))
        .or_else(|| old[..at].iter().rev().find(|id| shown.contains(id)))
        .or_else(|| new.first())
        .cloned()
}

/// The 14px ring that says what a column is for.
pub(super) fn role_icon(role: Option<&str>) -> AnyElement {
    let (color, fill): (Hsla, f32) = match role {
        Some("todo") => (content(0.45), 0.),
        Some("doing") => (rgb(0xe8c547).into(), 0.5),
        Some("review") => (accent_color(), 0.75),
        Some("done") => (diff_foreground(true), 1.),
        _ => (content(0.45), 0.),
    };
    let ring = div()
        .size(px(13.))
        .flex_shrink_0()
        .rounded_full()
        .border(px(1.5))
        .border_color(color)
        .flex()
        .items_center()
        .justify_center()
        .when(role == Some("todo"), |d| d.border_dashed());
    if fill >= 1. {
        return ring
            .bg(color)
            .child(icon("check", 9.).text_color(pane_fill()))
            .into_any_element();
    }
    ring.when(fill > 0., |d| {
        d.child(
            div()
                .size(px(if fill > 0.6 { 7. } else { 5. }))
                .rounded_full()
                .bg(color),
        )
    })
    .into_any_element()
}

impl BoardView {
    /// One list per column for the column's visible cards. A list whose ids
    /// are unchanged is kept as it is, scroll position included. Otherwise
    /// only the changed run of ids is spliced, and the card that was at the
    /// top of the view stays there while it still exists.
    pub(crate) fn sync_column_lists(&mut self) {
        // Every change to the cards or the query comes through here: the
        // lane list rebuilds on its next render.
        self.lane_inputs_version += 1;
        let display = self.effective_display();
        if self.list_display.as_ref() == Some(&display) {
            self.learn_item_height(&display);
        }
        let mut lists = std::mem::take(&mut self.column_lists);
        let mut old_ids = std::mem::take(&mut self.column_ids);
        if self.detail.is_none() {
            self.card_slots.clear();
            return;
        }
        // The first guess for unpainted cards, from what their faces show.
        self.face_estimate = self
            .mean_face_estimate(&display)
            .map(|height| (display.clone(), height));
        self.heights_version.set(self.heights_version.get() + 1);
        let Some(detail) = &self.detail else {
            return;
        };
        let slots = detail
            .cards
            .iter()
            .enumerate()
            .map(|(index, card)| (card.id.clone(), index))
            .collect();
        let estimate = self.item_estimate(&display);
        let focus = |ids: &[String]| -> Vec<Option<gpui::FocusHandle>> {
            ids.iter().map(|id| self.item_focus(id)).collect()
        };
        // The focused card, while it is still on the board: if the new ids
        // leave it out, the nearest card still shown in its column takes over.
        let focused = self
            .focused_card_id
            .clone()
            .filter(|id| self.card_focus.contains_key(id));
        let mut heir: Option<(String, String)> = None;
        let composer_in = |column_id: &str| {
            self.composer
                .as_ref()
                .filter(|composer| composer.column_id == column_id)
        };
        let mut new_lists = HashMap::new();
        let mut new_ids = HashMap::new();
        for column in &detail.columns {
            let ids: Vec<String> = self
                .column_cards(detail, &column.id)
                .into_iter()
                .map(|card| card.id.clone())
                .collect();
            let state = match (lists.remove(&column.id), old_ids.remove(&column.id)) {
                (Some(state), Some(old)) => {
                    if old != ids {
                        if let Some(focused) = &focused
                            && let Some(next) = nearest_shown(&old, &ids, focused)
                        {
                            heir = Some((focused.clone(), next));
                        }
                        let top = state.logical_scroll_top();
                        // At the very start nothing is above the view to keep
                        // in place: whatever lands first shows.
                        let at_start = top.item_ix == 0 && top.offset_in_item <= px(0.);
                        let top_id = old.get(top.item_ix).filter(|_| !at_start);
                        let (prefix, suffix) = common_ends(&old, &ids);
                        state.splice_focusable(
                            prefix..old.len() - suffix,
                            focus(&ids[prefix..ids.len() - suffix]),
                        );
                        // New items start from the estimate, not zero, so the
                        // scroll range and a jump far down stay close.
                        let _ = state.clone().with_uniform_item_height(estimate);
                        if at_start {
                            // The splice shifts a view that has ever scrolled;
                            // put it back at the start.
                            state.scroll_to(ListOffset {
                                item_ix: 0,
                                offset_in_item: px(0.),
                            });
                        } else if let Some(item_ix) =
                            top_id.and_then(|id| ids.iter().position(|new| new == id))
                        {
                            state.scroll_to(ListOffset {
                                item_ix,
                                offset_in_item: top.offset_in_item,
                            });
                        }
                        // Typing in the composer adds cards above it: keep
                        // the field in view.
                        if composer_in(&column.id).is_some() {
                            state.scroll_to_reveal_item(ids.len());
                        }
                    }
                    state
                }
                _ => {
                    let state = ListState::new(0, ListAlignment::Top, CARD_LIST_OVERDRAW);
                    // The cards, then the composer row.
                    let composer = composer_in(&column.id).map(|composer| composer.focus.clone());
                    state.splice_focusable(0..0, focus(&ids).into_iter().chain([composer]));
                    state.with_uniform_item_height(estimate)
                }
            };
            new_lists.insert(column.id.clone(), state);
            new_ids.insert(column.id.clone(), ids);
        }
        // A card that only moved to another column keeps its focus.
        if let Some((hidden, next)) = heir
            && !new_ids.values().any(|ids| ids.contains(&hidden))
        {
            self.refocus = Some((hidden, next));
        }
        self.card_slots = slots;
        self.column_lists = new_lists;
        self.column_ids = new_ids;
    }

    /// The focus handle a column list keeps a card's row mounted by: the
    /// card's own, or, for the card whose drawer is open, the drawer's, so it
    /// stays mounted while focus is inside the drawer. GPUI's list keeps only
    /// one focused row mounted off screen, so a card that really has focus
    /// always wins over the drawer's card, the drawer's own card included.
    fn item_focus(&self, card_id: &str) -> Option<gpui::FocusHandle> {
        if self.drawer_pinned.as_deref() == Some(card_id)
            && self.focused_card_id.as_deref() != Some(card_id)
        {
            return Some(self.drawer_focus.clone());
        }
        self.card_focus.get(card_id).cloned()
    }

    /// Keep the card whose drawer is open mounted, and let a card whose
    /// drawer closed go like any other. Runs again once focus is known: when
    /// the drawer's own card gains or loses keyboard focus, its row is
    /// re-spliced with the handle that really holds focus.
    pub(crate) fn pin_drawer_card(&mut self) {
        let open = self
            .card_detail
            .as_ref()
            .map(|detail| detail.card_id.clone());
        let focused = open.is_some() && self.focused_card_id == open;
        if open == self.drawer_pinned && focused == self.drawer_card_focused {
            return;
        }
        self.drawer_card_focused = focused;
        let old = std::mem::replace(&mut self.drawer_pinned, open.clone());
        let mut touched: Vec<String> = old.into_iter().collect();
        if let Some(open) = open
            && !touched.contains(&open)
        {
            touched.push(open);
        }
        for card_id in touched {
            let found = self.column_ids.iter().find_map(|(column, ids)| {
                ids.iter()
                    .position(|id| *id == card_id)
                    .map(|ix| (column.clone(), ix))
            });
            if let Some((column, ix)) = found
                && let Some(state) = self.column_lists.get(&column)
            {
                // Re-splicing the top row would reset its offset: keep it.
                let top = state.logical_scroll_top();
                state.splice_focusable(ix..ix + 1, [self.item_focus(&card_id)]);
                state.scroll_to(top);
            }
        }
    }

    /// Before each frame: hand focus on from a focused card a reload or
    /// filter hid (unless the user has put focus somewhere else meanwhile),
    /// then note which card has focus now.
    pub(crate) fn follow_card_focus(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if let Some((hidden, next)) = self.refocus.take() {
            let still_there = self
                .card_focus
                .get(&hidden)
                .is_some_and(|handle| handle.is_focused(window));
            if (still_there || window.focused(cx).is_none())
                && let Some(handle) = self.card_focus.get(&next).cloned()
            {
                self.reveal_card(&next);
                window.focus(&handle, cx);
            }
        }
        let known = self
            .focused_card_id
            .as_ref()
            .and_then(|id| self.card_focus.get(id))
            .is_some_and(|handle| handle.is_focused(window));
        if !known {
            self.focused_card_id = self
                .card_focus
                .iter()
                .find(|(_, handle)| handle.is_focused(window))
                .map(|(id, _)| id.clone());
        }
    }

    /// The column's accessible name: its name and how many cards it shows,
    /// counted from the data, not from the cards its window mounted.
    pub(crate) fn column_label(&self, column_id: &str) -> String {
        let name = self
            .column(column_id)
            .map_or("", |column| column.name.as_str());
        let total = self.column_ids.get(column_id).map_or(0, Vec::len);
        let noun = if total == 1 { "card" } else { "cards" };
        format!("{name}, {total} {noun}")
    }

    /// The status element that reads board-layout keyboard drags aloud. It
    /// takes no room on screen, and is there (empty) before the first drag so
    /// screen readers have registered the live region by then.
    pub(super) fn render_drag_announce(&self) -> AnyElement {
        let text = self.board_announcement().unwrap_or_default();
        div()
            .id("board-drag-announce")
            .debug_selector(|| "board-drag-announce".into())
            .role(Role::Status)
            .aria_label(gpui::SharedString::from(text.clone()))
            .absolute()
            .top_0()
            .left_0()
            .size(px(1.))
            .overflow_hidden()
            .opacity(0.)
            .child(text)
            .into_any_element()
    }

    /// A first guess at a card row's height (card and gap) under `display`:
    /// the mean painted height last seen under it, else the mean of what the
    /// shown cards' faces will lay out, else the fixed estimate.
    pub(super) fn item_estimate(&self, display: &DisplayOptions) -> Pixels {
        if let Some((_, height)) = self
            .learned_heights
            .iter()
            .find(|(seen, _)| seen == display)
        {
            return *height;
        }
        match &self.face_estimate {
            Some((seen, height)) if seen == display => *height,
            _ => card_estimate(display) + px(ITEM_GAP),
        }
    }

    /// The mean of the shown cards' row estimates under `display`, or `None`
    /// when no card shows. One pass over the cards, with map lookups only.
    pub(super) fn mean_face_estimate(&self, display: &DisplayOptions) -> Option<Pixels> {
        let detail = self.detail.as_ref()?;
        let face = &display.face;
        let fields: Vec<&String> = face
            .fields
            .iter()
            .filter(|id| self.fields.iter().any(|field| &field.id == *id))
            .collect();
        let agent: Vec<&String> = face
            .agent_fields
            .iter()
            .filter(|field| field.as_str() != "cost")
            .collect();
        let (mut sum, mut count) = (px(0.), 0usize);
        for card in detail.cards.iter().filter(|card| self.card_matches(card)) {
            let extras = self.extras.get(&card.id);
            let labels = face.labels && extras.is_some_and(|extras| !extras.labels.is_empty());
            let mut rows = 0usize;
            if face.subtasks && extras.is_some_and(|extras| extras.subtasks_total > 0) {
                rows += 1;
            }
            if let Some(row) = self.index.get(&card.id) {
                rows += fields
                    .iter()
                    .filter(|id| row.fields.contains_key(**id))
                    .count();
            }
            if let Some(values) = self.agent.get(&card.id) {
                rows += agent
                    .iter()
                    .filter(|field| values.get(field).is_some())
                    .count();
            }
            if face.link_status
                && self
                    .chips
                    .get(&card.id)
                    .is_some_and(|chips| !chips.is_empty())
            {
                rows += 1;
            }
            if self.card_tickets.contains_key(&card.id) {
                rows += 1;
            }
            sum += card_row_estimate(rows as f32, labels, &card.title, display);
            count += 1;
        }
        (count > 0).then(|| sum / count as f32)
    }

    /// Remember the mean painted card row height under `display`, which the
    /// painted heights in `item_heights` were measured with.
    pub(crate) fn learn_item_height(&mut self, display: &DisplayOptions) {
        let (sum, count) = self
            .item_heights
            .borrow()
            .iter()
            .filter(|(key, _)| !key.starts_with("footer:"))
            .fold((px(0.), 0usize), |(sum, count), (_, height)| {
                (sum + *height, count + 1)
            });
        if count == 0 {
            return;
        }
        let mean = sum / count as f32;
        match self
            .learned_heights
            .iter_mut()
            .find(|(seen, _)| seen == display)
        {
            Some((_, height)) => *height = mean,
            None => self.learned_heights.push((display.clone(), mean)),
        }
    }

    /// After a density or card-face change: drop every cached card height
    /// (a cached height would keep scrolling and reveals on the old sizes),
    /// start unpainted cards from the new estimate, and keep the top card.
    pub(crate) fn remeasure_column_lists(&self) {
        let estimate = self.item_estimate(&self.display);
        for (column_id, state) in &self.column_lists {
            let Some(ids) = self.column_ids.get(column_id) else {
                continue;
            };
            let top = state.logical_scroll_top();
            let composer = self
                .composer
                .as_ref()
                .filter(|composer| &composer.column_id == column_id)
                .map(|composer| composer.focus.clone());
            let handles: Vec<_> = ids
                .iter()
                .map(|id| self.item_focus(id))
                .chain([composer])
                .collect();
            state.splice_focusable(0..state.item_count(), handles);
            let _ = state.clone().with_uniform_item_height(estimate);
            state.scroll_to(ListOffset {
                item_ix: top.item_ix,
                offset_in_item: px(0.),
            });
        }
    }

    /// Scroll a board column so the card is in view. A card that isn't in a
    /// column's visible cards is left alone.
    pub(crate) fn reveal_card(&mut self, card_id: &str) {
        // Swimlanes have one list of lane rows instead of a list a column.
        if self.lane_list.is_some() {
            return self.reveal_lane_card(card_id);
        }
        let found = self.column_ids.iter().find_map(|(column, ids)| {
            ids.iter()
                .position(|id| id == card_id)
                .map(|ix| (column.clone(), ix))
        });
        if let Some((column, ix)) = found {
            self.reveal_item(&column, ix);
        }
    }

    /// Scroll a column so its item `ix` (a card, or the composer row past the
    /// last card) is fully in view. A jump far past the painted cards scrolls
    /// on estimated heights, so the next few frames check again once the
    /// cards around it have been measured (`settle_reveal`).
    pub(crate) fn reveal_item(&mut self, column_id: &str, ix: usize) {
        if !self.scroll_into_view(column_id, ix) {
            self.settling = Some((column_id.to_string(), ix, SETTLE_FRAMES));
        }
    }

    /// Scroll item `ix` into view unless it is already fully shown. True
    /// when it was shown.
    fn scroll_into_view(&self, column_id: &str, ix: usize) -> bool {
        let Some(state) = self.column_lists.get(column_id) else {
            return true;
        };
        let viewport = state.viewport_bounds();
        let shown = state
            .bounds_for_item(ix)
            .is_some_and(|item| item.top() >= viewport.top() && item.bottom() <= viewport.bottom());
        if !shown {
            state.scroll_to_reveal_item(ix.min(state.item_count().saturating_sub(1)));
        }
        shown
    }

    /// Before each frame: finish a reveal whose first scroll used estimated
    /// heights, from where the last frame painted the card. GPUI's list
    /// paints a focused card it has not reached yet just below the visible
    /// ones, and its own idea of an item's place can be stale, so a card
    /// painted wholly outside the view is scrolled to again; one the list
    /// already believes is in view is brought to the top of the view.
    pub(crate) fn settle_reveal(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some((column_id, ix, left)) = self.settling.take() else {
            return;
        };
        let painted = self
            .column_ids
            .get(&column_id)
            .and_then(|ids| ids.get(ix))
            .and_then(|id| self.card_anchors.get(id))
            .and_then(|anchor| anchor.get());
        let shown = match (painted, self.column_lists.get(&column_id)) {
            (Some(card), Some(state)) => {
                let view = state.viewport_bounds();
                let gap = px(ITEM_GAP);
                if card.top() >= view.top() && card.bottom() <= view.bottom() {
                    true
                } else if card.bottom() > view.top() && card.top() < view.bottom() {
                    // Partly in view, painted in place: scroll the rest in.
                    let by = if card.top() < view.top() {
                        card.top() - view.top() - gap
                    } else {
                        card.bottom() - view.bottom() + gap
                    };
                    state.scroll_by(by);
                    false
                } else if self.scroll_into_view(&column_id, ix) {
                    state.scroll_to(ListOffset {
                        item_ix: ix,
                        offset_in_item: px(0.),
                    });
                    false
                } else {
                    false
                }
            }
            // Not painted yet: the list's own idea of its place is all there
            // is, so scroll by it and look again once the card is painted.
            (None, Some(_)) if ix < self.column_ids.get(&column_id).map_or(0, Vec::len) => {
                self.scroll_into_view(&column_id, ix);
                false
            }
            _ => self.scroll_into_view(&column_id, ix),
        };
        if !shown && left > 1 {
            self.settling = Some((column_id, ix, left - 1));
            // Look again on the next frame (a notify during render is lost).
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        }
    }

    /// The column body's natural height: every card's painted height (or a
    /// guess until it paints) and the composer row. A taller column shrinks
    /// to the board and scrolls.
    ///
    /// Summed again only when a painted height, the column's ids or the
    /// estimate changed since the last frame (`heights_version`), not on
    /// every frame.
    pub(super) fn column_body_height(&self, column_id: &str) -> Pixels {
        let estimate = self.item_estimate(&self.display);
        let version = self.heights_version.get();
        if let Some(&(seen, seen_estimate, height)) = self.body_heights.borrow().get(column_id)
            && seen == version
            && seen_estimate == estimate
        {
            return height;
        }
        let heights = self.item_heights.borrow();
        let cards = self.column_ids.get(column_id).map_or(px(0.), |ids| {
            ids.iter().fold(px(0.), |sum, id| {
                sum + heights.get(id).copied().unwrap_or(estimate)
            })
        });
        let height = cards
            + heights
                .get(&footer_key(column_id))
                .copied()
                .unwrap_or(px(FOOTER_ESTIMATE));
        self.body_heights
            .borrow_mut()
            .insert(column_id.to_string(), (version, estimate, height));
        height
    }

    /// Item `ix` of a column's list: a card with the drop line above it when
    /// a drop would land there, or, past the last card, the composer row.
    fn render_column_item(&self, column_id: &str, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(detail) = self.detail.as_ref() else {
            return div().into_any_element();
        };
        let Some((index, column)) = detail
            .columns
            .iter()
            .enumerate()
            .find(|(_, column)| column.id == column_id)
        else {
            return div().into_any_element();
        };
        let ids = self
            .column_ids
            .get(column_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let card_target = self
            .drop_target
            .as_ref()
            .filter(|(target, _)| target == column_id)
            .map(|(_, slot)| *slot);
        let drop_line = || {
            div()
                .flex_shrink_0()
                .h(px(2.))
                .mx(px(2.))
                .rounded(px(2.))
                .bg(accent_color())
        };
        let card = ids
            .get(ix)
            .and_then(|id| self.card_slots.get(id))
            .and_then(|&slot| detail.cards.get(slot));
        if let Some(card) = card {
            let done = column.role.as_deref() == Some("done");
            return div()
                .relative()
                .w_full()
                .px(px(ITEM_GAP))
                .pt(px(ITEM_GAP))
                .flex()
                .flex_col()
                .gap(px(ITEM_GAP))
                .when(card_target == Some(ix), |d| d.child(drop_line()))
                .child(self.render_card(card, done, cx))
                .child(height_probe(
                    self.item_heights.clone(),
                    self.heights_version.clone(),
                    card.id.clone(),
                ))
                .into_any_element();
        }
        let composer = self
            .composer
            .as_ref()
            .filter(|composer| composer.column_id == column.id)
            .map(|composer| composer.input.clone());
        let add_id = column.id.clone();
        div()
            .relative()
            .p(px(ITEM_GAP))
            .flex()
            .flex_col()
            .gap(px(ITEM_GAP))
            .when(card_target.is_some_and(|slot| slot >= ids.len()), |d| {
                d.child(drop_line())
            })
            .child(match composer {
                Some(input) => div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p(px(8.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(content(0.12))
                    .bg(pane_fill())
                    .child(input)
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child("Enter to add · Esc to close"),
                    )
                    .into_any_element(),
                None => div()
                    .id(("board-add-card", index))
                    .debug_selector(move || format!("board-add-card-{index}"))
                    .role(Role::Button)
                    .aria_label("Add card")
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(12.))
                    .text_color(content(0.45))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.05)).text_color(content(0.75)))
                    .child(icon("plus", 12.))
                    .child("Add card")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_composer(add_id.clone(), window, cx)
                    }))
                    .into_any_element(),
            })
            .child(height_probe(
                self.item_heights.clone(),
                self.heights_version.clone(),
                footer_key(column_id),
            ))
            .into_any_element()
    }

    pub(super) fn render_column(
        &self,
        index: usize,
        column: &Column,
        detail: &BoardDetail,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if column.collapsed {
            return self.render_collapsed(index, column, cx);
        }
        let over = self.over_wip(column);
        let anchor = self
            .column_anchors
            .get(&column.id)
            .cloned()
            .unwrap_or_default();
        let group = format!("board-column-group-{index}");
        let header_name = match self.renaming_column(&column.id) {
            Some(input) => div().flex_1().min_w_0().child(input).into_any_element(),
            None => div()
                .min_w_0()
                .truncate()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(content(0.9))
                .child(column.name.clone())
                .into_any_element(),
        };
        let menu_id = column.id.clone();
        let card_target = self
            .drop_target
            .as_ref()
            .filter(|(target, _)| *target == column.id)
            .map(|(_, slot)| *slot);
        let column_slot = self.column_drop;
        let last = index + 1 == detail.columns.len();
        let body_height = self.column_body_height(&column.id);
        let cards = self.column_lists.get(&column.id).cloned().map(|state| {
            let column_id = column.id.clone();
            list(
                state,
                cx.processor(move |this, ix: usize, _, cx| {
                    this.render_column_item(&column_id, ix, cx)
                }),
            )
            .size_full()
        });
        let header_payload = DraggedColumn {
            column_id: column.id.clone(),
            index,
            name: column.name.clone(),
        };
        let weak = cx.weak_entity();
        let (move_over_id, drop_over_id) = (column.id.clone(), column.id.clone());
        div()
            .id(("board-column", index))
            .debug_selector(move || format!("board-column-{index}"))
            .group(group.clone())
            .role(Role::Group)
            .aria_label(self.column_label(&column.id))
            .w(px(COLUMN_WIDTH))
            .relative()
            .flex_shrink_0()
            .max_h_full()
            .flex()
            .flex_col()
            .rounded(px(10.))
            .border_1()
            .border_color(content(0.06))
            .bg(content(0.02))
            .when(card_target.is_some(), |d| {
                d.bg(accent_color().opacity(0.06))
                    .border_color(accent_color().opacity(0.38))
            })
            .when(column_slot == Some(index), |d| {
                d.border_l_2().border_color(accent_color())
            })
            .when(last && column_slot == Some(index + 1), |d| {
                d.border_r_2().border_color(accent_color())
            })
            .on_drag_move::<DraggedColumn>(cx.listener(
                move |this, event: &gpui::DragMoveEvent<DraggedColumn>, _, cx| {
                    this.column_drag_over(index, event.bounds, event.event.position, cx)
                },
            ))
            .on_drop::<DraggedColumn>(
                cx.listener(|this, column: &DraggedColumn, _, cx| this.drop_column(column, cx)),
            )
            // Drawn over the column's top edge, so a coloured column's header
            // lines up with its neighbours'.
            .children(column.color.as_deref().map(|color| {
                div()
                    .id(("board-column-stripe", index))
                    .debug_selector(move || format!("board-column-stripe-{index}"))
                    .absolute()
                    .top(px(2.))
                    .left(px(8.))
                    .right(px(8.))
                    .h(px(3.))
                    .rounded_full()
                    .bg(super::style::label_color(color))
            }))
            .child(
                div()
                    .id(("board-column-header", index))
                    .debug_selector(move || format!("board-column-header-{index}"))
                    .on_drag(header_payload, move |payload, _, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.begin_column_drag();
                            cx.notify();
                        });
                        cx.new(|_| payload.clone())
                    })
                    .h(px(36.))
                    .flex_shrink_0()
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(role_icon(column.role.as_deref()))
                    .child(header_name)
                    .when(over, |d| {
                        d.child(
                            div()
                                .id(("board-column-over", index))
                                .debug_selector(move || format!("board-column-over-{index}"))
                                .aria_label("Over the WIP limit")
                                .size(px(6.))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(diff_foreground(false)),
                        )
                    })
                    .child(
                        div()
                            .id(("board-column-count", index))
                            .debug_selector(move || format!("board-column-count-{index}"))
                            .text_size(px(12.))
                            .text_color(if over {
                                diff_foreground(false)
                            } else {
                                content(0.4)
                            })
                            .child(self.count_label(&column.id)),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .opacity(0.)
                            .group_hover(group.clone(), |s| s.opacity(1.))
                            .child(
                                div()
                                    .id(("board-add-card-top", index))
                                    .role(Role::Button)
                                    .aria_label("Add card")
                                    .size(px(22.))
                                    .rounded_md()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(content(0.08)))
                                    .child(icon("plus", 12.).text_color(content(0.6)))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener({
                                        let id = column.id.clone();
                                        move |this, _, window, cx| {
                                            this.open_composer(id.clone(), window, cx)
                                        }
                                    })),
                            )
                            .child(
                                div()
                                    .id(("board-column-menu", index))
                                    .debug_selector(move || format!("board-column-menu-{index}"))
                                    .relative()
                                    .role(Role::Button)
                                    .aria_label("Column options")
                                    .size(px(22.))
                                    .rounded_md()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(content(0.08)))
                                    .child(anchor.probe())
                                    .child(icon("more-horizontal", 12.).text_color(content(0.6)))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_column_menu(menu_id.clone(), window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .id(("board-column-body", index))
                    .debug_selector(move || format!("board-column-body-{index}"))
                    .on_drag_move::<DraggedCard>(cx.listener(
                        move |this, event: &gpui::DragMoveEvent<DraggedCard>, _, cx| {
                            this.card_drag_over(
                                &move_over_id,
                                event.bounds,
                                event.event.position,
                                cx,
                            )
                        },
                    ))
                    .on_drop::<DraggedCard>(cx.listener(move |this, card: &DraggedCard, _, cx| {
                        this.drop_card(card, &drop_over_id, cx)
                    }))
                    // As tall as its cards, shrinking to the board's height.
                    .h(body_height)
                    .flex_shrink(1.)
                    .min_h_0()
                    .children(cards),
            )
            .into_any_element()
    }

    /// A 36px strip with the name and count. It still takes dropped cards and
    /// columns; a click, Enter or Space expands it.
    fn render_collapsed(
        &self,
        index: usize,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let target = self
            .drop_target
            .as_ref()
            .is_some_and(|(target, _)| *target == column.id);
        let column_slot = self.column_drop;
        let last = self
            .detail
            .as_ref()
            .is_some_and(|detail| index + 1 == detail.columns.len());
        let (move_over_id, drop_over_id) = (column.id.clone(), column.id.clone());
        let (click_id, key_id) = (column.id.clone(), column.id.clone());
        // GPUI has no rotated text, so the name reads top to bottom.
        let name: Vec<AnyElement> = column
            .name
            .chars()
            .take(18)
            .map(|c| div().child(c.to_string()).into_any_element())
            .collect();
        div()
            .id(("board-column-collapsed", index))
            .debug_selector(move || format!("board-column-collapsed-{index}"))
            .role(Role::Button)
            .aria_label(format!("Expand {}", column.name))
            .w(px(36.))
            .min_h(px(120.))
            .flex_shrink_0()
            .py(px(10.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .rounded(px(10.))
            .border_1()
            .border_color(content(0.06))
            .bg(content(0.02))
            .cursor_pointer()
            .hover(|s| s.bg(content(0.05)))
            .when(target, |d| {
                d.bg(accent_color().opacity(0.06))
                    .border_color(accent_color().opacity(0.38))
            })
            .when(column_slot == Some(index), |d| {
                d.border_l_2().border_color(accent_color())
            })
            .when(last && column_slot == Some(index + 1), |d| {
                d.border_r_2().border_color(accent_color())
            })
            .when_some(self.collapsed_focus.get(&column.id), |d, focus| {
                d.track_focus(focus).tab_index(0)
            })
            .focus_visible(|s| s.border_color(accent_color()))
            .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                if !event.keystroke.modifiers.modified()
                    && matches!(event.keystroke.key.as_str(), "enter" | "space")
                {
                    cx.stop_propagation();
                    this.set_collapsed(key_id.clone(), false, cx);
                }
            }))
            .on_click(
                cx.listener(move |this, _, _, cx| this.set_collapsed(click_id.clone(), false, cx)),
            )
            .on_drag_move::<DraggedCard>(cx.listener(
                move |this, event: &gpui::DragMoveEvent<DraggedCard>, _, cx| {
                    this.card_drag_over(&move_over_id, event.bounds, event.event.position, cx)
                },
            ))
            .on_drop::<DraggedCard>(cx.listener(move |this, card: &DraggedCard, _, cx| {
                this.drop_card(card, &drop_over_id, cx)
            }))
            .on_drag_move::<DraggedColumn>(cx.listener(
                move |this, event: &gpui::DragMoveEvent<DraggedColumn>, _, cx| {
                    this.column_drag_over(index, event.bounds, event.event.position, cx)
                },
            ))
            .on_drop::<DraggedColumn>(
                cx.listener(|this, column: &DraggedColumn, _, cx| this.drop_column(column, cx)),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(content(0.4))
                    .child(self.count_label(&column.id)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(0.8))
                    .children(name),
            )
            .into_any_element()
    }
}
