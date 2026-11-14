//! The welcome card: the logo mark, what Gespan is, what a thread is, the
//! agents it works with, and "Get started" or "Skip setup". No step header;
//! Welcome sits before the three numbered cards.
use super::{SetupStep, card::SetupCard, ui};
use crate::theme::content;
use gpui::{AnyElement, Context, Div, Window, div, prelude::*, px, rgb};

/// The agents named on the card; the rest are "and 6 more".
const AGENTS: [&str; 6] = ["Claude Code", "Codex", "Cursor", "Grok", "OpenCode", "Kiro"];

pub(super) fn render(_: &mut SetupCard, _: &mut Window, cx: &mut Context<SetupCard>) -> AnyElement {
    let skip = ui::skip_link("setup-skip-setup")
        .debug_selector(|| "setup-skip-setup".into())
        .on_click(cx.listener(|card, _, _, cx| card.skip_setup(cx)));
    let note = ui::t3("You can change all of this later in Settings.")
        .id("setup-welcome-note")
        .into_any_element();
    let start = ui::primary("setup-continue", "Get started", true)
        .debug_selector(|| "setup-continue".into())
        .on_click(cx.listener(|card, _, _, cx| card.go(SetupStep::Agents, cx)));

    ui::card()
        .child(ui::header(mark(), Some(skip)))
        .child(ui::intro(
            "Welcome to Gespan.",
            "Run Claude Code, Codex, Cursor and nine more agents side by side, on the plans you already pay for.",
        ))
        .child(explainer())
        .child(ui::footer(Some(note), vec![start.into_any_element()]))
        .into_any_element()
}

/// The 2x2 logo mark: 14pt squares, 4pt radius, 3pt apart.
fn mark() -> Div {
    let square = |hex: u32| div().size(px(14.)).rounded(px(4.)).bg(rgb(hex));
    let row = |a: u32, b: u32| div().flex().gap(px(3.)).child(square(a)).child(square(b));
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(row(0xede9fe, 0xc0affc))
        .child(row(0x8f64f6, 0x5b22b4))
}

/// The inset box: what a thread is, and the agents Gespan works with.
fn explainer() -> Div {
    let chip = |label: &'static str| {
        div()
            .h(px(26.))
            .px(px(11.))
            .flex()
            .items_center()
            .rounded_full()
            .border_1()
            .border_color(content(0.09))
            .bg(content(0.05))
            .text_size(px(12.))
            .text_color(content(0.86))
            .child(label)
    };
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .px(px(16.))
        .py(px(14.))
        .rounded(px(14.))
        .bg(ui::inset())
        .border_1()
        .border_color(ui::inset_edge())
        .child(
            div()
                .text_size(px(13.5))
                .line_height(px(21.))
                .text_color(content(0.8))
                .child(
                    "Each thread is one agent working in your project. Start several, watch them side by side, and review every change before it lands.",
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .children(AGENTS.map(chip))
                .child(ui::t3("and 6 more").ml(px(4.))),
        )
}

#[cfg(test)]
mod tests {
    use super::super::{SetupStep, card::SetupEvent, test_support::*};
    use gpui::TestAppContext;

    #[gpui::test]
    fn welcome_get_started_goes_to_agents(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Welcome, "/Users/a/code/app", cx);
        click(&opened, "setup-continue", cx);
        assert_eq!(
            opened.events.borrow().last(),
            Some(&SetupEvent::Step(SetupStep::Agents))
        );
        assert_eq!(
            opened.card.read_with(cx, |card, _| card.step()),
            SetupStep::Agents
        );
    }

    #[gpui::test]
    fn welcome_skip_setup_goes_to_project_and_marks_both_cards_skipped(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Welcome, "/Users/a/code/app", cx);
        click(&opened, "setup-skip-setup", cx);
        assert_eq!(
            opened.events.borrow().last(),
            Some(&SetupEvent::Step(SetupStep::Project))
        );
        opened.card.read_with(cx, |card, _| {
            assert!(card.skipped.contains(&SetupStep::Agents));
            assert!(card.skipped.contains(&SetupStep::Look));
            assert!(!card.skipped.contains(&SetupStep::Welcome));
        });
    }

    #[gpui::test]
    fn welcome_has_no_step_header(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Welcome, "/Users/a/code/app", cx);
        assert!(!has(&opened, "setup-step-header", cx));
        // Both actions are on screen, so the absence above is not an empty card.
        assert!(has(&opened, "setup-continue", cx));
        assert!(has(&opened, "setup-skip-setup", cx));
    }
}
