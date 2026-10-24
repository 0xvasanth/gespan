//! The run dialog and the run page: header, visit timeline, approvals,
//! cancel and "Resume from…".
use super::{
    ui::{Tooltip, button, disabled, run_status_label, status_chip},
    view::{Event, FlowsView, Page},
};
use crate::{
    input::{self, Input},
    modal,
    theme::*,
};
use gpui::{AnyElement, Context, Entity, SharedString, Subscription, Window, div, prelude::*, px};
use rebase_core::flow::{
    Flow, FlowInput, FlowRun, FlowState, FlowVisit, RunStatus, VisitStatus, resolve_inputs,
};
use std::collections::{HashMap, HashSet};

pub(super) const ERROR_PREVIEW: usize = 300;

/// A visit error for the timeline: its first line, at most 300 characters.
pub(super) fn short_error(error: &str) -> String {
    let line = error.lines().next().unwrap_or_default();
    let more = line.len() < error.trim_end().len();
    if !more && line.chars().count() <= ERROR_PREVIEW {
        return line.to_owned();
    }
    let mut short: String = line.chars().take(ERROR_PREVIEW - 1).collect();
    short.push('…');
    short
}

/// `resolve_inputs`, with each missing input named by its label.
pub(super) fn checked_inputs(
    flow: &Flow,
    given: &HashMap<String, String>,
) -> Result<HashMap<String, String>, Vec<String>> {
    resolve_inputs(flow, given).map_err(|missing| {
        missing
            .iter()
            .map(|name| {
                let label = flow
                    .inputs
                    .iter()
                    .find(|input| &input.name == name)
                    .map_or(name.as_str(), |input| input.label.as_str());
                format!("{label} is required")
            })
            .collect()
    })
}

/// The state "Resume from…" starts on: the one that failed or was cancelled.
pub(super) fn default_resume_state(run: &FlowRun, visits: &[FlowVisit]) -> Option<String> {
    visits
        .iter()
        .filter(|v| matches!(v.status, VisitStatus::Failed | VisitStatus::Cancelled))
        .max_by_key(|v| v.seq)
        .map(|v| v.state_id.clone())
        .or_else(|| run.current_state.clone())
        .or_else(|| Some(run.flow.initial.clone()))
        .filter(|id| run.flow.states.iter().any(|s| s.id() == id))
}

pub(super) fn duration(visit: &FlowVisit, now: i64) -> String {
    let end = visit.finished_at.unwrap_or(now);
    let seconds = (end - visit.started_at).max(0) / 1000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

fn visit_status_label(status: VisitStatus) -> &'static str {
    match status {
        VisitStatus::Running => "Running",
        VisitStatus::WaitingApproval => "Needs approval",
        VisitStatus::Completed => "Completed",
        VisitStatus::Failed => "Failed",
        VisitStatus::Cancelled => "Cancelled",
    }
}

fn state_title(flow: &Flow, id: &str) -> String {
    flow.states
        .iter()
        .find(|s| s.id() == id)
        .map(|state| match state {
            FlowState::Agent(s) => s.title.clone(),
            FlowState::Approval(s) => s.title.clone(),
        })
        .unwrap_or_else(|| id.to_owned())
}

fn is_agent(flow: &Flow, id: &str) -> bool {
    flow.states
        .iter()
        .any(|s| s.id() == id && matches!(s, FlowState::Agent(_)))
}

pub(super) struct RunDialog {
    pub flow: Flow,
    pub fields: Vec<(FlowInput, Entity<Input>)>,
    _subscriptions: Vec<Subscription>,
}

impl RunDialog {
    pub(super) fn values(&self, cx: &gpui::App) -> HashMap<String, String> {
        self.fields
            .iter()
            .map(|(input, field)| (input.name.clone(), field.read(cx).content.to_string()))
            .collect()
    }
}

