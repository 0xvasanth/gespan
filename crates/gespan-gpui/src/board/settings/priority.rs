//! Priority: rename the five levels and hide some from pickers.
use super::{Edit, button, card_row, target};
use crate::board::BoardView;
use gespan_core::board::extras::{PriorityScheme, SettingsPatch};
use gpui::{AnyElement, Context, div, prelude::*, px};

impl BoardView {
    /// The board's name for a priority level.
    pub(crate) fn priority_name(&self, level: u8) -> String {
        self.priority
            .names
            .get(level as usize)
            .cloned()
            .unwrap_or_default()
    }

    fn patch_scheme(&mut self, scheme: &PriorityScheme, cx: &mut Context<Self>) -> bool {
        let json = serde_json::to_string(scheme).ok();
        self.patch_settings(
            SettingsPatch {
                priority_json: Some(json),
                ..Default::default()
            },
            cx,
        )
    }

    pub(super) fn commit_priority_name(
        &mut self,
        level: u8,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if text.is_empty() {
            return true;
        }
        let mut scheme = self.priority.clone();
        let Some(slot) = scheme.names.get_mut(level as usize) else {
            return true;
        };
        *slot = text.to_string();
        self.patch_scheme(&scheme, cx)
    }

    fn toggle_priority_hidden(&mut self, level: u8, cx: &mut Context<Self>) {
        let mut scheme = self.priority.clone();
        if scheme.hidden.contains(&level) {
            scheme.hidden.retain(|hidden| *hidden != level);
        } else {
            scheme.hidden.push(level);
            scheme.hidden.sort_unstable();
        }
        self.patch_scheme(&scheme, cx);
    }

    pub(super) fn render_priority(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows: Vec<AnyElement> = (0..self.priority.names.len() as u8)
            .rev()
            .map(|level| {
                let name = self.priority.names[level as usize].clone();
                let hidden = self.priority.hidden.contains(&level);
                card_row()
                    .child(self.editable_text(
                        format!("settings-priority-{level}-name"),
                        Edit::Priority(level),
                        name,
                        cx,
                    ))
                    .when(level > 0, |d| {
                        d.child(
                            button(
                                format!("settings-priority-{level}-hide"),
                                if hidden { "Hidden" } else { "Shown" },
                                hidden,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.toggle_priority_hidden(level, cx),
                            )),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(rows)
            .child(
                div().flex().child(
                    target("settings-priority-reset".into())
                        .aria_label("Reset to defaults")
                        .h(px(24.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .text_size(px(12.))
                        .cursor_pointer()
                        .child("Reset to defaults")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.patch_settings(
                                SettingsPatch {
                                    priority_json: Some(None),
                                    ..Default::default()
                                },
                                cx,
                            );
                        })),
                ),
            )
            .into_any_element()
    }
}
