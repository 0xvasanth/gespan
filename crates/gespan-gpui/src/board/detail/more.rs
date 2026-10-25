//! The More menu: the drawer's "…" and a right-click on a card. Open, the
//! quick pickers, Move to board…, the two copies, Duplicate, Archive and
//! Delete. Archive and Delete go through the undo toast.
use super::{Choice, Picker};
use crate::{
    board::{
        BoardView, CardMenu,
        undo::{Undo, archived_label},
    },
    popover::{Anchor, Menu, MenuAction, MenuEvent, MenuItem},
};
use gpui::{AppContext as _, ClipboardItem, Context, Window};

/// The link "Copy link" writes; routing it inside the app comes later.
pub(crate) fn card_link(board_id: &str, card_id: &str) -> String {
    format!("gespan://board/{board_id}/card/{card_id}")
}

/// What "Copy reference" puts on the clipboard: `#N Title`.
pub(crate) fn card_reference(number: i64, title: &str) -> String {
    format!("#{number} {title}")
}

const COPY_LABEL_TITLE: usize = 40;

/// The menu row for the reference, with a long title cut short.
pub(crate) fn copy_label(number: i64, title: &str) -> String {
    let title = title.trim();
    let shown = if title.chars().count() > COPY_LABEL_TITLE {
        let cut: String = title.chars().take(COPY_LABEL_TITLE).collect();
        format!("{}…", cut.trim_end())
    } else {
        title.to_string()
    };
    format!("Copy {}", card_reference(number, &shown))
}

impl BoardView {
    pub(crate) fn open_more_menu(
        &mut self,
        card_id: String,
        anchor: Anchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dragging {
            return;
        }
        let Some(card) = self.card(&card_id) else {
            return;
        };
        let copy_ref = copy_label(card.number, &card.title);
        let items: Vec<MenuItem> = vec![
            MenuAction::new("open", "Open").into(),
            MenuAction::new("status", "Status…").into(),
            MenuAction::new("priority", "Priority…").into(),
            MenuAction::new("labels", "Labels…").into(),
            MenuAction::new("move-board", "Move to board…").into(),
            MenuAction::new("copy-link", "Copy link").into(),
            MenuAction::new("copy-ref", copy_ref).into(),
            MenuAction::new("duplicate", "Duplicate").into(),
            MenuAction::new("archive", "Archive").into(),
            MenuAction::new("delete", "Delete").danger().into(),
        ];
        self.show_card_menu("Card actions", items, card_id, anchor, window, cx);
    }

    fn show_card_menu(
        &mut self,
        title: &'static str,
        items: Vec<MenuItem>,
        card_id: String,
        anchor: Anchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        let menu = cx.new(|cx| Menu::new(title, items, window, cx));
        let events = cx.subscribe_in(
            &menu,
            window,
            move |this, _, event: &MenuEvent, window, cx| {
                let Some(open) = this.card_menu.take() else {
                    return;
                };
                if let MenuEvent::Picked(id, _) = event {
                    let anchor = open.anchor.clone();
                    this.apply_more(id, open.card_id, anchor, window, cx);
                }
                cx.notify();
            },
        );
        self.card_menu = Some(CardMenu {
            card_id,
            anchor: Some(anchor),
            menu,
            _events: events,
        });
        cx.notify();
    }

