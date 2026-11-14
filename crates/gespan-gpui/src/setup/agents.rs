//! The agents card (step 1 of 3): the installed coding agents, which one new
//! threads start with, and sign-in for the ones that are signed out. With no
//! agent installed it says so and still lets setup finish.
use super::{
    SetupStep,
    card::{Agent, SetupCard},
    rules::{found_count, not_found_line},
    ui,
};
use crate::glass::{self, Button};
use crate::theme::{content, provider_icon};
use gespan_core::provider::Kind;
use gpui::{
    AnyElement, Context, Div, FontWeight, Role, SharedString, Stateful, Window, div, prelude::*, px,
};

/// The installed-agents list scrolls past this height (five rows).
const LIST_MAX_H: f32 = 270.;
/// The agents the empty state suggests installing.
const SUGGESTED: [Kind; 3] = [Kind::Claude, Kind::Codex, Kind::Cursor];

pub(super) fn render(
    card: &mut SetupCard,
    _: &mut Window,
    cx: &mut Context<SetupCard>,
) -> AnyElement {
    let empty = card.agents.is_empty();
    let (title, lead) = if empty {
        (
            "No coding agent found.",
            "Install one, then press Rescan. You can also finish setup now and add one later.",
        )
    } else {
        (
            "Your coding agents.",
            "Gespan found these on this Mac. Pick the one new threads start with.",
        )
    };
    let skip_setup = ui::skip_link("setup-skip-setup")
        .debug_selector(|| "setup-skip-setup".into())
        .on_click(cx.listener(|card, _, _, cx| card.skip_setup(cx)));

    let list = if empty {
        install_list(card, cx)
    } else {
        agent_list(card, cx)
    };

    let back = ui::ghost("setup-back", "Back", 32.)
        .debug_selector(|| "setup-back".into())
        .on_click(cx.listener(|card, _, _, cx| card.go(SetupStep::Welcome, cx)));
    let skip = ui::ghost("setup-skip", "Skip", 34.)
        .debug_selector(|| "setup-skip".into())
        .on_click(cx.listener(|card, _, _, cx| card.skip(SetupStep::Look, cx)));
    let next = ui::primary("setup-continue", "Continue", true)
        .debug_selector(|| "setup-continue".into())
        .on_click(cx.listener(|card, _, _, cx| card.go(SetupStep::Look, cx)));

    ui::card()
        .child(ui::header(ui::steps(1), Some(skip_setup)))
        .child(ui::intro(title, lead))
        .child(list)
        .child(ui::footer(
            Some(back.into_any_element()),
            vec![skip.into_any_element(), next.into_any_element()],
        ))
        .into_any_element()
}

/// The line under the list naming what is not installed.
fn missing_line(card: &SetupCard) -> String {
    let labels: Vec<&str> = card.missing.iter().map(|kind| kind.label()).collect();
    not_found_line(&labels)
}

/// The list's last row: what is not installed, the count and Rescan.
fn scan_row(card: &SetupCard, first: bool, cx: &mut Context<SetupCard>) -> Div {
    ui::row(first)
        .h(px(38.))
        .text_size(px(12.))
        .child(ui::t3(missing_line(card)).flex_1().min_w_0().truncate())
        .child(ui::t3(found_count(card.agents.len(), Kind::ALL.len())).flex_shrink_0())
        .child(
            glass::button("setup-rescan", "Rescan", Button::Ghost)
                .debug_selector(|| "setup-rescan".into())
                .role(Role::Button)
                .aria_label("Rescan for coding agents")
                .h(px(24.))
                .px(px(10.))
                .text_size(px(11.5))
                .on_click(cx.listener(|card, _, _, cx| card.rescan(cx))),
        )
}

/// The installed agents, then the not-found line.
fn agent_list(card: &SetupCard, cx: &mut Context<SetupCard>) -> Div {
    let rows: Vec<_> = card
        .agents
        .iter()
        .enumerate()
        .map(|(ix, agent)| {
            let on = card.default_agent == Some(agent.kind);
            let waiting = card.signing_in == Some(agent.kind);
            agent_row(agent, ix == 0, on, waiting, cx)
        })
        .collect();
    ui::list()
        .child(
            div()
                .id("setup-agents-list")
                .role(Role::RadioGroup)
                .aria_label("Default agent for new threads")
                .max_h(px(LIST_MAX_H))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .children(rows),
        )
        .child(scan_row(card, false, cx))
}

