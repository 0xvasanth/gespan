//! "Run with…": start an agent session on a card. The drawer's button shows
//! only while the board's `run_with` setting is on (and readable). A click
//! re-reads the card's blockers: a blocked card asks first, and a read that
//! fails starts nothing. The picker offers provider, model and project (or a
//! new worktree), starting on the user's usual provider and model. The shell
//! builds the session with `prepare_run`: its first message is exactly
//! `Service::run_prompt`, it carries the card's cached ticket, and it is
//! linked to the card before anything opens.
use super::{BoardView, Event};
use crate::{
    input::Input,
    modal,
    runtime::{Database, locked},
    theme::*,
};
use gespan_core::{board::Service, session_groups::WorkItem, store::Session};
use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, Role, SharedString, Toggled, Window, div,
    prelude::*, px,
};
use std::{collections::HashMap, path::Path};

/// What the picker asks the shell to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunRequest {
    pub card_id: String,
    pub provider: String,
    pub model: Option<String>,
    pub project: String,
    /// Work in a new worktree of `project`.
    pub new_worktree: bool,
}

/// The providers the picker offers and the user's usual choices, from the
/// shell's preferences.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RunOptions {
    /// (id, label), in the order the app lists providers.
    pub providers: Vec<(String, String)>,
    pub default_provider: Option<String>,
    /// Provider id → the user's default model for it.
    pub default_models: HashMap<String, String>,
}

impl RunOptions {
    fn first_provider(&self) -> String {
        self.default_provider
            .clone()
            .filter(|id| self.providers.iter().any(|(known, _)| known == id))
            .or_else(|| self.providers.first().map(|(id, _)| id.clone()))
            .unwrap_or_else(|| "codex".to_string())
    }
}

/// The open picker.
pub(crate) struct RunPicker {
    pub card_id: String,
    pub provider: String,
    pub model: Entity<Input>,
    pub project: Option<String>,
    pub new_worktree: bool,
    pub error: Option<String>,
    start_focus: FocusHandle,
}

/// "#{n} is blocked by #{a}, #{b}. Run anyway?"
pub(crate) struct RunConfirm {
    pub card_id: String,
    pub number: i64,
    pub blockers: Vec<i64>,
}

