//! Labels: create, rename, recolour, reorder, merge and delete.
use super::{Edit, button, card_row, heading, palette_hexes, swatch, target};
use crate::board::BoardView;
use gpui::{AnyElement, Context, div, prelude::*, px};

impl BoardView {
    fn recolor_label(&mut self, id: &str, hex: &str, cx: &mut Context<Self>) {
        self.settings_write(cx, |service| service.label_update(id, None, Some(hex)));
    }

    fn move_label_to(&mut self, id: &str, index: usize, cx: &mut Context<Self>) {
        self.settings_write(cx, |service| service.label_move(id, index));
    }

    fn merge_label(&mut self, from: &str, into: &str, cx: &mut Context<Self>) {
        self.settings_write(cx, |service| service.labels_merge(from, into));
    }

    fn delete_label_now(&mut self, id: &str, cx: &mut Context<Self>) {
        self.settings_write(cx, |service| service.label_delete(id));
    }

    pub(super) fn render_labels(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = self.settings.as_ref();
        let merging = page.and_then(|page| page.merging.clone());
        let merge_into = page.and_then(|page| page.merge_into.clone());
        let deleting = page.and_then(|page| page.deleting_label.clone());
        let last = self.labels.len().saturating_sub(1);
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, label) in self.labels.iter().enumerate() {
            let id = label.id.clone();
            let used = self
                .extras
                .values()
                .filter(|extras| extras.labels.contains(&label.id))
                .count();
            let colors: Vec<AnyElement> = palette_hexes()
                .map(|hex| {
                    let on = label.color.eq_ignore_ascii_case(&hex);
                    let (id, value) = (id.clone(), hex.clone());
                    swatch(format!("settings-label-{index}-color-{hex}"), &hex, on)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.recolor_label(&id, &value, cx)),
                        )
                        .into_any_element()
                })
                .collect();
            let (up_id, down_id, merge_id, delete_id) =
                (id.clone(), id.clone(), id.clone(), id.clone());
            let mut row = div().flex().flex_col().gap_2().child(
                card_row()
                    .flex_col()
                    .items_stretch()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(self.editable_text(
                                format!("settings-label-{index}-name"),
                                Edit::LabelName(id.clone()),
                                label.name.clone(),
                                cx,
                            ))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(crate::theme::content(0.45))
                                    .child(format!("{used} cards")),
                            )
                            .child(
                                button(format!("settings-label-{index}-up"), "Up", false)
                                    .opacity(if index == 0 { 0.4 } else { 1. })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if index > 0 {
                                            this.move_label_to(&up_id, index - 1, cx)
                                        }
                                    })),
                            )
                            .child(
                                button(format!("settings-label-{index}-down"), "Down", false)
                                    .opacity(if index == last { 0.4 } else { 1. })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if index < last {
                                            this.move_label_to(&down_id, index + 1, cx)
                                        }
                                    })),
                            )
                            .child(
                                button(format!("settings-label-{index}-merge"), "Merge", false)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(page) = &mut this.settings {
                                            page.clear_prompts();
                                            page.merging = Some(merge_id.clone());
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                button(format!("settings-label-{index}-delete"), "Delete", false)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(page) = &mut this.settings {
                                            page.clear_prompts();
                                            page.deleting_label = Some(delete_id.clone());
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(div().flex().gap(px(6.)).children(colors)),
            );
            if merging.as_deref() == Some(id.as_str()) {
                let targets: Vec<AnyElement> = self
                    .labels
                    .iter()
                    .enumerate()
                    .filter(|(_, other)| other.id != id)
                    .map(|(to, other)| {
                        let to_id = other.id.clone();
                        button(
                            format!("settings-label-{index}-merge-into-{to}"),
                            &other.name,
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(page) = &mut this.settings {
                                page.merge_into = Some(to_id.clone());
                            }
                            cx.notify();
                        }))
                        .into_any_element()
                    })
                    .collect();
                row = row.child(
                    div()
                        .px(px(10.))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(crate::theme::content(0.6))
                                .child("Merge into…"),
                        )
                        .children(targets),
                );
            }
            let target_label = merge_into
                .as_deref()
                .filter(|_| merging.as_deref() == Some(id.as_str()))
                .and_then(|to| self.labels.iter().find(|other| other.id == to));
            if let Some(into) = target_label {
                let (from_id, to_id) = (id.clone(), into.id.clone());
                row =
                    row.child(
                        div().px(px(10.)).child(
                            button(
                                format!("settings-label-{index}-merge-confirm"),
                                &format!("Merge \"{}\" into \"{}\"", label.name, into.name),
                                false,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.merge_label(&from_id, &to_id, cx),
                            )),
                        ),
                    );
            }
            if deleting.as_deref() == Some(id.as_str()) {
                let confirm_id = id.clone();
                row =
                    row.child(
                        div().px(px(10.)).child(
                            button(
                                format!("settings-label-{index}-delete-confirm"),
                                &format!("Delete \"{}\" from {used} cards", label.name),
                                false,
                            )
                            .text_color(crate::theme::diff_foreground(false))
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.delete_label_now(&confirm_id, cx),
                            )),
                        ),
                    );
            }
            rows.push(row.into_any_element());
        }
        let adding = self.editing(&Edit::LabelNew);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(heading("LABELS"))
            .children(rows)
            .child(match adding {
                Some(input) => div()
                    .id("settings-edit")
                    .debug_selector(|| "settings-edit".into())
                    .h(px(26.))
                    .px_2()
                    .rounded_md()
                    .border_1()
                    .border_color(crate::theme::content(0.14))
                    .flex()
                    .items_center()
                    .child(div().flex_1().min_w_0().child(input))
                    .into_any_element(),
                None => div()
                    .flex()
                    .child(
                        target("settings-label-add".into())
                            .aria_label("New label")
                            .h(px(26.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .rounded_md()
                            .border_1()
                            .border_color(crate::theme::content(0.12))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .child("New label")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_edit(Edit::LabelNew, "", window, cx)
                            })),
                    )
                    .into_any_element(),
            })
            .into_any_element()
    }
}
