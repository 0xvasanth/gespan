//! Keyboard drag. On a focused card, arrows move focus; Space lifts it. While
//! lifted, arrows move the drop line, Space or Enter drops and Escape (handled
//! by the board's `on_key`) puts it back. Only cards the search and filter show
//! count; the drop maps the visible slot to the whole column like a mouse drop.
use super::{BoardView, KeyboardDrag};
use gpui::{App, Context, KeyBinding, KeyDownEvent, Window, actions};

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
            if let Some(handle) = self
                .lane_focus_target(card_id, direction)
                .and_then(|id| self.card_focus.get(&id))
            {
                window.focus(handle, cx);
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
        if let Some(handle) = target.and_then(|id| self.card_focus.get(&id)) {
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
        self.kb_drag = Some(drag);
        cx.notify();
    }

    /// Escape, or the view leaving the screen: put the card back, write
    /// nothing, and catch up on a reload that waited.
    pub(super) fn cancel_keyboard_drag(&mut self, cx: &mut Context<Self>) {
        self.kb_blur = None;
        if self.kb_drag.take().is_none() {
            return;
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
