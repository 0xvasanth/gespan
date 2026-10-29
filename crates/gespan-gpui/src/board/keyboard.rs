//! Keyboard drag. On a focused card, arrows move focus; Space lifts it. While
//! lifted, arrows move the drop line, Space or Enter drops and Escape (handled
//! by the board's `on_key`) puts it back. Only cards the search and filter show
//! count; the drop maps the visible slot to the whole column like a mouse drop.
use super::{BoardView, KeyboardDrag};
use gpui::{App, Context, IntoElement, KeyBinding, KeyDownEvent, Styled, Window, actions};

actions!(
    board,
    [NewCardKey, FocusSearch, OpenFocusedCard, DeleteSelected]
);

/// Plain-key shortcuts. `Composer` is the key context of every text field
/// (search, composer, inline edits, drawer fields), so with one focused these
/// keys are not bound at all and type normally.
pub(crate) fn bind(cx: &mut App) {
    let context = Some("Board && !Composer");
    cx.bind_keys([
        KeyBinding::new("n", NewCardKey, context),
        KeyBinding::new("/", FocusSearch, context),
        KeyBinding::new("e", OpenFocusedCard, context),
        KeyBinding::new("delete", DeleteSelected, context),
        KeyBinding::new("backspace", DeleteSelected, context),
    ]);
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Arrow {
    Up,
    Down,
    Left,
    Right,
}

fn arrow(key: &str) -> Option<Arrow> {
    match key {
        "up" => Some(Arrow::Up),
        "down" => Some(Arrow::Down),
        "left" => Some(Arrow::Left),
        "right" => Some(Arrow::Right),
        _ => None,
    }
}

impl BoardView {
    /// The ids of the visible cards in a column, top to bottom.
    fn visible_ids(&self, column_id: &str) -> Vec<String> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        // A collapsed column shows no cards: focus skips it, and a lifted card
        // dropped there lands at its end.
        if self.is_collapsed(column_id) {
            return Vec::new();
        }
        let cards = self.column_cards(detail, column_id);
        cards.into_iter().map(|card| card.id.clone()).collect()
    }

    fn column_ids(&self) -> Vec<String> {
        self.detail.as_ref().map_or_else(Vec::new, |detail| {
            detail.columns.iter().map(|c| c.id.clone()).collect()
        })
    }

    fn column_of_card(&self, card_id: &str) -> Option<String> {
        let detail = self.detail.as_ref()?;
        let card = detail.cards.iter().find(|card| card.id == card_id)?;
        Some(card.column_id.clone())
    }

    /// True while something sits in front of the board's cards: the shortcut
    /// keys leave it alone.
    pub(super) fn shortcuts_blocked(&self) -> bool {
        self.kb_drag.is_some()
            || self.settings.is_some()
            || self.new_card.is_some()
            || self.run_picker.is_some()
            || self.run_confirm.is_some()
            || self.composer.is_some()
            || self.filter_open
            || self.sort_open
            || self.open_in_open
            || self.switcher.is_some()
            || self.renaming.is_some()
            || self.pop.is_some()
            || self.table_edit.is_some()
            || self.menu.is_some()
            || self.card_menu.is_some()
            || self.views.menu.is_some()
            || self.views.naming.is_some()
    }

    /// True while the card drawer is open and focus is inside it: N, E and
    /// Delete then belong to the drawer, not to the board behind it. Focus on
    /// the board itself or one of its cards keeps the keys.
    pub(super) fn drawer_holds_keys(&self, window: &Window, cx: &App) -> bool {
        self.card_detail.is_some() && self.drawer_focus.contains_focused(window, cx)
    }

    /// The card that has keyboard focus, if any.
    fn focused_card(&self, window: &Window, cx: &App) -> Option<String> {
        self.card_focus
            .iter()
            .find(|(_, handle)| handle.contains_focused(window, cx))
            .map(|(id, _)| id.clone())
    }

    /// `E`: open the focused card's detail. False when no card has focus.
    pub(super) fn open_focused_card(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.shortcuts_blocked() || self.drawer_holds_keys(window, cx) {
            return false;
        }
        let Some(id) = self.focused_card(window, cx) else {
            return false;
        };
        self.open_card_detail(id, window, cx);
        true
    }

    /// `Delete`: a held delete of the selection, else of the focused card.
    /// False when there is neither.
    pub(super) fn delete_focused_or_selected(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.shortcuts_blocked() || self.drawer_holds_keys(window, cx) {
            return false;
        }
        if self.selected_count() > 0 {
            self.bulk_delete(cx);
        } else if let Some(id) = self.focused_card(window, cx) {
            self.delete_card(&id, cx);
        } else {
            return false;
        }
        // The card that had focus is gone: keep the board's keys alive.
        window.focus(&self.focus, cx);
        true
    }

    /// An arrow pressed while the board itself has focus (after a click on
    /// empty space) focuses the first card on screen. True when it did.
    pub(super) fn arrow_into_cards(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.keystroke.modifiers.modified()
            || arrow(event.keystroke.key.as_str()).is_none()
            || !self.focus.is_focused(window)
            || self.layout_kind() != super::list::Layout::Board
        {
            return false;
        }
        let Some(detail) = &self.detail else {
            return false;
        };
        let columns: Vec<String> = detail.columns.iter().map(|c| c.id.clone()).collect();
        let Some(first) = columns
            .iter()
            .find_map(|column| self.visible_ids(column).into_iter().next())
        else {
            return false;
        };
        self.reveal_card(&first);
        let Some(handle) = self.card_focus.get(&first).cloned() else {
            return false;
        };
        window.focus(&handle, cx);
        cx.notify();
        true
    }

    /// Handles a key on a card. True when the key was used.
    pub(super) fn card_key(
        &mut self,
        card_id: &str,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let plain = !event.keystroke.modifiers.modified();
        if !plain {
            return false;
        }
        if self.kb_drag.is_some() {
            return match key {
                "space" | "enter" => {
                    self.drop_keyboard_card(cx);
                    true
                }
                _ => match arrow(key) {
                    Some(direction) => {
                        self.move_drop_target(direction, cx);
                        true
                    }
                    // Anything else is swallowed so Enter and typing never act on the card.
                    None => key != "escape",
                },
            };
        }
        match key {
            "enter" => {
                self.open_card_actions(card_id.to_string(), window, cx);
                true
            }
            "space" => self.lift_card(card_id, window, cx),
            _ => match arrow(key) {
                Some(direction) => {
                    self.move_card_focus(card_id, direction, window, cx);
                    true
                }
                None => false,
            },
        }
    }

    fn move_card_focus(
        &mut self,
        card_id: &str,
        direction: Arrow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(column_id) = self.column_of_card(card_id) else {
            return;
        };
        if self.lane_mode().is_some() {
            // A card outside the lanes' window scrolls into view first.
            if let Some(target) = self.lane_focus_target(card_id, direction) {
                self.reveal_card(&target);
                if let Some(handle) = self.card_focus.get(&target).cloned() {
                    window.focus(&handle, cx);
                }
            }
            return;
        }
        let here = self.visible_ids(&column_id);
        let index = here.iter().position(|id| id == card_id).unwrap_or(0);
        let target = match direction {
            Arrow::Up => here.get(index.wrapping_sub(1)).cloned(),
            Arrow::Down => here.get(index + 1).cloned(),
            Arrow::Left | Arrow::Right => {
                let columns = self.column_ids();
                let at = columns.iter().position(|id| *id == column_id).unwrap_or(0);
                let neighbours: Vec<&String> = if direction == Arrow::Left {
                    columns[..at].iter().rev().collect()
                } else {
                    columns[at + 1..].iter().collect()
                };
                // Columns with nothing visible have nothing to focus: skip them.
                neighbours.into_iter().find_map(|column| {
                    let cards = self.visible_ids(column);
                    cards.get(index.min(cards.len().saturating_sub(1))).cloned()
                })
            }
        };
        let Some(target) = target else {
            return;
        };
        // A card outside its column's window scrolls into view first.
        self.reveal_card(&target);
        if let Some(handle) = self.card_focus.get(&target) {
            window.focus(handle, cx);
        }
    }

    fn lift_card(&mut self, card_id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(column_id) = self.column_of_card(card_id) else {
            return false;
        };
        let slot = self
            .visible_ids(&column_id)
            .iter()
            .position(|id| id == card_id)
            .unwrap_or(0);
        self.menu = None;
        // Focus moving off the card (a click elsewhere) abandons the lift.
        self.kb_blur = self.card_focus.get(card_id).cloned().map(|handle| {
            cx.on_blur(&handle, window, |this, _, cx| {
                this.cancel_keyboard_drag(cx);
                cx.notify();
            })
        });
        let lane = self.lane_mode().and_then(|_| self.first_lane_of(card_id));
        match &lane {
            Some(lane) => self.announce_lane_pick_up(card_id, lane, &column_id),
            None => self.announce_pick_up(card_id),
        }
        self.lane_drop = lane.clone().map(|lane| (lane, column_id.clone()));
        self.drop_target = Some((column_id.clone(), slot));
        self.kb_drag = Some(KeyboardDrag {
            card_id: card_id.to_string(),
            column_id,
            slot,
            lane,
        });
        cx.notify();
        true
    }

    fn move_drop_target(&mut self, direction: Arrow, cx: &mut Context<Self>) {
        let Some(mut drag) = self.kb_drag.clone() else {
            return;
        };
        if drag.lane.is_some() {
            return self.move_lane_target(direction, cx);
        }
        let origin = self.column_of_card(&drag.card_id);
        let old_slot = |this: &Self, column: &str| {
            (origin.as_deref() == Some(column))
                .then(|| {
                    this.visible_ids(column)
                        .iter()
                        .position(|id| *id == drag.card_id)
                })
                .flatten()
        };
        // The slot after the lifted card is the slot before it: one stop.
        let settle = |slot: usize, old: Option<usize>, going_down: bool| match old {
            Some(old) if slot == old + 1 => {
                if going_down {
                    slot + 1
                } else {
                    old
                }
            }
            _ => slot,
        };
        match direction {
            Arrow::Up | Arrow::Down => {
                let len = self.visible_ids(&drag.column_id).len();
                let old = old_slot(self, &drag.column_id);
                let down = direction == Arrow::Down;
                let next = if down {
                    (drag.slot + 1).min(len)
                } else {
                    drag.slot.saturating_sub(1)
                };
                let next = settle(next, old, down);
                // Past the end there is nowhere further to go.
                drag.slot = if next > len { drag.slot } else { next };
            }
            Arrow::Left | Arrow::Right => {
                let columns = self.column_ids();
                let at = columns.iter().position(|id| *id == drag.column_id);
                let next = match (at, direction) {
                    (Some(at), Arrow::Left) => at.checked_sub(1),
                    (Some(at), _) => (at + 1 < columns.len()).then_some(at + 1),
                    _ => None,
                };
                if let Some(next) = next {
                    let column = columns[next].clone();
                    let len = self.visible_ids(&column).len();
                    let old = old_slot(self, &column);
                    drag.slot = settle(drag.slot.min(len), old, false);
                    drag.column_id = column;
                }
            }
        }
        self.drop_target = Some((drag.column_id.clone(), drag.slot));
        // The drop line shows above the card at the slot (or above the
        // composer row past the last card): keep it in view.
        // A collapsed column has no window to scroll; its own scroll stays.
        if !self.is_collapsed(&drag.column_id) {
            self.reveal_item(&drag.column_id, drag.slot);
        }
        self.announce_over(&drag);
        self.kb_drag = Some(drag);
        cx.notify();
    }

    /// Watches every mouse press in the window while a card is lifted, in
    /// the capture phase, so a press on a popover or the drawer (which take
    /// the press themselves) still ends the lift.
    pub(super) fn lift_press_watch(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let lifted = self.kb_drag.is_some();
        let weak = cx.weak_entity();
        gpui::canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                if !lifted {
                    return;
                }
                let weak = weak.clone();
                window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Capture {
                        let _ =
                            weak.update(cx, |this, cx| this.press_during_lift(event.position, cx));
                    }
                });
            },
        )
        .absolute()
        .size_0()
        .into_any_element()
    }

    /// A mouse press while a card is lifted: one outside the lifted card
    /// cancels the lift, and leaves menus, the drawer and the selection be.
    pub(super) fn press_during_lift(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = &self.kb_drag else {
            return;
        };
        let on_card = self
            .card_anchors
            .get(&drag.card_id)
            .and_then(|anchor| anchor.get())
            .is_some_and(|bounds| bounds.contains(&position));
        if !on_card {
            self.cancel_keyboard_drag(cx);
            cx.notify();
        }
    }

    /// Escape, or the view leaving the screen: put the card back, write
    /// nothing, and catch up on a reload that waited.
    pub(super) fn cancel_keyboard_drag(&mut self, cx: &mut Context<Self>) {
        self.kb_blur = None;
        let Some(drag) = self.kb_drag.take() else {
            return;
        };
        if drag.lane.is_none() {
            self.announce_cancel(&drag.card_id);
        } else {
            self.announce_lane_cancel(&drag.card_id);
        }
        self.drop_target = None;
        self.lane_drop = None;
        if self.pending_reload {
            self.reload(cx);
        }
    }

    fn drop_keyboard_card(&mut self, cx: &mut Context<Self>) {
        if self
            .kb_drag
            .as_ref()
            .is_some_and(|drag| drag.lane.is_some())
        {
            return self.drop_keyboard_lane_card(cx);
        }
        let Some(drag) = self.kb_drag.take() else {
            return;
        };
        self.kb_blur = None;
        self.drop_target = None;
        self.announce_drop(&drag);
        let slot = self.full_slot(&drag.column_id, drag.slot);
        let resolved = self.resolve_drop(&drag.card_id, &drag.column_id, slot);
        match resolved {
            Some((column, index)) => {
                self.move_with_undo(&drag.card_id, &column, index, cx);
            }
            None if self.pending_reload => self.reload(cx),
            None => cx.notify(),
        }
    }
}

