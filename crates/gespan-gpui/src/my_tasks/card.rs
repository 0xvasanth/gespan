//! The answer card pinned under the history: one strong glass card with what
//! is asked, the request itself, and the answers in three weights.
use super::*;
use crate::conversation::approval_card::Card;
use crate::glass::{self, Button};
use gpui::{AnyElement, Div, ElementId, FontWeight, SharedString, Stateful, px};

/// The card's headline for a shell command, in place of the request's own
/// question: "Claude Code wants to run a command".
pub(super) fn command_line(agent: Option<&str>) -> String {
    format!("{} wants to run a command", agent.unwrap_or("The agent"))
}

/// The command as the design's code well: monospace, one line per line,
/// never wrapped; a line too long for the card scrolls sideways.
fn code_well(command: &str) -> Stateful<Div> {
    div()
        .id("my-tasks-command")
        .debug_selector(|| "my-tasks-command".into())
        .overflow_x_scroll()
        .px(px(12.))
        .py(px(10.))
        .rounded(px(10.))
        .bg(if is_light() {
            content(0.05)
        } else {
            black(0.35)
        })
        .border_1()
        .border_color(glass::edge())
        .font_family(crate::fonts::monospace())
        .text_size(px(12.))
        .line_height(px(19.))
        .text_color(if is_light() {
            content(0.85)
        } else {
            gpui::rgb(0xd6d6d6).into()
        })
        .children(
            command
                .lines()
                .map(|line| div().whitespace_nowrap().child(line.to_owned())),
        )
}

/// The ↵ chip on the answer Enter gives.
fn enter_chip() -> Div {
    div()
        .px(px(5.))
        .rounded(px(4.))
        .border_1()
        // On the primary plate: light in the dark theme, dark in the light.
        .border_color(if is_light() { white(0.3) } else { black(0.2) })
        .font_family(crate::fonts::monospace())
        .text_size(px(10.5))
        .text_color(if is_light() {
            white(0.75)
        } else {
            gpui::rgb(0x444444).into()
        })
        .child("\u{21b5}")
}

/// An answer button in one of the glass weights.
pub(super) fn answer(
    id: impl Into<ElementId>,
    selector: String,
    label: impl Into<SharedString>,
    weight: Button,
) -> Stateful<Div> {
    glass::button(id, label, weight).debug_selector(move || selector.clone())
}

/// A tool approval's answers, in order: allowing once is the main answer,
/// allowing for the session the secondary one, denying the quiet way out.
pub(super) fn approval_answers(
    can_remember: bool,
) -> Vec<(&'static str, &'static str, Button, Decision)> {
    let mut answers = vec![(
        "my-tasks-allow",
        "Allow once",
        Button::Primary,
        Decision::Allow,
    )];
    if can_remember {
        answers.push((
            "my-tasks-allow-session",
            "Allow for this session",
            Button::Glass,
            Decision::Session,
        ));
    }
    answers.push(("my-tasks-deny", "Deny", Button::Ghost, Decision::Deny));
    answers
}

/// A Flow approval's answers: approving is the main one; rejecting the quiet
/// one. The flag is whether the answer approves.
pub(super) const FLOW_ANSWERS: [(&str, &str, Button, bool); 2] = [
    ("my-tasks-flow-approve", "Approve", Button::Primary, true),
    ("my-tasks-flow-reject", "Reject", Button::Ghost, false),
];

/// A draft review's answers: submitting is the main one; keeping it as a
/// draft the secondary one. The flag is whether the answer submits.
pub(super) const DRAFT_ANSWERS: [(&str, &str, Button, bool); 2] = [
    (
        "my-tasks-draft-submit",
        "Submit review",
        Button::Primary,
        true,
    ),
    ("my-tasks-draft-keep", "Keep as draft", Button::Glass, false),
];

/// A Pilot choice's options: the first leads, the rest are glass.
pub(super) fn choice_weight(ix: usize) -> Button {
    if ix == 0 {
        Button::Primary
    } else {
        Button::Glass
    }
}