impl RunConfirm {
    pub(crate) fn message(&self) -> String {
        let blockers = self
            .blockers
            .iter()
            .map(|n| format!("#{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("#{} is blocked by {blockers}. Run anyway?", self.number)
    }
}

/// A session made for a run, ready to open: the summary (with its ticket),
/// the settings and the first message.
pub(crate) struct PreparedRun {
    pub summary: Session,
    pub settings: gespan_core::agent::Settings,
    pub prompt: String,
}

/// Make the session for `request` and link it to the card. The prompt is
/// exactly `run_prompt(card)`; the ticket is the card's cached snapshot (no
/// tracker call, so a disconnected tracker still starts). The session's
/// access and intent are the ones `settings` already carries. If the link
/// can't be written, the session is deleted again: no empty session is left
/// behind. Blocking; call it off the UI thread.
pub(crate) fn prepare_run(
    store: &Database,
    db_path: &Path,
    request: &RunRequest,
    settings: gespan_core::agent::Settings,
) -> Result<PreparedRun, String> {
    prepare_run_with(
        store,
        db_path,
        request,
        settings,
        |service, card_id, key| service.add_link(card_id, "session", key, None),
    )
}

/// `prepare_run` with the card link written by `link`, so a test can make
/// it fail.
pub(crate) fn prepare_run_with(
    store: &Database,
    db_path: &Path,
    request: &RunRequest,
    settings: gespan_core::agent::Settings,
    link: impl FnOnce(&mut Service, &str, &str) -> Result<(), String>,
) -> Result<PreparedRun, String> {
    let mut service = Service::open(db_path)?;
    let prompt = service.run_prompt(&request.card_id)?;
    let ticket = service.card_ticket(&request.card_id)?;
    let remote = gespan_core::files::remote::is_remote(Path::new(&request.project));
    if !remote && !Path::new(&request.project).is_dir() {
        return Err("Choose a project folder before running this card".into());
    }
    let mut store = locked(store)?;
    let mut summary = store.create(&request.provider, &request.project)?;
    let id = summary.id;
    let made = (|| {
        if let Some(ticket) = ticket {
            let item = WorkItem::Ticket(Box::new(ticket));
            store.set_work_item(id, Some(&item))?;
            summary.work_item = Some(item);
        }
        store.save_agent_settings(id, &settings)?;
        let key = store.persisted_session_id(id)?;
        link(&mut service, &request.card_id, &key)
    })();
    if let Err(error) = made {
        if let Err(undo) = store.delete(id) {
            tracing::warn!(%undo, "removing the unlinked run session failed");
        }
        return Err(error);
    }
    Ok(PreparedRun {
        summary,
        settings,
        prompt,
    })
}

impl BoardView {
    /// Whether the drawer offers "Run with…": the board's setting is on and
    /// was read. Unreadable settings hide it.
    pub(crate) fn run_with_shown(&self) -> bool {
        self.settings_loaded && self.board_settings.run_with
    }

    /// The providers and default choices the picker starts from.
    pub(crate) fn set_run_options(&mut self, options: RunOptions) {
        self.run_options = options;
    }

    /// "Run with…" pressed: re-read the blockers now. A blocked card asks
    /// first; a read that fails shows why and starts nothing.
    pub(crate) fn run_with(&mut self, card_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.run_with_shown() {
            return;
        }
        let Some(board_id) = self.open_board_id.clone() else {
            return;
        };
        let Some(number) = self.card(card_id).map(|card| card.number) else {
            return;
        };
        let blocked = self
            .service()
            .and_then(|service| service.blocked(&board_id));
        match blocked {
            Err(error) => {
                self.error = Some(format!("Couldn't check what blocks #{number}: {error}"));
                cx.notify();
            }
            Ok(blocked) => match blocked.get(card_id).filter(|b| !b.is_empty()) {
                Some(blockers) => {
                    self.blocked = blocked.clone();
                    self.run_confirm = Some(RunConfirm {
                        card_id: card_id.to_string(),
                        number,
                        blockers: blockers.clone(),
                    });
                    cx.notify();
                }
                None => {
                    self.blocked = blocked;
                    self.open_run_picker(card_id.to_string(), window, cx);
                }
            },
        }
    }

    pub(super) fn run_anyway(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(confirm) = self.run_confirm.take() {
            self.open_run_picker(confirm.card_id, window, cx);
        }
    }

    pub(super) fn cancel_run_confirm(&mut self, cx: &mut Context<Self>) -> bool {
        if self.run_confirm.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn open_run_picker(&mut self, card_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let provider = self.run_options.first_provider();
        let model = cx.new(|cx| Input::field("Model", "Default model", content(0.4), cx));
        if let Some(default) = self.run_options.default_models.get(&provider) {
            let default = default.clone();
            model.update(cx, |input, cx| input.set_text(default, cx));
        }
        let card_projects = self.card_projects(&card_id);
        let project = card_projects
            .iter()
            .find(|path| self.projects.iter().any(|(known, _)| known == *path))
            .cloned()
            .or_else(|| self.projects.first().map(|(path, _)| path.clone()))
            .or_else(|| card_projects.first().cloned());
        let start_focus = cx.focus_handle();
        window.focus(&start_focus, cx);
        self.run_picker = Some(RunPicker {
            card_id,
            provider,
            model,
            project,
            new_worktree: false,
            error: None,
            start_focus,
        });
        cx.notify();
    }

    pub(super) fn close_run_picker(
        &mut self,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.run_picker.take().is_none() {
            return false;
        }
        if let Some(window) = window {
            window.focus(&self.focus, cx);
        }
        cx.notify();
        true
    }

    fn pick_provider(&mut self, provider: String, cx: &mut Context<Self>) {
        let default = self.run_options.default_models.get(&provider).cloned();
        if let Some(picker) = &mut self.run_picker {
            picker.provider = provider;
            let model = default.unwrap_or_default();
            picker
                .model
                .update(cx, |input, cx| input.set_text(model, cx));
        }
        cx.notify();
    }

    fn pick_project(&mut self, project: String, cx: &mut Context<Self>) {
        if let Some(picker) = &mut self.run_picker {
            picker.project = Some(project);
            picker.error = None;
        }
        cx.notify();
    }

    fn toggle_run_worktree(&mut self, cx: &mut Context<Self>) {
        if let Some(picker) = &mut self.run_picker {
            picker.new_worktree = !picker.new_worktree;
        }
        cx.notify();
    }

    /// Start: hand the request to the shell, which makes the session.
    pub(super) fn submit_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.run_picker else {
            return;
        };
        let Some(project) = picker.project.clone() else {
            picker.error = Some("Choose a project".into());
            cx.notify();
            return;
        };
        let model = picker.model.read(cx).content.trim().to_string();
        let request = RunRequest {
            card_id: picker.card_id.clone(),
            provider: picker.provider.clone(),
            model: (!model.is_empty()).then_some(model),
            project,
            new_worktree: picker.new_worktree,
        };
        self.close_run_picker(Some(window), cx);
        cx.emit(Event::RunWith(request));
    }

    pub(super) fn render_run_confirm(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let confirm = self.run_confirm.as_ref()?;
        let message = confirm.message();
        let weak = cx.weak_entity();
        let body = modal::card_body()
            .id("board-run-blocked")
            .debug_selector(|| "board-run-blocked".into())
            .role(Role::AlertDialog)
            .aria_label(message.clone())
            .child(modal::title("Card is blocked"))
            .child(div().mt(px(8.)).child(modal::body(message)))
            .child(
                modal::actions()
                    .child(
                        modal::ghost_button("board-run-blocked-cancel", "Cancel")
                            .debug_selector(|| "board-run-blocked-cancel".into())
                            .cursor_pointer()
                            .tab_index(0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_run_confirm(cx);
                            })),
                    )
                    .child(
                        modal::primary_button("board-run-anyway", "Run anyway")
                            .debug_selector(|| "board-run-anyway".into())
                            .cursor_pointer()
                            .tab_index(0)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.run_anyway(window, cx)),
                            ),
                    ),
            );
        Some(
            div()
                .absolute()
                .inset_0()
                .child(modal::overlay(
                    "board-run-blocked",
                    modal::card().w(px(400.)).child(body),
                    move |_, cx| {
                        let _ = weak.update(cx, |this, cx| this.cancel_run_confirm(cx));
                    },
                ))
                .into_any_element(),
        )
    }

    pub(super) fn render_run_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let picker = self.run_picker.as_ref()?;
        let number = self.card(&picker.card_id).map_or(0, |card| card.number);
        let option = |id: String, label: String, selected: bool| {
            let selector = id.clone();
            div()
                .id(SharedString::from(id))
                .debug_selector(move || selector.clone())
                .role(Role::RadioButton)
                .aria_label(label.clone())
                .aria_selected(selected)
                .tab_index(0)
                .px(px(10.))
                .py(px(4.))
                .rounded(px(6.))
                .border_1()
                .border_color(if selected {
                    accent_color()
                } else {
                    content(0.1)
                })
                .text_size(px(12.))
                .text_color(content(if selected { 0.95 } else { 0.6 }))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.06)))
                .focus_visible(|s| s.border_color(accent_color()))
                .child(label)
        };
        let group = |label: &'static str| {
            div().flex().flex_col().gap(px(6.)).child(
                div()
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child(label),
            )
        };
        let providers = self
            .run_options
            .providers
            .iter()
            .enumerate()
            .map(|(index, (id, label))| {
                let pick = id.clone();
                option(
                    format!("board-run-provider-{index}"),
                    label.clone(),
                    *id == picker.provider,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.pick_provider(pick.clone(), cx)))
                .on_key_down(cx.listener({
                    let pick = id.clone();
                    move |this, event: &gpui::KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.pick_provider(pick.clone(), cx)
                        }
                    }
                }))
            })
            .collect::<Vec<_>>();
        let projects = self
            .projects
            .iter()
            .enumerate()
            .map(|(index, (path, name))| {
                let pick = path.clone();
                option(
                    format!("board-run-project-{index}"),
                    name.clone(),
                    picker.project.as_deref() == Some(path.as_str()),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.pick_project(pick.clone(), cx)))
                .on_key_down(cx.listener({
                    let pick = path.clone();
                    move |this, event: &gpui::KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.pick_project(pick.clone(), cx)
                        }
                    }
                }))
            })
            .collect::<Vec<_>>();
        let worktree = picker.new_worktree;
        let checkbox = div()
            .id("board-run-worktree")
            .debug_selector(|| "board-run-worktree".into())
            .role(Role::CheckBox)
            .aria_label("New worktree")
            .aria_toggled(if worktree {
                Toggled::True
            } else {
                Toggled::False
            })
            .tab_index(0)
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(content(0.7))
            .cursor_pointer()
            .focus_visible(|s| s.text_color(content(1.)))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "space" {
                    cx.stop_propagation();
                    this.toggle_run_worktree(cx);
                }
            }))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_run_worktree(cx)))
            .child(
                div()
                    .size(px(14.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(content(0.3))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(worktree, |d| {
                        d.bg(content(0.9))
                            .child(icon("check", 9.).text_color(pane_fill()))
                    }),
            )
            .child("New worktree");
        let body = modal::card_body()
            .gap(px(14.))
            .child(modal::title(format!("Run #{number} with…")))
            .child(
                group("Provider").child(
                    div()
                        .id("board-run-providers")
                        .role(Role::RadioGroup)
                        .aria_label("Provider")
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .children(providers),
                ),
            )
            .child(
                group("Model").child(
                    div()
                        .id("board-run-model")
                        .debug_selector(|| "board-run-model".into())
                        .h(px(30.))
                        .px(px(10.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(content(0.12))
                        .flex()
                        .items_center()
                        .on_click({
                            let focus = picker.model.focus_handle(cx);
                            move |_, window, cx| window.focus(&focus, cx)
                        })
                        .child(picker.model.clone()),
                ),
            )
            .child(
                group("Project").child(
                    div()
                        .id("board-run-projects")
                        .role(Role::RadioGroup)
                        .aria_label("Project")
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .children(projects)
                        .when(self.projects.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(content(0.45))
                                    .child("Add a project to the workspace first"),
                            )
                        }),
                ),
            )
            .child(checkbox)
            .children(picker.error.clone().map(|error| {
                div()
                    .id("board-run-error")
                    .debug_selector(|| "board-run-error".into())
                    .text_size(px(12.))
                    .text_color(diff_foreground(false))
                    .child(error)
            }))
            .child(
                modal::actions()
                    .mt(px(4.))
                    .child(
                        modal::ghost_button("board-run-cancel", "Cancel")
                            .debug_selector(|| "board-run-cancel".into())
                            .cursor_pointer()
                            .tab_index(0)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_run_picker(Some(window), cx);
                            })),
                    )
                    .child(
                        modal::primary_button("board-run-start", "Start")
                            .debug_selector(|| "board-run-start".into())
                            .cursor_pointer()
                            .track_focus(&picker.start_focus)
                            .tab_index(0)
                            .focus_visible(|s| s.border_1().border_color(accent_color()))
                            .on_key_down(cx.listener(
                                |this, event: &gpui::KeyDownEvent, window, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        cx.stop_propagation();
                                        this.submit_run(window, cx);
                                    }
                                },
                            ))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.submit_run(window, cx)),
                            ),
                    ),
            );
        let weak = cx.weak_entity();
        Some(
            div()
                .id("board-run-picker")
                .debug_selector(|| "board-run-picker".into())
                .role(Role::Dialog)
                .aria_label(format!("Run #{number} with"))
                .absolute()
                .inset_0()
                .child(modal::overlay(
                    "board-run-picker",
                    modal::card().w(px(480.)).child(body),
                    move |window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.close_run_picker(Some(window), cx);
                        });
                    },
                ))
                .into_any_element(),
        )
    }
}
