//! Settings → MCP's own catalogue: the servers Gespan runs for every agent.
use crate::{
    settings::{
        SettingsView,
        widgets::{note, secondary_button},
    },
    theme::*,
};
use gespan_core::mcp_catalog::{self, Catalog, Class, Entry};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};
use std::path::Path;

/// A catalogue change, on a background thread: the keychain can block.
type Edit = dyn FnOnce(&mut Catalog, &Path) -> Result<(), String> + Send;

fn class_label(class: Class) -> &'static str {
    match class {
        Class::Shared => "shared",
        Class::PerWorktree => "per-worktree",
        Class::PerSession => "per-session",
    }
}

fn status(entry: &Entry) -> &'static str {
    match (entry.enabled, entry.approved.is_some()) {
        (true, _) => "On",
        (false, true) => "Off",
        (false, false) => "Needs approval",
    }
}

impl SettingsView {
    pub(in crate::settings) fn load_catalog(&mut self, cx: &mut Context<Self>) {
        self.catalog_edit(Box::new(|_, _| Ok(())), cx);
    }

    fn catalog_edit(&mut self, edit: Box<Edit>, cx: &mut Context<Self>) {
        let Some(data_dir) = gespan_core::tool_servers::data_dir() else {
            return;
        };
        let work = cx.background_executor().spawn(async move {
            let mut catalog = Catalog::load(&data_dir)?;
            let before = catalog.clone();
            edit(&mut catalog, &data_dir)?;
            if catalog != before {
                catalog.save(&data_dir)?;
                gespan_core::tool_servers::sync_servers();
            }
            Ok::<_, String>(catalog)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(catalog) => {
                        this.mcp_catalog = catalog;
                        this.mcp_catalog_error = None;
                    }
                    Err(error) => this.mcp_catalog_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn catalog_import(&mut self, cx: &mut Context<Self>) {
        let cwd = self.selected_project.clone();
        self.catalog_edit(
            Box::new(move |catalog, data_dir| {
                let found = gespan_core::mcp::mcp_importable(cwd)?;
                catalog.import(data_dir, found).map(|_| ())
            }),
            cx,
        );
    }

    /// One sign-in here serves every agent, which never sees the token.
    fn catalog_sign_in(&mut self, id: String, url: String, cx: &mut Context<Self>) {
        let (opener, opening) = async_channel::bounded::<String>(1);
        let work = cx.background_executor().spawn(async move {
            gespan_core::tool_servers::sign_in(&id, &url, |address| {
                let _ = opener.send_blocking(address.to_owned());
            })
        });
        cx.spawn(async move |this, cx| {
            if let Ok(address) = opening.recv().await {
                cx.update(|cx| cx.open_url(&address));
            }
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.mcp_catalog_error = result.err();
                this.load_catalog(cx);
            });
        })
        .detach();
    }

    /// Asks with the exact command and environment names on screen, since
    /// that is what's being approved.
    fn catalog_approve(&mut self, entry: Entry, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let environment = if entry.env.is_empty() {
            "No environment variables.".to_owned()
        } else {
            format!("Environment: {}", entry.env.join(", "))
        };
        let detail = format!("{}\n\n{environment}", entry.command_line());
        let prompt = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Let Gespan run {}?", entry.name),
            Some(&detail),
            &["Approve and turn on", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if prompt.await == Ok(0) {
                let _ = this.update(cx, |this, cx| {
                    let id = entry.id.clone();
                    this.catalog_edit(
                        Box::new(move |catalog, data_dir| catalog.approve(data_dir, &id)),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    fn catalog_row(&mut self, ix: usize, entry: Entry, cx: &mut Context<Self>) -> AnyElement {
        let project = self.selected_project.clone();
        let here = !entry.disabled_projects.contains(&project);
        let detail = match &entry.url {
            Some(url) => url.clone(),
            None => entry.command_line(),
        };
        let mut facts = vec![
            status(&entry).to_owned(),
            class_label(entry.class).to_owned(),
        ];
        facts.extend(entry.source.clone());
        if let Some(tools) = &entry.tools {
            facts.push(format!("{} tools", tools.len()));
        }
        let mut row = div()
            .id(SharedString::from(format!("catalog-server-{ix}")))
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .px_4()
            .py(px(14.))
            .border_b_1()
            .border_color(content(0.05))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(entry.name.clone()),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(12.))
                            .text_color(content(0.45))
                            .child(facts.join(" · ")),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(content(0.35))
                            .child(detail),
                    ),
            );
        let id = entry.id.clone();
        let current = class_label(entry.class);
        let classes = [
            ("shared", "Shared"),
            ("per-worktree", "Worktree"),
            ("per-session", "Session"),
        ];
        row = row.child(self.segmented(
            "catalog-class",
            "How many processes this server gets",
            current,
            &classes,
            cx,
            move |this, value, cx| {
                let id = id.clone();
                let class = match value {
                    "shared" => Class::Shared,
                    "per-session" => Class::PerSession,
                    _ => Class::PerWorktree,
                };
                this.catalog_edit(
                    Box::new(move |catalog, _| {
                        if let Some(entry) = catalog.entry_mut(&id) {
                            entry.class = class;
                        }
                        Ok(())
                    }),
                    cx,
                );
            },
        ));
        if entry.enabled {
            let id = entry.id.clone();
            row = row.child(
                secondary_button(
                    SharedString::from(format!("catalog-here-{ix}")),
                    "This project",
                    false,
                    false,
                )
                .child(if here { "On here" } else { "Off here" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    let (id, project) = (id.clone(), project.clone());
                    this.catalog_edit(
                        Box::new(move |catalog, _| {
                            if let Some(entry) = catalog.entry_mut(&id)
                                && !entry.disabled_projects.remove(&project)
                            {
                                entry.disabled_projects.insert(project);
                            }
                            Ok(())
                        }),
                        cx,
                    );
                })),
            );
            let id = entry.id.clone();
            row = row.child(
                secondary_button(
                    SharedString::from(format!("catalog-off-{ix}")),
                    "Turn off",
                    false,
                    false,
                )
                .child("Turn off")
                .on_click(cx.listener(move |this, _, _, cx| {
                    let id = id.clone();
                    this.catalog_edit(
                        Box::new(move |catalog, _| {
                            if let Some(entry) = catalog.entry_mut(&id) {
                                entry.enabled = false;
                            }
                            Ok(())
                        }),
                        cx,
                    );
                })),
            );
        } else {
            let approving = entry.clone();
            row = row.child(
                secondary_button(
                    SharedString::from(format!("catalog-approve-{ix}")),
                    "Review and turn on",
                    false,
                    false,
                )
                .child("Review and turn on")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.catalog_approve(approving.clone(), window, cx);
                })),
            );
        }
        if let Some(url) = entry.url.clone() {
            let id = entry.id.clone();
            row = row.child(
                secondary_button(
                    SharedString::from(format!("catalog-sign-in-{ix}")),
                    "Sign in",
                    false,
                    false,
                )
                .child("Sign in")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.catalog_sign_in(id.clone(), url.clone(), cx);
                })),
            );
        }
        let id = entry.id.clone();
        row.child(
            secondary_button(
                SharedString::from(format!("catalog-remove-{ix}")),
                "Remove",
                false,
                false,
            )
            .child("Remove")
            .on_click(cx.listener(move |this, _, _, cx| {
                let id = id.clone();
                this.catalog_edit(
                    Box::new(move |catalog, data_dir| mcp_catalog::remove(catalog, data_dir, &id)),
                    cx,
                );
            })),
        )
        .into_any_element()
    }

    pub(in crate::settings) fn catalog_section(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if !self.mcp_catalog_loaded {
            self.mcp_catalog_loaded = true;
            self.load_catalog(cx);
        }
        let managed = self.mcp_catalog.managed;
        let mut section = vec![
            div()
                .mt_6()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("Gespan servers"),
                        )
                        .child(
                            div()
                                .mt_1()
                                .text_size(px(12.))
                                .text_color(content(0.55))
                                .child(
                                    "Install once here. Every agent in every worktree shares them.",
                                ),
                        ),
                )
                .child(
                    secondary_button("catalog-import", "Import from agents", false, false)
                        .debug_selector(|| "settings-catalog-import".into())
                        .child(icon("plus", 14.))
                        .child("Import from agents")
                        .on_click(cx.listener(|this, _, _, cx| this.catalog_import(cx))),
                )
                .into_any_element(),
            div()
                .mt_3()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(content(0.55))
                        .child("Agents load only these servers, not their own."),
                )
                .child(self.toggle(
                    "catalog-managed",
                    "Agents load only Gespan's servers",
                    managed,
                    false,
                    cx,
                    |this, on, cx| {
                        this.catalog_edit(
                            Box::new(move |catalog, _| {
                                catalog.managed = on;
                                Ok(())
                            }),
                            cx,
                        );
                    },
                ))
                .into_any_element(),
        ];
        if let Some(error) = &self.mcp_catalog_error {
            section.push(
                div()
                    .id("settings-catalog-error")
                    .role(gpui::Role::Alert)
                    .mt_3()
                    .text_size(px(12.))
                    .text_color(gpui::rgb(0xf87171))
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        let entries = self.mcp_catalog.entries.clone();
        if entries.is_empty() {
            section.push(note(
                "None yet. Import copies your agents' servers here, without changing their files.",
            ));
            return section;
        }
        let rows: Vec<_> = entries
            .into_iter()
            .enumerate()
            .map(|(ix, entry)| self.catalog_row(ix, entry, cx))
            .collect();
        section.push(
            div()
                .mt_3()
                .overflow_hidden()
                .rounded_xl()
                .border_1()
                .border_color(content(0.1))
                .bg(content(0.03))
                .children(rows)
                .into_any_element(),
        );
        section
    }
}
