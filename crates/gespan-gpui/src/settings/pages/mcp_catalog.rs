//! Settings → MCP's own catalogue: the servers Gespan runs for every agent.
use crate::{
    settings::{
        SettingsView,
        widgets::{glass_secondary, note},
    },
    theme::*,
};
use gespan_core::{
    mcp_catalog::{self, Catalog, Class, Edited, Entry},
    mcp_registry::{self, Listing},
    tool_servers::{RunningServer, ServerFailure},
};
use gpui::{AnyElement, Context, Entity, SharedString, div, prelude::*, px};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

const WATCH_INTERVAL: Duration = Duration::from_secs(3);

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

/// One running process as a line of facts.
fn running_line(process: &RunningServer, memory: Option<u64>) -> String {
    let mut facts = Vec::new();
    facts.push(if process.pid == 0 {
        "remote".to_owned()
    } else {
        format!("pid {}", process.pid)
    });
    if let Some(workspace) = &process.workspace {
        facts.push(
            workspace
                .file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
        );
    }
    facts.push(match process.sessions.len() {
        1 => "1 session".to_owned(),
        count => format!("{count} sessions"),
    });
    if let Some(memory) = memory {
        facts.push(format!("{} MB", memory / 1_048_576));
    }
    facts.push(match process.idle {
        Some(idle) => format!("idle {}s", idle.as_secs()),
        None => "busy".to_owned(),
    });
    facts.join(" · ")
}

