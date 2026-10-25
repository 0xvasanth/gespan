//! The Flows app view: saved Flows with their last run, "New Flow" from a
//! starter, and the editor, run dialog and run pages beside them. Reads come
//! from `FlowStore`; anything that drives a run is an [`Event`] for the shell.
use super::{
    editor, run_view,
    ui::{Tooltip, button, disabled, error_banner, run_status_label, status_chip},
};
use crate::{
    pickers::{ProjectPicked, ProjectPicker},
    popover::{Anchor, AnchorSlot, popover},
    theme::*,
};
use gespan_core::flow::{Flow, FlowRun, FlowStore, starter_flows};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Role, SharedString,
    Subscription, Window, div, prelude::*, px,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

pub(crate) type StoreHandle = Arc<Mutex<FlowStore>>;

pub(super) const DELETE_BLOCKED: &str = "Stop or finish its active run before deleting this Flow";

pub(crate) enum Event {
    Close,
    Start {
        flow: Box<Flow>,
        inputs: HashMap<String, String>,
    },
    Approve {
        run_id: String,
        approved: bool,
        note: String,
    },
    Cancel(String),
    Resume {
        run_id: String,
        state_id: String,
    },
    OpenSession(i64),
    /// The user confirmed deleting this Flow; the shell checks its runs.
    Delete(String),
}

/// A Flow session whose running turn waits for the user's approval.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Waiting {
    pub run_id: String,
    pub session_title: String,
}

/// A list row's status pill: its last run's, or the approval wait inside it.
pub(super) fn row_status(
    last: Option<&FlowRun>,
    waiting: &HashMap<i64, Waiting>,
) -> (&'static str, &'static str) {
    match last {
        Some(run)
            if run.status == gespan_core::flow::RunStatus::Running
                && waiting.values().any(|w| w.run_id == run.id) =>
        {
            ("waiting-approval", "Waiting for approval")
        }
        Some(run) => (run.status.as_str(), run_status_label(run.status)),
        None => ("none", "Never run"),
    }
}

pub(super) enum Page {
    Empty,
    New,
    Editor(editor::Editor),
    Run(run_view::RunPage),
}

pub(crate) struct FlowsView {
    pub(super) store: Option<StoreHandle>,
    pub(super) cwd: String,
    pub(super) projects: Vec<String>,
    pub(super) appearances: HashMap<String, gespan_core::project_appearance::ProjectAppearance>,
    pub(super) flows: Vec<Flow>,
    pub(super) last_runs: HashMap<String, FlowRun>,
    /// Flows with a running or waiting-approval run.
    pub(super) active: HashSet<String>,
    /// Flow sessions waiting for approval, by session id.
    pub(super) waiting: HashMap<i64, Waiting>,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    pub(super) page: Page,
    /// The project a new Flow is saved with.
    pub(super) new_project: String,
    pub(super) creating: bool,
    pub(super) dialog: Option<run_view::RunDialog>,
    project_picker: Option<Entity<ProjectPicker>>,
    picker_subscription: Option<Subscription>,
    project_anchor: AnchorSlot,
    focus: FocusHandle,
    generation: u64,
}

impl EventEmitter<Event> for FlowsView {}
impl Focusable for FlowsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub(super) fn store_lock(
    store: &StoreHandle,
) -> Result<std::sync::MutexGuard<'_, FlowStore>, String> {
    store.lock().map_err(|_| "Flows are unavailable".into())
}

pub(super) fn now() -> i64 {
    gespan_core::time::now_ms().min(i64::MAX as u64) as i64
}

pub(super) fn project_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.into())
}

/// A starter saved as a new Flow in `project`.
pub(super) fn from_starter(starter: &Flow, project: &str, id: String, now: i64) -> Flow {
    Flow {
        id,
        cwd: project.to_owned(),
        created_at: now,
        updated_at: now,
        ..starter.clone()
    }
}

