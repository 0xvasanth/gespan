use crate::{
    settings::{Section, SettingsEvent, SettingsView},
    theme::*,
};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};
impl SettingsView {
    /// Which teams' issues reach the inbox, the Inbox's Teams filter here.
    pub(super) fn linear_teams(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let teams = self.ctx.linear_teams.clone();
        div()
            .debug_selector(|| "settings-linear-teams".into())
            .px_4()
            .py_3()
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Teams"),
            )
            .child(
                div()
                    .mt_1()
                    .text_size(px(12.))
                    .text_color(content(0.45))
                    .child("Unchecked teams stay out of the inbox."),
            )
            .child(div().mt_2().flex().flex_col().gap(px(2.)).children(
                teams.into_iter().enumerate().map(|(ix, (team, shown))| {
                    let id = team.id.clone();
                    div()
                        .id(SharedString::from(format!("settings-linear-team-{ix}")))
                        .debug_selector(move || format!("settings-linear-team-{ix}"))
                        .role(gpui::Role::CheckBox)
                        .aria_label(team.name.clone())
                        .aria_toggled(if shown {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .h(px(28.))
                        .px_2()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(13.))
                        .cursor_pointer()
                        .hover(|s| s.bg(content(0.05)))
                        .child(div().flex_1().min_w_0().truncate().child(team.name))
                        .when(!team.key.is_empty(), |d| {
                            d.child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(content(0.4))
                                    .child(team.key),
                            )
                        })
                        .when(shown, |d| d.child(icon("check", 14.)))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(SettingsEvent::ToggleLinearTeam(id.clone()))
                        }))
                }),
            ))
            .into_any_element()
    }

    pub(super) fn inbox_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        // Services moved to Connections; say where, once, above the projects.
        let pointer = crate::glass::card()
            .debug_selector(|| "settings-inbox-services-moved".into())
            .px(px(14.))
            .py(px(10.))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(icon("link-01", 15.).text_color(content(0.7)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(content(0.62))
                    .child(
                        gpui::StyledText::new(
                            "Services moved to Connections. GitHub, GitLab, Azure DevOps, Jira, Plane and Linear are set up there.",
                        )
                        .with_highlights([(
                            0.."Services moved to Connections.".len(),
                            gpui::HighlightStyle {
                                color: Some(content(1.)),
                                font_weight: Some(gpui::FontWeight::MEDIUM),
                                ..Default::default()
                            },
                        )]),
                    ),
            )
            .child(
                crate::glass::button(
                    "open-connections",
                    "Open Connections",
                    crate::glass::Button::Ghost,
                )
                .debug_selector(|| "settings-open-connections".into())
                .role(gpui::Role::Button)
                .aria_label("Open Connections")
                .h(px(28.))
                .text_color(accent_color())
                .child(icon("chevron-right", 12.).text_color(accent_color()))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.select_section(Section::Services, cx);
                    cx.emit(SettingsEvent::Section(Section::Services));
                })),
            )
            .into_any_element();
        vec![pointer, self.project_notification_settings(cx)]
    }
}