/// `NAME=value` lines. A line without `=` is a mistake worth reporting, though
/// not by repeating it: it may well be the secret.
fn parse_settings(text: &str) -> Result<BTreeMap<String, String>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(ix, line)| {
            let (name, value) = line
                .split_once('=')
                .ok_or_else(|| format!("Line {} needs the form NAME=value", ix + 1))?;
            Ok((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

impl SettingsView {
    pub(in crate::settings) fn load_catalog(&mut self, cx: &mut Context<Self>) {
        self.catalog_edit(Box::new(|_, _| Ok(())), cx);
        self.watch_servers(cx);
    }

    /// Keeps the running list current while this page is open.
    fn watch_servers(&mut self, cx: &mut Context<Self>) {
        if self.mcp_watching {
            return;
        }
        self.mcp_watching = true;
        cx.spawn(async move |this, cx| {
            loop {
                let sample = cx
                    .background_executor()
                    .spawn(async {
                        let running = gespan_core::tool_servers::running_servers();
                        let pids: Vec<u32> = running.iter().map(|process| process.pid).collect();
                        let memory = gespan_core::resources::tree_memory(&pids);
                        let running: Vec<_> = running
                            .into_iter()
                            .map(|process| {
                                let held = memory.get(&process.pid).copied();
                                (process, held)
                            })
                            .collect();
                        (running, gespan_core::tool_servers::server_errors())
                    })
                    .await;
                let watching = this.update(cx, |this, cx| {
                    let watching = this.section == crate::settings::Section::Mcp;
                    this.mcp_watching = watching;
                    if watching {
                        (this.mcp_running, this.mcp_server_errors) = sample;
                        cx.notify();
                    }
                    watching
                });
                if !matches!(watching, Ok(true)) {
                    return;
                }
                cx.background_executor().timer(WATCH_INTERVAL).await;
            }
        })
        .detach();
    }

    fn catalog_search(&mut self, cx: &mut Context<Self>) {
        let Some(input) = &self.mcp_search else {
            return;
        };
        let query = input.read(cx).content.trim().to_owned();
        if query.is_empty() || self.mcp_searching {
            return;
        }
        self.mcp_searching = true;
        let work = cx
            .background_executor()
            .spawn(async move { mcp_registry::search(&query) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.mcp_searching = false;
                match result {
                    Ok(listings) => {
                        this.mcp_listings = listings;
                        this.mcp_catalog_error = None;
                    }
                    Err(error) => this.mcp_catalog_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn catalog_save_settings(&mut self, cx: &mut Context<Self>) {
        let Some((id, input)) = self.mcp_secrets.take() else {
            return;
        };
        let text = input.read(cx).content.to_string();
        self.catalog_edit(
            Box::new(move |catalog, data_dir| {
                catalog.set_secrets(data_dir, &id, parse_settings(&text)?)
            }),
            cx,
        );
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

    /// One sign-in here serves every agent, which never sees the token.
    pub(in crate::settings) fn catalog_sign_in(
        &mut self,
        id: String,
        url: String,
        cx: &mut Context<Self>,
    ) {
        // A second flow would race this one for the same keychain record.
        if !self.mcp_signing_in.insert(id.clone()) {
            return;
        }
        cx.notify();
        let (opener, opening) = async_channel::bounded::<String>(1);
        let server = id.clone();
        let work = cx.background_executor().spawn(async move {
            gespan_core::tool_servers::sign_in(&server, &url, |address| {
                let _ = opener.send_blocking(address.to_owned());
            })
        });
        cx.spawn(async move |this, cx| {
            if let Ok(address) = opening.recv().await {
                cx.update(|cx| cx.open_url(&address));
            }
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.mcp_signing_in.remove(&id);
                match result {
                    Ok(()) => this.load_catalog(cx),
                    Err(error) => {
                        this.mcp_catalog_error = Some(error);
                        cx.notify();
                    }
                }
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
        let state = status(&entry, stale);
        let tone = match state {
            "On" => crate::glass::Tone::Ok,
            "Off" => crate::glass::Tone::Muted,
            _ => crate::glass::Tone::Waiting,
        };
        let mut facts = vec![
            // Older remote entries kept a class that never applied to them.
            class_label(match (&entry.url, entry.class) {
                (Some(_), Class::PerWorktree) => Class::Shared,
                (_, class) => class,
            })
            .to_owned(),
        ];
        if !entry.needed.is_empty() {
            facts.push(format!("needs {}", entry.needed.join(", ")));
        }
        facts.extend(entry.source.clone());
        if let Some(tools) = &entry.tools {
            facts.push(format!("{} tools", tools.len()));
        }
        let mut row =
            div()
                .id(SharedString::from(format!("catalog-server-{ix}")))
                .flex()
                .flex_wrap()
                .items_center()
                .gap_3()
                .px_4()
                .py(px(14.))
                .border_t_1()
                .border_color(crate::glass::hairline())
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
                        .child(icon("package", 16.).text_color(content(0.8))),
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
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .child(entry.name.clone()),
                                )
                                .child(crate::glass::status(state, tone).debug_selector(
                                    move || format!("settings-catalog-status-{ix}"),
                                )),
                        )
                        .child(
                            div()
                                .mt_1()
                                .text_size(px(11.5))
                                .text_color(content(0.55))
                                .child(facts.join(" · ")),
                        )
                        .child(
                            div()
                                .debug_selector(move || format!("settings-catalog-detail-{ix}"))
                                .mt(px(2.))
                                .truncate()
                                .font_family(crate::fonts::monospace())
                                .text_size(px(11.))
                                .text_color(content(0.55))
                                .child(detail),
                        ),
                );
        // A remote server's connection is shared whatever this would say.
        if entry.url.is_none() {
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
        }
        if entry.enabled {
            let id = entry.id.clone();
            row = row.child(
                glass_secondary(
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
                glass_secondary(
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
        if !entry.enabled || stale {
            let approving = entry.clone();
            row = row.child(
                glass_secondary(
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
        if entry.url.is_none() {
            let id = entry.id.clone();
            row = row.child(
                glass_secondary(
                    SharedString::from(format!("catalog-settings-{ix}")),
                    "Settings",
                    false,
                    false,
                )
                .debug_selector(move || format!("settings-catalog-settings-{ix}"))
                .child("Settings")
                .on_click(cx.listener(move |this, _, _, cx| {
                    let input: Entity<crate::input::Input> = cx.new(|cx| {
                        let mut input = crate::input::Input::multiline_field(
                            "Environment",
                            "NAME=value, one per line",
                            cx,
                        );
                        input.set_secret(true);
                        input
                    });
                    this.mcp_secrets = Some((id.clone(), input));
                    cx.notify();
                })),
            );
        }
        if let Some(url) = entry.url.clone() {
            let id = entry.id.clone();
            let waiting = self.mcp_signing_in.contains(&id);
            row = row.child(
                glass_secondary(
                    SharedString::from(format!("catalog-sign-in-{ix}")),
                    "Sign in",
                    false,
                    waiting,
                )
                .debug_selector(move || format!("settings-catalog-sign-in-{ix}"))
                .child(if waiting {
                    "Waiting for the browser"
                } else {
                    "Sign in"
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.catalog_sign_in(id.clone(), url.clone(), cx);
                })),
            );
        }
        let id = entry.id.clone();
        row.child(
            glass_secondary(
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

    fn running_section(&self) -> Vec<AnyElement> {
        // What runs now and what failed, each a card of its own.
        let mut section = Vec::new();
        if !self.mcp_running.is_empty() {
            section.push(
                crate::glass::card()
                    .debug_selector(|| "settings-catalog-running".into())
                    .px_4()
                    .py(px(12.))
                    .child(subheading("Running now"))
                    .children(self.mcp_running.iter().map(|(process, memory)| {
                        fact_row(process.server.clone(), running_line(process, *memory))
                    }))
                    .into_any_element(),
            );
        }
        let failures: Vec<&ServerFailure> = self.mcp_server_errors.iter().rev().take(5).collect();
        if !failures.is_empty() {
            section.push(
                crate::glass::card()
                    .debug_selector(|| "settings-catalog-errors".into())
                    .px_4()
                    .py(px(12.))
                    .child(subheading("Recent errors"))
                    .children(failures.into_iter().enumerate().map(|(ix, failure)| {
                        div()
                            .debug_selector(move || format!("settings-catalog-failure-{ix}"))
                            .child(fact_row(failure.server.clone(), failure.message.clone()))
                    }))
                    .into_any_element(),
            );
        }
        section
    }

    pub(super) fn registry_section(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let input = self
            .mcp_search
            .get_or_insert_with(|| {
                cx.new(|cx| {
                    crate::input::Input::field(
                        "Search the MCP Registry",
                        "Search the MCP Registry",
                        content(0.4),
                        cx,
                    )
                })
            })
            .clone();
        let search = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                crate::glass::field()
                    .flex_1()
                    .child(icon("search-01", 14.).text_color(content(0.5)))
                    .child(div().min_w_0().flex_1().child(input)),
            )
            .child(
                glass_secondary("catalog-search", "Search", false, self.mcp_searching)
                    .h(px(32.))
                    .child("Search")
                    .on_click(cx.listener(|this, _, _, cx| this.catalog_search(cx))),
            )
            .into_any_element();
        let listings = self.mcp_listings.clone();
        let cards = listings.into_iter().enumerate().map(|(ix, listing)| {
            let adding: Listing = listing.clone();
            crate::glass::card()
                .id(SharedString::from(format!("catalog-listing-{ix}")))
                .debug_selector(move || format!("settings-catalog-listing-{ix}"))
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
                                .size(px(28.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(8.))
                                .bg(content(0.07))
                                .border_1()
                                .border_color(crate::glass::edge())
                                .child(icon("package", 14.).text_color(content(0.8))),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(13.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(listing.name.clone()),
                                )
                                .child(
                                    div()
                                        .font_family(crate::fonts::monospace())
                                        .text_size(px(11.))
                                        .text_color(content(0.55))
                                        .child(listing.version.clone()),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .line_height(px(17.))
                        .text_color(content(0.62))
                        .child(listing.description),
                )
                .child(
                    div().flex().justify_end().child(
                        glass_secondary(
                            SharedString::from(format!("catalog-add-{ix}")),
                            "Add",
                            false,
                            false,
                        )
                        .debug_selector(move || format!("settings-catalog-add-{ix}"))
                        .child("Add")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let adding = adding.clone();
                            this.catalog_edit(
                                Box::new(move |catalog, data_dir| {
                                    catalog.install(data_dir, &adding).map(|_| ())
                                }),
                                cx,
                            );
                        })),
                    ),
                )
        });
        let grid = div()
            .debug_selector(|| "settings-catalog-registry-results".into())
            .grid()
            .grid_cols(3)
            .gap(px(10.))
            .children(cards)
            .into_any_element();
        vec![
            self.glass_group(
                Some("mcp-registry"),
                "Add from the MCP Registry",
                None,
                None,
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(search)
                        .when(!self.mcp_listings.is_empty(), |d| d.child(grid))
                        .into_any_element(),
                ),
                Vec::new(),
            ),
        ]
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
        let mut section = Vec::new();
        // The switch, then each server, all in one card.
        let mut card_rows = vec![self.row(
            Some("catalog-managed"),
            true,
            "Agents load only Gespan's servers",
            Some("Agents load only these servers, not their own.".into()),
            vec![self.toggle(
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
            )],
        )];
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
            card_rows.push(
                div()
                    .debug_selector(|| "settings-catalog-empty".into())
                    .border_t_1()
                    .border_color(crate::glass::hairline())
                    .child(note(
                        "None yet. Import copies your agents' servers here, without changing their files.",
                    ))
                    .into_any_element(),
            );
        }
        card_rows.extend(
            entries
                .into_iter()
                .enumerate()
                .map(|(ix, entry)| self.catalog_row(ix, entry, cx)),
        );
        let import = glass_secondary("catalog-import", "Import from agents", false, false)
            .debug_selector(|| "settings-catalog-import".into())
            .child(icon("plus", 14.))
            .child("Import from agents")
            .on_click(cx.listener(|this, _, _, cx| this.catalog_import(cx)))
            .into_any_element();
        section.insert(
            0,
            self.glass_group(
                None,
                "Gespan servers",
                Some("Install once here. Every agent in every worktree shares them.".into()),
                Some(import),
                None,
                card_rows,
            ),
        );
        if let Some((id, input)) = self.mcp_secrets.clone() {
            section.push(subheading(format!("Settings for {id}")));
            section.push(div().mt_2().child(input).into_any_element());
            section.push(
                div()
                    .mt_2()
                    .child(
                        glass_secondary("catalog-settings-save", "Save", false, false)
                            .debug_selector(|| "settings-catalog-settings-save".into())
                            .child("Save")
                            .on_click(cx.listener(|this, _, _, cx| this.catalog_save_settings(cx))),
                    )
                    .into_any_element(),
            );
        }
        section.extend(self.running_section());
        section
    }
}

fn subheading(text: impl Into<SharedString>) -> AnyElement {
    div()
        .mt_4()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(text.into())
        .into_any_element()
}

fn fact_row(name: String, facts: String) -> AnyElement {
    div()
        .mt_1()
        .flex()
        .gap_2()
        .text_size(px(12.))
        .child(div().child(name))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(content(0.5))
                .child(facts),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process() -> RunningServer {
        RunningServer {
            server: "docs".into(),
            pid: 41,
            sharing: gespan_core::tool_servers::ServerSharing::PerWorkspace,
            workspace: Some("/work/feature-a".into()),
            sessions: [1, 2].into(),
            uptime: Duration::from_secs(60),
            idle: Some(Duration::from_secs(12)),
        }
    }

    #[test]
    fn a_running_server_says_who_uses_it_and_what_it_costs() {
        assert_eq!(
            running_line(&process(), Some(52_428_800)),
            "pid 41 · feature-a · 2 sessions · 50 MB · idle 12s"
        );
        let busy = RunningServer {
            pid: 0,
            workspace: None,
            sessions: [1].into(),
            idle: None,
            ..process()
        };
        assert_eq!(running_line(&busy, None), "remote · 1 session · busy");
    }

    #[test]
    fn settings_are_name_value_lines_and_a_bad_line_is_reported() {
        let parsed = parse_settings("API_KEY=abc=def\n\n  OTHER = two \n").unwrap();
        assert_eq!(parsed["API_KEY"], "abc=def");
        assert_eq!(parsed["OTHER"], "two");
        assert_eq!(
            parse_settings("oops").unwrap_err(),
            "Line 1 needs the form NAME=value"
        );
    }
}
