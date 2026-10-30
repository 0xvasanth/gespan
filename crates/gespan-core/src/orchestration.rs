//! Reviewable orchestration plans. Workers share a checkout; overlapping scopes
//! are serialized even when they have no explicit dependency.
use crate::{
    accounts::Profile,
    agent::Settings,
    catalog::Model,
    provider::{self, Kind},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Component, Path},
};

/// orchestrationSummary.ts: what the sidebar shows of a lead's run, kept after
/// it ends so the lead still lists its agents.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Summary {
    pub tasks: Vec<SummaryTask>,
    /// The run is still dispatching or waiting on its workers.
    pub live: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SummaryTask {
    pub session: Option<i64>,
    pub title: String,
    pub provider: String,
    pub model: Option<String>,
    /// queued, running, completed, failed or blocked.
    pub status: String,
}
impl SummaryTask {
    /// orchestrationTaskLabel, given whether the worker needs input.
    pub fn label(&self, summary: &Summary, needs_input: bool) -> &'static str {
        if needs_input {
            return "Needs input";
        }
        if !summary.live && matches!(self.status.as_str(), "running" | "queued") {
            return "Saved";
        }
        match self.status.as_str() {
            "queued" => "Queued",
            "running" => "Working",
            "completed" => "Done",
            "failed" => "Failed",
            "blocked" => "Needs review",
            _ => "Queued",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub files: Vec<String>,
    #[serde(default, alias = "dependsOn")]
    pub depends_on: Vec<String>,
    /// The worker's harness; without one it uses the lead's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// The worker's model; without one it follows the harness default, or the
    /// lead's model when the harness is the lead's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Effort and the other model controls chosen on the card.
    #[serde(
        default,
        alias = "modelSettings",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub model_settings: BTreeMap<String, String>,
}
impl Task {
    /// The harness this task runs on, given the lead's.
    pub fn kind(&self, lead_provider: &str) -> Kind {
        Kind::from_id(self.harness.as_deref().unwrap_or(lead_provider))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub title: String,
    pub summary: String,
    pub tasks: Vec<Task>,
}
impl Plan {
    /// The card's model pick. Settings belong to the model they were set on.
    pub fn set_task_model(&mut self, id: &str, kind: Kind, model: Option<String>) {
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            task.harness = Some(kind.id().to_owned());
            task.model = model;
            task.model_settings.clear();
        }
    }
    pub fn set_task_setting(&mut self, id: &str, key: &str, value: &str) {
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            task.model_settings.insert(key.to_owned(), value.to_owned());
        }
    }
}

/// One harness/model pair a lead may assign to a worker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub harness: String,
    pub model: String,
    pub name: String,
}

/// More would crowd out the instructions in the planning prompt.
const MAX_CHOICES: usize = 200;

