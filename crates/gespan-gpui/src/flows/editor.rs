//! The JSON editor for a saved Flow. Every change is parsed and checked live
//! for the problems panel; Save re-checks the text and refuses while it has
//! errors.
use super::{
    ui::{Tooltip, button, disabled},
    view::{FlowsView, Page, store_lock},
};
use crate::{
    input::{self, Input},
    theme::*,
};
use gespan_core::flow::definition::unknown_fields;
use gespan_core::flow::{Flow, validate_flow};
use gpui::{AnyElement, Context, Entity, Subscription, Window, div, prelude::*, px, rgb};

pub(super) const RUN_BLOCKED: &str = "Save your changes first";

/// What the editor's text currently says about the Flow.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Check {
    pub flow: Option<Flow>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Parses `text` as a Flow, rejects keys the format doesn't define (serde would
/// drop them on save) and validates it. A JSON or field error (such as a
/// fractional `maxVisits`) is reported with serde's message, line and column.
pub(super) fn check(text: &str) -> Check {
    match serde_json::from_str::<Flow>(text) {
        Ok(flow) => {
            let validation = validate_flow(&flow);
            let mut errors = serde_json::from_str::<serde_json::Value>(text)
                .map(|value| unknown_fields(&value))
                .unwrap_or_default();
            errors.extend(validation.errors);
            Check {
                flow: Some(flow),
                errors,
                warnings: validation.warnings,
            }
        }
        Err(error) => {
            // Valid JSON of the wrong shape: still name the misspelled keys.
            let mut errors = serde_json::from_str::<serde_json::Value>(text)
                .map(|value| unknown_fields(&value))
                .unwrap_or_default();
            errors.push(format!("Invalid Flow JSON: {error}"));
            Check {
                flow: None,
                errors,
                warnings: Vec::new(),
            }
        }
    }
}

impl Check {
    pub(super) fn can_save(&self) -> bool {
        self.flow.is_some() && self.errors.is_empty()
    }
}

/// The text the editor opens with.
pub(super) fn pretty(flow: &Flow) -> String {
    serde_json::to_string_pretty(flow).unwrap_or_default()
}

/// The edited Flow keeps the saved Flow's identity: an edited `id` would
/// otherwise save a second Flow.
pub(super) fn to_save(saved: &Flow, edited: &Flow) -> Flow {
    Flow {
        id: saved.id.clone(),
        created_at: saved.created_at,
        ..edited.clone()
    }
}

pub(super) struct Editor {
    /// The Flow as stored.
    pub flow: Flow,
    pub input: Entity<Input>,
    pub check: Check,
    pub dirty: bool,
    pub saving: bool,
    _subscription: Subscription,
}

impl Editor {
    /// Why Run is disabled: it starts the saved Flow, not the editor's text.
    pub(super) fn run_blocked(&self) -> Option<&'static str> {
        self.dirty.then_some(RUN_BLOCKED)
    }
}

