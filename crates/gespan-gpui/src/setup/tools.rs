//! The shared tools card: Gespan's MCP hub, the servers found in the agents'
//! own configs, and whether agents should use only the hub's.
use super::{SetupStep, card::SetupCard, ui};
use gpui::{AnyElement, Context, Window, prelude::*};

pub(super) fn render(
    _card: &mut SetupCard,
    _: &mut Window,
    cx: &mut Context<SetupCard>,
) -> AnyElement {
    let skip_setup = ui::skip_link("setup-skip-setup")
        .debug_selector(|| "setup-skip-setup".into())
        .on_click(cx.listener(|card, _, _, cx| card.skip_setup(cx)));
    let back = ui::ghost("setup-back", "Back", 32.)
        .debug_selector(|| "setup-back".into())
        .on_click(cx.listener(|card, _, _, cx| card.go(SetupStep::Agents, cx)));
    let skip = ui::ghost("setup-skip", "Skip", 34.)
        .debug_selector(|| "setup-skip".into())
        .on_click(cx.listener(|card, _, _, cx| card.skip(SetupStep::Look, cx)));
    let next = ui::primary("setup-continue", "Continue", true)
        .debug_selector(|| "setup-continue".into())
        .on_click(cx.listener(|card, _, _, cx| card.go(SetupStep::Look, cx)));
    ui::card()
        .child(ui::header(ui::steps(2), Some(skip_setup)))
        .child(ui::intro("Shared tools.", ""))
        .child(ui::footer(
            Some(back.into_any_element()),
            vec![skip.into_any_element(), next.into_any_element()],
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::super::{SetupStep, card::SetupEvent, test_support::*};
    use gpui::TestAppContext;

    #[gpui::test]
    fn tools_skip_goes_to_look_and_records_the_skip(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Tools, "/Users/a/code", cx);
        click(&opened, "setup-skip", cx);
        assert_eq!(
            opened.events.borrow().last(),
            Some(&SetupEvent::Step(SetupStep::Look))
        );
        assert!(
            opened
                .card
                .read_with(cx, |c, _| c.skipped.contains(&SetupStep::Tools))
        );
    }

    #[gpui::test]
    fn tools_back_goes_to_agents(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Tools, "/Users/a/code", cx);
        click(&opened, "setup-back", cx);
        assert_eq!(
            opened.events.borrow().last(),
            Some(&SetupEvent::Step(SetupStep::Agents))
        );
    }

    #[gpui::test]
    fn tools_has_the_step_header_and_skip_setup(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Tools, "/Users/a/code", cx);
        assert!(has(&opened, "setup-step-header", cx));
        click(&opened, "setup-skip-setup", cx);
        assert_eq!(
            opened.events.borrow().last(),
            Some(&SetupEvent::Step(SetupStep::Project))
        );
    }
}