fn agent_row(
    agent: &Agent,
    first: bool,
    on: bool,
    waiting: bool,
    cx: &mut Context<SetupCard>,
) -> Stateful<Div> {
    let kind = agent.kind;
    let id = kind.id();
    let (color, status) = match agent.signed_in {
        Some(true) => (ui::green(), "Signed in"),
        Some(false) => (ui::yellow(), "Not signed in"),
        None => (ui::faint(), "Signs in on first use"),
    };
    ui::row(first)
        .id(SharedString::from(format!("setup-agent-{id}")))
        .debug_selector(move || format!("setup-agent-{id}"))
        .role(Role::RadioButton)
        .aria_label(SharedString::from(format!(
            "Make {} the default",
            kind.label()
        )))
        .aria_selected(on)
        .cursor_pointer()
        .when(on, |d| d.bg(ui::picked()))
        .when(!on, |d| d.hover(|s| s.bg(content(0.03))))
        .child(ui::radio(on))
        .child(ui::tile().child(provider_icon(id, 15.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    ui::t1(kind.label())
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate(),
                )
                .child(ui::t3("Installed")),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .flex_shrink_0()
                .text_size(px(11.5))
                .text_color(ui::faint())
                .child(ui::dot(color))
                .child(status),
        )
        .when(agent.signed_in == Some(false), |d| {
            d.child(sign_in_button(kind, waiting, cx))
        })
        .on_click(cx.listener(move |card, _, _, cx| card.pick_agent(kind, cx)))
}

/// "Sign in", or "Waiting for browser…" while that sign-in runs.
fn sign_in_button(kind: Kind, waiting: bool, cx: &mut Context<SetupCard>) -> Stateful<Div> {
    let id = kind.id();
    let label = if waiting {
        "Waiting for browser…"
    } else {
        "Sign in"
    };
    glass::button(
        SharedString::from(format!("setup-sign-in-{id}")),
        label,
        Button::Glass,
    )
    .debug_selector(move || format!("setup-sign-in-{id}"))
    .role(Role::Button)
    .aria_label(SharedString::from(format!("Sign in to {}", kind.label())))
    .h(px(28.))
    .px(px(12.))
    .text_size(px(12.))
    .on_click(cx.listener(move |card, _, _, cx| {
        // The row behind is a radio: signing in should not also pick it.
        cx.stop_propagation();
        card.sign_in(kind, cx);
    }))
}

/// No agent installed: the agents worth installing first, then Rescan.
fn install_list(card: &SetupCard, cx: &mut Context<SetupCard>) -> Div {
    ui::list()
        .children(SUGGESTED.into_iter().enumerate().map(|(ix, kind)| {
            ui::row(ix == 0)
                .child(ui::tile().child(provider_icon(kind.id(), 15.)))
                .child(ui::t1(kind.label()).font_weight(FontWeight::SEMIBOLD))
            // "How to install" opens the agent's install docs once the provider
            // spec carries a docs URL; none does yet, so the link is left out.
        }))
        .child(scan_row(card, false, cx))
}

#[cfg(test)]
mod tests {
    use super::super::{
        SetupStep,
        card::{Agent, SetupEvent},
        rules::not_found_line,
        test_support::*,
    };
    use super::missing_line;
    use gespan_core::provider::Kind;
    use gpui::TestAppContext;

    fn agent(kind: Kind, signed_in: Option<bool>) -> Agent {
        Agent { kind, signed_in }
    }

    fn opened_with(agents: Vec<Agent>, cx: &mut TestAppContext) -> Opened {
        let opened = open(SetupStep::Agents, "/Users/a/code/app", cx);
        opened
            .card
            .update(cx, |card, cx| card.set_agents(agents, cx));
        cx.run_until_parked();
        opened
    }

    #[gpui::test]
    fn agents_lists_only_installed(cx: &mut TestAppContext) {
        let opened = opened_with(
            vec![agent(Kind::Claude, Some(true)), agent(Kind::Codex, None)],
            cx,
        );
        assert!(has(&opened, "setup-agent-claude", cx));
        assert!(has(&opened, "setup-agent-codex", cx));
        assert!(!has(&opened, "setup-agent-cursor", cx));

        let others: Vec<&str> = Kind::ALL
            .into_iter()
            .filter(|k| !matches!(k, Kind::Claude | Kind::Codex))
            .map(Kind::label)
            .collect();
        assert_eq!(others.len(), 10);
        let line = opened.card.read_with(cx, |card, _| missing_line(card));
        assert_eq!(line, not_found_line(&others));
        assert!(
            line.starts_with("Not found: Cursor, ") && line.ends_with(" and 8 more"),
            "{line}"
        );
    }

    #[gpui::test]
    fn every_agent_installed_says_so(cx: &mut TestAppContext) {
        let all = Kind::ALL.into_iter().map(|k| agent(k, None)).collect();
        let opened = opened_with(all, cx);
        let line = opened.card.read_with(cx, |card, _| missing_line(card));
        assert_eq!(line, "Every supported agent is installed");
    }

    #[gpui::test]
    fn picking_a_row_makes_it_the_default(cx: &mut TestAppContext) {
        let opened = opened_with(
            vec![
                agent(Kind::Claude, Some(true)),
                agent(Kind::Codex, Some(true)),
            ],
            cx,
        );
        click(&opened, "setup-agent-codex", cx);
        assert_eq!(
            opened.events.borrow().last(),
            Some(&SetupEvent::DefaultAgent(Kind::Codex))
        );
        assert_eq!(
            opened.card.read_with(cx, |card, _| card.default_agent),
            Some(Kind::Codex)
        );
    }

    #[gpui::test]
    fn sign_in_shows_only_when_signed_out(cx: &mut TestAppContext) {
        let opened = opened_with(
            vec![
                agent(Kind::Claude, Some(true)),
                agent(Kind::Codex, Some(false)),
                agent(Kind::Cursor, None),
            ],
            cx,
        );
        assert!(has(&opened, "setup-sign-in-codex", cx));
        assert!(!has(&opened, "setup-sign-in-claude", cx));
        // Unknown sign-in state signs in on first use: no button either.
        assert!(!has(&opened, "setup-sign-in-cursor", cx));
    }

    #[gpui::test]
    fn sign_in_does_not_also_pick_the_row(cx: &mut TestAppContext) {
        let opened = opened_with(
            vec![
                agent(Kind::Claude, Some(true)),
                agent(Kind::Codex, Some(false)),
            ],
            cx,
        );
        click(&opened, "setup-sign-in-codex", cx);
        let events = opened.events.borrow();
        assert_eq!(events.as_slice(), &[SetupEvent::SignIn(Kind::Codex)]);
    }

    #[gpui::test]
    fn rescan_during_sign_in_starts_no_second_sign_in(cx: &mut TestAppContext) {
        let opened = opened_with(
            vec![
                agent(Kind::Claude, Some(true)),
                agent(Kind::Codex, Some(false)),
            ],
            cx,
        );
        click(&opened, "setup-sign-in-codex", cx);
        click(&opened, "setup-rescan", cx);
        click(&opened, "setup-sign-in-codex", cx);
        let events = opened.events.borrow();
        let sign_ins = events
            .iter()
            .filter(|e| matches!(e, SetupEvent::SignIn(_)))
            .count();
        let rescans = events.iter().filter(|e| **e == SetupEvent::Rescan).count();
        assert_eq!(sign_ins, 1);
        assert_eq!(rescans, 1);
    }

    #[gpui::test]
    fn no_agents_still_lets_you_continue(cx: &mut TestAppContext) {
        let opened = opened_with(vec![], cx);
        assert!(!has(&opened, "setup-agent-claude", cx));
        assert!(has(&opened, "setup-rescan", cx));
        click(&opened, "setup-continue", cx);
        assert_eq!(
            opened.events.borrow().last(),
            Some(&SetupEvent::Step(SetupStep::Look))
        );
    }

    #[gpui::test]
    fn back_and_skip_leave_the_card(cx: &mut TestAppContext) {
        let opened = opened_with(vec![agent(Kind::Claude, None)], cx);
        click(&opened, "setup-skip", cx);
        assert_eq!(
            opened.events.borrow().last(),
            Some(&SetupEvent::Step(SetupStep::Look))
        );
        assert!(
            opened
                .card
                .read_with(cx, |card, _| card.skipped.contains(&SetupStep::Agents))
        );

        let opened = opened_with(vec![agent(Kind::Claude, None)], cx);
        click(&opened, "setup-back", cx);
        assert_eq!(
            opened.events.borrow().last(),
            Some(&SetupEvent::Step(SetupStep::Welcome))
        );
    }
}
