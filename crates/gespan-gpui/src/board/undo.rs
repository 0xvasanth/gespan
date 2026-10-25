//! Undo for board actions. Each undoable action records its inverse before it
//! runs, then shows one toast for 8 seconds with an Undo button. A new action
//! replaces the toast. A delete is held: the card is archived at once and only
//! deleted when its toast goes (expiry, a new action or a board switch). If the
//! view goes away first the timer goes with it, so the card stays archived.
use super::BoardView;
use crate::theme::*;
use gespan_core::board::Service;
use gpui::{AnyElement, Context, Role, Task, div, prelude::*, px};
use std::time::Duration;

/// How long a toast, and so a held delete, lasts.
pub(crate) const UNDO_WINDOW: Duration = Duration::from_secs(8);

/// Where a card was: (card, column, index among the column's live cards).
pub(crate) type Place = (String, String, usize);

/// The inverse of an undoable action, captured before it ran.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Undo {
    Move {
        card_id: String,
        column_id: String,
        index: usize,
    },
    MoveMany(Vec<Place>),
    Archive(Vec<Place>),
    Delete {
        card_id: String,
        column_id: String,
        index: usize,
    },
    Sort {
        column_id: String,
        order: Vec<String>,
    },
}

impl Undo {
    /// The card a held delete will delete.
    fn pending_delete(&self) -> Option<&str> {
        match self {
            Undo::Delete { card_id, .. } => Some(card_id),
            _ => None,
        }
    }
}

/// The toast: an action that can be undone, or a message (`undo` is `None`).
pub(crate) struct Toast {
    pub label: String,
    pub undo: Option<Undo>,
}

#[derive(Default)]
pub(crate) struct UndoController {
    pub(crate) toast: Option<Toast>,
    /// Bumped per toast, so an older timer never closes a newer toast.
    generation: u64,
    timer: Option<Task<()>>,
}

impl UndoController {
    /// The card whose delete is being held, if any.
    pub(crate) fn pending_delete(&self) -> Option<&str> {
        self.toast.as_ref()?.undo.as_ref()?.pending_delete()
    }

    /// The column the held delete's card was archived from, if any.
    pub(crate) fn pending_delete_column(&self) -> Option<&str> {
        match self.toast.as_ref()?.undo.as_ref()? {
            Undo::Delete { column_id, .. } => Some(column_id),
            _ => None,
        }
    }
}

/// What the toast says when an undo hit a store error.
pub(crate) fn undo_failure(error: &str) -> String {
    match error {
        "Card not found" => "Couldn't undo: the card was deleted".into(),
        "Column not on this board" | "Column not found" => {
            "Couldn't undo: the column was deleted".into()
        }
        other => format!("Couldn't undo: {other}"),
    }
}

/// "Moved #N to {column}" for one card, "Moved N cards to {column}" for more.
pub(crate) fn moved_label(numbers: &[i64], column: &str) -> String {
    match numbers {
        [number] => format!("Moved #{number} to {column}"),
        _ => format!("Moved {} cards to {column}", numbers.len()),
    }
}

pub(crate) fn archived_label(numbers: &[i64]) -> String {
    match numbers {
        [number] => format!("Archived #{number}"),
        _ => format!("Archived {} cards", numbers.len()),
    }
}

/// Put cards back where they were, one at a time and lowest index first per
/// column, so each lands at its old index. Every card is tried; the first
/// error is returned.
fn put_back(
    places: &[Place],
    mut place: impl FnMut(&str, &str, usize) -> Result<(), String>,
) -> Result<(), String> {
    let mut places: Vec<&Place> = places.iter().collect();
    places.sort_by(|a, b| (&a.1, a.2).cmp(&(&b.1, b.2)));
    let mut first = Ok(());
    for (card, column, index) in places {
        let result = place(card, column, *index);
        if first.is_ok() {
            first = result;
        }
    }
    first
}