/// Where a card is or would land, as Tauri's announcements word it.
struct Place {
    column: String,
    index: usize,
    total: usize,
}

impl Place {
    fn words(&self) -> String {
        format!(
            "{}, position {} of {}",
            self.column,
            self.index + 1,
            self.total
        )
    }
}

impl BoardView {
    /// What the last board-layout keyboard drag step said: picked up, over,
    /// dropped or cancelled, in Tauri's words. Positions count every card the
    /// column shows, not the cards its window has mounted.
    pub(crate) fn board_announcement(&self) -> Option<String> {
        self.drag_announcement.clone()
    }

    /// The ids of the cards a column shows (search and filter applied),
    /// collapsed or not.
    fn shown_ids(&self, column_id: &str) -> Vec<String> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        self.column_cards(detail, column_id)
            .into_iter()
            .map(|card| card.id.clone())
            .collect()
    }

    /// Where the card is now.
    fn place_now(&self, card_id: &str) -> Option<Place> {
        let column_id = self.column_of_card(card_id)?;
        let ids = self.shown_ids(&column_id);
        let index = ids.iter().position(|id| id == card_id)?;
        Some(Place {
            column: self.column(&column_id)?.name.clone(),
            index,
            total: ids.len(),
        })
    }

    /// Where the lifted card would land at the drag's slot: the slot counts
    /// the column's shown cards, the lifted one included, and a collapsed
    /// column takes it at its end.
    fn place_at(&self, drag: &KeyboardDrag) -> Option<Place> {
        let ids = self.shown_ids(&drag.column_id);
        let column = self.column(&drag.column_id)?.name.clone();
        let (index, total) = match ids.iter().position(|id| *id == drag.card_id) {
            Some(old) => {
                let index = if self.is_collapsed(&drag.column_id) {
                    ids.len() - 1
                } else if drag.slot > old {
                    drag.slot - 1
                } else {
                    drag.slot
                };
                (index.min(ids.len() - 1), ids.len())
            }
            None => {
                let index = if self.is_collapsed(&drag.column_id) {
                    ids.len()
                } else {
                    drag.slot.min(ids.len())
                };
                (index, ids.len() + 1)
            }
        };
        Some(Place {
            column,
            index,
            total,
        })
    }

    fn card_label(&self, card_id: &str) -> Option<(i64, String)> {
        let card = self.card(card_id)?;
        Some((card.number, card.title.clone()))
    }

    fn announce_pick_up(&mut self, card_id: &str) {
        self.drag_announcement = self.card_label(card_id).zip(self.place_now(card_id)).map(
            |((number, title), place)| {
                format!("Picked up card #{number}, {title}. {}.", place.words())
            },
        );
    }

    fn announce_over(&mut self, drag: &KeyboardDrag) {
        self.drag_announcement = self
            .card_label(&drag.card_id)
            .zip(self.place_at(drag))
            .map(|((number, _), place)| format!("Card #{number} is over {}.", place.words()));
    }

    fn announce_cancel(&mut self, card_id: &str) {
        self.drag_announcement =
            self.card_label(card_id)
                .zip(self.place_now(card_id))
                .map(|((number, _), place)| {
                    format!(
                        "Moving card #{number} was cancelled. It is back in {}.",
                        place.words()
                    )
                });
    }

    /// Said before the drop is written, from where the card lands. A sorted
    /// board keeps a card's place inside its own column (the drop is
    /// cancelled) and appends it to another.
    fn announce_drop(&mut self, drag: &KeyboardDrag) {
        let inside = self.column_of_card(&drag.card_id).as_deref() == Some(drag.column_id.as_str());
        if self.sorted() && inside {
            return self.announce_cancel(&drag.card_id);
        }
        let place = self.place_at(drag).map(|mut place| {
            if self.sorted() {
                place.index = place.total - 1;
            }
            place
        });
        self.drag_announcement = self
            .card_label(&drag.card_id)
            .zip(place)
            .map(|((number, _), place)| format!("Dropped card #{number} in {}.", place.words()));
    }
}
