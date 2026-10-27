//! Fields: create one of each kind, rename, reorder, edit select options and
//! delete. A field's kind never changes.
use super::{Edit, button, card_row, heading};
use crate::board::{BoardView, quick::field_options};
use gespan_core::board::extras::FIELD_KINDS;
use gpui::{AnyElement, Context, div, prelude::*, px};

fn has_options(kind: &str) -> bool {
    matches!(kind, "select" | "multi-select")
}

fn options_json(options: &[String]) -> String {
    serde_json::json!(options).to_string()
}

impl BoardView {
    pub(super) fn commit_field_new(
        &mut self,
        kind: &str,
        name: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(board_id) = self.open_board_id.clone() else {
            return true;
        };
        if name.is_empty() {
            return true;
        }
        self.settings_write(cx, |service| {
            service.field_create(&board_id, name, kind, None).map(drop)
        })
    }

    pub(super) fn commit_field_name(
        &mut self,
        id: &str,
        name: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if name.is_empty() {
            return true;
        }
        self.settings_write(cx, |service| service.field_update(id, Some(name), None))
    }

    /// Rename an option with the dedicated call, which rewrites stored values
    /// (a plain update would clear them on a case-only change).
    pub(super) fn commit_field_option(
        &mut self,
        id: &str,
        from: &str,
        to: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if to.is_empty() || to == from {
            return true;
        }
        self.settings_write(cx, |service| service.field_rename_option(id, from, to))
    }

    /// Add an option, built on the options stored now.
    pub(super) fn commit_field_option_new(
        &mut self,
        id: &str,
        option: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if option.is_empty() {
            return true;
        }
        self.add_option_latest(id, option, cx)
    }

    fn add_option_latest(&mut self, id: &str, option: &str, cx: &mut Context<Self>) -> bool {
        let Some(board_id) = self.open_board_id.clone() else {
            return true;
        };
        self.settings_write(cx, |service| {
            let mut options = service
                .list_fields(&board_id)?
                .into_iter()
                .find(|field| field.id == id)
                .map(|field| field_options(field.options_json.as_deref()))
                .ok_or("Field not found")?;
            options.push(option.to_string());
            service.field_update(id, None, Some(&options_json(&options)))
        })
    }

    fn remove_option(&mut self, id: &str, option: &str, cx: &mut Context<Self>) {
        let Some(board_id) = self.open_board_id.clone() else {
            return;
        };
        self.settings_write(cx, |service| {
            let mut options = service
                .list_fields(&board_id)?
                .into_iter()
                .find(|field| field.id == id)
                .map(|field| field_options(field.options_json.as_deref()))
                .ok_or("Field not found")?;
            options.retain(|existing| existing != option);
            service.field_update(id, None, Some(&options_json(&options)))
        });
    }

    fn move_field_to(&mut self, id: &str, index: usize, cx: &mut Context<Self>) {
        self.settings_write(cx, |service| service.field_move(id, index));
    }

    fn delete_field_now(&mut self, id: &str, cx: &mut Context<Self>) {
        self.settings_write(cx, |service| service.field_delete(id));
    }

