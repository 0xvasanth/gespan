//! Native counterpart of AutomationsView: persisted list, example gallery,
//! trigger editor and run history. All scheduling lives in gespan-core.
use crate::{
    glass,
    input::{self, Input},
    pickers::{ModelList, ModelPicked, ProjectPicked, ProjectPicker, cached_model_name},
    popover::{Anchor, AnchorSlot, popover},
    runtime::{Database, locked},
    theme::*,
};
use gespan_core::{
    automations::{
        self, Automation, AutomationAction, AutomationRun, AutomationTrigger, AutomationUpsert,
        DueAutomationRun, Service,
    },
    provider::Kind,
    session_groups::Folder,
};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Role, SharedString,
    Subscription, Window, div, prelude::*, px, rgb,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[cfg(test)]
mod contract_tests;
mod editor;
mod gallery;
mod layout;
#[cfg(test)]
mod layout_tests;
mod review_setup;
#[cfg(test)]
mod review_setup_tests;
mod trigger_menu;

/// Event providers and schedules are separate from agent harness branding.
fn trigger_mark(kind: &str, side: f32) -> AnyElement {
    if kind == "gitlab" {
        return gpui::img("inbox-gitlab-color.svg")
            .size(px(side))
            .flex_shrink_0()
            .object_fit(gpui::ObjectFit::Contain)
            .into_any_element();
    }
    let (path, color) = match kind {
        "time" => ("clock", content(0.45)),
        "github" => ("inbox-github", content(0.65)),
        "linear" => ("inbox-linear", content(0.65)),
        "jira" => ("inbox-jira", rgb(0x2684ff).into()),
        "azuredevops" => ("inbox-azuredevops", content(0.65)),
        _ => ("zap", content(0.45)),
    };
    icon(path, side).text_color(color).into_any_element()
}

pub(crate) enum Event {
    Close,
    Launch(Box<DueAutomationRun>),
    OpenSession(String),
    /// The review setup sheet's "Sign in": run `gh auth login` in a
    /// terminal, then call `recheck_github` once it exits.
    SignInGithub {
        cwd: String,
    },
}

