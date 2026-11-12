use crate::{
    settings::{
        SettingsView,
        widgets::{glass_secondary, note},
    },
    theme::*,
};
use gespan_core::provider::Kind;
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};
impl SettingsView {
    pub(super) fn providers_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.prefs.clone();
        let center = crate::accounts::get(cx);
        if let Some(center) = &center {
            center.update(cx, |center, cx| {
                center.mask_emails = p.mask_emails;
                center.show_remaining = p.show_remaining_usage;
                for kind in gespan_core::accounts::providers() {
                    center.load_provider(kind, false, cx);
                }
            });
        }
        let account_kinds: Vec<Kind> = gespan_core::accounts::providers().collect();
        let usage = vec![
            self.row(
                Some("show-remaining-usage"),
                true,
                "Show remaining usage",
                Some("Show remaining capacity in usage meters.".into()),
                vec![self.toggle(
                    "show-remaining-usage",
                    "Show remaining usage",
                    p.show_remaining_usage,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.show_remaining_usage = on),
                )],
            ),
            self.row(
                Some("mask-emails"),
                false,
                "Mask account emails",
                Some("Hide email addresses until clicked.".into()),
                vec![self.toggle(
                    "mask-emails",
                    "Mask account emails",
                    p.mask_emails,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.mask_emails = on),
                )],
            ),
        ];
        let mut options = vec![(
            "global".into(),
            SharedString::from("Global"),
            Some(icon("globe", 14.).into_any_element()),
        )];
        options.extend(self.ctx.projects.iter().map(|(path, label)| {
            (
                path.clone(),
                SharedString::from(label.clone()),
                Some(icon("folder", 14.).into_any_element()),
            )
        }));
        let scope = self.provider_scope.clone();
        let action = self.select(
            "provider-scope",
            "Provider defaults scope",
            &scope,
            options,
            None,
            cx,
            |this, value, cx| {
                this.provider_scope = value;
                cx.notify();
            },
        );
        let binaries = self.discovery.binaries.clone();
        let mut clis = Vec::new();
        let mut missing: Vec<AnyElement> = Vec::new();
        for (index, kind) in Kind::ALL.into_iter().enumerate() {
            let installed = binaries
                .iter()
                .find(|(k, _)| *k == kind)
                .and_then(|(_, p)| p.as_ref());
            let project = if scope == "global" {
                None
            } else {
                p.project_providers.get(&scope)
            };
            // The global default is the last used provider (desktop lastModel).
            let selected = project
                .and_then(|p| p.provider.as_deref())
                .map(Kind::parse)
                .unwrap_or_else(|| p.last_used_provider())
                == Some(kind);
            let globally_hidden = p.hidden_providers.iter().any(|id| id == kind.id());
            let locked = scope != "global" && globally_hidden;
            let visible = !globally_hidden
                && !project.is_some_and(|p| p.hidden.iter().any(|id| id == kind.id()));
            let models = self
                .ctx
                .catalogs
                .get(kind.id())
                .cloned()
                .unwrap_or_default();
            let selected_model = project
                .and_then(|p| p.models.get(kind.id()))
                .or(p.default_models.get(kind.id()))
                .cloned()
                .or_else(|| models.first().map(|m| m.0.clone()));
            let details = {
                let slot = self
                    .binary_anchors
                    .borrow_mut()
                    .entry(kind)
                    .or_default()
                    .clone();
                crate::glass::icon_button(
                    SharedString::from(format!("binary-{}", kind.id())),
                    "folder-open",
                )
                .relative()
                .role(gpui::Role::Button)
                .aria_label(format!("Show {} CLI details", kind.label()))
                .child(slot.probe())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_management(super::super::management::Target::Cli(kind), window, cx)
                }))
            };
            let mut controls: Vec<AnyElement> = Vec::new();
            let default_button = installed.is_some().then(|| {
                glass_secondary(
                    SharedString::from(format!("default-provider-{}", kind.id())),
                    if selected {
                        "Default"
                    } else {
                        "Use by default"
                    },
                    false,
                    selected || selected_model.is_none() || installed.is_none(),
                )
                .child(if selected {
                    "Default"
                } else {
                    "Use by default"
                })
                .when(
                    !selected && selected_model.is_some() && installed.is_some(),
                    |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            let scope = this.provider_scope.clone();
                            this.update(cx, |p| {
                                if scope == "global" {
                                    p.use_by_default(kind);
                                } else {
                                    p.project_providers.entry(scope).or_default().provider =
                                        Some(kind.id().into());
                                }
                            })
                        }))
                    },
                )
                .into_any_element()
            });
            if let Some(model) = selected_model.as_ref().filter(|_| installed.is_some()) {
                controls.push(
                    div()
                        .text_size(px(12.))
                        .text_color(content(0.62))
                        .child("Model")
                        .into_any_element(),
                );
                controls.push(
                    self.select(
                        kind.id(),
                        kind.label(),
                        model,
                        models
                            .iter()
                            .map(|(id, name)| (id.clone(), SharedString::from(name.clone()), None))
                            .collect(),
                        None,
                        cx,
                        move |this, model, cx| {
                            let scope = this.provider_scope.clone();
                            this.update(cx, |p| {
                                if scope == "global" {
                                    p.default_models.insert(kind.id().into(), model);
                                } else {
                                    p.project_providers
                                        .entry(scope)
                                        .or_default()
                                        .models
                                        .insert(kind.id().into(), model);
                                }
                            });
                        },
                    ),
                );
            }
            // Model on the left; the picker switch and Default on the right.
            controls.push(div().flex_1().into_any_element());
            if installed.is_some() {
                controls.push(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(content(0.5))
                                .child(if locked {
                                    "Hidden globally"
                                } else {
                                    "Show in picker"
                                }),
                        )
                        .child(self.toggle(
                            kind.id(),
                            "Show provider in the model picker",
                            visible,
                            locked,
                            cx,
                            move |this, on, cx| {
                                let scope = this.provider_scope.clone();
                                this.update(cx, |p| {
                                    let hidden = if scope == "global" {
                                        &mut p.hidden_providers
                                    } else {
                                        &mut p.project_providers.entry(scope).or_default().hidden
                                    };
                                    hidden.retain(|id| id != kind.id());
                                    if !on {
                                        hidden.push(kind.id().into());
                                    }
                                })
                            },
                        ))
                        .into_any_element(),
                );
            }
            controls.extend(default_button);
            if installed.is_none() {
                controls.clear();
            }
            let description = installed
                .map(|_| {
                    format!(
                        "{} {} available.",
                        models.len(),
                        if models.len() == 1 { "model" } else { "models" }
                    )
                })
                .unwrap_or_else(|| kind.unavailable_hint());
            let (accounts, add) = match &center {
                Some(center) if account_kinds.contains(&kind) => {
                    center.update(cx, |center, cx| center.settings_accounts(kind, cx))
                }
                _ => (Vec::new(), None),
            };
            // Once the check is done, a CLI that isn't there joins the
            // compact Not installed grid instead of a full card.
            if installed.is_none() && !self.discovery_loading {
                missing.push(missing_card(kind, accounts));
                continue;
            }
            clis.push(self.agent_card(
                index,
                kind,
                installed.is_some(),
                description,
                details.into_any_element(),
                controls,
                accounts,
                add,
            ));
        }
        if let Some(error) = center
            .as_ref()
            .and_then(|center| center.read(cx).settings_error())
        {
            clis.push(error);
        }
        let advanced=vec![
            self.row(Some("idle-agents"),true,"Keep idle agents ready",Some("Keep up to this many idle agents running for faster follow-ups. Uses additional memory.".into()),vec![self.segmented("idle-agents","Keep idle agents ready",&p.idle_agents.to_string(),&[("0","None"),("1","1"),("2","2"),("3","3"),("5","5")],cx,|this,value,cx|if let Ok(value)=value.parse(){this.update(cx,|p|p.idle_agents=value)})]),
            self.row(Some("claude-hooks"),false,"Claude Code hooks",Some("Run your configured Claude Code hooks. Changes apply on the next turn.".into()),vec![self.toggle("claude-hooks","Claude Code hooks",p.claude_hooks,false,cx,|this,on,cx|this.update(cx,|p|p.claude_hooks=on))])];
        let mut page = vec![self.glass_group(
            Some("agent-clis"),
            "Agent CLIs",
            Some("Choose available agents and default models for new conversations. Manage sign-ins and choose accounts for each project.".into()),
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(action)
                    .child(self.refresh_button("refresh-provider-accounts", "Refresh", cx))
                    .into_any_element(),
            ),
            self.discovery_loading.then(|| note("Checking installed CLIs…")),
            Vec::new(),
        )];
        page.extend(clis);
        if !missing.is_empty() {
            let count = missing.len();
            page.push(
                div()
                    .debug_selector(|| "settings-agents-missing".into())
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .px(px(4.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Not installed"),
                            )
                            .child(crate::glass::count(count, false))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(content(0.62))
                                    .child("Install a CLI, then Refresh. Restart Gespan if it is already installed."),
                            ),
                    )
                    .child(div().grid().grid_cols(2).gap(px(8.)).children(missing))
                    .into_any_element(),
            );
        }
        page.push(self.group(None, false, "Usage and privacy", None, None, usage));
        page.push(self.group(None, false, "Advanced", None, None, advanced));
        page
    }

    /// One agent: its tile, whether it is installed, the model and picker
    /// controls when it is, then its accounts. An agent that isn't installed
    /// stays one line with how to install it.
    #[allow(clippy::too_many_arguments)]
    fn agent_card(
        &self,
        index: usize,
        kind: Kind,
        installed: bool,
        description: String,
        details: AnyElement,
        controls: Vec<AnyElement>,
        accounts: Vec<AnyElement>,
        add: Option<AnyElement>,
    ) -> AnyElement {
        let id = kind.id();
        crate::glass::card()
            .debug_selector(move || format!("settings-agent-{id}"))
            .relative()
            .overflow_hidden()
            .when(index == 0, |d| {
                d.child(self.reveal_probe("provider-accounts"))
            })
            .child(
                div()
                    .px_4()
                    .py(px(14.))
                    .flex()
                    .items_center()
                    .gap_3()
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
                            .child(provider_icon(id, 16.)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(kind.label()),
                                    )
                                    // Until the check finishes nothing is
                                    // known, so no pill claims either way.
                                    .when(!self.discovery_loading, |d| {
                                        d.child(if installed {
                                            crate::glass::status(
                                                "Installed",
                                                crate::glass::Tone::Ok,
                                            )
                                            .debug_selector(move || {
                                                format!("settings-agent-installed-{id}")
                                            })
                                        } else {
                                            crate::glass::status(
                                                "Not installed",
                                                crate::glass::Tone::Muted,
                                            )
                                        })
                                    }),
                            )
                            .child(
                                div()
                                    .mt(px(2.))
                                    .text_size(px(11.5))
                                    .text_color(content(0.55))
                                    .child(description),
                            ),
                    )
                    .child(details),
            )
            .when(!controls.is_empty(), |d| {
                d.child(
                    div()
                        .px_4()
                        .py(px(10.))
                        .border_t_1()
                        .border_color(crate::glass::hairline())
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .children(controls),
                )
            })
            .children(accounts)
            .when_some(add, |d, add| {
                d.child(
                    div()
                        .pl(px(54.))
                        .pr_4()
                        .py(px(8.))
                        .border_t_1()
                        .border_color(crate::glass::hairline())
                        .flex()
                        .child(add),
                )
            })
            .into_any_element()
    }
}