/// Carry out a held delete, but only while the card is still archived: the
/// other app, another window or an agent may have restored it meanwhile, and
/// a live card is never deleted. A card already gone is left alone.
fn delete_if_archived(service: &mut Service, card_id: &str) -> Result<(), String> {
    let card = match service.card(card_id) {
        Ok(card) => card,
        Err(error) if error == "Card not found" => return Ok(()),
        Err(error) => return Err(error),
    };
    let archived = service
        .archived(&card.board_id)?
        .iter()
        .any(|archived| archived.id == card_id);
    if !archived {
        return Ok(());
    }
    match service.card_delete(card_id) {
        Err(error) if error != "Card not found" => Err(error),
        _ => Ok(()),
    }
}

/// Run an inverse. Restores everything it can and returns the first error.
fn run(service: &mut Service, board_id: &str, undo: &Undo) -> Result<(), String> {
    match undo {
        Undo::Move {
            card_id,
            column_id,
            index,
        } => service.card_move(card_id, column_id, *index, "user"),
        Undo::MoveMany(places) => put_back(places, |card, column, index| {
            service.card_move(card, column, index, "user")
        }),
        Undo::Archive(places) => put_back(places, |card, column, index| {
            service.restore(card, column, index)
        }),
        Undo::Delete {
            card_id,
            column_id,
            index,
        } => service.restore(card_id, column_id, *index),
        Undo::Sort { column_id, order } => {
            // Cards that left the column (or the board) since are skipped.
            let live = service.get(board_id)?;
            let still: Vec<String> = order
                .iter()
                .filter(|id| {
                    live.cards
                        .iter()
                        .any(|card| &card.id == *id && &card.column_id == column_id)
                })
                .cloned()
                .collect();
            let result = service.move_many(&still, column_id, 0, "user");
            match order.iter().find(|id| !still.contains(id)) {
                Some(gone) if result.is_ok() => service.card(gone).map(drop),
                _ => result,
            }
        }
    }
}

impl BoardView {
    /// Show the toast for an action that just ran. A delete held by the toast
    /// it replaces is committed first.
    pub(crate) fn push_undo(&mut self, label: String, undo: Undo, cx: &mut Context<Self>) {
        self.commit_pending(cx);
        self.show_toast(label, Some(undo), cx);
    }

