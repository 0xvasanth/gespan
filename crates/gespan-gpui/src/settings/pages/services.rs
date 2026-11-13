//! Settings › Connections: the services you have connected, an Add
//! connection picker over the whole catalogue, and a first-run empty state.
//! Connecting and managing run through the Inbox's own connect dialog.
use crate::glass::{self, Button, Tone};
use crate::settings::{Section, SettingsEvent, SettingsView};
use crate::theme::*;
use gpui::{
    AnyElement, Context, Div, Focusable, FontWeight, Role, SharedString, div, prelude::*, px, rgb,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Service {
    pub id: &'static str,
    pub name: &'static str,
    pub mark: &'static str,
    pub fill: u32,
    pub ink: u32,
    pub category: &'static str,
    pub blurb: &'static str,
    /// The one line a row or suggestion has room for.
    pub short: &'static str,
}

pub(crate) const CODE_HOSTING: &str = "Code hosting";
pub(crate) const ISSUES: &str = "Issues";
/// The picker's chips, after All.
pub(crate) const CATEGORIES: [&str; 2] = [CODE_HOSTING, ISSUES];

/// Every service Gespan connects to, in the order the catalogue lists them.
pub(crate) const SERVICES: [Service; 6] = [
    Service {
        id: "github",
        name: "GitHub",
        mark: "GH",
        fill: 0xebebeb,
        ink: 0x111111,
        category: CODE_HOSTING,
        blurb: "Pull requests, reviews, and issues, read through the GitHub CLI.",
        short: "Pull requests, reviews and issues",
    },
    Service {
        id: "gitlab",
        name: "GitLab",
        mark: "GL",
        fill: 0xfc6d26,
        ink: 0xffffff,
        category: CODE_HOSTING,
        blurb: "Merge requests from GitLab.com or a self-managed instance.",
        short: "Merge requests and issues",
    },
    Service {
        id: "azuredevops",
        name: "ADO",
        mark: "AZ",
        fill: 0x0078d4,
        ink: 0xffffff,
        category: CODE_HOSTING,
        blurb: "Pull requests and Boards work items from your ADO organization.",
        short: "Pull requests and Boards work items",
    },
    Service {
        id: "linear",
        name: "Linear",
        mark: "LN",
        fill: 0x5e6ad2,
        ink: 0xffffff,
        category: ISSUES,
        blurb: "Issues assigned to you, from the teams you pick.",
        short: "Issues and projects",
    },
    Service {
        id: "jira",
        name: "Jira",
        mark: "JI",
        fill: 0x2684ff,
        ink: 0xffffff,
        category: ISSUES,
        blurb: "Jira Cloud issues from the projects you pick.",
        short: "Issues from the projects you pick",
    },
    Service {
        id: "plane",
        name: "Plane",
        mark: "PL",
        fill: 0x3f76ff,
        ink: 0xffffff,
        category: ISSUES,
        blurb: "Link sessions to Plane work items by key or link. Plane lists no issues in the inbox.",
        short: "Work items by key or link",
    },
];

/// The empty page's suggestions: where code lives first.
const SUGGESTED: [&str; 3] = ["github", "linear", "gitlab"];

/// Services the Inbox reports connected, in catalogue order, narrowed by the
/// page's filter.
pub(crate) fn connected<'a>(
    statuses: &'a std::collections::HashMap<String, (bool, String)>,
    query: &str,
) -> Vec<(Service, &'a str)> {
    SERVICES
        .iter()
        .filter_map(|s| match statuses.get(s.id) {
            Some((true, detail)) => Some((*s, detail.as_str())),
            _ => None,
        })
        .filter(|(s, detail)| matches(s, query) || contains(detail, query))
        .collect()
}

fn contains(text: &str, query: &str) -> bool {
    text.to_lowercase().contains(&query.trim().to_lowercase())
}

/// Whether a service answers a search by its name, what it does or its
/// category.
pub(crate) fn matches(service: &Service, query: &str) -> bool {
    [service.name, service.blurb, service.category, service.id]
        .iter()
        .any(|text| contains(text, query))
}

/// The picker's rows for a category chip (`None` is All) and a search.
pub(crate) fn picker_rows(category: Option<&str>, query: &str) -> Vec<Service> {
    SERVICES
        .iter()
        .filter(|s| category.is_none_or(|c| s.category == c))
        .filter(|s| matches(s, query))
        .copied()
        .collect()
}

