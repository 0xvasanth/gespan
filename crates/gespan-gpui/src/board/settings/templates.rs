//! Templates: the built-in ones, the saved ones (delete) and "Save as
//! template" for the open board.
use super::{Edit, button, card_row, heading};
use crate::board::BoardView;
use gespan_core::board::Service;
use gpui::{AnyElement, Context, div, prelude::*, px};

const NAME_MISSING: &str = "Name the template first";
const BUILT_IN_DELETE: &str = "Built-in templates can't be deleted";

impl BoardView {
    /// Template commands don't emit, so the list is read again after every
    /// save or delete (and when the section opens).
    pub(super) fn refresh_templates(&mut self) {
        let result = self.service().and_then(Service::templates);
        if let Some(page) = &mut self.settings {
            match result {
                Ok(templates) => page.templates = templates,
                Err(error) => page.error = Some(error),
            }
        }
    }

    /// Enter in the name field. Blank stays open with a message; a refused
    /// name stays open too. Only an accepted save clears the field.
    pub(super) fn commit_template_name(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if text.is_empty() {
            if let Some(page) = &mut self.settings {
                page.error = Some(NAME_MISSING.into());
            }
            return false;
        }
        // A template copies the board's settings, which could not be read:
        // keep the name and say so rather than fail on the store's error.
        if !self.refuse_unreadable_settings(cx) {
            return false;
        }
        let Some(board_id) = self.open_board_id.clone() else {
            return true;
        };
        let ok = self.settings_write(cx, |service| {
            service.save_as_template(&board_id, text).map(drop)
        });
        self.refresh_templates();
        ok
    }

    fn delete_template(&mut self, id: &str, cx: &mut Context<Self>) {
        self.settings_write(cx, |service| service.delete_template(id));
        self.refresh_templates();
        cx.notify();
    }

    pub(super) fn render_templates(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(page) = &self.settings else {
            return div().into_any_element();
        };
        let mut built_in: Vec<AnyElement> = Vec::new();
        let mut saved: Vec<AnyElement> = Vec::new();
        for (index, template) in page.templates.iter().enumerate() {
            if template.built_in {
                built_in.push(
                    card_row()
                        .id(gpui::SharedString::from(format!(
                            "settings-template-{}",
                            template.id
                        )))
                        .debug_selector({
                            let id = format!("settings-template-{}", template.id);
                            move || id.clone()
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(template.name.clone()),
                        )
                        .into_any_element(),
                );
                continue;
            }
            let id = template.id.clone();
            saved.push(
                card_row()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .child(template.name.clone()),
                    )
                    .child(
                        button(format!("settings-template-delete-{index}"), "Delete", false)
                            .aria_label(format!("Delete template {}", template.name))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.delete_template(&id, cx)),
                            ),
                    )
                    .into_any_element(),
            );
        }
        let save = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().min_w_0().child(self.editable_text_with(
                "settings-template-name".into(),
                Edit::TemplateName,
                "Name this board's layout".into(),
                String::new(),
                cx,
            )))
            .child(
                button("settings-template-save".into(), "Save", false).on_click(cx.listener(
                    |this, _, window, cx| {
                        if this.editing(&Edit::TemplateName).is_some() {
                            this.commit_edit(window, cx);
                        } else if let Some(page) = &mut this.settings {
                            page.error = Some(NAME_MISSING.into());
                            cx.notify();
                        }
                    },
                )),
            );
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("SAVE THIS BOARD AS A TEMPLATE"))
            .child(save)
            .child(heading("BUILT-IN"))
            .child(div().flex().flex_col().gap(px(6.)).children(built_in))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(crate::theme::content(0.45))
                    .child(BUILT_IN_DELETE),
            )
            .child(heading("SAVED"))
            .child(div().flex().flex_col().gap(px(6.)).children(saved))
            .into_any_element()
    }
}