impl FlowsView {
    pub(super) fn open_editor(&mut self, flow: Flow, cx: &mut Context<Self>) {
        let text = pretty(&flow);
        let input = cx.new(|cx| {
            let mut input = Input::editor(cx);
            input.set_editor_path("flow.json".into());
            input.set_text(text.clone(), cx);
            input
        });
        let subscription = cx.subscribe(&input, |this, input, _: &input::Changed, cx| {
            let text = input.read(cx).content.to_string();
            if let Page::Editor(editor) = &mut this.page {
                editor.check = check(&text);
                editor.dirty = text != pretty(&editor.flow);
            }
            cx.notify();
        });
        self.dialog = None;
        self.error = None;
        self.page = Page::Editor(Editor {
            check: check(&text),
            flow,
            input,
            dirty: false,
            saving: false,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let Page::Editor(editor) = &mut self.page else {
            return;
        };
        if editor.saving || !editor.dirty {
            return;
        }
        // Save checks the text itself rather than trusting the live check.
        editor.check = check(editor.input.read(cx).content.as_ref());
        let Some(edited) = editor
            .check
            .flow
            .as_ref()
            .filter(|_| editor.check.can_save())
        else {
            let count = editor.check.errors.len();
            let noun = if count == 1 { "problem" } else { "problems" };
            self.error = Some(format!("Fix {count} {noun} before saving"));
            cx.notify();
            return;
        };
        let flow = to_save(&editor.flow, edited);
        editor.saving = true;
        self.error = None;
        let work = cx
            .background_executor()
            .spawn(async move { store_lock(&store)?.upsert(&flow) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(saved) => {
                        this.open_editor(saved, cx);
                        this.refresh(cx);
                    }
                    Err(error) => {
                        if let Page::Editor(editor) = &mut this.page {
                            editor.saving = false;
                        }
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Escape on an editor with unsaved edits asks before dropping them.
    pub(super) fn confirm_discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Page::Editor(editor) = &self.page else {
            return;
        };
        let id = editor.flow.id.clone();
        let prompt = window.prompt(
            gpui::PromptLevel::Warning,
            "Discard your changes to this Flow?",
            Some("Your unsaved edits will be lost."),
            &["Discard", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if prompt.await == Ok(0) {
                let _ = this.update(cx, |this, cx| {
                    if matches!(&this.page, Page::Editor(editor) if editor.flow.id == id) {
                        this.page = Page::Empty;
                        this.error = None;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn revert_editor(&mut self, cx: &mut Context<Self>) {
        if let Page::Editor(editor) = &self.page {
            let flow = editor.flow.clone();
            self.open_editor(flow, cx);
        }
    }

    pub(super) fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let Page::Editor(editor) = &self.page else {
            return div().into_any_element();
        };
        let blocked = editor.saving || !editor.dirty;
        let id = editor.flow.id.clone();
        let header = div()
            .flex_shrink_0()
            .px(px(24.))
            .py(px(14.))
            .border_b_1()
            .border_color(content(0.07))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(16.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(editor.flow.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child(self.project_label(&editor.flow.cwd)),
                    ),
            )
            .when(editor.dirty, |d| {
                d.child(
                    button("flow-editor-revert", "Revert")
                        .on_click(cx.listener(|this, _, _, cx| this.revert_editor(cx))),
                )
            })
            .child({
                let blocked = editor.run_blocked();
                let run = disabled(
                    button("flow-editor-run", "Run").debug_selector(|| "flow-editor-run".into()),
                    blocked.is_some(),
                );
                if let Some(reason) = blocked {
                    run.tooltip(move |_, cx| cx.new(|_| Tooltip(reason.into())).into())
                } else {
                    run.on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.open_run_dialog(&id, window, cx)
                        }),
                    )
                }
            })
            .child(
                disabled(
                    button(
                        "flow-editor-save",
                        if editor.saving { "Saving…" } else { "Save" },
                    )
                    .debug_selector(|| "flow-editor-save".into())
                    .when(!blocked, |d| {
                        d.bg(content(0.92))
                            .text_color(shell_fill())
                            .font_weight(gpui::FontWeight::MEDIUM)
                    }),
                    blocked,
                )
                .on_click(cx.listener(|this, _, _, cx| this.save_editor(cx))),
            );
        let mut problems = div()
            .id("flow-editor-problems")
            .debug_selector(|| "flow-editor-problems".into())
            .flex_shrink_0()
            .max_h(px(180.))
            .overflow_y_scroll()
            .border_t_1()
            .border_color(content(0.07))
            .px(px(24.))
            .py(px(10.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .text_size(px(12.));
        if editor.check.errors.is_empty() && editor.check.warnings.is_empty() {
            problems = problems.child(div().text_color(content(0.45)).child("No problems found"));
        }
        for (index, error) in editor.check.errors.iter().enumerate() {
            problems = problems.child(
                div()
                    .debug_selector(move || format!("flow-editor-error-{index}"))
                    .flex()
                    .gap(px(6.))
                    .text_color(rgb(0xf87171))
                    .child("Error")
                    .child(div().text_color(content(0.8)).child(error.clone())),
            );
        }
        for warning in &editor.check.warnings {
            problems = problems.child(
                div()
                    .flex()
                    .gap(px(6.))
                    .text_color(rgb(0xf59e0b))
                    .child("Warning")
                    .child(div().text_color(content(0.8)).child(warning.clone())),
            );
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("flow-editor")
                    .debug_selector(|| "flow-editor".into())
                    .flex_1()
                    .min_h_0()
                    .px(px(16.))
                    .py(px(8.))
                    .child(editor.input.clone()),
            )
            .child(problems)
            .into_any_element()
    }
}

#[cfg(test)]
impl FlowsView {
    pub(crate) fn editor_input(&self) -> Option<Entity<Input>> {
        match &self.page {
            Page::Editor(editor) => Some(editor.input.clone()),
            _ => None,
        }
    }
    pub(crate) fn editor_errors(&self) -> Vec<String> {
        match &self.page {
            Page::Editor(editor) => editor.check.errors.clone(),
            _ => Vec::new(),
        }
    }
    pub(crate) fn editor_run_blocked(&self) -> Option<&'static str> {
        match &self.page {
            Page::Editor(editor) => editor.run_blocked(),
            _ => None,
        }
    }
    pub(crate) fn editor_can_save(&self) -> bool {
        matches!(&self.page, Page::Editor(e) if e.check.can_save() && e.dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::flow::{FlowState, Guard, GuardOp, VisitResult, starter_flows};

    fn flow() -> Flow {
        Flow {
            id: "f1".into(),
            cwd: "/work/gespan".into(),
            created_at: 7,
            updated_at: 9,
            ..starter_flows().remove(0)
        }
    }

    #[test]
    fn the_saved_text_checks_clean() {
        let checked = check(&pretty(&flow()));
        assert_eq!(checked.flow, Some(flow()));
        assert!(checked.errors.is_empty() && checked.can_save());
    }

    #[test]
    fn an_unknown_target_is_an_error_that_blocks_saving() {
        let text = pretty(&flow()).replacen("\"to\": \"implement\"", "\"to\": \"nowhere\"", 1);
        let checked = check(&text);
        assert!(
            checked
                .errors
                .iter()
                .any(|e| e.contains("unknown target \"nowhere\"")),
            "{:?}",
            checked.errors
        );
        assert!(!checked.can_save());
    }

    #[test]
    fn a_fractional_max_visits_reports_serdes_message() {
        let text = pretty(&flow()).replacen("\"maxVisits\": 3", "\"maxVisits\": 2.5", 1);
        let checked = check(&text);
        assert_eq!(checked.flow, None);
        assert_eq!(checked.errors.len(), 1);
        assert!(checked.errors[0].starts_with("Invalid Flow JSON: invalid type: floating point"));
        assert!(!checked.can_save());
    }

    #[test]
    fn broken_json_is_one_error() {
        let checked = check("{ \"name\": ");
        assert_eq!(checked.flow, None);
        assert_eq!(checked.errors.len(), 1);
        assert!(!checked.can_save());
    }

    #[test]
    fn a_misspelled_key_in_a_state_is_an_error() {
        let text =
            pretty(&flow()).replacen("\"maxVisits\"", "\"maxVisit\": 5,\n  \"maxVisits\"", 1);
        let checked = check(&text);
        let id = flow().states[0].id().to_owned();
        let expected = format!("Unknown field \"maxVisit\" in state {id}");
        assert!(checked.errors.contains(&expected), "{:?}", checked.errors);
        assert!(!checked.can_save());
    }

    #[test]
    fn a_renamed_required_key_reports_both_the_typo_and_the_missing_field() {
        let text = pretty(&flow()).replacen("\"maxVisits\"", "\"maxVisit\"", 1);
        let checked = check(&text);
        let id = flow().states[0].id().to_owned();
        assert_eq!(checked.flow, None);
        assert_eq!(checked.errors.len(), 2, "{:?}", checked.errors);
        assert_eq!(
            checked.errors[0],
            format!("Unknown field \"maxVisit\" in state {id}")
        );
        assert!(
            checked.errors[1].starts_with("Invalid Flow JSON: missing field `maxVisits`"),
            "{:?}",
            checked.errors
        );
    }

    #[test]
    fn a_wrongly_shaped_flow_does_not_panic_and_keeps_serdes_error() {
        let checked = check(r#"{"nmae": 1, "states": "oops", "inputs": [3]}"#);
        assert_eq!(checked.flow, None);
        assert_eq!(checked.errors[0], "Unknown field \"nmae\" in the Flow");
        assert!(
            checked
                .errors
                .last()
                .unwrap()
                .starts_with("Invalid Flow JSON:")
        );
        // Items without an id are named by position.
        let checked =
            check(r#"{"states": [{"kind": "agent", "id": "a"}, {"kind": "approval", "idd": 1}]}"#);
        assert!(
            checked
                .errors
                .contains(&"Unknown field \"idd\" in state 2".to_string()),
            "{:?}",
            checked.errors
        );
    }

    #[test]
    fn an_unknown_top_level_key_is_an_error() {
        let text = pretty(&flow()).replacen("\"name\"", "\"nmae\": 1,\n  \"name\"", 1);
        let checked = check(&text);
        assert_eq!(
            checked.errors,
            vec!["Unknown field \"nmae\" in the Flow".to_string()]
        );
        assert!(checked.flow.is_some() && !checked.can_save());
    }

    #[test]
    fn an_unknown_key_in_a_guard_is_an_error() {
        let mut with_guard = flow();
        let FlowState::Agent(state) = &mut with_guard.states[0] else {
            panic!("the starter begins with an agent");
        };
        state.transitions[0].when = Some(Guard::Status {
            is: VisitResult::Completed,
        });
        let id = state.id.clone();
        let text = pretty(&with_guard).replacen("\"is\"", "\"iss\": 1, \"is\"", 1);
        let checked = check(&text);
        let expected = format!("Unknown field \"iss\" in state {id} transition 1 guard");
        assert_eq!(checked.errors, vec![expected]);
    }

    #[test]
    fn optional_fields_that_serialise_as_absent_are_not_flagged() {
        let mut with_optionals = flow();
        with_optionals
            .inputs
            .iter_mut()
            .for_each(|i| i.default = None);
        for state in &mut with_optionals.states {
            let transitions = match state {
                FlowState::Agent(s) => &mut s.transitions,
                FlowState::Approval(s) => &mut s.transitions,
            };
            transitions.iter_mut().for_each(|t| t.when = None);
        }
        let text = pretty(&with_optionals);
        assert!(!text.contains("\"default\"") && !text.contains("\"when\""));
        let checked = check(&text);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        // Written out, they are known too.
        let mut written = with_optionals;
        written.inputs[0].default = Some("x".into());
        let FlowState::Agent(state) = &mut written.states[0] else {
            panic!("the starter begins with an agent");
        };
        state.transitions[0].when = Some(Guard::Output {
            field: "done".into(),
            op: GuardOp::Truthy,
            value: Some(serde_json::json!("yes")),
        });
        let written = pretty(&written);
        assert!(written.contains("\"default\"") && written.contains("\"value\""));
        let checked = check(&written);
        let unknown: Vec<_> = checked
            .errors
            .iter()
            .filter(|e| e.starts_with("Unknown field"))
            .collect();
        assert!(unknown.is_empty(), "{unknown:?}");
    }

    #[test]
    fn a_syntax_error_keeps_serdes_line_and_column() {
        let text = pretty(&flow()).replacen("\"name\": ", "\"name\" ", 1);
        let checked = check(&text);
        assert_eq!(checked.flow, None);
        assert_eq!(checked.errors.len(), 1);
        let message = &checked.errors[0];
        assert!(
            message.starts_with("Invalid Flow JSON: expected `:`"),
            "{message}"
        );
        assert!(message.contains("at line 3 column"), "{message}");
    }

    #[test]
    fn saving_keeps_the_flows_identity() {
        let mut edited = flow();
        edited.id = "other".into();
        edited.created_at = 1;
        edited.name = "Renamed".into();
        let saved = to_save(&flow(), &edited);
        assert_eq!((saved.id.as_str(), saved.created_at), ("f1", 7));
        assert_eq!(saved.name, "Renamed");
    }
}