/// The command a CLI's hint names, between backticks, if any.
pub(in crate::settings) fn install_command(hint: &str) -> Option<&str> {
    let start = hint.find('`')? + 1;
    let len = hint[start..].find('`')?;
    Some(&hint[start..start + len]).filter(|c| !c.is_empty())
}

/// A CLI that isn't installed: its mark and name, a Not installed pill,
/// and how to install it, as a command when the hint gives one.
fn missing_card(kind: Kind, accounts: Vec<AnyElement>) -> AnyElement {
    let id = kind.id();
    let hint = kind.unavailable_hint();
    let how = match install_command(&hint) {
        Some(command) => div()
            .debug_selector(move || format!("settings-agent-install-{id}"))
            .h(px(28.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(7.))
            .bg(if is_light() { white(0.7) } else { black(0.3) })
            .border_1()
            .border_color(crate::glass::edge())
            .font_family(crate::fonts::monospace())
            .text_size(px(11.5))
            .child(div().truncate().child(command.to_owned())),
        None => div()
            .text_size(px(11.5))
            .line_height(px(16.))
            .text_color(content(0.62))
            .child(hint.clone()),
    };
    crate::glass::card()
        .debug_selector(move || format!("settings-agent-missing-{id}"))
        .overflow_hidden()
        .child(
            div()
                .p(px(14.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            div()
                                .size(px(26.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(7.))
                                .bg(content(0.07))
                                .border_1()
                                .border_color(crate::glass::edge())
                                .child(provider_icon(id, 14.)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(kind.label()),
                        )
                        .child(crate::glass::status(
                            "Not installed",
                            crate::glass::Tone::Muted,
                        )),
                )
                .child(how),
        )
        .children(accounts)
        .into_any_element()
}
