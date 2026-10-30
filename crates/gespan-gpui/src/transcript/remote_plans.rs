//! The original PlanPreview actions, backed by host-owned plan identities.
use super::*;
use crate::conversation::remote_plans::{Plan, PlanAction};
use gpui::AnyElement;

impl TranscriptView {
    pub(super) fn remote_plan_card(&self, plan: &Plan, cx: &mut Context<Self>) -> AnyElement {
        let open = plan.clone();
        let build = plan.clone();
        let enabled = plan.can_build(self.streaming);
        let summary = plan.summary();
        div()
            .id(SharedString::from(format!("remote-plan-{}", plan.id)))
            .w_full()
            .my_2()
            .p_3()
            .rounded(px(12.))
            .border_1()
            .border_color(content(0.1))
            .bg(content(0.06))
            .flex()
            .items_start()
            .gap_2()
            .child(
                icon(
                    if plan.streaming {
                        "loader"
                    } else {
                        "file-text"
                    },
                    16.,
                )
                .text_color(content(0.45)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(content(0.9))
                            .truncate()
                            .child(plan.title()),
                    )
                    .when(!summary.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .line_height(px(18.))
                                .text_color(content(0.5))
                                .child(summary),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                crate::settings::widgets::secondary_button(
                                    SharedString::from(format!("open-plan-{}", plan.id)),
                                    "Open plan",
                                    false,
                                    false,
                                )
                                .debug_selector(|| "plan-open".into())
                                .child("Open")
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        cx.emit(plan_request(&open, PlanAction::Open))
                                    },
                                )),
                            )
                            .child(
                                crate::settings::widgets::secondary_button(
                                    SharedString::from(format!("build-plan-{}", plan.id)),
                                    "Build approved plan",
                                    false,
                                    !enabled,
                                )
                                .debug_selector(|| "plan-build".into())
                                .child(plan.build_label())
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        if enabled {
                                            cx.emit(plan_request(&build, PlanAction::Build));
                                        }
                                    },
                                )),
                            )
                            // SecondOpinionButton: build with another provider or model.
                            .when(plan.is_local(), |row| {
                                let other = plan.clone();
                                row.child(
                                    crate::settings::widgets::secondary_button(
                                        SharedString::from(format!("build-plan-with-{}", plan.id)),
                                        "Build with another model",
                                        false,
                                        !enabled,
                                    )
                                    .debug_selector(|| "plan-build-with".into())
                                    .child(icon("chevron-down", 12.))
                                    .on_click(cx.listener(
                                        move |_, _, _, cx| {
                                            if enabled {
                                                cx.emit(plan_request(
                                                    &other,
                                                    PlanAction::BuildWith,
                                                ));
                                            }
                                        },
                                    )),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }
}

/// A local plan card's status, as the Tauri app's plan block kept it.
pub(super) fn plan_status(rows: &[Row], at: usize, running: bool) -> &'static str {
    match gespan_core::plan::build_state(rows, at, running) {
        Some(gespan_core::plan::BuildState::Building) => "building",
        Some(gespan_core::plan::BuildState::Built) => "built",
        None => "ready",
    }
}

