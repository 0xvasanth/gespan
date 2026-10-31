//! Host-owned plan previews and their read-only Markdown reader.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone, Default, Deserialize, Serialize)]
pub(crate) struct Plan {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub status: String,
}
impl Plan {
    /// A plan this app's own plan-mode turn left at source row `at`.
    pub(crate) fn local(at: usize, text: String, status: &str) -> Self {
        Self {
            id: format!("{LOCAL}{at}"),
            text,
            streaming: false,
            status: status.into(),
        }
    }
    pub(crate) fn is_local(&self) -> bool {
        self.id.starts_with(LOCAL)
    }
    pub(crate) fn can_build(&self, busy: bool) -> bool {
        !busy
            && !self.streaming
            && !self.text.trim().is_empty()
            && !matches!(self.status.as_str(), "streaming" | "building" | "built")
    }
    pub(crate) fn build_label(&self) -> &'static str {
        match self.status.as_str() {
            "building" => "Building…",
            "built" => "Built",
            _ => "Build",
        }
    }
    pub(crate) fn title(&self) -> String {
        self.text
            .lines()
            .find_map(|line| {
                let line = line.trim();
                if line.starts_with('#') {
                    Some(line.trim_start_matches('#').trim().to_owned())
                } else {
                    None
                }
            })
            .or_else(|| {
                self.text
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty() && !line.starts_with("```") && *line != "---")
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "Plan".into())
            .chars()
            .take(80)
            .collect()
    }
    pub(crate) fn summary(&self) -> String {
        let title = self.title();
        let text = self
            .text
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.is_empty()
                    && !line.starts_with('#')
                    && !line.starts_with("```")
                    && *line != title
            })
            .take(3)
            .collect::<Vec<_>>()
            .join(" ");
        if text.chars().count() > 140 {
            format!("{}…", text.chars().take(139).collect::<String>())
        } else {
            text
        }
    }
    fn command(&self) -> Value {
        json!({"type":"send","intent":"build","planBlockId":self.id})
    }
    fn prompt(&self) -> String {
        format!("Build the approved plan:\n\n{}", self.text)
    }
}
const LOCAL: &str = "local:";
struct Reader {
    id: String,
    text: String,
    view: Entity<TranscriptView>,
}
#[derive(Default)]
pub(super) struct State {
    plans: HashMap<String, Plan>,
    reader: Option<Reader>,
    /// The plan the open model menu builds once a model is picked.
    pub(super) build_with: Option<String>,
}

#[cfg(test)]
impl State {
    pub(super) fn reader_text(&self) -> Option<&str> {
        self.reader.as_ref().map(|reader| reader.text.as_str())
    }
}

/// What a local plan card asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlanAction {
    Open,
    Build,
    BuildWith,
}

#[cfg(test)]
impl Conversation {
    pub(crate) fn open_plan_reader_for_test(&mut self, text: &str, cx: &mut Context<Self>) {
        self.remote_plans.reader = Some(Reader {
            id: "plan".into(),
            text: text.into(),
            view: cx.new(|cx| TranscriptView::document(text.into(), cx)),
        });
        cx.notify();
    }
    pub(crate) fn plan_reader_open(&self) -> bool {
        self.remote_plans.reader.is_some()
    }
}