    fn show_toast(&mut self, label: String, undo: Option<Undo>, cx: &mut Context<Self>) {
        self.undo.generation += 1;
        let generation = self.undo.generation;
        self.undo.toast = Some(Toast { label, undo });
        self.undo.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(UNDO_WINDOW).await;
            let _ = this.update(cx, |this, cx| this.expire_undo(generation, cx));
        }));
        cx.notify();
    }

    fn expire_undo(&mut self, generation: u64, cx: &mut Context<Self>) {
        if self.undo.generation != generation {
            return;
        }
        let held = self.undo.pending_delete().is_some();
        self.commit_pending(cx);
        if held {
            self.reload(cx);
        }
        cx.notify();
    }

    /// Close the toast. A held delete is carried out now. Call before any
    /// undoable action and on a board switch.
    pub(crate) fn commit_pending(&mut self, cx: &mut Context<Self>) {
        let Some(toast) = self.undo.toast.take() else {
            return;
        };
        self.undo.generation += 1;
        if let Some(card_id) = toast.undo.as_ref().and_then(Undo::pending_delete) {
            match self
                .service()
                .and_then(|service| delete_if_archived(service, card_id))
            {
                Ok(()) => {}
                Err(error) => self.error = Some(error),
            }
        }
        cx.notify();
    }

    /// Undo the toast's action. What can't be undone is said in the toast.
    pub(crate) fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(undo) = self
            .undo
            .toast
            .as_ref()
            .and_then(|toast| toast.undo.clone())
        else {
            return;
        };
        self.undo.toast = None;
        self.undo.generation += 1;
        let board_id = self.open_board_id.clone().unwrap_or_default();
        let result = self
            .service()
            .and_then(|service| run(service, &board_id, &undo));
        self.reload(cx);
        if let Err(error) = result {
            self.show_toast(undo_failure(&error), None, cx);
        }
        cx.notify();
    }

    /// Where a card sits now, and its number, read from the loaded board.
    pub(crate) fn place_of(&self, card_id: &str) -> Option<(Place, i64)> {
        let card = self
            .detail
            .as_ref()?
            .cards
            .iter()
            .find(|card| card.id == card_id)?;
        let place = (
            card.id.clone(),
            card.column_id.clone(),
            card.position.max(0) as usize,
        );
        Some((place, card.number))
    }

    pub(crate) fn column_name(&self, column_id: &str) -> String {
        self.column(column_id)
            .map(|column| column.name.clone())
            .unwrap_or_default()
    }

    /// Move one card as the user, with an undo toast. True when it moved.
    pub(crate) fn move_with_undo(
        &mut self,
        card_id: &str,
        column_id: &str,
        index: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(((_, from_column, from_index), number)) = self.place_of(card_id) else {
            return false;
        };
        let label = moved_label(&[number], &self.column_name(column_id));
        self.commit_pending(cx);
        let moved = self.write(cx, |service| {
            service.card_move(card_id, column_id, index, "user")
        });
        if moved {
            self.push_undo(
                label,
                Undo::Move {
                    card_id: card_id.to_string(),
                    column_id: from_column,
                    index: from_index,
                },
                cx,
            );
        }
        moved
    }

    /// The places and numbers of several cards, as they sit now.
    pub(crate) fn places_of(&self, card_ids: &[String]) -> (Vec<Place>, Vec<i64>) {
        card_ids.iter().filter_map(|id| self.place_of(id)).unzip()
    }

    pub(super) fn render_undo_toast(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let toast = self.undo.toast.as_ref()?;
        let undo_button = toast.undo.is_some().then(|| {
            div()
                .id("board-undo")
                .debug_selector(|| "board-undo".into())
                .role(Role::Button)
                .aria_label("Undo")
                .flex_shrink_0()
                .h(px(24.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .text_size(px(12.))
                .text_color(accent_color())
                .cursor_pointer()
                .hover(|s| s.bg(content(0.08)))
                .child("Undo")
                .on_click(cx.listener(|this, _, _, cx| this.undo(cx)))
        });
        Some(
            div()
                .absolute()
                .bottom(px(16.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    crate::notifications::surface::toast_frame()
                        .id("board-undo-toast")
                        .debug_selector(|| "board-undo-toast".into())
                        .role(Role::Status)
                        .aria_label(toast.label.clone())
                        .w(px(340.))
                        .px(px(12.))
                        .py(px(8.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.))
                                .truncate()
                                .child(toast.label.clone()),
                        )
                        .children(undo_button),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_errors_read_as_what_was_lost() {
        assert_eq!(
            undo_failure("Card not found"),
            "Couldn't undo: the card was deleted"
        );
        assert_eq!(
            undo_failure("Column not on this board"),
            "Couldn't undo: the column was deleted"
        );
        assert_eq!(
            undo_failure("Column not found"),
            "Couldn't undo: the column was deleted"
        );
        assert_eq!(undo_failure("disk full"), "Couldn't undo: disk full");
    }

    #[test]
    fn labels_name_one_card_by_number_and_more_by_count() {
        assert_eq!(moved_label(&[4], "Done"), "Moved #4 to Done");
        assert_eq!(moved_label(&[4, 5], "Done"), "Moved 2 cards to Done");
        assert_eq!(archived_label(&[7]), "Archived #7");
        assert_eq!(archived_label(&[7, 8, 9]), "Archived 3 cards");
    }

    #[test]
    fn put_back_goes_lowest_index_first_and_tries_every_card() {
        let places = vec![
            ("c".to_string(), "x".to_string(), 2),
            ("a".to_string(), "x".to_string(), 0),
            ("b".to_string(), "x".to_string(), 1),
        ];
        let mut seen = Vec::new();
        let result = put_back(&places, |card, _, index| {
            seen.push((card.to_string(), index));
            if card == "b" {
                Err("Card not found".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Err("Card not found".into()));
        assert_eq!(seen, [("a".into(), 0), ("b".into(), 1), ("c".into(), 2)]);
    }
}
