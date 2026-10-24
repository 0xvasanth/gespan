//! One board card: title, mono number, link chip and how long ago it changed.
use super::{BoardView, LinkLabel};
use crate::theme::*;
use gpui::{AnyElement, Context, Role, div, prelude::*, px};
use rebase_core::board::Card;

impl BoardView {
    pub(super) fn render_card(&self, card: &Card, done: bool, _: &mut Context<Self>) -> AnyElement {
        let number = card.number;
        let chip = card.link_kind.as_deref().map(|kind| {
            let glyph = if kind == "pilot" { "bot" } else { "chatting" };
            let (text, alpha) = match self.link_label(&card.id) {
                Some(LinkLabel::Found(title)) => (title, 0.7),
                _ => ("Linked work no longer exists".to_string(), 0.45),
            };
            div()
                .max_w(px(140.))
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_color(content(alpha))
                .child(icon(glyph, 11.))
                .child(div().min_w_0().truncate().child(text))
        });
        let updated =
            rebase_core::time::relative(card.updated_at.max(0) as u64, rebase_core::time::now_ms());
        div()
            .id(("board-card", number as u64))
            .debug_selector(move || format!("board-card-{number}"))
            .role(Role::ListItem)
            .aria_label(card.title.clone())
            .flex_shrink_0()
            .px(px(12.))
            .py(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(content(0.07))
            .bg(pane_fill())
            .hover(|s| s.border_color(content(0.16)))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .line_clamp(2)
                    .text_ellipsis()
                    .text_color(content(if done { 0.6 } else { 0.92 }))
                    .child(card.title.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(11.))
                    .child(
                        div()
                            .font_family("Menlo")
                            .text_color(content(0.45))
                            .child(format!("#{number}")),
                    )
                    .children(chip)
                    .child(div().flex_1())
                    .child(div().text_color(content(0.4)).child(updated)),
            )
            .into_any_element()
    }
}
