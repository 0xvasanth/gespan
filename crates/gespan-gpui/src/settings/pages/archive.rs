use crate::{
    settings::{SettingsEvent, SettingsView},
    theme::*,
};
use gpui::{AnyElement, Context, Role, SharedString, div, prelude::*, px};
impl SettingsView {
    pub(super) fn archive_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let switch = self.row(
            Some("show-archived"),
            true,
            "Show archived in the sidebar",
            Some("Keep archived conversations listed alongside the active ones.".into()),
            vec![self.toggle(
                "show-archived",
                "Show archived in the sidebar",
                self.prefs.show_archived,
                false,
                cx,
                |this, on, cx| this.update(cx, |p| p.show_archived = on),
            )],
        );
        let mut rows: Vec<AnyElement> = Vec::new();
        if self.ctx.cwd.is_empty() {
            rows.push(
                crate::settings::widgets::empty(
                    "settings-archived-no-project",
                    "folder-open",
                    "No project open",
                    "Open a project to see its archived conversations.",
                )
                .into_any_element(),
            );
        } else if self.ctx.archived.is_empty() {
            rows.push(
                crate::settings::widgets::empty(
                    "settings-archived-empty",
                    "archive",
                    "Nothing archived",
                    "Conversations you archive in this project show here.",
                )
                .into_any_element(),
            );
        }
        for session in &self.ctx.archived {
            let id = session.id;
            let date = chrono::DateTime::from_timestamp_millis(
                session.updated_at.min(i64::MAX as u64) as i64,
            )
            .filter(|_| session.updated_at > 0)
            .map(|d| d.format("%b %-d").to_string())
            .unwrap_or_default();
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(!rows.is_empty(), |d| {
                        d.border_t_1().border_color(crate::glass::hairline())
                    })
                    .debug_selector(move || format!("settings-archived-row-{id}"))
                    .h(px(43.))
                    .px_4()
                    .child(
                        div()
                            .size(px(18.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .bg(content(0.08))
                            .child(provider_icon(&session.provider, 12.)),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("archived-session-{id}")))
                            .role(Role::Button)
                            .aria_label(session.title.clone())
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(13.))
                            .cursor_pointer()
                            .child(session.title.clone())
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(SettingsEvent::OpenSession(id))
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(content(0.35))
                            .child(date),
                    )
                    .child(
                        crate::glass::button(
                            SharedString::from(format!("unarchive-{id}")),
                            "Unarchive",
                            crate::glass::Button::Glass,
                        )
                        .role(Role::Button)
                        .aria_label("Unarchive")
                        .h(px(28.))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(SettingsEvent::Archive(id, false))
                        })),
                    )
                    .child(
                        delete_button(SharedString::from(format!("delete-{id}"))).on_click(
                            cx.listener(move |_, _, _, cx| cx.emit(SettingsEvent::Delete(id))),
                        ),
                    )
                    .into_any_element(),
            );
        }
        let title = if self.ctx.cwd.is_empty() {
            "Archived conversations".into()
        } else {
            format!("Archived in {}", self.ctx.project_label)
        };
        let mut projects: Vec<AnyElement> = self
            .ctx
            .archived_projects
            .iter()
            .enumerate()
            .map(|(ix, (path, label))| {
                let (unarchive, delete) = (path.clone(), path.clone());
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(ix > 0, |d| {
                        d.border_t_1().border_color(crate::glass::hairline())
                    })
                    .px_4()
                    .py(px(12.))
                    .child(
                        div()
                            .size(px(32.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(9.))
                            .bg(content(0.07))
                            .border_1()
                            .border_color(crate::glass::edge())
                            .child(icon("folder", 16.).text_color(content(0.75))),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().truncate().text_size(px(13.)).child(label.clone()))
                            .child(
                                div()
                                    .truncate()
                                    .font_family(crate::fonts::monospace())
                                    .text_size(px(11.))
                                    .text_color(content(0.55))
                                    .child(path.clone()),
                            ),
                    )
                    .child(
                        crate::glass::button(
                            SharedString::from(format!("unarchive-project-{ix}")),
                            "Unarchive",
                            crate::glass::Button::Glass,
                        )
                        .role(Role::Button)
                        .aria_label("Unarchive")
                        .h(px(28.))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(SettingsEvent::UnarchiveProject(unarchive.clone()))
                        })),
                    )
                    .child(
                        delete_button(SharedString::from(format!("delete-project-{ix}"))).on_click(
                            cx.listener(move |_, _, _, cx| {
                                cx.emit(SettingsEvent::DeleteProject(delete.clone()))
                            }),
                        ),
                    )
                    .into_any_element()
            })
            .collect();
        if projects.is_empty() {
            projects.push(
                crate::settings::widgets::empty(
                    "settings-archived-projects-empty",
                    "folder-01",
                    "No archived projects",
                    "Archived projects keep their chats here, out of the sidebar.",
                )
                .into_any_element(),
            );
        }
        vec![
            self.group(
                None,
                true,
                "Archived projects",
                Some("Archive a project from its menu in the sidebar to keep its chats without listing it in the sidebar.".into()),
                None,
                projects,
            ),
            // The sidebar switch has its own card; the conversations follow
            // in theirs.
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(self.glass_group(
                None,
                SharedString::from(title),
                None,
                (!self.ctx.archived.is_empty()).then(|| {
                    let n = self.ctx.archived.len();
                    div()
                        .debug_selector(|| "settings-archived-count".into())
                        .text_size(px(11.5))
                        .text_color(content(0.55))
                        .child(format!(
                            "{n} {}",
                            if n == 1 { "conversation" } else { "conversations" }
                        ))
                        .into_any_element()
                }),
                None,
                vec![switch],
            ))
                .when(!rows.is_empty(), |d| {
                    d.child(crate::glass::card().overflow_hidden().children(rows))
                })
                .into_any_element(),
        ]
    }
}

/// Delete, as red text: it cannot be undone, but it is not the main answer.
fn delete_button(id: SharedString) -> gpui::Stateful<gpui::Div> {
    use crate::settings::widgets::red;
    let selector = format!("settings-{id}");
    div()
        .id(id)
        .debug_selector(move || selector.clone())
        .role(Role::Button)
        .aria_label("Delete")
        .cursor_pointer()
        .h(px(28.))
        .px(px(12.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(9.))
        .text_size(px(13.))
        .whitespace_nowrap()
        .text_color(red(1.))
        .hover(|s| s.bg(red(0.1)))
        .child("Delete")
}
