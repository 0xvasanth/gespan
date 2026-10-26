//! The Archive view: the open board's archived cards, newest first, each with
//! Restore (back to the end of its column) and Delete forever. A card whose
//! delete is being held by the undo toast is not listed.
use super::BoardView;
use crate::theme::*;
use gespan_core::board::Card;
use gpui::{AnyElement, Context, FontWeight, Role, Window, div, prelude::*, px};

impl BoardView {
    /// Open or close the Archive view.
    pub(crate) fn toggle_archive(&mut self, cx: &mut Context<Self>) {
        self.archive_open = !self.archive_open;
        if self.archive_open {
            self.reload(cx);
        } else {
            self.archived.clear();
        }
        cx.notify();
    }

    /// The archived cards the view lists.
    pub(crate) fn archived_cards(&self) -> Vec<&Card> {
        let pending = self.undo.pending_deletes();
        self.archived
            .iter()
            .filter(|card| !pending.contains(&card.id.as_str()))
            .collect()
    }

    fn restore_archived(&mut self, card_id: String, cx: &mut Context<Self>) {
        self.write(cx, |service| service.unarchive(&card_id));
    }

    fn delete_forever(&mut self, card: Card, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Delete “{}” forever?", card.title),
            Some("Its description, subtasks, comments and activity are removed."),
            &["Delete forever", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update(cx, |this, cx| {
                    this.write(cx, |service| service.card_delete(&card.id));
                });
            }
        })
        .detach();
    }

    pub(super) fn render_archive(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.archive_open {
            return None;
        }
        let cards = self.archived_cards();
        let button = |id: String, label: &'static str| {
            let selector = id.clone();
            div()
                .id(gpui::SharedString::from(id))
                .debug_selector(move || selector.clone())
                .role(Role::Button)
                .aria_label(label)
                .h(px(22.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .text_size(px(11.))
                .text_color(content(0.6))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
                .child(label)
        };
        let rows: Vec<AnyElement> = cards
            .iter()
            .enumerate()
            .map(|(n, card)| {
                let (restore_id, deleted) = (card.id.clone(), (*card).clone());
                div()
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(content(0.08))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_clamp(2)
                            .child(card.title.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(
                                div()
                                    .flex_1()
                                    .font_family("Menlo")
                                    .text_size(px(11.))
                                    .text_color(content(0.45))
                                    .child(format!(
                                        "#{} · {}",
                                        card.number,
                                        self.column_name(&card.column_id)
                                    )),
                            )
                            .child(
                                button(format!("board-archive-restore-{n}"), "Restore").on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.restore_archived(restore_id.clone(), cx)
                                    }),
                                ),
                            )
                            .child(
                                button(format!("board-archive-delete-{n}"), "Delete forever")
                                    .text_color(diff_foreground(false))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.delete_forever(deleted.clone(), window, cx)
                                    })),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        let empty = cards.is_empty().then(|| {
            div()
                .id("board-archive-empty")
                .debug_selector(|| "board-archive-empty".into())
                .py(px(24.))
                .text_size(px(12.))
                .text_color(content(0.45))
                .flex()
                .justify_center()
                .child("No archived cards")
        });
        Some(
            div()
                .id("board-archive-panel")
                .debug_selector(|| "board-archive-panel".into())
                .role(Role::Region)
                .aria_label("Archive")
                .w(px(320.))
                .h_full()
                .flex_shrink_0()
                .border_l_1()
                .border_color(content(0.07))
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(PANE_HEADER_HEIGHT))
                        .flex_shrink_0()
                        .px_4()
                        .flex()
                        .items_center()
                        .border_b_1()
                        .border_color(content(0.07))
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child("Archive"),
                )
                .child(
                    div()
                        .id("board-archive-list")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(rows)
                        .children(empty),
                )
                .into_any_element(),
        )
    }
}
