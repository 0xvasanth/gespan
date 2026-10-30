use crate::{
    settings::{SettingsEvent, SettingsView, widgets::secondary_button},
    theme::*,
};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};
impl SettingsView {
    /// Which teams' issues reach the inbox, the Inbox's Teams filter here.
    fn linear_teams(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
        let mut page = vec![self.project_notification_settings(cx)];
        for (id, title, description, label, hint) in [
            (
                "github",
                "GitHub",
                "Pull requests, reviews, and issues, read through the GitHub CLI.",
                "Connection",
                "Install GitHub CLI from cli.github.com, run gh auth login in a terminal, then check again.",
            ),
            (
                "gitlab",
                "GitLab",
                "Merge requests from GitLab.com or a self-managed instance.",
                "Connection",
                "Connect GitLab.com or a self-managed GitLab instance. Use a personal access token with API access; the token is stored locally and Disconnect deletes it.",
            ),
            (
                "azuredevops",
                "ADO",
                "Pull requests and Boards work items from your ADO organization.",
                "Connection",
                "Connect your ADO organization with a personal access token (Boards + Repos read & write for comments). The token is stored locally and Disconnect deletes it.",
            ),
            (
                "jira",
                "Jira",
                "Jira Cloud issues from the projects you pick.",
                "Connection",
                "Connect your Jira Cloud site with your email and an API token.",
            ),
            (
                "plane",
                "Plane",
                "Link sessions to Plane work items by key or link. Plane lists no issues in the inbox.",
                "Connection",
                "Connect Plane Cloud or a self-hosted instance with a workspace slug and a personal API key. Disconnect deletes the key.",
            ),
            (
                "linear",
                "Linear",
                "Issues assigned to you, from the teams you pick.",
                "API key",
                "Create a personal API key in Linear → Settings → Security & Access. Disconnect deletes it.",
            ),
        ] {
            let (connected, connection_label) = self
                .ctx
                .inbox_connections
                .get(id)
                .cloned()
                .unwrap_or_else(|| (false, "Checking connection…".into()));
            let status = div()
                .text_size(px(12.))
                .text_color(content(if connected { 0.7 } else { 0.5 }))
                .child(connection_label)
                .into_any_element();
            let controls = if id == "github" {
                vec![
                    status,
                    secondary_button(
                        "github-installation-guide",
                        "Installation guide",
                        false,
                        false,
                    )
                    .child("Installation guide")
                    .on_click(|_, _, cx| cx.open_url("https://cli.github.com/"))
                    .into_any_element(),
                    secondary_button("github-check-again", "Check again", false, false)
                        .child("Check again")
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(SettingsEvent::ManageInboxProvider("github".into()))
                        }))
                        .into_any_element(),
                ]
            } else {
                vec![
                    status,
                    secondary_button(
                        SharedString::from(format!("connect-{id}")),
                        if connected { "Manage" } else { "Connect" },
                        false,
                        false,
                    )
                    .debug_selector(move || format!("settings-connect-{id}"))
                    .child(if connected { "Manage" } else { "Connect" })
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(SettingsEvent::ManageInboxProvider(id.into()))
                    }))
                    .into_any_element(),
                ]
            };
            let mut rows = vec![self.row(None, false, label, Some(hint.into()), controls)];
            if id == "linear" && connected && !self.ctx.linear_teams.is_empty() {
                rows.push(self.linear_teams(cx));
            }
            page.push(
                self.group(
                    Some(id),
                    false,
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon(
                            match id {
                                "linear" => "inbox-linear",
                                "jira" => "inbox-jira",
                                "plane" => "inbox-plane",
                                "gitlab" => "inbox-gitlab",
                                "azuredevops" => "inbox-azuredevops",
                                _ => "inbox-github",
                            },
                            16.,
                        ))
                        .child(title),
                    Some(description.into()),
                    None,
                    rows,
                ),
            );
        }
        page
    }
}
