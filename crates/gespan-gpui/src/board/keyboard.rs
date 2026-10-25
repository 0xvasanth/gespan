//! Keyboard drag. On a focused card, arrows move focus; Space lifts it. While
//! lifted, arrows move the drop line, Space or Enter drops and Escape (handled
//! by the board's `on_key`) puts it back. Only cards the search and filter show
//! count; the drop maps the visible slot to the whole column like a mouse drop.
use super::{BoardView, KeyboardDrag, drop};
use gpui::{App, Context, KeyDownEvent, Window};

#[derive(Clone, Copy, PartialEq)]
enum Arrow {
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
    fn visible_ids(&self, column_id: &str, cx: &App) -> Vec<String> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        let query = self.query(cx);
        let mut cards: Vec<_> = detail
            .cards
            .iter()
            .filter(|card| card.column_id == column_id && self.card_matches(card, &query))
            .collect();
        cards.sort_by_key(|card| card.position);
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
            "space" => self.lift_card(card_id, cx),
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
        let here = self.visible_ids(&column_id, cx);
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
                    let cards = self.visible_ids(column, cx);
                    cards.get(index.min(cards.len().saturating_sub(1))).cloned()
                })
            }
        };
        if let Some(handle) = target.and_then(|id| self.card_focus.get(&id)) {
            window.focus(handle, cx);
        }
    }

    fn lift_card(&mut self, card_id: &str, cx: &mut Context<Self>) -> bool {
        let Some(column_id) = self.column_of_card(card_id) else {
            return false;
        };
        let slot = self
            .visible_ids(&column_id, cx)
            .iter()
            .position(|id| id == card_id)
            .unwrap_or(0);
        self.menu = None;
        self.drop_target = Some((column_id.clone(), slot));
        self.kb_drag = Some(KeyboardDrag {
            card_id: card_id.to_string(),
            column_id,
            slot,
        });
        cx.notify();
        true
    }

    fn move_drop_target(&mut self, direction: Arrow, cx: &mut Context<Self>) {
        let Some(mut drag) = self.kb_drag.clone() else {
            return;
        };
        let origin = self.column_of_card(&drag.card_id);
        let old_slot = |this: &Self, column: &str, cx: &App| {
            (origin.as_deref() == Some(column))
                .then(|| {
                    this.visible_ids(column, cx)
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
                let len = self.visible_ids(&drag.column_id, cx).len();
                let old = old_slot(self, &drag.column_id, cx);
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
                    let len = self.visible_ids(&column, cx).len();
                    let old = old_slot(self, &column, cx);
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
        if self.kb_drag.take().is_none() {
            return;
        }
        self.drop_target = None;
        if self.pending_reload {
            self.reload(cx);
        }
    }

    pub(super) fn drop_lifted_card_if_gone(&mut self) {
        let gone = match (&self.kb_drag, &self.detail) {
            (Some(drag), Some(detail)) => !detail.cards.iter().any(|c| c.id == drag.card_id),
            _ => false,
        };
        if gone {
            self.kb_drag = None;
            self.drop_target = None;
        }
    }

    fn drop_keyboard_card(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.kb_drag.take() else {
            return;
        };
        self.drop_target = None;
        let slot = self.full_slot(&drag.column_id, drag.slot, cx);
        let resolved = self.detail.as_ref().and_then(|detail| {
            drop::resolve_card_drop(detail, &drag.card_id, &drag.column_id, slot)
        });
        match resolved {
            Some((column, index)) => {
                let card_id = drag.card_id;
                self.write(cx, |service| {
                    service.card_move(&card_id, &column, index, "user")
                });
            }
            None if self.pending_reload => self.reload(cx),
            None => cx.notify(),
        }
    }
}