impl FlowsView {
    pub(crate) fn new(
        store: Option<StoreHandle>,
        cwd: String,
        projects: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            store,
            new_project: cwd.clone(),
            cwd,
            projects,
            appearances: HashMap::new(),
            flows: Vec::new(),
            last_runs: HashMap::new(),
            active: HashSet::new(),
            waiting: HashMap::new(),
            loading: true,
            error: None,
            page: Page::Empty,
            creating: false,
            dialog: None,
            project_picker: None,
            picker_subscription: None,
            project_anchor: AnchorSlot::default(),
            focus: cx.focus_handle(),
            generation: 0,
        };
        this.refresh(cx);
        this
    }

    /// The Flow store opened after the view did.
    pub(crate) fn set_store(&mut self, store: StoreHandle, cx: &mut Context<Self>) {
        self.store = Some(store);
        self.refresh(cx);
    }

    pub(crate) fn set_appearances(
        &mut self,
        appearances: HashMap<String, gespan_core::project_appearance::ProjectAppearance>,
        cx: &mut Context<Self>,
    ) {
        self.appearances = appearances;
        cx.notify();
    }

    pub(crate) fn set_context(
        &mut self,
        cwd: String,
        projects: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.page, Page::New) {
            self.new_project = cwd.clone();
        }
        self.cwd = cwd;
        self.projects = projects;
        cx.notify();
    }

    pub(crate) fn set_waiting(&mut self, waiting: HashMap<i64, Waiting>, cx: &mut Context<Self>) {
        if self.waiting != waiting {
            self.waiting = waiting;
            cx.notify();
        }
    }

    pub(crate) fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }

    pub(super) fn project_label(&self, path: &str) -> String {
        self.appearances
            .get(path)
            .map_or_else(|| project_name(path), |appearance| appearance.label(path))
    }

    pub(super) fn project_mark(&self, path: &str, side: f32) -> AnyElement {
        crate::project_settings::project_icon(
            &self.appearances.get(path).cloned().unwrap_or_default(),
            path,
            side,
        )
    }

    pub(super) fn has_active_run(&self, flow_id: &str) -> bool {
        self.active.contains(flow_id)
    }

    /// Reloads the list, last runs and the open run from the store.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        let open_run = match &self.page {
            Page::Run(page) => Some(page.run_id.clone()),
            _ => None,
        };
        let work = cx.background_executor().spawn(async move {
            let store = store_lock(&store)?;
            let flows = store.list()?;
            let mut last_runs = HashMap::new();
            for flow in &flows {
                if let Some(run) = store.runs(&flow.id, 1)?.into_iter().next() {
                    last_runs.insert(flow.id.clone(), run);
                }
            }
            let active: HashSet<String> = store
                .active_runs()?
                .into_iter()
                .map(|run| run.flow_id)
                .collect();
            let run = match open_run {
                Some(id) => Some((store.run(&id)?, store.visits(&id)?)),
                None => None,
            };
            Ok::<_, String>((flows, last_runs, active, run))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok((flows, last_runs, active, run)) => {
                        this.flows = flows;
                        this.last_runs = last_runs;
                        this.active = active;
                        if let (Page::Run(page), Some((run, visits))) = (&mut this.page, run) {
                            page.loaded(run, visits);
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn show_new(&mut self, cx: &mut Context<Self>) {
        self.page = Page::New;
        self.new_project = self.cwd.clone();
        self.dialog = None;
        self.error = None;
        cx.notify();
    }

    /// Saves `starter` with the chosen project, then opens it in the editor.
    pub(super) fn create_from_starter(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            self.error = Some("Flows are still opening".into());
            cx.notify();
            return;
        };
        let Some(starter) = starter_flows().into_iter().nth(index) else {
            return;
        };
        if self.creating {
            return;
        }
        if self.new_project.trim().is_empty() {
            self.error = Some("Choose a project for this Flow".into());
            cx.notify();
            return;
        }
        self.creating = true;
        self.error = None;
        let flow = from_starter(
            &starter,
            &self.new_project,
            gespan_core::automations::new_id(),
            now(),
        );
        let work = cx
            .background_executor()
            .spawn(async move { store_lock(&store)?.upsert(&flow) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.creating = false;
                match result {
                    Ok(saved) => {
                        this.open_editor(saved, cx);
                        this.refresh(cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// A row's click: its last run, or the editor when it has never run.
    fn open_flow(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(run) = self.last_runs.get(id).cloned() {
            self.show_run(run, cx);
        } else if let Some(flow) = self.flows.iter().find(|f| f.id == id).cloned() {
            self.open_editor(flow, cx);
        }
    }

    pub(super) fn edit(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(flow) = self.flows.iter().find(|f| f.id == id).cloned() {
            self.open_editor(flow, cx);
        }
    }

    /// Opens a run page; the store fills in its visits.
    pub(crate) fn show_run(&mut self, run: FlowRun, cx: &mut Context<Self>) {
        self.dialog = None;
        self.error = None;
        let page = run_view::RunPage::new(run, cx);
        self.page = Page::Run(page);
        self.refresh(cx);
        cx.notify();
    }

    fn ask_delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_active_run(&id) {
            return;
        }
        let prompt = window.prompt(
            gpui::PromptLevel::Warning,
            "Delete this Flow?",
            Some("Its run history will be deleted too."),
            &["Delete", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if prompt.await == Ok(0) {
                let _ = this.update(cx, |_, cx| cx.emit(Event::Delete(id)));
            }
        })
        .detach();
    }

    /// Deletes without asking the shell; `Shell::delete_flow` is the way in.
    pub(crate) fn delete(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let deleted = id.clone();
        let work = cx
            .background_executor()
            .spawn(async move { store_lock(&store)?.delete(&id) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        let open = match &this.page {
                            Page::Editor(editor) => editor.flow.id == deleted,
                            Page::Run(page) => page.flow_id() == Some(deleted.as_str()),
                            _ => false,
                        };
                        if open {
                            this.page = Page::Empty;
                        }
                        this.error = None;
                    }
                    // The store refuses while a run is active.
                    Err(error) => this.error = Some(error),
                }
                this.refresh(cx);
            });
        })
        .detach();
    }

    fn choose_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picker = cx.new(|cx| {
            ProjectPicker::new(&self.projects, &self.new_project, cx)
                .appearance(self.appearances.clone())
        });
        self.picker_subscription = Some(cx.subscribe_in(
            &picker,
            window,
            |this, _, picked: &ProjectPicked, window, cx| {
                this.project_picker = None;
                match picked {
                    ProjectPicked::Project(path) => this.new_project = path.clone(),
                    ProjectPicked::New => this.browse_project(cx),
                    ProjectPicked::Remote | ProjectPicked::Pilot(_) => {}
                }
                window.focus(&this.focus, cx);
                cx.notify();
            },
        ));
        window.focus(&picker.read(cx).search_focus(cx), cx);
        self.project_picker = Some(picker);
        cx.notify();
    }

    fn browse_project(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose project".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = prompt.await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(Ok(Some(paths))) = result
                    && let Some(path) = paths.first()
                {
                    let path = path.to_string_lossy().to_string();
                    if !this.projects.contains(&path) {
                        this.projects.push(path.clone());
                    }
                    this.new_project = path;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div()
            .id("flows-list")
            .role(Role::ListBox)
            .aria_label("Flows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(2.));
        if self.flows.is_empty() {
            list = list.child(
                div()
                    .px(px(12.))
                    .py(px(32.))
                    .text_size(px(12.))
                    .text_color(content(0.45))
                    .child(if self.loading || self.store.is_none() {
                        "Loading Flows…"
                    } else {
                        "No Flows yet"
                    }),
            );
        }
        let open_id = match &self.page {
            Page::Editor(editor) => Some(editor.flow.id.clone()),
            Page::Run(page) => page.flow_id().map(str::to_owned),
            _ => None,
        };
        for flow in &self.flows {
            let id = flow.id.clone();
            let active_row = open_id.as_deref() == Some(&id);
            let blocked = self.has_active_run(&id);
            let last = self.last_runs.get(&id);
            let open = id.clone();
            let run = id.clone();
            let edit = id.clone();
            let delete = id.clone();
            let mut actions = div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    button(SharedString::from(format!("flow-run-{id}")), "Run")
                        .debug_selector({
                            let id = id.clone();
                            move || format!("flow-run-{id}")
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.open_run_dialog(&run, window, cx)
                        })),
                )
                .child(
                    button(SharedString::from(format!("flow-edit-{id}")), "Edit")
                        .debug_selector({
                            let id = id.clone();
                            move || format!("flow-edit-{id}")
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.edit(&edit, cx)
                        })),
                );
            let mut delete_button = disabled(
                button(SharedString::from(format!("flow-delete-{id}")), "Delete").debug_selector({
                    let id = id.clone();
                    move || format!("flow-delete-{id}")
                }),
                blocked,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.ask_delete(delete.clone(), window, cx)
            }));
            if blocked {
                delete_button = delete_button
                    .tooltip(|_, cx| cx.new(|_| Tooltip(DELETE_BLOCKED.into())).into());
            }
            actions = actions.child(delete_button);
            list = list.child(
                div()
                    .id(SharedString::from(format!("flow-{id}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("flow-row-{id}")
                    })
                    .role(Role::Button)
                    .aria_label(format!("Open {}", flow.name))
                    .rounded(px(6.))
                    .px(px(10.))
                    .py(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .cursor_pointer()
                    .when(active_row, |d| d.bg(content(0.1)))
                    .hover(|s| s.bg(content(0.05)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(flow.name.clone()),
                            )
                            .child({
                                let (status, label) = row_status(last, &self.waiting);
                                status_chip(status, label)
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child(self.project_mark(&flow.cwd, 12.))
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .child(self.project_label(&flow.cwd)),
                            ),
                    )
                    .child(actions)
                    .on_click(cx.listener(move |this, _, _, cx| this.open_flow(&open, cx))),
            );
        }
        list.into_any_element()
    }

    fn render_new(&self, cx: &mut Context<Self>) -> AnyElement {
        let project = if self.new_project.is_empty() {
            "Choose project".to_string()
        } else {
            self.project_label(&self.new_project)
        };
        div()
            .id("flow-new")
            .debug_selector(|| "flow-new".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(32.))
            .py(px(24.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .text_size(px(20.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("New Flow"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child("Project")
                    .child(
                        div()
                            .id("flow-new-project")
                            .debug_selector(|| "flow-new-project".into())
                            .role(Role::Button)
                            .aria_label("Choose Flow project")
                            .px(px(8.))
                            .h(px(26.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_color(content(0.9))
                            .hover(|s| s.bg(content(0.05)))
                            .child(self.project_anchor.probe())
                            .child(self.project_mark(&self.new_project, 14.))
                            .child(div().min_w_0().truncate().child(project))
                            .child(icon("chevron-down", 12.).text_color(content(0.45)))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.choose_project(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child("Start from a starter, then edit its JSON."),
            )
            .children(
                starter_flows()
                    .into_iter()
                    .enumerate()
                    .map(|(index, starter)| {
                        div()
                            .id(("flow-starter", index))
                            .debug_selector(move || format!("flow-starter-{index}"))
                            .role(Role::Button)
                            .aria_label(format!("New Flow from {}", starter.name))
                            .max_w(px(560.))
                            .p(px(14.))
                            .rounded(px(10.))
                            .border_1()
                            .border_color(content(0.1))
                            .cursor_pointer()
                            .hover(|s| s.bg(content(0.05)).border_color(content(0.2)))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .when(self.creating, |d| d.opacity(0.5))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(starter.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(content(0.55))
                                    .child(starter.description.clone()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.create_from_starter(index, cx)
                            }))
                    }),
            )
            .into_any_element()
    }

    fn render_empty(&self) -> AnyElement {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(13.))
            .text_color(content(0.45))
            .child("Choose a Flow, or start a new one")
            .into_any_element()
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.project_picker.is_some() {
            self.project_picker = None;
        } else if self.dialog.is_some() {
            // An open access menu closes first; the dialog stays.
            if !self.close_access_menu() {
                self.dialog = None;
            }
        } else if matches!(&self.page, Page::Editor(editor) if editor.dirty) {
            self.confirm_discard(window, cx);
        } else if !matches!(self.page, Page::Empty) {
            self.page = Page::Empty;
            self.error = None;
        } else {
            cx.emit(Event::Close);
        }
        cx.notify();
    }
}

impl Render for FlowsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let aside = div()
            .w(px(300.))
            .flex_shrink_0()
            .border_r_1()
            .border_color(content(0.07))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(36.))
                    .px(px(12.))
                    .border_b_1()
                    .border_color(content(0.07))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(content(0.6))
                            .child("Flows"),
                    )
                    .child(
                        icon_button("new-flow", "New Flow", "plus")
                            .debug_selector(|| "new-flow".into())
                            .on_click(cx.listener(|this, _, _, cx| this.show_new(cx))),
                    ),
            )
            .child(self.render_list(cx));
        let main = match &self.page {
            Page::Empty => self.render_empty(),
            Page::New => self.render_new(cx),
            Page::Editor(_) => self.render_editor(cx),
            Page::Run(_) => self.render_run(window, cx),
        };
        let mut root = div()
            .id("flows-view")
            .debug_selector(|| "flows-view".into())
            .role(Role::Region)
            .aria_label("Flows")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .text_color(content(1.))
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.escape(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "s"
                    && event.keystroke.modifiers.secondary()
                    && matches!(this.page, Page::Editor(_))
                {
                    // Cmd-S saves, but only inside the Flow editor.
                    this.save_editor(cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div().flex_1().min_h_0().flex().child(aside).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .when_some(self.error.clone(), |d, error| {
                            d.child(error_banner(error).debug_selector(|| "flows-error".into()))
                        })
                        .child(main),
                ),
            );
        if self.dialog.is_some() {
            root = root.child(self.render_dialog(cx));
        }
        if let Some(menu) = self.render_access_menu(cx) {
            root = root.child(menu);
        }
        if let Some(picker) = &self.project_picker {
            let weak = cx.weak_entity();
            root = root.child(
                popover(
                    "flow-project-picker",
                    Anchor::Slot(self.project_anchor.clone()),
                )
                .width(320.)
                .on_dismiss(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.project_picker = None;
                        cx.notify();
                    });
                })
                .child(picker.clone()),
            );
        }
        root
    }
}

#[cfg(test)]
impl FlowsView {
    pub(crate) fn page_name(&self) -> &'static str {
        match self.page {
            Page::Empty => "empty",
            Page::New => "new",
            Page::Editor(_) => "editor",
            Page::Run(_) => "run",
        }
    }
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub(crate) fn row_status_of(&self, flow_id: &str) -> &'static str {
        row_status(self.last_runs.get(flow_id), &self.waiting).1
    }
    /// Confirms a delete as the prompt's Delete button does.
    pub(crate) fn confirm_delete_for_test(&mut self, id: &str, cx: &mut Context<Self>) {
        cx.emit(Event::Delete(id.to_owned()));
    }
    /// Deletes without the confirmation prompt.
    pub(crate) fn delete_for_test(&mut self, id: &str, cx: &mut Context<Self>) {
        self.delete(id.to_owned(), cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::flow::validate_flow;

    #[test]
    fn a_starter_becomes_a_valid_flow_in_the_chosen_project() {
        for starter in starter_flows() {
            let flow = from_starter(&starter, "/work/gespan", "f1".into(), 5);
            assert_eq!(flow.id, "f1");
            assert_eq!(flow.cwd, "/work/gespan");
            assert_eq!((flow.created_at, flow.updated_at), (5, 5));
            assert_eq!(flow.states, starter.states);
            assert!(validate_flow(&flow).errors.is_empty(), "{}", flow.name);
        }
    }
}