    fn apply_more(
        &mut self,
        id: &str,
        card_id: String,
        anchor: Option<Anchor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(board_id) = id.strip_prefix("to-board-") {
            return self.move_card_to_board(&card_id, board_id.to_string(), cx);
        }
        match id {
            "open" => self.open_card_detail(card_id, window, cx),
            "status" | "priority" | "labels" => {
                // The pickers live in the drawer, so a card that isn't open
                // opens first.
                if self.shown_in_drawer(&card_id).is_none() {
                    self.open_card_detail(card_id.clone(), window, cx);
                }
                if self.shown_in_drawer(&card_id).is_none() {
                    return;
                }
                match id {
                    "status" => self.open_choice(Choice::Status, window, cx),
                    "priority" => self.open_choice(Choice::Priority, window, cx),
                    _ => self.open_picker(Picker::Labels, window, cx),
                }
            }
            "move-board" => self.open_board_menu(card_id, anchor, window, cx),
            "copy-link" => {
                if let Some(card) = self.card(&card_id) {
                    let link = card_link(&card.board_id, &card.id);
                    cx.write_to_clipboard(ClipboardItem::new_string(link));
                }
            }
            "copy-ref" => {
                if let Some(card) = self.card(&card_id) {
                    let text = card_reference(card.number, &card.title);
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            "duplicate" => self.duplicate_card(&card_id, cx),
            "archive" => self.archive_card(&card_id, cx),
            "delete" => self.delete_card(&card_id, cx),
            _ => {}
        }
    }

    fn shown_in_drawer(&self, card_id: &str) -> Option<()> {
        (self.card_detail.as_ref()?.card_id == card_id).then_some(())
    }

    /// "Move to board…": every other board, by name.
    fn open_board_menu(
        &mut self,
        card_id: String,
        anchor: Option<Anchor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.card(&card_id) else {
            return;
        };
        let mut items: Vec<MenuItem> = self
            .boards
            .iter()
            .filter(|board| board.id != card.board_id)
            .map(|board| {
                MenuAction::new(format!("to-board-{}", board.id), board.name.clone()).into()
            })
            .collect();
        if items.is_empty() {
            items.push(
                MenuAction::new("no-boards", "No other boards")
                    .disabled(true)
                    .into(),
            );
        }
        let anchor = anchor.unwrap_or_else(|| Anchor::Slot(Default::default()));
        self.show_card_menu("Move to board", items, card_id, anchor, window, cx);
    }

    fn move_card_to_board(&mut self, card_id: &str, board_id: String, cx: &mut Context<Self>) {
        let Some(card) = self.card(card_id).cloned() else {
            return;
        };
        let target = self
            .boards
            .iter()
            .find(|board| board.id == board_id)
            .map(|board| board.name.clone())
            .unwrap_or_default();
        if !self.close_detail_for(card_id, cx) {
            return;
        }
        self.commit_pending(cx);
        let mut moved = None;
        if self.write(cx, |service| {
            moved = Some(service.move_card_to_board(&card.id, &board_id)?);
            Ok(())
        }) && let Some(moved) = moved
        {
            let label = format!("Moved #{} to {target} as #{}", card.number, moved.number);
            self.show_toast(label, None, cx);
        }
    }

    pub(crate) fn duplicate_card(&mut self, card_id: &str, cx: &mut Context<Self>) {
        let Some(card) = self.card(card_id).cloned() else {
            return;
        };
        // A typed but unsaved description is part of what gets copied.
        if self.shown_in_drawer(card_id).is_some() && !self.save_card_details(cx) {
            return;
        }
        self.commit_pending(cx);
        let mut copy = None;
        if self.write(cx, |service| {
            copy = Some(service.duplicate_card(&card.id)?);
            Ok(())
        }) && let Some(copy) = copy
        {
            let label = format!("Duplicated #{} as #{}", card.number, copy.number);
            self.show_toast(label, None, cx);
        }
    }

    fn archive_card(&mut self, card_id: &str, cx: &mut Context<Self>) {
        let Some((place, number)) = self.place_of(card_id) else {
            return;
        };
        if !self.close_detail_for(card_id, cx) {
            return;
        }
        self.commit_pending(cx);
        let ids = [card_id.to_string()];
        if self.write(cx, |service| service.archive_many(&ids)) {
            self.push_undo(archived_label(&[number]), Undo::Archive(vec![place]), cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_row_names_the_real_card() {
        assert_eq!(copy_label(7, "Fix the bug"), "Copy #7 Fix the bug");
        assert_eq!(card_reference(7, "Fix the bug"), "#7 Fix the bug");
    }

    #[test]
    fn a_long_title_is_cut_in_the_row_but_not_on_the_clipboard() {
        let title = "x".repeat(60);
        assert_eq!(
            copy_label(1, &title),
            format!("Copy #1 {}…", "x".repeat(40))
        );
        assert_eq!(card_reference(1, &title), format!("#1 {title}"));
        // Cuts on characters, never inside one.
        let wide = "é".repeat(45);
        assert_eq!(copy_label(2, &wide), format!("Copy #2 {}…", "é".repeat(40)));
    }
}