/// Installed harnesses and their models, cached catalogs first. Discovery
/// runs on workers, so a slow harness delays only its own entries.
pub fn choices(profiles: Vec<(Kind, Profile)>) -> Vec<Choice> {
    let jobs: Vec<_> = profiles
        .into_iter()
        .map(|(kind, profile)| {
            std::thread::spawn(move || {
                if provider::resolve(kind).is_none() {
                    return Vec::new();
                }
                crate::catalog::cached_models_for(kind, &profile)
                    .filter(|models| !models.is_empty())
                    .or_else(|| crate::catalog::models_for(kind, &profile).ok())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|model| Choice {
                        harness: kind.id().to_owned(),
                        model: model.id,
                        name: model.name,
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    jobs.into_iter()
        .filter_map(|job| job.join().ok())
        .flatten()
        .take(MAX_CHOICES)
        .collect()
}

pub fn instructions(choices: &[Choice]) -> String {
    let workers = if choices.is_empty() {
        "Each worker uses this session's provider, model and access policy.".to_owned()
    } else {
        format!(
            "Choose each worker's harness and model from the catalog below, using exact pairs. Prefer a fast, economical model for straightforward work and a more capable one when complexity warrants it; do not invent model capabilities. Reuse a suitable pair across tasks when that is sufficient. The user can change your choices before starting. Workers use this session's access policy.\n<available_models>\n{}\n</available_models>",
            serde_json::to_string(choices).unwrap_or_default()
        )
    };
    format!(
        r#"Prepare an orchestration plan for the user's request. Inspect the project with read-only tools. Do not edit or launch workers yet. The user will review and start the plan in Gespan.
Return a short explanation followed by exactly one JSON block between <gespan_orchestration> and </gespan_orchestration>.
Schema: {{"title":"short title","summary":"approach","tasks":[{{"id":"task-1","title":"assignment title","prompt":"complete self-contained instructions, validation and expected report","harness":"exact harness ID","model":"exact model ID","files":["src/path"],"depends_on":[]}}]}}
Use 1 to 12 assignments, unique ASCII IDs, exact project-relative file or directory scopes ('.' for the whole project), and dependency IDs. Independent assignments may run concurrently (up to four); overlapping scopes run serially. {workers} Include verification in the assignments. Never claim that workers have already started."#
    )
}
pub fn parse(text: &str) -> Result<Plan, String> {
    let body = text
        .split_once("<gespan_orchestration>")
        .and_then(|(_, s)| s.split_once("</gespan_orchestration>").map(|(s, _)| s))
        .ok_or("The response did not include a task plan. Retry Orchestrator to prepare one.")?;
    if body.len() > 256 * 1024 {
        return Err("Task plan is too large.".into());
    }
    let plan: Plan =
        serde_json::from_str(body.trim()).map_err(|e| format!("Invalid task plan: {e}"))?;
    validate(&plan)?;
    Ok(plan)
}
pub fn validate(plan: &Plan) -> Result<(), String> {
    if plan.title.trim().is_empty()
        || plan.title.len() > 160
        || plan.summary.len() > 4_000
        || plan.tasks.is_empty()
        || plan.tasks.len() > 12
    {
        return Err("Provide a title and 1–12 assignments.".into());
    }
    let mut ids = HashSet::new();
    for task in &plan.tasks {
        if task.id.is_empty()
            || task.id.len() > 64
            || !task
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !ids.insert(task.id.as_str())
        {
            return Err(
                "Task IDs must be unique and contain only letters, numbers, '-' and '_'.".into(),
            );
        }
        if task.title.trim().is_empty()
            || task.title.len() > 160
            || task.prompt.trim().is_empty()
            || task.prompt.len() > 30_000
            || task.files.is_empty()
            || task.files.len() > 64
            || task.depends_on.len() > 12
        {
            return Err(format!(
                "Assignment {} needs a title, instructions and file scopes.",
                task.id
            ));
        }
        if task
            .harness
            .as_deref()
            .is_some_and(|harness| Kind::parse(harness).is_none())
            || task.model.as_ref().is_some_and(|model| model.len() > 256)
            || task.model_settings.len() > 32
        {
            return Err(format!(
                "Assignment {} has an unknown harness or model.",
                task.id
            ));
        }
        for file in &task.files {
            if file.trim().is_empty()
                || file.len() > 512
                || file.contains('\\')
                || file.contains(':')
                || Path::new(file).is_absolute()
                || Path::new(file).components().any(|c| {
                    matches!(
                        c,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                return Err(format!("Invalid file scope: {file}"));
            }
        }
    }
    let mut completed = HashSet::new();
    loop {
        let before = completed.len();
        for task in &plan.tasks {
            if task
                .depends_on
                .iter()
                .all(|id| completed.contains(id.as_str()))
            {
                completed.insert(task.id.as_str());
            }
        }
        if completed.len() == plan.tasks.len() {
            return Ok(());
        }
        if completed.len() == before {
            return Err("Task dependencies contain a cycle or unknown task ID.".into());
        }
    }
}
pub fn overlaps(a: &[String], b: &[String]) -> bool {
    a.iter().any(|a| {
        b.iter().any(|b| {
            a == "." || b == "." || Path::new(a).starts_with(b) || Path::new(b).starts_with(a)
        })
    })
}
/// A task with its own choice needs the harness's live model list to check it.
pub fn needs_catalog(task: &Task, lead_provider: &str) -> bool {
    task.model.is_some()
        || !task.model_settings.is_empty()
        || task.kind(lead_provider).id() != lead_provider
}

/// The lead's settings with this task's harness, model and controls. The
/// access policy stays the lead's. `lead` carries the account for `kind`.
pub fn worker_settings(
    task: &Task,
    lead_provider: &str,
    lead: Settings,
    kind: Kind,
    models: &[Model],
) -> Result<Settings, String> {
    let wanted = task
        .model
        .as_deref()
        .and_then(|model| crate::store::native_model(model, kind.id()));
    if wanted
        .as_ref()
        .is_some_and(|model| !models.iter().any(|m| m.id == *model))
    {
        return Err(UNAVAILABLE.into());
    }
    let mut input = json!({"prompt": task.prompt});
    if let Some(model) = &task.model {
        input["model"] = json!(model);
    }
    if !task.model_settings.is_empty() {
        input["modelSettings"] = json!(task.model_settings);
    }
    let mut settings =
        crate::app_access::launch_settings(&input, lead_provider, lead, kind, models)?;
    settings.orchestration_worker = true;
    Ok(settings)
}
pub const UNAVAILABLE: &str =
    "The assigned harness/model is no longer available. Review this task before retrying.";
pub fn worker_prompt(task: &Task, previous: &str) -> String {
    format!("You are completing assignment {} in a Gespan orchestration.\n{}\n\n{}\n\nWrite only within these project-relative scopes: {}. Other agents may work in this checkout; preserve their changes and do not reset the repository. If you need a change outside your scopes, report the blocker. Run focused checks and report your changes, validation and any remaining issues.\n\nDependency reports:\n{}", task.id, task.title, task.prompt, task.files.join(", "), previous)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn task(id: &str, deps: &[&str], file: &str) -> Task {
        Task {
            id: id.into(),
            title: id.into(),
            prompt: "Work".into(),
            files: vec![file.into()],
            depends_on: deps.iter().map(|s| (*s).into()).collect(),
            harness: None,
            model: None,
            model_settings: BTreeMap::new(),
        }
    }
    fn model(id: &str, efforts: &[&str]) -> Model {
        Model {
            id: id.into(),
            name: id.into(),
            alias_target: None,
            efforts: efforts.iter().map(|e| (*e).into()).collect(),
            default_effort: None,
            fast: false,
            thinking: false,
            default: false,
            tiers: Vec::new(),
            controls: Vec::new(),
        }
    }
    fn plan_text(tasks: serde_json::Value) -> String {
        format!(
            "Plan.\n<gespan_orchestration>{}</gespan_orchestration>",
            json!({"title":"Work","summary":"","tasks":tasks})
        )
    }
    #[test]
    fn plans_carry_each_tasks_harness_model_and_settings() {
        let plan = parse(&plan_text(json!([
            {"id":"a","title":"A","prompt":"Do","files":["src"],"harness":"codex","model":"gpt-6",
             "modelSettings":{"reasoningEffort":"high"},"dependsOn":[]},
            {"id":"b","title":"B","prompt":"Do","files":["docs"],"depends_on":["a"]}
        ])))
        .unwrap();
        assert_eq!(plan.tasks[0].kind("claude"), Kind::Codex);
        assert_eq!(plan.tasks[0].model.as_deref(), Some("gpt-6"));
        assert_eq!(plan.tasks[0].model_settings["reasoningEffort"], "high");
        assert_eq!(plan.tasks[1].kind("claude"), Kind::Claude);
        assert!(plan.tasks[1].model.is_none());
    }
    #[test]
    fn a_plan_naming_an_unknown_harness_is_refused() {
        let text = plan_text(json!([
            {"id":"a","title":"A","prompt":"Do","files":["src"],"harness":"nope"}
        ]));
        assert!(parse(&text).unwrap_err().contains("unknown harness"));
    }
    #[test]
    fn picking_a_model_on_the_card_drops_the_old_models_settings() {
        let mut plan = Plan {
            title: "Work".into(),
            summary: String::new(),
            tasks: vec![task("a", &[], "src")],
        };
        plan.set_task_setting("a", "effort", "high");
        plan.set_task_model("a", Kind::Codex, Some("gpt-6".into()));
        assert_eq!(plan.tasks[0].harness.as_deref(), Some("codex"));
        assert!(plan.tasks[0].model_settings.is_empty());
        plan.set_task_setting("a", "reasoningEffort", "low");
        plan.set_task_setting("missing", "effort", "high");
        assert_eq!(plan.tasks[0].model_settings.len(), 1);
    }
    #[test]
    fn a_worker_gets_its_tasks_model_and_effort_on_the_leads_access() {
        let lead = Settings {
            model: Some("claude-opus-5".into()),
            effort: Some("max".into()),
            access: crate::agent::AccessMode::ReadOnly,
            ..Settings::default()
        };
        let mut assigned = task("a", &[], "src");
        assigned.harness = Some("codex".into());
        assigned.model = Some("gpt-6".into());
        assigned
            .model_settings
            .insert("reasoningEffort".into(), "high".into());
        let models = [model("gpt-5", &[]), model("gpt-6", &["low", "high"])];
        let settings =
            worker_settings(&assigned, "claude", lead.clone(), Kind::Codex, &models).unwrap();
        assert_eq!(settings.model.as_deref(), Some("gpt-6"));
        assert_eq!(settings.effort.as_deref(), Some("high"));
        assert_eq!(settings.access, crate::agent::AccessMode::ReadOnly);
        assert!(settings.orchestration_worker);
        assert!(needs_catalog(&assigned, "claude"));
        // No choice of its own: the lead's model, with no catalog to fetch.
        let plain = task("b", &[], "src");
        assert!(!needs_catalog(&plain, "claude"));
        let settings = worker_settings(&plain, "claude", lead, Kind::Claude, &[]).unwrap();
        assert_eq!(settings.model.as_deref(), Some("claude-opus-5"));
    }
    #[test]
    fn a_model_gone_from_the_catalog_fails_with_the_review_message() {
        let mut assigned = task("a", &[], "src");
        assigned.harness = Some("codex".into());
        assigned.model = Some("gpt-6".into());
        let error = worker_settings(
            &assigned,
            "claude",
            Settings::default(),
            Kind::Codex,
            &[model("gpt-5", &[])],
        )
        .unwrap_err();
        assert_eq!(error, UNAVAILABLE);
    }
    #[test]
    fn the_planning_prompt_lists_the_catalog_only_when_there_is_one() {
        let choice = Choice {
            harness: "codex".into(),
            model: "gpt-6".into(),
            name: "GPT-6".into(),
        };
        let text = instructions(&[choice]);
        assert!(text.contains(r#"{"harness":"codex","model":"gpt-6","name":"GPT-6"}"#));
        assert!(text.contains(r#""harness":"exact harness ID""#));
        assert!(!text.contains("Each worker uses this session's provider"));
        let text = instructions(&[]);
        assert!(text.contains("Each worker uses this session's provider"));
        assert!(!text.contains("<available_models>"));
    }
    #[test]
    fn rejects_cycles_unknown_dependencies_and_scope_escapes() {
        let mut plan = Plan {
            title: "Work".into(),
            summary: String::new(),
            tasks: vec![task("a", &["b"], "src"), task("b", &["a"], "tests")],
        };
        assert!(validate(&plan).is_err());
        plan.tasks[1].depends_on.clear();
        assert!(validate(&plan).is_ok());
        plan.tasks[1].files = vec!["../elsewhere".into()];
        assert!(validate(&plan).is_err());
        assert!(overlaps(&["src".into()], &["src/main.rs".into()]));
        assert!(!overlaps(&["src".into()], &["src2/main.rs".into()]));
    }
}
