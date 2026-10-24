//! The JSON editor for a saved Flow. Every change is parsed and run through
//! `validate_flow`; Save waits until there are no errors.
use super::{
    ui::{button, disabled},
    view::{FlowsView, Page, store_lock},
};
use crate::{
    input::{self, Input},
    theme::*,
};
use gpui::{AnyElement, Context, Entity, Subscription, div, prelude::*, px, rgb};
use rebase_core::flow::{Flow, validate_flow};

/// What the editor's text currently says about the Flow.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Check {
    pub flow: Option<Flow>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Parses `text` as a Flow and validates it. A JSON or field error (such as a
/// fractional `maxVisits`) is reported with serde's message.
pub(super) fn check(text: &str) -> Check {
    match serde_json::from_str::<Flow>(text) {
        Ok(flow) => {
            let validation = validate_flow(&flow);
            Check {
                flow: Some(flow),
                errors: validation.errors,
                warnings: validation.warnings,
            }
        }
        Err(error) => Check {
            flow: None,
            errors: vec![format!("Invalid Flow JSON: {error}")],
            warnings: Vec::new(),
        },
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
        if editor.saving || !editor.check.can_save() {
            return;
        }
        let Some(edited) = editor.check.flow.as_ref() else {
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
        let blocked = !editor.check.can_save() || editor.saving || !editor.dirty;
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
            .child(button("flow-editor-run", "Run").on_click(
                cx.listener(move |this, _, window, cx| this.open_run_dialog(&id, window, cx)),
            ))
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
    pub(crate) fn editor_can_save(&self) -> bool {
        matches!(&self.page, Page::Editor(e) if e.check.can_save() && e.dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rebase_core::flow::starter_flows;

    fn flow() -> Flow {
        Flow {
            id: "f1".into(),
            cwd: "/work/rebase".into(),
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