impl MyTasks {
    /// Enter on the page itself (not in the search or a note): allows the
    /// selected tool request once, as the ↵ chip on that answer says.
    pub(super) fn enter(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key != "enter"
            || event.keystroke.modifiers.modified()
            || event.is_held
            || self.tab != Tab::Waiting
            || !self.focus.is_focused(window)
        {
            return;
        }
        let shown = self.visible(cx);
        let Some(task) = self.selected_in(&shown) else {
            return;
        };
        if let (Body::Approval { approval, .. }, Source::Session(session)) =
            (&task.body, &task.source)
        {
            let event = Event::Decide {
                session: *session,
                request: approval.id.clone(),
                decision: Decision::Allow,
            };
            cx.emit(event);
        }
    }

    pub(super) fn card(&mut self, task: &TaskItem, cx: &mut Context<Self>) -> AnyElement {
        let mut buttons: Vec<AnyElement> = Vec::new();
        let mut body: Vec<AnyElement> = Vec::new();
        let mut title = task.headline.clone();
        let mut aside = task.who.clone();
        let mut hint = None;
        match &task.body {
            Body::Approval { approval, cwd } => {
                let Source::Session(session) = task.source else {
                    return div().into_any_element();
                };
                let card = Card::new(approval, cwd.as_deref());
                if let Some(command) = card.lone_command() {
                    title = command_line(task.agent.as_deref());
                    if let Some(tool) = &approval.tool {
                        aside = tool.name.clone();
                    }
                    body.push(code_well(command).into_any_element());
                } else if !card.is_empty() {
                    body.push(card.render(true));
                }
                hint = Some("Answering moves to the next task");
                for (id, label, weight, decision) in approval_answers(approval.can_remember) {
                    let request = approval.id.clone();
                    buttons.push(
                        answer(id, id.into(), label, weight)
                            .when(matches!(decision, Decision::Allow), |d| {
                                d.child(enter_chip())
                            })
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(Event::Decide {
                                    session,
                                    request: request.clone(),
                                    decision: decision.clone(),
                                })
                            }))
                            .into_any_element(),
                    );
                }
            }
            // The session's own card: its controls answer the session directly.
            // Its chat is never on screen with this page: opening the page
            // closes an open Pilot, and opening a chat replaces the page.
            Body::Question(card) => body.push(card.clone().into_any_element()),
            Body::Flow { prompt } => {
                let Source::FlowRun(run) = &task.source else {
                    return div().into_any_element();
                };
                if !prompt.trim().is_empty() {
                    body.push(
                        div()
                            .text_size(px(13.))
                            .line_height(px(20.))
                            .text_color(content(0.8))
                            .child(prompt.clone())
                            .into_any_element(),
                    );
                }
                body.push(
                    glass::field()
                        .debug_selector(|| "my-tasks-flow-note".into())
                        .h(px(36.))
                        .child(div().flex_1().min_w_0().child(self.note.clone()))
                        .into_any_element(),
                );
                for (id, label, weight, approved) in FLOW_ANSWERS {
                    let run = run.clone();
                    buttons.push(
                        answer(id, id.into(), label, weight)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let note = this.note.read(cx).content.to_string();
                                cx.emit(Event::FlowDecide {
                                    run: run.clone(),
                                    approved,
                                    note,
                                })
                            }))
                            .into_any_element(),
                    );
                }
            }
            Body::DraftReview {
                body: text,
                comments,
                outside,
            } => {
                let Source::DraftReview { run, .. } = &task.source else {
                    return div().into_any_element();
                };
                let line = |id: String, text: String, strong: bool| {
                    div()
                        .debug_selector(move || id.clone())
                        .text_size(px(13.))
                        .line_height(px(20.))
                        .text_color(content(if strong { 0.85 } else { 0.7 }))
                        .child(text)
                        .into_any_element()
                };
                if !text.trim().is_empty() {
                    body.push(line("my-tasks-draft-body".into(), text.clone(), true));
                }
                for (ix, comment) in comments.iter().enumerate() {
                    let side = match comment.side {
                        gespan_core::review_draft::diff_map::Side::Old => " (old)",
                        gespan_core::review_draft::diff_map::Side::New => "",
                    };
                    body.push(line(
                        format!("my-tasks-draft-comment-{ix}"),
                        format!(
                            "{}:{}{side} \u{2014} {}",
                            comment.path, comment.line, comment.body
                        ),
                        false,
                    ));
                }
                if *outside > 0 {
                    body.push(line(
                        "my-tasks-draft-outside".into(),
                        format!(
                            "{outside} finding{} on lines outside the diff {} in the summary.",
                            if *outside == 1 { "" } else { "s" },
                            if *outside == 1 { "is" } else { "are" }
                        ),
                        false,
                    ));
                }
                for (id, label, weight, submit) in DRAFT_ANSWERS {
                    let run = run.clone();
                    buttons.push(
                        answer(id, id.into(), label, weight)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(Event::DraftDecide {
                                    run: run.clone(),
                                    submit,
                                })
                            }))
                            .into_any_element(),
                    );
                }
            }
            Body::Decision { .. } => self.decision_card(task, &mut body, &mut buttons, cx),
            Body::Choice { options } => {
                let Source::Pilot { pilot, .. } = &task.source else {
                    return div().into_any_element();
                };
                for (ix, option) in options.iter().enumerate() {
                    let pilot = pilot.clone();
                    let text = option.clone();
                    let key = task.key.clone();
                    buttons.push(
                        answer(
                            ("my-tasks-choice", ix),
                            format!("my-tasks-choice-{ix}"),
                            option.clone(),
                            choice_weight(ix),
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(Event::Choose {
                                key: key.clone(),
                                pilot: pilot.clone(),
                                text: text.clone(),
                            })
                        }))
                        .into_any_element(),
                    );
                }
            }
        }
        glass::card_strong()
            .id("my-tasks-card")
            .debug_selector(|| "my-tasks-card".into())
            .w_full()
            .max_w(px(720.))
            .p(px(18.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(super::list::tile(task.kind, 28.))
                    .child(
                        div()
                            .debug_selector(|| "my-tasks-card-title".into())
                            .flex_1()
                            .min_w_0()
                            .text_size(px(17.))
                            .line_height(px(28.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .debug_selector(|| "my-tasks-card-aside".into())
                            .flex_shrink_0()
                            .pt(px(6.))
                            .text_size(px(11.5))
                            .text_color(super::list::faint())
                            .child(aside),
                    ),
            )
            .children(body)
            .when(!buttons.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .children(buttons)
                        .when_some(hint, |d, hint| {
                            d.child(
                                div()
                                    .debug_selector(|| "my-tasks-card-hint".into())
                                    .ml_auto()
                                    .text_size(px(11.5))
                                    .text_color(super::list::faint())
                                    .child(hint),
                            )
                        }),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each answer's label with its weight.
    fn weights<'a, T>(answers: &'a [(&'a str, &'a str, Button, T)]) -> Vec<(&'a str, Button)> {
        answers
            .iter()
            .map(|(_, label, weight, _)| (*label, *weight))
            .collect()
    }

    #[test]
    fn allowing_once_leads_an_approval_and_deny_is_the_quiet_answer() {
        let answers = approval_answers(true);
        assert!(
            weights(&answers)
                == [
                    ("Allow once", Button::Primary),
                    ("Allow for this session", Button::Glass),
                    ("Deny", Button::Ghost),
                ]
        );
        assert!(matches!(answers[0].3, Decision::Allow));
        assert!(matches!(answers[2].3, Decision::Deny));
    }

    #[test]
    fn a_tool_that_cannot_be_remembered_offers_no_session_answer() {
        let answers = approval_answers(false);
        assert!(weights(&answers) == [("Allow once", Button::Primary), ("Deny", Button::Ghost)]);
    }

    #[test]
    fn approve_leads_a_flow_step_and_reject_is_the_quiet_answer() {
        assert!(
            weights(&FLOW_ANSWERS) == [("Approve", Button::Primary), ("Reject", Button::Ghost)]
        );
        // The flag the page sends must match the word on the button.
        assert!(FLOW_ANSWERS[0].3 && !FLOW_ANSWERS[1].3);
    }

    #[test]
    fn only_a_pilot_choice_first_option_is_primary() {
        assert!(choice_weight(0) == Button::Primary);
        assert!(choice_weight(1) == Button::Glass);
        assert!(choice_weight(4) == Button::Glass);
    }
}
