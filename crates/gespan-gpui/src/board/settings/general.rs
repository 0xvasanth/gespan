//! General: name, icon, colour and default view. The agent switches are in
//! Automation.
use super::{Edit, button, heading, palette_hexes, swatch};
use crate::board::BoardView;
use gespan_core::board::extras::{BOARD_ICONS, SettingsPatch};
use gpui::{AnyElement, Context, div, prelude::*, px};

impl BoardView {
    pub(super) fn render_general(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.board_settings.clone();
        // Unknown settings show nothing as chosen.
        let loaded = self.settings_loaded;
        let name = self
            .detail
            .as_ref()
            .map(|detail| detail.board.name.clone())
            .unwrap_or_default();
        let icons: Vec<AnyElement> = BOARD_ICONS
            .iter()
            .map(|&icon_name| {
                let on = loaded && settings.icon.as_deref() == Some(icon_name);
                let patch_value = (!on).then(|| icon_name.to_string());
                button(format!("settings-icon-{icon_name}"), icon_name, on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.patch_settings(
                            SettingsPatch {
                                icon: Some(patch_value.clone()),
                                ..Default::default()
                            },
                            cx,
                        );
                    }))
                    .into_any_element()
            })
            .collect();
        let colors: Vec<AnyElement> = palette_hexes()
            .map(|hex| {
                let on = loaded && settings.color.as_deref() == Some(hex.as_str());
                let patch_value = (!on).then(|| hex.clone());
                swatch(format!("settings-color-{hex}"), &hex, on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.patch_settings(
                            SettingsPatch {
                                color: Some(patch_value.clone()),
                                ..Default::default()
                            },
                            cx,
                        );
                    }))
                    .into_any_element()
            })
            .collect();
        let mut views: Vec<AnyElement> = vec![
            button(
                "settings-default-view-none".into(),
                "All cards",
                loaded && settings.default_view_id.is_none(),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.patch_settings(
                    SettingsPatch {
                        default_view_id: Some(None),
                        ..Default::default()
                    },
                    cx,
                );
            }))
            .into_any_element(),
        ];
        for (index, view) in self.views.saved.iter().enumerate() {
            let id = view.id.clone();
            let on = loaded && settings.default_view_id.as_deref() == Some(view.id.as_str());
            views.push(
                button(format!("settings-default-view-{index}"), &view.name, on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.patch_settings(
                            SettingsPatch {
                                default_view_id: Some(Some(id.clone())),
                                ..Default::default()
                            },
                            cx,
                        );
                    }))
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("NAME"))
            .child(div().flex().child(self.editable_text(
                "settings-name".into(),
                Edit::BoardName,
                name,
                cx,
            )))
            .child(heading("ICON"))
            .child(div().flex().flex_wrap().gap(px(6.)).children(icons))
            .child(heading("COLOUR"))
            .child(div().flex().flex_wrap().gap(px(8.)).children(colors))
            .child(heading("DEFAULT VIEW"))
            .child(div().flex().flex_wrap().gap(px(6.)).children(views))
            .into_any_element()
    }
}