pub(super) struct RunPage {
    pub run_id: String,
    pub run: FlowRun,
    pub visits: Vec<FlowVisit>,
    pub expanded: HashSet<String>,
    pub note: Entity<Input>,
    /// The "Resume from…" choice; `None` until the user picks one.
    pub resume: Option<String>,
}

impl RunPage {
    pub(super) fn new(run: FlowRun, cx: &mut Context<FlowsView>) -> Self {
        Self {
            run_id: run.id.clone(),
            run,
            visits: Vec::new(),
            expanded: HashSet::new(),
            note: cx.new(|cx| Input::memory_fact("Approval note", cx)),
            resume: None,
        }
    }

    pub(super) fn loaded(&mut self, run: Option<FlowRun>, mut visits: Vec<FlowVisit>) {
        if let Some(run) = run {
            self.run = run;
        }
        visits.sort_by_key(|v| v.seq);
        self.visits = visits;
    }

    pub(super) fn flow_id(&self) -> Option<&str> {
        Some(self.run.flow_id.as_str())
    }

    pub(super) fn resume_state(&self) -> Option<String> {
        self.resume
            .clone()
            .or_else(|| default_resume_state(&self.run, &self.visits))
    }

    fn can_resume(&self) -> bool {
        matches!(self.run.status, RunStatus::Failed | RunStatus::Cancelled)
    }
}

impl FlowsView {
    pub(super) fn open_run_dialog(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(flow) = self.flows.iter().find(|f| f.id == id).cloned() else {
            return;
        };
        let mut subscriptions = Vec::new();
        let fields: Vec<_> = flow
            .inputs
            .iter()
            .map(|input| {
                let field = cx.new(|cx| {
                    let mut field = Input::memory_fact(&input.label, cx);
                    field.set_text(input.default.clone().unwrap_or_default(), cx);
                    field
                });
                subscriptions
                    .push(cx.subscribe(&field, |_, _, _: &input::Changed, cx| cx.notify()));
                subscriptions.push(cx.subscribe(&field, |this, _, _: &input::Submitted, cx| {
                    this.start_run(cx)
                }));
                (input.clone(), field)
            })
            .collect();
        if let Some((_, first)) = fields.first() {
            window.focus(&gpui::Focusable::focus_handle(first.read(cx), cx), cx);
        }
        self.dialog = Some(RunDialog {
            flow,
            fields,
            _subscriptions: subscriptions,
        });
        self.error = None;
        cx.notify();
    }

    pub(super) fn start_run(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &self.dialog else {
            return;
        };
        let Ok(inputs) = checked_inputs(&dialog.flow, &dialog.values(cx)) else {
            return;
        };
        let flow = Box::new(dialog.flow.clone());
        self.dialog = None;
        cx.emit(Event::Start { flow, inputs });
        cx.notify();
    }