/// "See all in Runs": open the Runs page filtered to this automation. Its
/// own event type so the Shell's handling of `Event` is untouched until it
/// subscribes.
pub(crate) struct OpenRuns {
    pub automation: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Template {
    id: String,
    category: String,
    #[serde(default)]
    popular: bool,
    icon: String,
    name: String,
    description: String,
    prompt: String,
    trigger: serde_json::Value,
    trigger_label: String,
    /// A quick setup the card opens instead of the editor: "review".
    #[serde(default)]
    setup: Option<String>,
}
struct TriggerFields {
    id: String,
    time: Entity<Input>,
    repos: Entity<Input>,
    actor: Entity<Input>,
}
struct Editor {
    draft: AutomationUpsert,
    name: Entity<Input>,
    prompt: Entity<Input>,
    completion: Entity<crate::composer_completion::Completion>,
    fields: Vec<TriggerFields>,
    saved: bool,
    dirty: bool,
    /// The Pilot last chosen here, so "Wake a Pilot" comes back to it.
    last_pilot: Option<String>,
    _subscriptions: Vec<Subscription>,
}
#[derive(Clone)]
enum Choice {
    /// What a run does: start a session or wake a Pilot.
    Action,
    Pilot,
    Access,
    Workspace,
    Conversation,
    Folder,
    Grace,
    AddTrigger,
    TriggerEvent(String),
    Schedule(String),
    Time(String),
    Minute(String),
    Weekday(String),
    ModelSetting(String),
    /// The review setup sheet's sentence.
    SetupRepo,
    SetupWhich,
    SetupResults,
    /// What a review automation does with its review.
    Results,
}
struct ChoiceMenu {
    field: Choice,
    options: Vec<(String, String)>,
    anchor: AnchorSlot,
}

pub(crate) struct Automations {
    service: Arc<Mutex<Service>>,
    database: Option<Database>,
    cwd: String,
    projects: Vec<String>,
    appearances: HashMap<String, gespan_core::project_appearance::ProjectAppearance>,
    automations: Vec<Automation>,
    runs: Vec<AutomationRun>,
    /// Runs per day over the last ten days, by automation id.
    sparklines: HashMap<String, Vec<usize>>,
    settings_scroll: gpui::ScrollHandle,
    selected: Option<String>,
    editor: Option<Editor>,
    loading: bool,
    error: Option<String>,
    saving: bool,
    running: bool,
    gallery: bool,
    category: String,
    templates: Vec<Template>,
    filter: Entity<Input>,
    focus: FocusHandle,
    _filter_subscription: Subscription,
    history: bool,
    advanced: bool,
    confirm_delete: bool,
    folders: Vec<Folder>,
    /// Every Pilot's id and name, for the "Wake a Pilot" picker.
    pilots: Vec<(String, String)>,
    choice: Option<ChoiceMenu>,
    models: Option<Entity<ModelList>>,
    model_controls: Vec<gespan_core::catalog::ModelControl>,
    project_picker: Option<Entity<ProjectPicker>>,
    picker_subscription: Option<Subscription>,
    project_anchor: AnchorSlot,
    model_anchor: AnchorSlot,
    trigger_anchor: AnchorSlot,
    trigger_menu: Option<Entity<trigger_menu::TriggerMenu>>,
    trigger_subscription: Option<Subscription>,
    trigger_connections: HashMap<String, bool>,
    request_generation: u64,
    /// The two-click review setup, shown in place of the gallery.
    review_setup: Option<review_setup::ReviewSetup>,
    review_generation: u64,
    github_probe: review_setup::GithubProbe,
}
impl EventEmitter<Event> for Automations {}
impl EventEmitter<OpenRuns> for Automations {}
impl Focusable for Automations {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Automations {
    pub(crate) fn new(
        service: Arc<Mutex<Service>>,
        cwd: String,
        projects: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter =
            cx.new(|cx| Input::field("Filter automations", "Filter automations", content(0.4), cx));
        let subscription = cx.subscribe(&filter, |_, _, _: &input::Changed, cx| cx.notify());
        let mut this = Self {
            service,
            database: None,
            cwd,
            projects,
            appearances: HashMap::new(),
            automations: Vec::new(),
            runs: Vec::new(),
            sparklines: HashMap::new(),
            settings_scroll: gpui::ScrollHandle::new(),
            selected: None,
            editor: None,
            loading: true,
            error: None,
            saving: false,
            running: false,
            gallery: true,
            category: "popular".into(),
            templates: serde_json::from_str(include_str!("automations/templates.json"))
                .unwrap_or_default(),
            filter,
            focus: cx.focus_handle(),
            _filter_subscription: subscription,
            history: false,
            advanced: false,
            confirm_delete: false,
            folders: Vec::new(),
            pilots: Vec::new(),
            choice: None,
            models: None,
            model_controls: Vec::new(),
            project_picker: None,
            picker_subscription: None,
            project_anchor: AnchorSlot::default(),
            model_anchor: AnchorSlot::default(),
            trigger_anchor: AnchorSlot::default(),
            trigger_menu: None,
            trigger_subscription: None,
            trigger_connections: HashMap::new(),
            request_generation: 0,
            review_setup: None,
            review_generation: 0,
            github_probe: review_setup::github_probe(),
        };
        this.refresh(cx);
        this
    }
    pub(crate) fn set_database(&mut self, database: Database, cx: &mut Context<Self>) {
        self.database = Some(database);
        self.load_folders(cx);
    }
    pub(crate) fn set_appearances(
        &mut self,
        appearances: HashMap<String, gespan_core::project_appearance::ProjectAppearance>,
        cx: &mut Context<Self>,
    ) {
        self.appearances = appearances;
        cx.notify();
    }
    pub(crate) fn set_pilots(&mut self, pilots: Vec<(String, String)>, cx: &mut Context<Self>) {
        if self.pilots != pilots {
            self.pilots = pilots;
            cx.notify();
        }
    }
    /// The name of Pilot `id`, or what stands in for one that is gone.
    fn pilot_label(&self, id: &str) -> String {
        self.pilots
            .iter()
            .find(|(pilot, _)| pilot == id)
            .map_or_else(|| "Unavailable Pilot".into(), |(_, name)| name.clone())
    }
    /// The name of the automation `id`, once the list has loaded it.
    pub(crate) fn automation_name(&self, id: &str) -> Option<String> {
        self.automations
            .iter()
            .find(|automation| automation.id == id)
            .map(|automation| automation.name.clone())
    }
    fn project_label(&self, path: &str) -> String {
        self.appearances
            .get(path)
            .map_or_else(|| project_name(path), |appearance| appearance.label(path))
    }
    fn project_mark(&self, path: &str, side: f32) -> AnyElement {
        crate::project_settings::project_icon(
            &self.appearances.get(path).cloned().unwrap_or_default(),
            path,
            side,
        )
    }
    pub(crate) fn set_context(
        &mut self,
        cwd: String,
        projects: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if self.cwd != cwd {
            self.review_setup = None;
        }
        self.cwd = cwd;
        self.projects = projects;
        self.load_folders(cx);
        cx.notify();
    }
    fn show_gallery(&mut self, cx: &mut Context<Self>) {
        self.editor = None;
        self.review_setup = None;
        self.selected = None;
        self.runs.clear();
        self.gallery = true;
        self.history = false;
        self.confirm_delete = false;
        self.choice = None;
        self.trigger_menu = None;
        self.models = None;
        self.project_picker = None;
        self.error = None;
        self.request_generation += 1;
        cx.notify();
    }
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let service = self.service.clone();
        let selected = self.selected.clone();
        let selected_for_work = selected.clone();
        let work = cx.background_executor().spawn(async move {
            let mut service = service
                .lock()
                .map_err(|_| "Automations unavailable".to_string())?;
            let list = service.list()?;
            let now = gespan_core::time::now_ms() as i64;
            let mut sparklines = HashMap::new();
            let mut runs = None;
            // Each automation keeps at most 100 runs, so this stays small.
            for automation in &list {
                let history = service.runs(&automation.id)?;
                sparklines.insert(
                    automation.id.clone(),
                    automations::runs_per_day(&history, layout::SPARK_DAYS, now),
                );
                if selected_for_work.as_deref() == Some(automation.id.as_str()) {
                    runs = Some(history);
                }
            }
            let runs = match (runs, selected_for_work.as_deref()) {
                (Some(runs), _) => runs,
                (None, Some(id)) => service.runs(id)?,
                (None, None) => Vec::new(),
            };
            Ok::<_, String>((list, runs, sparklines))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((list, runs, sparklines)) => {
                        this.automations = list;
                        this.sparklines = sparklines;
                        if this.selected == selected {
                            this.runs = runs;
                        }
                        if this
                            .selected
                            .as_ref()
                            .is_some_and(|id| !this.automations.iter().any(|a| &a.id == id))
                        {
                            this.selected = None;
                            if !this.editor.as_ref().is_some_and(|e| e.dirty) {
                                this.editor = None;
                                this.gallery = true;
                            }
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn load_folders(&mut self, cx: &mut Context<Self>) {
        let Some(database) = self.database.clone() else {
            return;
        };
        let cwd = self
            .editor
            .as_ref()
            .map(|e| e.draft.cwd.clone())
            .unwrap_or_else(|| self.cwd.clone());
        let expected = cwd.clone();
        let work = cx
            .background_executor()
            .spawn(async move { locked(&database)?.groups(&cwd) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .editor
                    .as_ref()
                    .map(|e| e.draft.cwd.as_str())
                    .unwrap_or(&this.cwd)
                    == expected
                {
                    this.folders = result.map(|g| g.folders).unwrap_or_default();
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn begin(&mut self, template: Option<Template>, cx: &mut Context<Self>) {
        let preferences = cx.try_global::<crate::settings::Preferences>();
        let (kind, model) = crate::installed_providers::new_session_choice(
            preferences.unwrap_or(&Default::default()),
            Some(&self.cwd),
            cx,
        );
        let model = model.unwrap_or_default();
        let mut draft = AutomationUpsert::new(self.cwd.clone(), kind.id().into(), model);
        if let Some(template) = template {
            draft.name = template.name;
            draft.prompt = template.prompt;
            let kind = template.trigger["kind"].as_str().unwrap_or("time");
            let event = template.trigger["event"].as_str().unwrap_or("weekdays");
            let mut trigger = AutomationTrigger::new(kind, event);
            if let Some(value) = template.trigger["time"].as_str() {
                trigger.time = value.into();
            }
            if let Some(value) = template.trigger["dayOfWeek"].as_i64() {
                trigger.day_of_week = value;
            }
            if let Some(value) = template.trigger["minute"].as_i64() {
                trigger.minute = value;
            }
            draft.triggers = Some(vec![trigger]);
        }
        self.selected = None;
        self.runs.clear();
        self.set_editor(draft, false, cx);
    }
    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(automation) = self.automations.iter().find(|a| a.id == id) else {
            return;
        };
        let draft = AutomationUpsert::from_automation(automation);
        self.selected = Some(id);
        self.runs.clear();
        self.set_editor(draft, true, cx);
        self.refresh(cx);
    }
    fn set_editor(&mut self, draft: AutomationUpsert, saved: bool, cx: &mut Context<Self>) {
        let name = cx.new(|cx| {
            let mut input = Input::automation_name(cx);
            input.set_text(draft.name.clone(), cx);
            input
        });
        let prompt = cx.new(|cx| {
            let mut input = Input::automation_instructions(cx);
            input.set_text(draft.prompt.clone(), cx);
            input
        });
        let completion = cx.new(|cx| {
            let completion =
                crate::composer_completion::Completion::new(prompt.clone(), draft.cwd.clone(), cx)
                    .inline_field();
            if let Some(store) = &self.database {
                completion.with_store(store.clone())
            } else {
                completion
            }
        });
        completion.update(cx, |completion, cx| {
            completion.set_provider(&draft.harness, cx)
        });
        let mut subscriptions = Vec::new();
        for field in [&name, &prompt] {
            subscriptions.push(cx.subscribe(field, |this, _, _: &input::Changed, cx| {
                if let Some(editor) = &mut this.editor {
                    editor.dirty = true;
                }
                cx.notify();
            }));
        }
        let last_pilot = draft.action.pilot_id().map(str::to_string);
        self.review_setup = None;
        self.editor = Some(Editor {
            draft,
            last_pilot,
            name,
            prompt,
            completion,
            fields: Vec::new(),
            saved,
            dirty: !saved,
            _subscriptions: subscriptions,
        });
        self.rebuild_trigger_fields(cx);
        self.gallery = false;
        self.history = false;
        self.confirm_delete = false;
        self.choice = None;
        self.trigger_menu = None;
        self.models = None;
        self.project_picker = None;
        self.error = None;
        self.load_folders(cx);
        self.load_controls(cx);
        cx.notify();
    }
    fn rebuild_trigger_fields(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let triggers = editor.draft.triggers.clone().unwrap_or_default();
        editor
            .fields
            .retain(|fields| triggers.iter().any(|t| t.id == fields.id));
        for trigger in triggers {
            if editor.fields.iter().any(|f| f.id == trigger.id) {
                continue;
            }
            let field = |label: &str, placeholder: &str, text: String, cx: &mut Context<Self>| {
                cx.new(|cx| {
                    let mut input = Input::field(
                        label.to_string(),
                        placeholder.to_string(),
                        content(0.35),
                        cx,
                    );
                    input.set_text(text, cx);
                    input
                })
            };
            let time = field("Trigger time", "09:00", trigger.time, cx);
            let repos = field(
                "Trigger repositories",
                "Any repository",
                trigger
                    .repos
                    .iter()
                    .chain(std::iter::once(&trigger.repo))
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
                cx,
            );
            let actor = field("Trigger actor", "Anyone", trigger.actor, cx);
            for input in [&time, &repos, &actor] {
                editor._subscriptions.push(cx.subscribe(
                    input,
                    |this, _, _: &input::Changed, cx| {
                        if let Some(e) = &mut this.editor {
                            e.dirty = true;
                        }
                        cx.notify();
                    },
                ));
            }
            editor.fields.push(TriggerFields {
                id: trigger.id,
                time,
                repos,
                actor,
            });
        }
    }
    fn collected(&self, cx: &App) -> Option<AutomationUpsert> {
        let editor = self.editor.as_ref()?;
        let mut draft = editor.draft.clone();
        draft.name = editor.name.read(cx).content.trim().to_string();
        draft.prompt = editor.prompt.read(cx).content.to_string();
        for trigger in draft.triggers.get_or_insert_with(Vec::new) {
            if let Some(fields) = editor.fields.iter().find(|f| f.id == trigger.id) {
                trigger.time = fields.time.read(cx).content.trim().to_string();
                trigger.repo.clear();
                trigger.repos = fields
                    .repos
                    .read(cx)
                    .content
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                trigger.actor = fields.actor.read(cx).content.trim().to_string();
            }
        }
        draft.sync_triggers(gespan_core::time::now_ms() as i64);
        Some(draft)
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.collected(cx) else {
            return;
        };
        self.upsert(draft, cx);
    }
    /// Saves `draft` and opens what was saved, selected in the list.
    fn upsert(&mut self, draft: AutomationUpsert, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.saving = true;
        self.error = None;
        let service = self.service.clone();
        let work = cx.background_executor().spawn(async move {
            service
                .lock()
                .map_err(|_| "Automations unavailable".to_string())?
                .upsert(draft)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(saved) => {
                        this.selected = Some(saved.id.clone());
                        this.set_editor(AutomationUpsert::from_automation(&saved), true, cx);
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
    fn toggle(&mut self, id: String, enabled: bool, cx: &mut Context<Self>) {
        let service = self.service.clone();
        let work = cx.background_executor().spawn(async move {
            service
                .lock()
                .map_err(|_| "Automations unavailable".to_string())?
                .set_enabled(&id, enabled)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(saved) => {
                        if let Some(editor) = &mut this.editor
                            && editor.draft.id == saved.id
                            && !editor.dirty
                        {
                            editor.draft.enabled = saved.enabled;
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                this.refresh(cx);
            });
        })
        .detach();
    }
    fn run(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let Some(automation) = self
            .selected
            .as_ref()
            .and_then(|id| self.automations.iter().find(|a| &a.id == id))
            .cloned()
        else {
            return;
        };
        self.running = true;
        let service = self.service.clone();
        let work = cx.background_executor().spawn(async move {
            let run = service
                .lock()
                .map_err(|_| "Automations unavailable".to_string())?
                .run_now(&automation.id, gespan_core::time::now_ms() as i64)?;
            Ok::<_, String>(DueAutomationRun { automation, run })
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.running = false;
                match result {
                    Ok(run) => cx.emit(Event::Launch(Box::new(run))),
                    Err(error) => this.error = Some(error),
                }
                this.refresh(cx);
            });
        })
        .detach();
        cx.notify();
    }
    /// Asks with the standard confirmation before deleting the open automation.
    fn ask_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = window.prompt(
            gpui::PromptLevel::Warning,
            "Delete this automation?",
            Some("Its run history will be deleted too."),
            &["Delete", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if prompt.await == Ok(0) {
                let _ = this.update(cx, |this, cx| this.delete(cx));
            }
        })
        .detach();
    }
    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let service = self.service.clone();
        let work = cx.background_executor().spawn(async move {
            service
                .lock()
                .map_err(|_| "Automations unavailable".to_string())?
                .delete(&id)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.confirm_delete = false;
                match result {
                    Ok(()) => {
                        this.editor = None;
                        this.selected = None;
                        this.gallery = true;
                        this.runs.clear();
                    }
                    Err(error) => this.error = Some(error),
                }
                this.refresh(cx);
            });
        })
        .detach();
    }
    fn load_controls(&mut self, cx: &mut Context<Self>) {
        self.model_controls.clear();
        self.request_generation += 1;
        let generation = self.request_generation;
        let Some(editor) = &self.editor else {
            return;
        };
        let kind = Kind::from_id(&editor.draft.harness);
        let model = gespan_core::store::native_model(&editor.draft.model, &editor.draft.harness)
            .unwrap_or_default();
        let profile = crate::accounts::get(cx)
            .map(|a| a.read(cx).selected(kind, &editor.draft.cwd))
            .unwrap_or_default();
        let work = cx
            .background_executor()
            .spawn(async move { gespan_core::catalog::models_for(kind, &profile) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.request_generation != generation {
                    return;
                }
                if let Ok(models) = result
                    && let Some(model) = models
                        .iter()
                        .find(|m| m.id == model)
                        .or_else(|| models.iter().find(|m| m.default))
                {
                    this.model_controls = model.controls.clone();
                    if !model.efforts.is_empty()
                        && !this.model_controls.iter().any(|c| {
                            matches!(
                                c.id.as_str(),
                                "reasoningEffort" | "reasoning_effort" | "effort" | "reasoning"
                            )
                        })
                    {
                        this.model_controls
                            .push(gespan_core::catalog::ModelControl {
                                id: "reasoningEffort".into(),
                                name: "Reasoning".into(),
                                current: model
                                    .default_effort
                                    .clone()
                                    .unwrap_or_else(|| "medium".into()),
                                choices: model
                                    .efforts
                                    .iter()
                                    .map(|e| (e.clone(), e.clone()))
                                    .collect(),
                            });
                    }
                    if !model.tiers.is_empty()
                        && !this
                            .model_controls
                            .iter()
                            .any(|c| c.id == "serviceTier" || c.id == "service_tier")
                    {
                        let mut choices = vec![("default".into(), "Standard".into())];
                        choices.extend(model.tiers.clone());
                        this.model_controls
                            .push(gespan_core::catalog::ModelControl {
                                id: "serviceTier".into(),
                                name: "Speed".into(),
                                current: "default".into(),
                                choices,
                            });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn choose_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The review sheet's Model phrase, else the editor's model.
        let (harness, model, cwd) = match (&self.review_setup, &self.editor) {
            (Some(setup), _) => (&setup.input.harness, &setup.input.model, &setup.input.cwd),
            (None, Some(editor)) => (
                &editor.draft.harness,
                &editor.draft.model,
                &editor.draft.cwd,
            ),
            (None, None) => return,
        };
        let kind = Kind::from_id(harness);
        let profile = crate::accounts::get(cx)
            .map(|a| a.read(cx).selected(kind, cwd))
            .unwrap_or_default();
        let picker = cx.new(|cx| {
            ModelList::new_for(
                kind,
                gespan_core::store::native_model(model, harness),
                cwd,
                profile,
                cx,
            )
        });
        self.picker_subscription = Some(cx.subscribe_in(
            &picker,
            window,
            |this, _, picked: &ModelPicked, window, cx| {
                if this.review_setup.is_some() {
                    this.set_review_model(picked.0, picked.1.clone(), cx);
                    this.models = None;
                    window.focus(&this.focus, cx);
                    cx.notify();
                    return;
                }
                if let Some(editor) = &mut this.editor {
                    editor.set_model(picked.0, picked.1.clone(), cx);
                }
                this.models = None;
                this.load_controls(cx);
                window.focus(&this.focus, cx);
                cx.notify();
            },
        ));
        window.focus(&picker.read(cx).search_focus(cx), cx);
        self.models = Some(picker);
        self.choice = None;
        self.trigger_menu = None;
        cx.notify();
    }
    fn choose_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else {
            return;
        };
        let picker = cx.new(|cx| {
            ProjectPicker::new(&self.projects, &editor.draft.cwd, cx)
                .appearance(self.appearances.clone())
        });
        self.picker_subscription = Some(cx.subscribe_in(
            &picker,
            window,
            |this, _, picked: &ProjectPicked, window, cx| {
                this.project_picker = None;
                match picked {
                    ProjectPicked::Project(path) => {
                        if let Some(editor) = &mut this.editor {
                            editor.draft.cwd = path.clone();
                            editor.draft.session_folder_id.clear();
                            editor
                                .completion
                                .update(cx, |completion, cx| completion.set_cwd(path, cx));
                            editor.dirty = true;
                        }
                        this.load_folders(cx);
                        this.load_controls(cx);
                    }
                    ProjectPicked::New => this.browse_project(cx),
                    ProjectPicked::Remote => {}
                }
                window.focus(&this.focus, cx);
                cx.notify();
            },
        ));
        window.focus(&picker.read(cx).search_focus(cx), cx);
        self.project_picker = Some(picker);
        self.choice = None;
        self.trigger_menu = None;
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
                    if let Some(e) = &mut this.editor {
                        e.draft.cwd = path;
                        e.completion
                            .update(cx, |completion, cx| completion.set_cwd(&e.draft.cwd, cx));
                        e.draft.session_folder_id.clear();
                        e.dirty = true;
                    }
                    this.load_folders(cx);
                    this.load_controls(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

fn project_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.into())
}
/// A glass plate button: the page's secondary actions.
fn button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> gpui::Stateful<gpui::Div> {
    glass::button(id, label, glass::Button::Glass).role(Role::Button)
}
/// The one main answer on a sheet: Create, Save, Turn on.
fn primary_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> gpui::Stateful<gpui::Div> {
    glass::button(id, label, glass::Button::Primary).role(Role::Button)
}
/// The quiet way out: Cancel, Reset.
fn ghost_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> gpui::Stateful<gpui::Div> {
    glass::button(id, label, glass::Button::Ghost).role(Role::Button)
}
impl Render for Automations {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.filter.read(cx).content.to_lowercase();
        let visible: Vec<_> = self
            .automations
            .iter()
            .filter(|a| {
                format!("{} {} {}", a.name, a.prompt, a.cwd)
                    .to_lowercase()
                    .contains(query.trim())
            })
            .cloned()
            .collect();
        let list = self.render_list(&visible, &query, cx);
        let aside = div()
            .w(px(340.))
            .flex_shrink_0()
            .border_r_1()
            .border_color(glass::hairline())
            .bg(if is_light() {
                content(0.03)
            } else {
                black(0.12)
            })
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(18.))
                    .pt(px(22.))
                    .pb(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(glass::page_header(
                        "Automations",
                        "Agents that start on their own, on a schedule or an event.",
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                glass::field()
                                    .flex_1()
                                    .min_w_0()
                                    .child(icon("search", 14.).text_color(content(0.55)))
                                    .child(div().flex_1().min_w_0().child(self.filter.clone())),
                            )
                            .child(
                                // Reversed so the plus leads the word.
                                glass::button("new-automation", "New", glass::Button::Primary)
                                    .debug_selector(|| "new-automation".into())
                                    .role(Role::Button)
                                    .aria_label("New automation")
                                    .px(px(12.))
                                    .gap(px(6.))
                                    .flex_row_reverse()
                                    .child(icon("plus", 13.).text_color(background_base()))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_gallery(cx);
                                    })),
                            ),
                    ),
            )
            .child(list);
        let main_content = if self.review_setup.is_some() {
            self.render_review_setup(cx)
        } else if self.gallery || self.editor.is_none() {
            self.render_gallery(cx)
        } else {
            self.render_editor(cx)
        };
        let mut root = div()
            .id("automations-view")
            .role(Role::Region)
            .aria_label("Automations")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .text_color(content(1.))
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    // Let the trigger menu close its active submenu first.
                    if this.trigger_menu.is_some() {
                        cx.propagate();
                        return;
                    }
                    if this.confirm_delete {
                        this.confirm_delete = false;
                    } else if this.choice.is_some()
                        || this.trigger_menu.is_some()
                        || this.models.is_some()
                        || this.project_picker.is_some()
                    {
                        this.choice = None;
                        this.trigger_menu = None;
                        this.models = None;
                        this.project_picker = None;
                    } else if this.review_setup.is_some()
                        || (this.editor.is_some() && !this.gallery)
                    {
                        this.show_gallery(cx);
                    } else {
                        cx.emit(Event::Close);
                    }
                    cx.stop_propagation();
                    cx.notify();
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
                            d.child(
                                div()
                                    .m(px(16.))
                                    .p(px(10.))
                                    .rounded(px(8.))
                                    .bg(gpui::rgba(0xf8717114))
                                    .text_size(px(12.))
                                    .text_color(rgb(0xfca5a5))
                                    .child(error),
                            )
                        })
                        .child(main_content),
                ),
            );
        if let Some(menu) = &self.trigger_menu {
            root = root.child(
                menu.read(cx)
                    .popover(Anchor::Slot(self.trigger_anchor.clone()), cx),
            );
        }
        if let Some(menu) = &self.choice {
            let weak = cx.weak_entity();
            let field = menu.field.clone();
            root = root.child(
                popover("automation-choice", Anchor::Slot(menu.anchor.clone()))
                    .width(250.)
                    .on_dismiss(move |_, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.choice = None;
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .id("automation-choices")
                            .max_h(px(360.))
                            .overflow_y_scroll()
                            .p(px(4.))
                            .children(menu.options.iter().enumerate().map(
                                |(index, (value, label))| {
                                    let field = field.clone();
                                    let value = value.clone();
                                    div()
                                        .id(("automation-choice", index))
                                        .role(Role::MenuItem)
                                        .aria_label(label.clone())
                                        .px(px(8.))
                                        .py(px(8.))
                                        .rounded(px(6.))
                                        .text_size(px(13.))
                                        .hover(|s| s.bg(content(0.07)))
                                        .child(label.clone())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.apply_choice(field.clone(), value.clone(), cx)
                                        }))
                                },
                            )),
                    ),
            );
        }
        if let Some(picker) = &self.models {
            let weak = cx.weak_entity();
            root = root.child(
                popover(
                    "automation-model-picker",
                    Anchor::Slot(self.model_anchor.clone()),
                )
                .width(340.)
                .on_dismiss(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.models = None;
                        cx.notify();
                    });
                })
                .child(self.render_model_settings(cx))
                .child(div().h(px(440.)).child(picker.clone())),
            );
        }
        if let Some(picker) = &self.project_picker {
            let weak = cx.weak_entity();
            root = root.child(
                popover(
                    "automation-project-picker",
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
