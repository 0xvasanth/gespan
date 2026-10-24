//! One board column: role ring, name, count, actions, cards and the composer.
use super::{BoardView, DraggedCard, DraggedColumn};
use crate::theme::*;
use gpui::{AnyElement, Context, FontWeight, Hsla, MouseButton, Role, div, prelude::*, px, rgb};
use rebase_core::board::{BoardDetail, Column};

pub(super) const COLUMN_WIDTH: f32 = 280.;

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
    pub(super) fn render_column(
        &self,
        index: usize,
        column: &Column,
        detail: &BoardDetail,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cards: Vec<_> = {
            let mut cards: Vec<_> = detail
                .cards
                .iter()
                .filter(|card| card.column_id == column.id && self.card_visible(card, cx))
                .collect();
            cards.sort_by_key(|card| card.position);
            cards
        };
        let done = column.role.as_deref() == Some("done");
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
        let composer = self
            .composer
            .as_ref()
            .filter(|composer| composer.column_id == column.id)
            .map(|composer| composer.input.clone());
        let add_id = column.id.clone();
        let menu_id = column.id.clone();
        let card_target = self
            .drop_target
            .as_ref()
            .filter(|(target, _)| *target == column.id)
            .map(|(_, slot)| *slot);
        let column_slot = self.column_drop;
        let last = index + 1 == detail.columns.len();
        let drop_line = || {
            div()
                .flex_shrink_0()
                .h(px(2.))
                .mx(px(2.))
                .rounded(px(2.))
                .bg(accent_color())
                .into_any_element()
        };
        let mut body: Vec<AnyElement> = Vec::new();
        for (slot, card) in cards.iter().enumerate() {
            if card_target == Some(slot) {
                body.push(drop_line());
            }
            body.push(self.render_card(card, done, cx));
        }
        if card_target.is_some_and(|slot| slot >= cards.len()) {
            body.push(drop_line());
        }
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
            .aria_label(column.name.clone())
            .w(px(COLUMN_WIDTH))
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
                    .child(
                        div()
                            .id(("board-column-count", index))
                            .debug_selector(move || format!("board-column-count-{index}"))
                            .text_size(px(12.))
                            .text_color(content(0.4))
                            .child(self.count_label(&column.id, cx)),
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
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .children(body)
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
                                    .child("Enter to add · Esc to cancel"),
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
                    }),
            )
            .into_any_element()
    }
}