fn mark(service: &Service, side: f32) -> Div {
    div()
        .size(px(side))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(9.))
        .bg(rgb(service.fill))
        .text_color(rgb(service.ink))
        .text_size(px(11.5))
        .font_weight(FontWeight::BOLD)
        .child(service.mark)
}

fn tag(label: &'static str) -> Div {
    div()
        .h(px(20.))
        .px(px(8.))
        .flex()
        .flex_none()
        .items_center()
        .rounded_full()
        .bg(content(0.07))
        .border_1()
        .border_color(content(0.1))
        .text_size(px(11.))
        .text_color(content(0.8))
        .whitespace_nowrap()
        .child(label)
}

impl SettingsView {
    /// The header's Add connection button, once something is connected.
    pub(in crate::settings) fn services_action(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        (self.section == Section::Services && self.has_connections())
            .then(|| self.add_connection_button(cx))
    }

    fn has_connections(&self) -> bool {
        self.ctx.inbox_connections.values().any(|(on, _)| *on)
    }

    /// The first run's subtitle says what connecting brings.
    pub(in crate::settings) fn services_description(&self) -> Option<&'static str> {
        (self.section == Section::Services && !self.has_connections())
            .then_some("Connect the services you work in. What they bring in reaches your Inbox, Pilots and automations.")
    }

    fn add_connection_button(&self, cx: &mut Context<Self>) -> AnyElement {
        glass::button("add-connection", "Add connection", Button::Primary)
            .debug_selector(|| "settings-add-connection".into())
            .role(Role::Button)
            .aria_label("Add connection")
            .on_click(cx.listener(|this, _, window, cx| {
                this.services_picker = true;
                this.services_category = None;
                this.services_search
                    .update(cx, |i, cx| i.set_text(String::new(), cx));
                window.focus(&this.services_search.read(cx).focus_handle(cx), cx);
                cx.notify();
            }))
            .into_any_element()
    }

    pub(super) fn services_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if !self.has_connections() {
            return self.services_empty(cx);
        }
        let statuses = self.ctx.inbox_connections.clone();
        let rows: Vec<AnyElement> = connected(&statuses, &self.services_query)
            .into_iter()
            .enumerate()
            .map(|(ix, (service, detail))| self.service_row(ix == 0, service, detail, cx))
            .collect();
        let total = statuses.values().filter(|(on, _)| *on).count();
        let filter = glass::field()
            .w(px(320.))
            .debug_selector(|| "settings-services-filter".into())
            .child(icon("search", 14.).text_color(content(0.5)))
            .child(div().flex_1().min_w_0().child(self.services_filter.clone()));
        let empty = rows.is_empty();
        vec![
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(filter)
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(content(0.55))
                        .child(format!("{total} connected")),
                )
                .into_any_element(),
            glass::card()
                .debug_selector(|| "settings-services-list".into())
                .overflow_hidden()
                .flex()
                .flex_col()
                .children(rows)
                .when(empty, |d| {
                    d.child(
                        div()
                            .p(px(16.))
                            .text_size(px(12.))
                            .text_color(content(0.62))
                            .child("No connection matches that filter."),
                    )
                })
                .into_any_element(),
        ]
    }

    fn service_row(
        &mut self,
        first: bool,
        service: Service,
        detail: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = service.id;
        let flash = self.flashing(Some(id));
        let teams =
            (id == "linear" && !self.ctx.linear_teams.is_empty()).then(|| self.linear_teams(cx));
        div()
            .debug_selector(move || format!("settings-service-{id}"))
            .relative()
            .when(!first, |d| d.border_t_1().border_color(glass::hairline()))
            .when(flash, |d| d.bg(accent_color().opacity(0.1)))
            .child(self.reveal_probe(id))
            .child(
                div()
                    .px(px(16.))
                    .py(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(mark(&service, 34.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(service.name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(content(0.55))
                                    .child(detail.to_owned()),
                            ),
                    )
                    .child(tag(service.category))
                    .child(glass::status("Connected", Tone::Ok))
                    .child(
                        glass::button(
                            SharedString::from(format!("connect-{id}")),
                            "Manage",
                            Button::Glass,
                        )
                        .debug_selector(move || format!("settings-connect-{id}"))
                        .role(Role::Button)
                        .aria_label(format!("Manage {}", service.name))
                        .h(px(28.))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(SettingsEvent::ManageInboxProvider(id.into()))
                        })),
                    ),
            )
            .children(teams.map(|teams| div().pl(px(46.)).child(teams)))
            .into_any_element()
    }

    fn services_empty(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        // In the suggested order, not the catalogue's.
        let suggestions = SUGGESTED
            .iter()
            .filter_map(|id| SERVICES.iter().find(|s| s.id == *id))
            .map(|s| {
                let id = s.id;
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .p(px(14.))
                    .rounded(px(14.))
                    .border_1()
                    .border_color(glass::edge())
                    .bg(glass::fill())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(11.))
                            .child(mark(s, 34.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(s.name),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(11.5))
                                            .text_color(content(0.55))
                                            .child(s.short),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(tag(s.category))
                            .child(div().flex_1())
                            .child(
                                glass::button(
                                    SharedString::from(format!("suggest-{id}")),
                                    "Connect",
                                    Button::Glass,
                                )
                                .debug_selector(move || format!("settings-suggest-{id}"))
                                .role(Role::Button)
                                .aria_label(format!("Connect {}", s.name))
                                .h(px(28.))
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        cx.emit(SettingsEvent::ManageInboxProvider(id.into()))
                                    },
                                )),
                            ),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        vec![
            glass::card()
                .debug_selector(|| "settings-services-empty".into())
                .p(px(44.))
                .child(
                    glass::empty_state(
                        "link-01",
                        "Nothing connected yet.",
                        "Most people start with where their code lives.",
                    )
                    .child(div().mt(px(6.)).child(self.add_connection_button(cx)))
                    // The Inbox hasn't reported yet: something may be
                    // connected after all.
                    .when(self.ctx.inbox_connections.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(content(0.55))
                                .child("Checking connections…"),
                        )
                    }),
                )
                .into_any_element(),
            glass::eyebrow("Suggested").px(px(4.)).into_any_element(),
            div()
                .grid()
                .grid_cols(3)
                .gap(px(10.))
                .children(suggestions)
                .into_any_element(),
        ]
    }

    /// The Add connection picker: search, category chips and every service.
    pub(in crate::settings) fn services_picker_overlay(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.services_picker || self.section != Section::Services {
            return None;
        }
        let category = self.services_category;
        let rows = picker_rows(category, &self.services_search_query);
        let empty = rows.is_empty();
        let statuses = self.ctx.inbox_connections.clone();
        let chip =
            |id: &'static str, label: &'static str, on: bool, value: Option<&'static str>| {
                glass::chip(SharedString::from(format!("services-chip-{id}")), label, on)
                    .debug_selector(move || format!("settings-services-chip-{id}"))
                    .role(Role::RadioButton)
                    .aria_label(label)
                    .aria_selected(on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.services_category = value;
                        cx.notify();
                    }))
            };
        let chips = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .child(chip("all", "All", category.is_none(), None))
            .children(CATEGORIES.map(|c| {
                let id = if c == CODE_HOSTING { "code" } else { "issues" };
                chip(id, c, category == Some(c), Some(c))
            }));
        let list = rows.into_iter().enumerate().map(|(ix, s)| {
            let id = s.id;
            let on = statuses.get(id).is_some_and(|(on, _)| *on);
            div()
                .debug_selector(move || format!("settings-picker-{id}"))
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(14.))
                .py(px(10.))
                .when(ix > 0, |d| d.border_t_1().border_color(glass::hairline()))
                .hover(|s| s.bg(glass::hover_fill()))
                .child(mark(&s, 30.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(px(13.)).child(s.name))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(content(0.55))
                                .child(s.short),
                        ),
                )
                .child(tag(s.category))
                .when(id == "github" && !on, |d| {
                    d.child(
                        glass::button(
                            "github-installation-guide",
                            "Installation guide",
                            Button::Ghost,
                        )
                        .role(Role::Button)
                        .aria_label("Installation guide")
                        .h(px(28.))
                        .on_click(|_, _, cx| cx.open_url("https://cli.github.com/")),
                    )
                })
                .child(
                    glass::button(
                        SharedString::from(format!("picker-connect-{id}")),
                        if on { "Manage" } else { "Connect" },
                        Button::Glass,
                    )
                    .debug_selector(move || format!("settings-picker-connect-{id}"))
                    .role(Role::Button)
                    .aria_label(format!(
                        "{} {}",
                        if on { "Manage" } else { "Connect" },
                        s.name
                    ))
                    .h(px(28.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.services_picker = false;
                        cx.emit(SettingsEvent::ManageInboxProvider(id.into()));
                        cx.notify();
                    })),
                )
        });
        let dialog = glass::card_strong()
            .id("services-picker")
            .debug_selector(|| "settings-services-picker".into())
            .role(Role::Dialog)
            .aria_label("Add a connection")
            .w(px(720.))
            .max_w(gpui::relative(0.92))
            .max_h(px(640.))
            .bg(floating_fill())
            .overflow_hidden()
            .flex()
            .flex_col()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .p(px(18.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Add a connection"),
                            )
                            .child(
                                div()
                                    .ml(px(10.))
                                    .text_size(px(11.5))
                                    .text_color(content(0.55))
                                    .child(format!("{} available", SERVICES.len())),
                            )
                            .child(div().flex_1())
                            .child(
                                glass::icon_button("services-picker-close", "x")
                                    .debug_selector(|| "settings-services-picker-close".into())
                                    .role(Role::Button)
                                    .aria_label("Close")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.services_picker = false;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        glass::field()
                            .h(px(38.))
                            .child(icon("search", 14.).text_color(content(0.5)))
                            .child(div().flex_1().min_w_0().child(self.services_search.clone())),
                    )
                    .child(chips),
            )
            .child(
                div()
                    .id("services-picker-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .border_t_1()
                    .border_color(glass::hairline())
                    .children(list)
                    .when(empty, |d| {
                        d.child(
                            div()
                                .p(px(18.))
                                .text_size(px(12.))
                                .text_color(content(0.62))
                                .child("Nothing matches. Try another word or category."),
                        )
                    }),
            )
            .child(
                div()
                    .px(px(18.))
                    .py(px(12.))
                    .border_t_1()
                    .border_color(glass::hairline())
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.5))
                    .text_color(content(0.55))
                    .child("Can't find it? Connect any MCP server in Settings › MCP.")
                    .child(
                        glass::button("services-open-mcp", "Open MCP", Button::Ghost)
                            .role(Role::Button)
                            .aria_label("Open MCP")
                            .h(px(26.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.services_picker = false;
                                this.select_section(Section::Mcp, cx);
                                cx.emit(SettingsEvent::Section(Section::Mcp));
                            })),
                    ),
            );
        Some(
            div()
                .id("services-picker-scrim")
                .absolute()
                .inset_0()
                .bg(black(0.45))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.services_picker = false;
                        cx.notify();
                    }),
                )
                .child(dialog)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn categories_follow_what_each_service_brings() {
        for id in ["github", "gitlab", "azuredevops"] {
            assert_eq!(
                SERVICES.iter().find(|s| s.id == id).unwrap().category,
                CODE_HOSTING
            );
        }
        for id in ["linear", "jira", "plane"] {
            assert_eq!(
                SERVICES.iter().find(|s| s.id == id).unwrap().category,
                ISSUES
            );
        }
        // Everything the Inbox can connect is in the catalogue.
        for id in gespan_core::inbox::PROVIDERS {
            assert!(SERVICES.iter().any(|s| s.id == id), "{id}");
        }
    }

    #[test]
    fn only_connected_services_are_listed_and_the_filter_narrows_them() {
        let statuses: HashMap<String, (bool, String)> = [
            ("github", (true, "@octo · GitHub CLI")),
            ("linear", (true, "acme · API key")),
            ("jira", (false, "Not connected")),
        ]
        .into_iter()
        .map(|(k, (on, d))| (k.to_string(), (on, d.to_string())))
        .collect();
        let ids = |q| {
            connected(&statuses, q)
                .iter()
                .map(|(s, _)| s.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(""), ["github", "linear"]);
        assert_eq!(ids("code hosting"), ["github"], "by category");
        assert_eq!(ids("octo"), ["github"], "by account detail");
        assert!(
            ids("jira").is_empty(),
            "a service that isn't connected never lists"
        );
    }

    #[test]
    fn the_picker_filters_by_chip_and_search() {
        let ids = |c, q| picker_rows(c, q).iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(ids(None, "").len(), SERVICES.len());
        assert_eq!(
            ids(Some(CODE_HOSTING), ""),
            ["github", "gitlab", "azuredevops"]
        );
        assert_eq!(ids(Some(ISSUES), "merge"), Vec::<&str>::new());
        assert_eq!(ids(None, "merge"), ["gitlab"]);
        assert_eq!(ids(None, "  LINEAR "), ["linear"]);
    }
}
