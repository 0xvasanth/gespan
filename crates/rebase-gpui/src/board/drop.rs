//! Where a dragged card or column lands. Slots count every visible item,
//! including the dragged one; the board backend takes the index after the item
//! is lifted out, so slots past its old place shift down by one.
use gpui::Pixels;
use rebase_core::board::BoardDetail;

/// The slot a pointer at `pointer_y` points at: before the first card whose
/// middle is below the pointer, otherwise after the last card.
pub(crate) fn card_drop_index(tops: &[Pixels], heights: &[Pixels], pointer_y: Pixels) -> usize {
    tops.iter()
        .zip(heights)
        .position(|(top, height)| pointer_y < *top + *height / 2.)
        .unwrap_or(tops.len())
}

/// The `(column, index)` to send to `card_move`, or `None` when the card would
/// land where it already is.
pub(crate) fn resolve_card_drop(
    detail: &BoardDetail,
    card_id: &str,
    column_id: &str,
    slot: usize,
) -> Option<(String, usize)> {
    let card = detail.cards.iter().find(|card| card.id == card_id)?;
    if card.column_id != column_id {
        return Some((column_id.to_string(), slot));
    }
    let old = card.position.max(0) as usize;
    let index = if slot > old { slot - 1 } else { slot };
    (index != old).then(|| (column_id.to_string(), index))
}

/// The index to send to `column_move` for a column dropped at `slot`.
pub(crate) fn resolve_column_drop(from: usize, slot: usize) -> Option<usize> {
    let index = if slot > from { slot - 1 } else { slot };
    (index != from).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;
    use rebase_core::board::{Board, BoardDetail, Card, Column};

    fn tops() -> (Vec<gpui::Pixels>, Vec<gpui::Pixels>) {
        // Three 40px cards stacked 6px apart starting at y=100.
        (vec![px(100.), px(146.), px(192.)], vec![px(40.); 3])
    }

    #[test]
    fn a_pointer_above_the_first_midpoint_inserts_first() {
        let (t, h) = tops();
        assert_eq!(card_drop_index(&t, &h, px(110.)), 0);
    }

    #[test]
    fn a_pointer_between_cards_inserts_between_them() {
        let (t, h) = tops();
        assert_eq!(card_drop_index(&t, &h, px(130.)), 1);
        assert_eq!(card_drop_index(&t, &h, px(170.)), 2);
    }

    #[test]
    fn a_pointer_below_every_card_inserts_last() {
        let (t, h) = tops();
        assert_eq!(card_drop_index(&t, &h, px(400.)), 3);
    }

    #[test]
    fn an_empty_column_inserts_at_zero() {
        assert_eq!(card_drop_index(&[], &[], px(50.)), 0);
    }

    fn detail() -> BoardDetail {
        let column = |id: &str, position| Column {
            id: id.into(),
            board_id: "b".into(),
            name: id.into(),
            position,
            role: None,
        };
        let card = |id: &str, column_id: &str, position| Card {
            id: id.into(),
            board_id: "b".into(),
            column_id: column_id.into(),
            number: position + 1,
            title: id.into(),
            body: String::new(),
            position,
            link_kind: None,
            link_id: None,
            created_at: 0,
            updated_at: 0,
        };
        BoardDetail {
            board: Board {
                id: "b".into(),
                name: "Board".into(),
                created_at: 0,
                updated_at: 0,
            },
            columns: vec![column("a", 0), column("b", 1), column("empty", 2)],
            cards: vec![
                card("a0", "a", 0),
                card("a1", "a", 1),
                card("a2", "a", 2),
                card("a3", "a", 3),
                card("b0", "b", 0),
            ],
        }
    }

    #[test]
    fn dropping_a_card_back_in_its_own_place_is_no_move() {
        let detail = detail();
        assert_eq!(resolve_card_drop(&detail, "a1", "a", 1), None);
        assert_eq!(resolve_card_drop(&detail, "a1", "a", 2), None);
    }

    #[test]
    fn moving_down_a_column_shifts_the_slot_past_the_card_itself() {
        // Slot 3 sits before a3; with a0 lifted out that is index 2.
        assert_eq!(
            resolve_card_drop(&detail(), "a0", "a", 3),
            Some(("a".into(), 2))
        );
    }

    #[test]
    fn moving_up_a_column_keeps_the_slot() {
        assert_eq!(
            resolve_card_drop(&detail(), "a3", "a", 1),
            Some(("a".into(), 1))
        );
    }

    #[test]
    fn moving_across_columns_uses_the_hovered_slot() {
        assert_eq!(
            resolve_card_drop(&detail(), "a2", "b", 0),
            Some(("b".into(), 0))
        );
        assert_eq!(
            resolve_card_drop(&detail(), "a2", "b", 1),
            Some(("b".into(), 1))
        );
    }

    #[test]
    fn an_empty_column_takes_the_card_at_zero() {
        assert_eq!(
            resolve_card_drop(&detail(), "b0", "empty", 0),
            Some(("empty".into(), 0))
        );
    }

    #[test]
    fn column_slots_resolve_like_card_slots() {
        assert_eq!(resolve_column_drop(0, 0), None);
        assert_eq!(resolve_column_drop(0, 1), None);
        assert_eq!(resolve_column_drop(0, 3), Some(2));
        assert_eq!(resolve_column_drop(2, 0), Some(0));
    }
}
