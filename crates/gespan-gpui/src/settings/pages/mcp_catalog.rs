//! Settings → MCP's own catalogue: the servers Gespan runs for every agent.
use crate::{
    settings::{
        SettingsView,
        widgets::{note, secondary_button},
    },
    theme::*,
};
use gespan_core::mcp_catalog::{self, Catalog, Class, Edited, Entry};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};
use std::path::{Path, PathBuf};

/// A catalogue change, on a background thread: the keychain can block.
type Edit = dyn FnOnce(&mut Catalog, &Path) -> Result<(), String> + Send;

fn class_label(class: Class) -> &'static str {
    match class {
        Class::Shared => "shared",
        Class::PerWorktree => "per-worktree",
        Class::PerSession => "per-session",
    }
}

fn status(entry: &Entry, stale: bool) -> &'static str {
    match (entry.enabled && !stale, entry.approved.is_some() && !stale) {
        (true, _) => "On",
        (false, true) => "Off",
        (false, false) => "Needs approval",
    }
}

impl SettingsView {
    pub(in crate::settings) fn load_catalog(&mut self, cx: &mut Context<Self>) {
        self.catalog_edit(Box::new(|_, _| Ok(())), cx);
    }

    fn catalog_dir(&self) -> Option<PathBuf> {
        #[cfg(test)]
        if self.mcp_catalog_dir.is_some() {
            return self.mcp_catalog_dir.clone();
        }
        gespan_core::tool_servers::data_dir()
    }

    fn catalog_edit(&mut self, edit: Box<Edit>, cx: &mut Context<Self>) {
        self.catalog_run(
            move |data_dir| mcp_catalog::update(data_dir, |catalog| edit(catalog, data_dir)),
            cx,
        );
    }

    fn catalog_run(
        &mut self,
        work: impl FnOnce(&Path) -> Result<Edited, String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(data_dir) = self.catalog_dir() else {
            return;
        };
        let work = cx.background_executor().spawn(async move {
            let edited = work(&data_dir)?;
            if edited.changed {
                gespan_core::tool_servers::sync_servers();
            }
            let stale = edited.catalog.stale(&data_dir);
            Ok::<_, String>((edited, stale))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok((edited, stale)) => {
                        if edited.revision >= this.mcp_catalog_revision {
                            this.mcp_catalog_revision = edited.revision;
                            this.mcp_catalog = edited.catalog;
                            this.mcp_catalog_stale = stale;
                        }
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
                    this.catalog_edit(
                        Box::new(move |catalog, data_dir| catalog.approve_shown(data_dir, &entry)),
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
        let stale = self.mcp_catalog_stale.contains(&entry.id);
        let mut facts = vec![
            status(&entry, stale).to_owned(),
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
                            .debug_selector(move || format!("settings-catalog-detail-{ix}"))
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
                .debug_selector(move || format!("settings-catalog-off-{ix}"))
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
        }
        if entry.url.is_none() && (!entry.enabled || stale) {
            let approving = entry.clone();
            row = row.child(
                secondary_button(
                    SharedString::from(format!("catalog-approve-{ix}")),
                    "Review and turn on",
                    false,
                    false,
                )
                .debug_selector(move || format!("settings-catalog-approve-{ix}"))
                .child("Review and turn on")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.catalog_approve(approving.clone(), window, cx);
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
            .debug_selector(move || format!("settings-catalog-remove-{ix}"))
            .child("Remove")
            .on_click(cx.listener(move |this, _, _, cx| {
                let id = id.clone();
                this.catalog_run(move |data_dir| mcp_catalog::remove(data_dir, &id), cx);
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
                    .debug_selector(|| "settings-catalog-error".into())
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