impl Conversation {
    pub(super) fn close_remote_plan(&mut self, cx: &mut Context<Self>) -> bool {
        if self.remote_plans.reader.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }
    pub(super) fn install_remote_plans(&mut self, snapshot: &Value, cx: &mut Context<Self>) {
        self.remote_plans.plans = snapshot["session"]["blocks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|block| block["role"] == "plan")
            .filter_map(|block| {
                Some((
                    block["id"].as_str()?.to_owned(),
                    Plan {
                        id: block["id"].as_str()?.into(),
                        text: block["text"].as_str().unwrap_or("").into(),
                        streaming: block["streaming"] == true,
                        status: block["plan"]["status"].as_str().unwrap_or("ready").into(),
                    },
                ))
            })
            .collect();
        if let Some(reader) = self.remote_plans.reader.as_mut()
            && let Some(plan) = self.remote_plans.plans.get(&reader.id)
            && reader.text != plan.text
        {
            reader.text = plan.text.clone();
            reader.view = cx.new(|cx| TranscriptView::document(plan.text.clone(), cx));
        }
    }
    /// PlanPreview's actions on a plan this app's own plan-mode turn wrote.
    pub(super) fn local_plan_request(
        &mut self,
        plan: &Plan,
        action: PlanAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_remote() || !plan.is_local() {
            return;
        }
        match action {
            PlanAction::Open => {
                self.remote_plans
                    .plans
                    .insert(plan.id.clone(), plan.clone());
                self.remote_plans.reader = Some(Reader {
                    id: plan.id.clone(),
                    text: plan.text.clone(),
                    view: cx.new(|cx| TranscriptView::document(plan.text.clone(), cx)),
                });
                cx.notify();
            }
            PlanAction::Build => self.build_local_plan(plan, cx),
            PlanAction::BuildWith => {
                if !self.can_build_local(plan) {
                    return;
                }
                if !self.menu_is(ComposerMenu::Model) {
                    self.toggle_menu(ComposerMenu::Model, window, cx);
                }
                // Set after opening: opening the menu forgets an earlier plan.
                self.remote_plans.build_with =
                    self.menu_is(ComposerMenu::Model).then(|| plan.text.clone());
            }
        }
    }
    fn can_build_local(&self, plan: &Plan) -> bool {
        plan.can_build(self.busy || self.stopping || !self.loaded)
    }
    /// Build sends the approved plan as a normal turn with plan mode off, so
    /// the provider may change files.
    fn build_local_plan(&mut self, plan: &Plan, cx: &mut Context<Self>) {
        if !self.can_build_local(plan) {
            return;
        }
        self.remote_plans.reader = None;
        self.remote_plans.build_with = None;
        self.clear_submitted_mode(cx);
        self.launch_prompt(gespan_core::plan::build_prompt(&plan.text), cx);
    }
    /// The model menu opened by Build with another model: switch, then build.
    pub(super) fn build_plan_with(&mut self, picked: &ModelPicked, cx: &mut Context<Self>) -> bool {
        let Some(text) = self.remote_plans.build_with.take() else {
            return false;
        };
        if self.apply_model_pick(picked, cx) {
            self.build_local_plan(&Plan::local(0, text, "ready"), cx);
        }
        true
    }
    pub(super) fn remote_plan_request(
        &mut self,
        id: &str,
        text: &str,
        build: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(plan) = self
            .remote_plans
            .plans
            .get(id)
            .filter(|plan| plan.is_local())
        {
            if build {
                let plan = plan.clone();
                self.build_local_plan(&plan, cx);
            }
            return;
        }
        if !self.is_remote() {
            return;
        }
        if !build {
            self.remote_plans.reader = Some(Reader {
                id: id.into(),
                text: text.into(),
                view: cx.new(|cx| TranscriptView::document(text.into(), cx)),
            });
            cx.notify();
            return;
        }
        if !self.loaded
            || !self.remote_state.online
            || self.remote_state.client.is_none()
            || self.remote_state.pending
            || self.busy
            || self.stopping
        {
            self.status =
                "Reconnect and wait for the remote turn to finish before building this plan."
                    .into();
            cx.notify();
            return;
        }
        if !self.remote_state.capabilities.contains("sessions.plan") {
            self.status = "Update Gespan Host in Connections settings to build plans.".into();
            cx.notify();
            return;
        }
        let Some(plan) = self
            .remote_plans
            .plans
            .get(id)
            .cloned()
            .filter(|plan| plan.can_build(false))
        else {
            return;
        };
        self.settings.remote_action = Some(plan.command());
        self.remote_plans.reader = None;
        // Launch the host command directly: the composer draft and its local
        // attachments belong to the user's next message, not this approval.
        self.launch_prompt(plan.prompt(), cx);
    }
    pub(super) fn remote_plan_reader(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let reader = self.remote_plans.reader.as_ref()?;
        let id = reader.id.clone();
        let build = self.remote_plans.plans.get(&id).is_some_and(|plan| {
            if plan.is_local() {
                return self.can_build_local(plan);
            }
            plan.can_build(
                self.busy
                    || self.stopping
                    || !self.remote_state.online
                    || self.remote_state.pending,
            )
        });
        let title = self
            .remote_plans
            .plans
            .get(&id)
            .map(Plan::title)
            .unwrap_or_else(|| "Plan".into());
        let mut fill = pane_fill();
        fill.a = 1.;
        Some(
            div()
                .id("remote-plan-reader")
                .absolute()
                .inset_0()
                .bg(fill)
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(
                    div()
                        .h(px(42.))
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_b_1()
                        .border_color(content(0.1))
                        .child(icon("file-text", 16.))
                        .child(div().flex_1().truncate().text_size(px(13.)).child(title))
                        .child(
                            crate::settings::widgets::secondary_button(
                                "remote-reader-build",
                                "Build approved plan",
                                false,
                                !build,
                            )
                            .child("Build")
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if build {
                                        this.remote_plan_request(&id, "", true, cx);
                                    }
                                },
                            )),
                        )
                        .child(
                            icon_button("close-remote-plan", "Close plan", "x").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.remote_plans.reader = None;
                                    cx.notify();
                                }),
                            ),
                        ),
                )
                .child(div().flex_1().min_h_0().child(reader.view.clone()))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approved_plan_actions_keep_host_identity_and_exact_text() {
        let mut plan = Plan {
            id: "host-plan".into(),
            text: "# Plan\n\n- Keep exact text\n".into(),
            status: "ready".into(),
            streaming: false,
        };
        assert!(plan.can_build(false));
        assert_eq!(
            plan.command(),
            json!({"type":"send","intent":"build","planBlockId":"host-plan"})
        );
        assert_eq!(
            plan.prompt(),
            "Build the approved plan:\n\n# Plan\n\n- Keep exact text\n"
        );
        for state in ["streaming", "building", "built"] {
            plan.status = state.into();
            assert!(!plan.can_build(false));
        }
        plan.status = "ready".into();
        assert!(!plan.can_build(true));
        plan.streaming = true;
        assert!(!plan.can_build(false));
    }
}
