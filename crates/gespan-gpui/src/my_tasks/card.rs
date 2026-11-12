//! The task card pinned under the history: what is asked, the answer controls
//! on the left, and "‹ i of N ›" on the right.
use super::*;
use crate::conversation::approval_card::Card;
use gpui::{AnyElement, Div, ElementId, FontWeight, SharedString, Stateful, px};

fn answer(
    id: impl Into<ElementId>,
    selector: String,
    label: impl Into<SharedString>,
    primary: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || selector.clone())
        .cursor_pointer()
        .h(px(32.))
        .px(px(14.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .text_size(px(13.))
        .when(primary, |d| {
            d.bg(content(0.92))
                .text_color(black(0.9))
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!primary, |d| {
            d.bg(content(0.1)).hover(|s| s.bg(content(0.14)))
        })
        .child(label.into())
}

impl MyTasks {
    pub(super) fn card(&mut self, task: &TaskItem, cx: &mut Context<Self>) -> AnyElement {
        let wait = super::list::waited(self.now_ms.saturating_sub(task.since_ms));
        let mut buttons: Vec<AnyElement> = Vec::new();
        let mut body: Vec<AnyElement> = Vec::new();
        match &task.body {
            Body::Approval { approval, cwd } => {
                let Source::Session(session) = task.source else {
                    return div().into_any_element();
                };
                let card = Card::new(approval, cwd.as_deref());
                if !card.is_empty() {
                    body.push(card.render(true));
                }
                let decide =
                    |id: &'static str, label: &'static str, primary, decision: Decision| {
                        let request = approval.id.clone();
                        answer(id, id.into(), label, primary)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(Event::Decide {
                                    session,
                                    request: request.clone(),
                                    decision: decision.clone(),
                                })
                            }))
                            .into_any_element()
                    };
                buttons.push(decide(
                    "my-tasks-allow",
                    "Allow once",
                    true,
                    Decision::Allow,
                ));
                if approval.can_remember {
                    buttons.push(decide(
                        "my-tasks-allow-session",
                        "Allow for this session",
                        false,
                        Decision::Session,
                    ));
                }
                buttons.push(decide("my-tasks-deny", "Deny", false, Decision::Deny));
            }
            // The session's own card: its controls answer the session directly.
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
                    div()
                        .debug_selector(|| "my-tasks-flow-note".into())
                        .h(px(34.))
                        .px(px(10.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(content(0.14))
                        .flex()
                        .items_center()
                        .child(self.note.clone())
                        .into_any_element(),
                );
                for (id, label, approved) in [
                    ("my-tasks-flow-approve", "Approve", true),
                    ("my-tasks-flow-reject", "Reject", false),
                ] {
                    let run = run.clone();
                    buttons.push(
                        answer(id, id.into(), label, approved)
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
                for (id, label, submit) in [
                    ("my-tasks-draft-submit", "Submit review", true),
                    ("my-tasks-draft-keep", "Keep as draft", false),
                ] {
                    let run = run.clone();
                    buttons.push(
                        answer(id, id.into(), label, submit)
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
                            ix == 0,
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
        let shown = self.visible(cx).len();
        let switcher = (shown > 1).then(|| {
            let step = |id: &'static str, label: &'static str, glyph: &'static str, by: isize| {
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .cursor_pointer()
                    .size(px(28.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(content(0.1)))
                    .child(icon(glyph, 14.))
                    .on_click(cx.listener(move |this, _, _, cx| this.step(by, cx)))
            };
            div()
                .flex_shrink_0()
                .p(px(2.))
                .rounded(px(8.))
                .bg(content(0.06))
                .flex()
                .items_center()
                .gap(px(2.))
                .child(step("my-tasks-prev", "Previous task", "chevron-left", -1))
                .child(
                    div()
                        .debug_selector(|| "my-tasks-position".into())
                        .min_w(px(40.))
                        .flex()
                        .justify_center()
                        .text_size(px(12.))
                        .text_color(content(0.55))
                        .child(self.position(cx)),
                )
                .child(step("my-tasks-next", "Next task", "chevron-right", 1))
        });
        div()
            .id("my-tasks-card")
            .debug_selector(|| "my-tasks-card".into())
            .relative()
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(content(0.1))
            .shadow_lg()
            .child(crate::notifications::surface::backdrop(12.))
            .child(
                div()
                    .relative()
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(12.))
                            .child(
                                div()
                                    .size(px(34.))
                                    .flex_shrink_0()
                                    .rounded(px(8.))
                                    .bg(content(0.08))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(super::list::kind_icon(task.kind), 16.)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .text_size(px(16.))
                                            .line_height(px(23.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(task.headline.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(content(0.55))
                                            .child(format!("{} \u{b7} {wait}", task.who)),
                                    ),
                            ),
                    )
                    .children(body)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(8.))
                            .children(buttons)
                            .child(div().flex_1())
                            .children(switcher),
                    ),
            )
            .into_any_element()
    }
}
