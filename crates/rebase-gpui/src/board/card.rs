//! One board card: title, mono number, link chip and how long ago it changed.
//! Cards drag between columns; the "…" menu offers Move to… for keyboards.
use super::{BoardView, DraggedCard, LinkLabel};
use crate::theme::*;
use gpui::{AnyElement, Context, MouseButton, Role, div, prelude::*, px};
use rebase_core::board::Card;

impl BoardView {
    pub(super) fn render_card(
        &self,
        card: &Card,
        done: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
        let anchor = self.card_anchors.get(&card.id).cloned().unwrap_or_default();
        let lifted = self.dragged_card.as_ref() == Some(&card.id);
        let group = format!("board-card-group-{number}");
        let payload = DraggedCard {
            card_id: card.id.clone(),
            number,
            title: card.title.clone(),
        };
        let weak = cx.weak_entity();
        let menu_id = card.id.clone();
        div()
            .id(("board-card", number as u64))
            .debug_selector(move || format!("board-card-{number}"))
            .group(group.clone())
            .relative()
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
            .when(lifted, |d| d.opacity(0.35))
            .flex()
            .flex_col()
            .gap(px(6.))
            .cursor_grab()
            .on_drag(payload, move |payload, _, _, cx| {
                let card_id = payload.card_id.clone();
                let _ = weak.update(cx, |this, cx| {
                    this.begin_card_drag(card_id);
                    cx.notify();
                });
                cx.new(|_| payload.clone())
            })
            .child(anchor.probe())
            .child(
                div()
                    .pr(px(20.))
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
            .child(
                div()
                    .id(("board-card-menu", number as u64))
                    .debug_selector(move || format!("board-card-menu-{number}"))
                    .absolute()
                    .top(px(6.))
                    .right(px(6.))
                    .size(px(22.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .role(Role::Button)
                    .aria_label("Card options")
                    .opacity(0.)
                    .group_hover(group, |s| s.opacity(1.))
                    .hover(|s| s.bg(content(0.08)))
                    .cursor_pointer()
                    .child(icon("more-horizontal", 12.).text_color(content(0.6)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_card_menu(menu_id.clone(), cx)),
                    ),
            )
            .into_any_element()
    }
}