    pub(super) fn render_fields(&self, cx: &mut Context<Self>) -> AnyElement {
        let deleting = self
            .settings
            .as_ref()
            .and_then(|page| page.deleting_field.clone());
        let last = self.fields.len().saturating_sub(1);
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, field) in self.fields.iter().enumerate() {
            let id = field.id.clone();
            let (up_id, down_id, delete_id) = (id.clone(), id.clone(), id.clone());
            let mut card = card_row().flex_col().items_stretch().child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(self.editable_text(
                        format!("settings-field-{index}-name"),
                        Edit::FieldName(id.clone()),
                        field.name.clone(),
                        cx,
                    ))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(crate::theme::content(0.45))
                            .child(field.kind.clone()),
                    )
                    .child(
                        button(format!("settings-field-{index}-up"), "Up", false)
                            .opacity(if index == 0 { 0.4 } else { 1. })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if index > 0 {
                                    this.move_field_to(&up_id, index - 1, cx)
                                }
                            })),
                    )
                    .child(
                        button(format!("settings-field-{index}-down"), "Down", false)
                            .opacity(if index == last { 0.4 } else { 1. })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if index < last {
                                    this.move_field_to(&down_id, index + 1, cx)
                                }
                            })),
                    )
                    .child(
                        button(format!("settings-field-{index}-delete"), "Delete", false).on_click(
                            cx.listener(move |this, _, _, cx| {
                                if let Some(page) = &mut this.settings {
                                    page.clear_prompts();
                                    page.deleting_field = Some(delete_id.clone());
                                }
                                cx.notify();
                            }),
                        ),
                    ),
            );
            if has_options(&field.kind) {
                let options = field_options(field.options_json.as_deref());
                for (n, option) in options.into_iter().enumerate() {
                    let (rename_id, remove_id, remove_name) =
                        (id.clone(), id.clone(), option.clone());
                    card = card.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .pl(px(12.))
                            .child(self.editable_text(
                                format!("settings-field-{index}-option-{n}"),
                                Edit::FieldOption(rename_id, option.clone()),
                                option,
                                cx,
                            ))
                            .child(
                                button(
                                    format!("settings-field-{index}-option-{n}-remove"),
                                    "Remove",
                                    false,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.remove_option(&remove_id, &remove_name, cx)
                                    },
                                )),
                            ),
                    );
                }
                let add_id = id.clone();
                card = card.child(
                    div().pl(px(12.)).flex().child(
                        match self.editing(&Edit::FieldOptionNew(id.clone())) {
                            Some(input) => div()
                                .id("settings-edit")
                                .debug_selector(|| "settings-edit".into())
                                .w(px(240.))
                                .h(px(26.))
                                .px_2()
                                .rounded_md()
                                .border_1()
                                .border_color(crate::theme::content(0.14))
                                .flex()
                                .items_center()
                                .child(div().flex_1().min_w_0().child(input))
                                .into_any_element(),
                            None => button(
                                format!("settings-field-{index}-option-add"),
                                "Add option",
                                false,
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_edit(
                                    Edit::FieldOptionNew(add_id.clone()),
                                    "",
                                    window,
                                    cx,
                                )
                            }))
                            .into_any_element(),
                        },
                    ),
                );
            }
            if deleting.as_deref() == Some(id.as_str()) {
                let confirm_id = id.clone();
                card =
                    card.child(
                        div().flex().child(
                            button(
                                format!("settings-field-{index}-delete-confirm"),
                                &format!("Delete \"{}\" and its values", field.name),
                                false,
                            )
                            .text_color(crate::theme::diff_foreground(false))
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.delete_field_now(&confirm_id, cx),
                            )),
                        ),
                    );
            }
            rows.push(card.into_any_element());
        }
        let adding_kind = FIELD_KINDS.iter().find_map(|kind| {
            self.editing(&Edit::FieldNew(kind.to_string()))
                .map(|input| (*kind, input))
        });
        let adders: Vec<AnyElement> = FIELD_KINDS
            .iter()
            .map(|&kind| {
                button(format!("settings-field-add-{kind}"), kind, false)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_edit(Edit::FieldNew(kind.to_string()), "", window, cx)
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(heading("FIELDS"))
            .children(rows)
            .child(heading("ADD A FIELD"))
            .child(div().flex().flex_wrap().gap(px(6.)).children(adders))
            .children(adding_kind.map(|(kind, input)| {
                div()
                    .id("settings-edit")
                    .debug_selector(|| "settings-edit".into())
                    .h(px(26.))
                    .px_2()
                    .rounded_md()
                    .border_1()
                    .border_color(crate::theme::content(0.14))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(crate::theme::content(0.45))
                            .child(format!("New {kind} field")),
                    )
                    .child(div().flex_1().min_w_0().child(input))
            }))
            .into_any_element()
    }
}