    pub(super) fn render_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(dialog) = &self.dialog else {
            return div().into_any_element();
        };
        let checked = checked_inputs(&dialog.flow, &dialog.values(cx));
        let missing = checked.as_ref().err().cloned().unwrap_or_default();
        let mut body = modal::card_body()
            .gap(px(12.))
            .child(modal::title(format!("Run {}", dialog.flow.name)));
        if dialog.fields.is_empty() {
            body = body.child(modal::body("This Flow has no inputs."));
        }
        for (input, field) in &dialog.fields {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex()
                            .gap(px(4.))
                            .text_size(px(12.))
                            .text_color(content(0.7))
                            .child(input.label.clone())
                            .when(input.required, |d| {
                                d.child(div().text_color(content(0.45)).child("· Required"))
                            }),
                    )
                    .child(
                        div()
                            .px(px(8.))
                            .py(px(6.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(content(0.15))
                            .child(field.clone()),
                    ),
            );
        }
        for message in &missing {
            body = body.child(
                div()
                    .text_size(px(11.))
                    .text_color(content(0.5))
                    .child(message.clone()),
            );
        }
        let blocked = checked.is_err();
        body = body.child(
            modal::actions()
                .child(
                    modal::ghost_button("flow-run-cancel", "Cancel").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.dialog = None;
                            cx.notify();
                        },
                    )),
                )
                .child(
                    modal::disabled(modal::primary_button("flow-run-start", "Run"), blocked)
                        .debug_selector(|| "flow-run-start".into())
                        .on_click(cx.listener(|this, _, _, cx| this.start_run(cx))),
                ),
        );
        let weak = cx.weak_entity();
        modal::overlay(
            "flow-run-dialog",
            modal::card().w(px(420.)).child(body),
            move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.dialog = None;
                    cx.notify();
                });
            },
        )
        .into_any_element()
    }

    pub(super) fn render_run(&self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Page::Run(page) = &self.page else {
            return div().into_any_element();
        };
        let run = &page.run;
        let now = super::view::now();
        let mut header = div()
            .flex_shrink_0()
            .px(px(24.))
            .py(px(14.))
            .border_b_1()
            .border_color(content(0.07))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(status_chip(
                        run.status.as_str(),
                        run_status_label(run.status),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(16.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(run.flow.name.clone()),
                    )
                    .when(run.status.is_active(), |d| {
                        let id = run.id.clone();
                        d.child(
                            button("flow-cancel", "Cancel run")
                                .debug_selector(|| "flow-cancel".into())
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(Event::Cancel(id.clone()))
                                })),
                        )
                    }),
            );
        let mut inputs: Vec<_> = run.inputs.iter().filter(|(_, v)| !v.is_empty()).collect();
        inputs.sort();
        for (name, value) in inputs {
            let label = run
                .flow
                .inputs
                .iter()
                .find(|i| &i.name == name)
                .map_or(name.clone(), |i| i.label.clone());
            header = header.child(
                div()
                    .flex()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .child(div().flex_shrink_0().text_color(content(0.45)).child(label))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(content(0.8))
                            .child(value.lines().next().unwrap_or_default().to_owned()),
                    ),
            );
        }
        if let Some(path) = run.worktree_cwd.clone() {
            header = header.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .child(div().text_color(content(0.45)).child("Worktree"))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(crate::markdown::PILOT)
                            .text_color(content(0.8))
                            .child(path.clone()),
                    )
                    .child(
                        icon_button("flow-reveal-worktree", "Reveal in Finder", "folder-open")
                            .on_click(move |_, _, cx| cx.reveal_path(std::path::Path::new(&path))),
                    ),
            );
        }
        if let Some(error) = &run.error {
            header = header.child(
                div()
                    .text_size(px(12.))
                    .text_color(gpui::rgb(0xf87171))
                    .child(short_error(error)),
            );
        }
        if page.can_resume() {
            header = header.child(self.render_resume(page, cx));
        }

        let mut timeline = div()
            .id("flow-timeline")
            .debug_selector(|| "flow-timeline".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(24.))
            .py(px(12.))
            .flex()
            .flex_col()
            .gap(px(6.));
        if page.visits.is_empty() {
            timeline = timeline.child(
                div()
                    .text_size(px(12.))
                    .text_color(content(0.45))
                    .child("No steps yet"),
            );
        }
        for visit in &page.visits {
            timeline = timeline.child(self.render_visit(page, visit, now, cx));
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(timeline)
            .into_any_element()
    }

    fn render_resume(&self, page: &RunPage, cx: &mut Context<Self>) -> AnyElement {
        let chosen = page.resume_state();
        let mut picker = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.))
            .child(div().text_color(content(0.45)).child("Resume from"));
        for state in &page.run.flow.states {
            let id = state.id().to_owned();
            let selected = chosen.as_deref() == Some(id.as_str());
            let pick = id.clone();
            picker = picker.child(
                div()
                    .id(SharedString::from(format!("flow-resume-state-{id}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("flow-resume-state-{id}")
                    })
                    .role(gpui::Role::RadioButton)
                    .aria_label(state_title(&page.run.flow, &id))
                    .aria_selected(selected)
                    .px(px(8.))
                    .h(px(22.))
                    .rounded(px(11.))
                    .flex()
                    .items_center()
                    .border_1()
                    .border_color(content(if selected { 0.5 } else { 0.12 }))
                    .when(selected, |d| d.bg(content(0.1)))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.06)))
                    .child(state_title(&page.run.flow, &id))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Page::Run(page) = &mut this.page {
                            page.resume = Some(pick.clone());
                        }
                        cx.notify();
                    })),
            );
        }
        let run_id = page.run.id.clone();
        picker
            .child(
                disabled(
                    button("flow-resume", "Resume").debug_selector(|| "flow-resume".into()),
                    chosen.is_none(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let Page::Run(page) = &this.page else {
                        return;
                    };
                    if let Some(state_id) = page.resume_state() {
                        cx.emit(Event::Resume {
                            run_id: run_id.clone(),
                            state_id,
                        });
                    }
                })),
            )
            .into_any_element()
    }

    fn render_visit(
        &self,
        page: &RunPage,
        visit: &FlowVisit,
        now: i64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let flow = &page.run.flow;
        let title = state_title(flow, &visit.state_id);
        let open = page.expanded.contains(&visit.id);
        let toggle = visit.id.clone();
        let row = div()
            .id(SharedString::from(format!("flow-visit-{}", visit.seq)))
            .debug_selector({
                let seq = visit.seq;
                move || format!("flow-visit-{seq}")
            })
            .role(gpui::Role::Button)
            .aria_label(format!("{title}, {}", visit_status_label(visit.status)))
            .aria_expanded(open)
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .child(
                div()
                    .w(px(20.))
                    .text_size(px(11.))
                    .text_color(content(0.4))
                    .child(visit.seq.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(title.clone()),
            )
            .when(visit.retried, |d| d.child(status_chip("none", "Retried")))
            .child(status_chip(
                visit.status.as_str(),
                visit_status_label(visit.status),
            ))
            .child(
                div()
                    .w(px(56.))
                    .text_size(px(11.))
                    .text_color(content(0.45))
                    .child(duration(visit, now)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Page::Run(page) = &mut this.page
                    && !page.expanded.remove(&toggle)
                {
                    page.expanded.insert(toggle.clone());
                }
                cx.notify();
            }));
        let mut card = div()
            .p(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(content(0.08))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(row);
        if let (Some(error), false) = (&visit.error, open) {
            card = card.child(
                div()
                    .text_size(px(12.))
                    .text_color(gpui::rgb(0xf87171))
                    .child(short_error(error)),
            );
        }
        if visit.status == VisitStatus::Running && is_agent(flow, &visit.state_id) {
            card = card.child(
                div()
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child(format!(
                        "Running in {} · {title} — open the session if it needs your approval",
                        flow.name
                    )),
            );
        }
        if visit.status == VisitStatus::WaitingApproval {
            let run_id = page.run.id.clone();
            let reject_run = run_id.clone();
            card = card
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(content(0.8))
                        .child(visit.prompt.clone().unwrap_or_default()),
                )
                .child(
                    div()
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(content(0.15))
                        .child(page.note.clone()),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            button("flow-approve", "Approve")
                                .debug_selector(|| "flow-approve".into())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.decide(run_id.clone(), true, cx)
                                })),
                        )
                        .child(
                            button("flow-reject", "Reject")
                                .debug_selector(|| "flow-reject".into())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.decide(reject_run.clone(), false, cx)
                                })),
                        ),
                );
        }
        if open {
            card = card.child(self.render_visit_detail(visit, cx));
        }
        card.into_any_element()
    }

    fn render_visit_detail(&self, visit: &FlowVisit, cx: &mut Context<Self>) -> AnyElement {
        let section = |label: &'static str, text: String, id: String| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(content(0.45))
                        .child(label),
                )
                .child(
                    div()
                        .id(SharedString::from(id))
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .p(px(8.))
                        .rounded(px(6.))
                        .bg(content(0.04))
                        .font_family(crate::markdown::PILOT)
                        .text_size(px(11.))
                        .text_color(content(0.8))
                        .child(text),
                )
        };
        let mut detail = div().flex().flex_col().gap(px(8.));
        if let Some(prompt) = &visit.prompt {
            detail = detail.child(section(
                "Prompt",
                prompt.clone(),
                format!("flow-visit-prompt-{}", visit.seq),
            ));
        }
        if let Some(outputs) = &visit.outputs {
            detail = detail.child(section(
                "Outputs",
                serde_json::to_string_pretty(outputs).unwrap_or_default(),
                format!("flow-visit-outputs-{}", visit.seq),
            ));
        }
        if let Some(error) = &visit.error {
            detail = detail.child(section(
                "Error",
                error.clone(),
                format!("flow-visit-error-{}", visit.seq),
            ));
        }
        if let Some(session) = visit
            .session_id
            .as_deref()
            .and_then(|s| s.parse::<i64>().ok())
        {
            detail = detail.child(
                div().flex().child(
                    button(
                        SharedString::from(format!("flow-open-session-{}", visit.seq)),
                        "Open session",
                    )
                    .tooltip(|_, cx| {
                        cx.new(|_| Tooltip("Focus this step's session tab".into()))
                            .into()
                    })
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(Event::OpenSession(session)))),
                ),
            );
        }
        detail.into_any_element()
    }

    fn decide(&mut self, run_id: String, approved: bool, cx: &mut Context<Self>) {
        let Page::Run(page) = &self.page else {
            return;
        };
        let note = page.note.read(cx).content.trim().to_string();
        // A later approval in the same run starts with an empty note.
        page.note
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        cx.emit(Event::Approve {
            run_id,
            approved,
            note,
        });
    }
}