#[cfg(test)]
impl TranscriptView {
    pub(crate) fn plan_cards(&self) -> Vec<Plan> {
        self.rows
            .iter()
            .filter_map(|row| match &row.body {
                Body::RemotePlan(plan) => Some(plan.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Host plans keep their host commands; local plans carry the whole plan.
fn plan_request(plan: &Plan, action: PlanAction) -> ResponseRequest {
    if plan.is_local() {
        ResponseRequest::LocalPlan {
            plan: plan.clone(),
            action,
        }
    } else {
        ResponseRequest::RemotePlan {
            id: plan.id.clone(),
            text: plan.text.clone(),
            build: action == PlanAction::Build,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn remote_plans_render_as_cards_in_their_source_position(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let view=cx.new(|cx| {
                let mut view=TranscriptView::new();
                view.patch(0,vec![
                    Row{role:"user".into(),text:"Make a plan".into(),first:true,sent_at:None},
                    Row{role:"remote-plan".into(),text:serde_json::json!({"id":"host-plan","text":"# Plan\n\n- Step","status":"ready","streaming":false}).to_string(),first:true,sent_at:None},
                    Row{role:"assistant".into(),text:"Ready for review".into(),first:true,sent_at:None}
                ],cx);
                view
            });
            let cards:Vec<_>=view.read(cx).rows.iter().filter(|row|matches!(row.body,Body::RemotePlan(_))).collect();
            assert_eq!(cards.len(),1);
            assert_eq!(cards[0].key.0,1);
            assert_eq!(view.read(cx).source_rows()[2].text,"Ready for review");
        });
    }

    fn row(role: &str, text: &str) -> Row {
        Row {
            role: role.into(),
            text: text.into(),
            first: true,
            sent_at: None,
        }
    }
    fn plan_cards(view: &TranscriptView) -> Vec<Plan> {
        view.plan_cards()
    }
    fn plain(view: &TranscriptView) -> Vec<String> {
        view.rows
            .iter()
            .filter_map(|row| match &row.body {
                Body::Plain(text) => Some(text.to_string()),
                _ => None,
            })
            .collect()
    }

    #[gpui::test]
    fn a_local_plan_is_a_card_that_follows_its_build(cx: &mut gpui::TestAppContext) {
        let plan = "# Login plan\n\n1. Add the form\n2. Wire auth";
        let view = cx.new(|cx| {
            let mut view = TranscriptView::new();
            view.patch(
                0,
                vec![
                    row("user", "Plan the login"),
                    row(gespan_core::plan::ROLE, plan),
                ],
                cx,
            );
            view
        });
        view.read_with(cx, |view, _| {
            let cards = plan_cards(view);
            assert_eq!(cards.len(), 1);
            assert!(cards[0].is_local());
            assert_eq!(cards[0].id, "local:1");
            assert_eq!(cards[0].text, plan);
            assert_eq!(cards[0].title(), "Login plan");
            assert_eq!(cards[0].status, "ready");
            assert!(cards[0].can_build(false));
            assert!(!cards[0].can_build(true), "never while a turn runs");
        });
        // Build sends the whole plan but the message names it; the card follows.
        view.update(cx, |view, cx| {
            view.streaming(true, cx);
            view.patch(
                2,
                vec![
                    row("user", &gespan_core::plan::build_prompt(plan)),
                    row("assistant", "Working on it"),
                ],
                cx,
            );
        });
        view.read_with(cx, |view, _| {
            assert!(plain(view).contains(&gespan_core::plan::BUILD_LABEL.to_owned()));
            assert!(
                !plain(view)
                    .iter()
                    .any(|text| text.contains("<approved_plan>"))
            );
            let card = &plan_cards(view)[0];
            assert_eq!(
                (card.status.as_str(), card.build_label()),
                ("building", "Building…")
            );
            assert!(!card.can_build(false));
        });
        // The conversation patches the final rows, then stops streaming.
        view.update(cx, |view, cx| {
            view.patch(3, vec![row("assistant", "Done")], cx);
            view.streaming(false, cx);
        });
        view.read_with(cx, |view, _| {
            let card = &plan_cards(view)[0];
            assert_eq!(
                (card.status.as_str(), card.build_label()),
                ("built", "Built")
            );
            assert!(!card.can_build(false), "a plan builds once");
        });
    }

    #[gpui::test]
    fn a_plan_answered_with_another_message_stays_buildable(cx: &mut gpui::TestAppContext) {
        let view = cx.new(|cx| {
            let mut view = TranscriptView::new();
            view.patch(
                0,
                vec![
                    row("user", "Plan it"),
                    row(gespan_core::plan::ROLE, "# Plan\n- a\n- b"),
                    row("user", "Make step two smaller"),
                    row("assistant", "Sure"),
                ],
                cx,
            );
            view
        });
        view.read_with(cx, |view, _| {
            let card = &plan_cards(view)[0];
            assert_eq!(card.status, "ready");
            assert!(card.can_build(false));
        });
    }
}
