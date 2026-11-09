//! The welcome card: the logo mark, what Gespan is, what a thread is, the
//! agents it works with, and "Get started" or "Skip setup". No step header;
//! Welcome sits before the three numbered cards.
use super::{
    SetupStep,
    card::SetupCard,
    ui::{self, muted},
};
use crate::theme::content;
use gpui::{AnyElement, Context, Div, Window, div, prelude::*, px, rgb};

const TOP: f32 = 36.;
const GAP: f32 = 28.;
/// `ui::card`'s bottom padding, which its (empty) inner block always carries.

/// The agents named on the card; the rest are "and 6 more".
const AGENTS: [&str; 6] = ["Claude Code", "Codex", "Cursor", "Grok", "OpenCode", "Kiro"];

pub(super) fn render(_: &mut SetupCard, _: &mut Window, cx: &mut Context<SetupCard>) -> AnyElement {
    let note = div()
        .id("setup-welcome-note")
        .text_size(px(13.))
        .text_color(muted())
        .child("You can change all of this later in Settings.");
    let skip = ui::ghost("setup-skip-setup", "Skip setup")
        .debug_selector(|| "setup-skip-setup".into())
        .on_click(cx.listener(|card, _, _, cx| card.skip_setup(cx)));
    let start = ui::primary("setup-continue", "Get started", true)
        .debug_selector(|| "setup-continue".into())
        .on_click(cx.listener(|card, _, _, cx| card.go(SetupStep::Agents, cx)));

    frame(
        TOP,
        GAP,
        vec![
            mark().into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(ui::title("Welcome to Gespan.", 34.))
                .child(ui::lead(
                    "Run Claude Code, Codex, Cursor and nine more agents side by side, on the plans you already pay for.",
                    16.,
                    25.,
                ))
                .into_any_element(),
            explainer().into_any_element(),
            ui::footer(
                Some(note),
                vec![skip.into_any_element(), start.into_any_element()],
            )
            .into_any_element(),
        ],
    )
    .into_any_element()
}

/// `ui::card` holding `children` inside its padding.
fn frame(top: f32, gap: f32, children: Vec<AnyElement>) -> Div {
    ui::card(top, gap).children(children)
}

/// The 2x2 logo mark: 14pt squares, 3pt radius, 3pt apart.
fn mark() -> Div {
    let square = |hex: u32| div().size(px(14.)).rounded(px(3.)).bg(rgb(hex));
    let row = |a: u32, b: u32| div().flex().gap(px(3.)).child(square(a)).child(square(b));
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(row(0xede9fe, 0xc0affc))
        .child(row(0x8f64f6, 0x5b22b4))
}

/// The quiet box: what a thread is, and the agents Gespan works with.
fn explainer() -> Div {
    let chip = |label: &'static str| {
        div()
            .px(px(10.))
            .py(px(4.))
            .rounded_full()
            .text_size(px(12.))
            .child(label)
    };
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .px(px(20.))
        .py(px(18.))
        .rounded(px(14.))
        .bg(ui::well())
        .border_1()
        .border_color(ui::rule())
        .child(
            div()
                .text_size(px(14.))
                .line_height(px(22.))
                .text_color(muted())
                .child(
                    "Each thread is one agent working in your project. Start several, watch them side by side, and review every change before it lands.",
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .children(AGENTS.map(|name| chip(name).bg(content(0.08))))
                .child(chip("and 6 more").text_color(muted())),
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