#[cfg(test)]
impl FlowsView {
    pub(crate) fn dialog_inputs(&self) -> Vec<Entity<Input>> {
        self.dialog
            .as_ref()
            .map(|d| d.fields.iter().map(|(_, f)| f.clone()).collect())
            .unwrap_or_default()
    }
    pub(crate) fn dialog_missing(&self, cx: &gpui::App) -> Option<Vec<String>> {
        let dialog = self.dialog.as_ref()?;
        Some(
            checked_inputs(&dialog.flow, &dialog.values(cx))
                .err()
                .unwrap_or_default(),
        )
    }
    pub(crate) fn run_note(&self) -> Option<Entity<Input>> {
        match &self.page {
            Page::Run(page) => Some(page.note.clone()),
            _ => None,
        }
    }
    pub(crate) fn open_run(&self) -> Option<(FlowRun, Vec<FlowVisit>)> {
        match &self.page {
            Page::Run(page) => Some((page.run.clone(), page.visits.clone())),
            _ => None,
        }
    }
    pub(crate) fn resume_choice(&self) -> Option<String> {
        match &self.page {
            Page::Run(page) => page.resume_state(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rebase_core::flow::starter_flows;

    fn fix() -> Flow {
        Flow {
            id: "f1".into(),
            cwd: "/r".into(),
            ..starter_flows().remove(0)
        }
    }

    fn visit(seq: u32, state: &str, status: VisitStatus) -> FlowVisit {
        FlowVisit {
            id: format!("v{seq}"),
            run_id: "r1".into(),
            seq,
            state_id: state.into(),
            status,
            prompt: None,
            session_id: None,
            outputs: None,
            error: None,
            retried: false,
            note: None,
            started_at: 1_000,
            finished_at: None,
        }
    }

    fn run(status: RunStatus, current: Option<&str>) -> FlowRun {
        FlowRun {
            id: "r1".into(),
            flow_id: "f1".into(),
            flow: fix(),
            status,
            inputs: HashMap::new(),
            worktree_cwd: None,
            current_state: current.map(str::to_owned),
            error: None,
            created_at: 1,
            finished_at: None,
        }
    }

    #[test]
    fn missing_required_inputs_are_named_by_label() {
        let flow = fix();
        let empty = HashMap::new();
        assert_eq!(
            checked_inputs(&flow, &empty),
            Err(vec!["Issue is required".to_string()])
        );
        let blank = HashMap::from([("issue".to_string(), String::new())]);
        assert!(checked_inputs(&flow, &blank).is_err());
        let given = HashMap::from([("issue".to_string(), "Crash on save".to_string())]);
        let resolved = checked_inputs(&flow, &given).unwrap();
        assert_eq!(resolved["issue"], "Crash on save");
        assert_eq!(resolved["notes"], "", "optional inputs resolve empty");
    }

    #[test]
    fn a_default_satisfies_a_required_input() {
        let mut flow = fix();
        flow.inputs[0].default = Some("From default".into());
        assert_eq!(
            checked_inputs(&flow, &HashMap::new()).unwrap()["issue"],
            "From default"
        );
    }

    #[test]
    fn resume_defaults_to_the_latest_failed_state() {
        let visits = vec![
            visit(1, "triage", VisitStatus::Completed),
            visit(2, "implement", VisitStatus::Failed),
            visit(3, "test", VisitStatus::Completed),
            visit(4, "implement", VisitStatus::Completed),
            visit(5, "test", VisitStatus::Failed),
        ];
        // The failed visit decides, not the run's current state.
        let failed = run(RunStatus::Failed, Some("review"));
        assert_eq!(
            default_resume_state(&failed, &visits).as_deref(),
            Some("test")
        );
        let visits = vec![
            visit(1, "triage", VisitStatus::Completed),
            visit(2, "implement", VisitStatus::Cancelled),
        ];
        let cancelled = run(RunStatus::Cancelled, Some("implement"));
        assert_eq!(
            default_resume_state(&cancelled, &visits).as_deref(),
            Some("implement")
        );
    }

    #[test]
    fn resume_without_a_failed_visit_uses_the_current_then_initial_state() {
        // A run that failed before any visit, such as an unknown binding.
        let failed = run(RunStatus::Failed, Some("implement"));
        assert_eq!(
            default_resume_state(&failed, &[]).as_deref(),
            Some("implement")
        );
        let unstarted = run(RunStatus::Failed, None);
        assert_eq!(
            default_resume_state(&unstarted, &[]).as_deref(),
            Some("triage")
        );
        let gone = run(RunStatus::Failed, Some("removed"));
        assert_eq!(default_resume_state(&gone, &[]), None);
    }

    #[test]
    fn timeline_errors_are_one_line_of_at_most_300_chars() {
        assert_eq!(short_error("Boom"), "Boom");
        assert_eq!(short_error("Boom\n"), "Boom");
        assert_eq!(
            short_error("Output contract not met\nmore\ndetail"),
            "Output contract not met…"
        );
        let long = "x".repeat(1_000);
        let short = short_error(&long);
        assert_eq!(short.chars().count(), ERROR_PREVIEW);
        assert!(short.ends_with('…'));
        let exact = "é".repeat(ERROR_PREVIEW);
        assert_eq!(short_error(&exact), exact, "300 multi-byte chars fit");
    }

    #[test]
    fn durations_read_in_seconds_minutes_and_hours() {
        let mut v = visit(1, "triage", VisitStatus::Completed);
        v.finished_at = Some(1_000 + 42_000);
        assert_eq!(duration(&v, 0), "42s");
        v.finished_at = Some(1_000 + 125_000);
        assert_eq!(duration(&v, 0), "2m 5s");
        v.finished_at = None;
        assert_eq!(duration(&v, 1_000 + 3_660_000), "1h 1m");
    }
}
