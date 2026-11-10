//! The look card: theme, accent and glass, each applied live behind
//! the card as it is picked. Built to the Look board of the design canvas.
use super::{SetupStep, card::SetupCard, ui};
use crate::settings::ACCENTS;
use crate::theme::*;
use gpui::{
    AnyElement, Context, Div, FontWeight, Role, SharedString, Window, div, prelude::*, px, rgb,
};

const THEMES: [(&str, &str); 3] = [("system", "System"), ("dark", "Dark"), ("light", "Light")];

/// An accent preset in the form the app stores it, as the appearance page
/// writes it: lowercase `#rrggbb`.
fn accent_hex(color: u32) -> String {
    format!("#{color:06x}")
}

pub(super) fn render(
    card: &mut SetupCard,
    _: &mut Window,
    cx: &mut Context<SetupCard>,
) -> AnyElement {
    let skip_setup = ui::text_button("setup-skip-setup", "Skip setup", 13.)
        .debug_selector(|| "setup-skip-setup".into())
        .on_click(cx.listener(|card, _, _, cx| card.skip_setup(cx)));
    ui::card(28., 22.)
        .child(ui::header(2, Some(skip_setup)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(ui::title("Make it yours.", 28.))
                .child(ui::lead(
                    "Changes show behind this card as you pick.",
                    14.,
                    22.,
                )),
        )
        .child(row("Theme").child(theme_control(&card.prefs.theme, cx)))
        .child(row("Accent color").child(accents(card.prefs.accent_color.as_deref(), cx)))
        .child(
            row("Glass")
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.))
                        .text_color(ui::muted())
                        .child("Let the desktop show through the window."),
                )
                .child(glass_switch(card.prefs.inset_glass, cx)),
        )
        .child(ui::footer(
            Some(
                ui::text_button("setup-back", "Back", 14.)
                    .debug_selector(|| "setup-back".into())
                    .py(px(9.))
                    .px(px(4.))
                    .on_click(cx.listener(|card, _, _, cx| card.go(SetupStep::Agents, cx))),
            ),
            vec![
                ui::ghost("setup-skip", "Skip")
                    .debug_selector(|| "setup-skip".into())
                    .on_click(cx.listener(|card, _, _, cx| card.skip(SetupStep::Project, cx)))
                    .into_any_element(),
                ui::primary("setup-continue", "Continue", true)
                    .debug_selector(|| "setup-continue".into())
                    .on_click(cx.listener(|card, _, _, cx| card.go(SetupStep::Project, cx)))
                    .into_any_element(),
            ],
        ))
        .into_any_element()
}

/// A labelled row: the 120pt semibold label column, then the control.
fn row(label: &'static str) -> Div {
    div().w_full().flex().gap(px(16.)).items_center().child(
        div()
            .w(px(120.))
            .flex_shrink_0()
            .font_weight(FontWeight::SEMIBOLD)
            .child(label),
    )
}

fn theme_control(current: &str, cx: &mut Context<SetupCard>) -> impl IntoElement {
    div()
        .id("setup-theme")
        .role(Role::RadioGroup)
        .aria_label("Theme")
        .flex()
        .p(px(3.))
        .gap(px(2.))
        .rounded(px(9.))
        .bg(content(0.06))
        .children(THEMES.iter().map(|&(id, label)| {
            let selected = current == id;
            div()
                .id(SharedString::from(format!("setup-theme-{id}")))
                .debug_selector(move || format!("setup-theme-{id}"))
                .role(Role::RadioButton)
                .aria_label(label)
                .aria_selected(selected)
                .cursor_pointer()
                .px(px(16.))
                .py(px(6.))
                .rounded(px(7.))
                .text_size(px(13.))
                .bg(if selected {
                    content(0.14)
                } else {
                    gpui::transparent_black()
                })
                .text_color(if selected { content(1.) } else { ui::muted() })
                .when(!selected, |d| d.hover(|s| s.text_color(content(1.))))
                .child(label)
                .on_click(
                    cx.listener(move |card, _, _, cx| card.set_prefs(|p| p.theme = id.into(), cx)),
                )
        }))
}

fn accents(current: Option<&str>, cx: &mut Context<SetupCard>) -> impl IntoElement {
    // The 2pt gap ring takes the card's own fill so the swatch looks cut out.
    let gap = if is_light() {
        floating_fill()
    } else {
        rgb(0x171717).into()
    };
    div()
        .id("setup-accent")
        .role(Role::RadioGroup)
        .aria_label("Accent color")
        .flex()
        .gap(px(10.))
        .children(ACCENTS.iter().map(|&(color, name)| {
            let hex = accent_hex(color);
            let selected = current.is_some_and(|c| c.eq_ignore_ascii_case(&hex));
            let selector = format!("setup-accent-{}", name.to_lowercase());
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(Role::RadioButton)
                .aria_label(name)
                .aria_selected(selected)
                .cursor_pointer()
                .size(px(26.))
                .rounded_full()
                .bg(rgb(color))
                .when(selected, |d| {
                    // GPUI paints later shadows on top: the 4pt ink ring
                    // first, then the 2pt gap over its inner half.
                    d.shadow(vec![
                        gpui::BoxShadow::new(px(0.), px(0.), content(1.)).spread_radius(px(4.)),
                        gpui::BoxShadow::new(px(0.), px(0.), gap).spread_radius(px(2.)),
                    ])
                })
                .on_click(cx.listener(move |card, _, _, cx| {
                    let hex = hex.clone();
                    card.set_prefs(|p| p.accent_color = Some(hex), cx)
                }))
        }))
}

fn glass_switch(on: bool, cx: &mut Context<SetupCard>) -> impl IntoElement {
    div()
        .id("setup-glass")
        .debug_selector(|| "setup-glass".into())
        .role(Role::Switch)
        .aria_label("Glass")
        .aria_toggled(if on {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .cursor_pointer()
        .relative()
        .flex_shrink_0()
        .w(px(36.))
        .h(px(22.))
        .rounded_full()
        .bg(if on { accent_color() } else { content(0.18) })
        .child(
            div()
                .absolute()
                .top(px(3.))
                .left(px(if on { 17. } else { 3. }))
                .size(px(16.))
                .rounded_full()
                .bg(rgb(0xffffff)),
        )
        .on_click(
            cx.listener(|card, _, _, cx| card.set_prefs(|p| p.inset_glass = !p.inset_glass, cx)),
        )
}

#[cfg(test)]
mod tests {
    use super::super::{
        SetupStep,
        card::SetupEvent,
        test_support::{click, open},
    };
    use gpui::TestAppContext;

    fn last(opened: &super::super::test_support::Opened) -> Option<SetupEvent> {
        opened.events.borrow().last().cloned()
    }

    #[gpui::test]
    fn theme_pick_emits_preferences_live(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Look, "/Users/a/code", cx);
        click(&opened, "setup-theme-light", cx);
        match last(&opened) {
            Some(SetupEvent::Preferences(p)) => assert_eq!(p.theme, "light"),
            other => panic!("expected Preferences, got {other:?}"),
        }
        assert_eq!(
            opened.card.read_with(cx, |c, _| c.prefs.theme.clone()),
            "light"
        );
    }

    #[gpui::test]
    fn accent_pick_uses_the_app_preset(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Look, "/Users/a/code", cx);
        let (green, _) = crate::settings::ACCENTS
            .into_iter()
            .find(|(_, name)| *name == "Green")
            .unwrap();
        assert_eq!(green, 0x10b981);
        click(&opened, "setup-accent-green", cx);
        match last(&opened) {
            Some(SetupEvent::Preferences(p)) => {
                assert_eq!(p.accent_color.as_deref(), Some("#10b981"));
                // The appearance page parses `#rrggbb` back to the Green preset.
                let hex = p
                    .accent_color
                    .as_deref()
                    .and_then(|h| h.strip_prefix('#'))
                    .unwrap();
                assert_eq!(u32::from_str_radix(hex, 16).ok(), Some(green));
            }
            other => panic!("expected Preferences, got {other:?}"),
        }
    }

    #[gpui::test]
    fn every_preset_has_a_swatch(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Look, "/Users/a/code", cx);
        for selector in [
            "setup-accent-blue",
            "setup-accent-violet",
            "setup-accent-pink",
            "setup-accent-red",
            "setup-accent-orange",
            "setup-accent-green",
        ] {
            assert!(
                super::super::test_support::has(&opened, selector, cx),
                "{selector}"
            );
        }
    }

    #[gpui::test]
    fn glass_switch_toggles_inset_glass(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Look, "/Users/a/code", cx);
        let before = opened.card.read_with(cx, |c, _| c.prefs.inset_glass);
        click(&opened, "setup-glass", cx);
        click(&opened, "setup-glass", cx);
        let glass: Vec<bool> = opened
            .events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                SetupEvent::Preferences(p) => Some(p.inset_glass),
                _ => None,
            })
            .collect();
        assert_eq!(glass, vec![!before, before]);
    }

    #[gpui::test]
    fn look_skip_goes_to_project_and_records_the_skip(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Look, "/Users/a/code", cx);
        click(&opened, "setup-skip", cx);
        assert_eq!(last(&opened), Some(SetupEvent::Step(SetupStep::Project)));
        opened.card.read_with(cx, |c, _| {
            assert_eq!(c.step(), SetupStep::Project);
            assert!(c.skipped.contains(&SetupStep::Look));
        });
    }

    #[gpui::test]
    fn back_and_continue_move_between_cards(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Look, "/Users/a/code", cx);
        click(&opened, "setup-continue", cx);
        assert_eq!(last(&opened), Some(SetupEvent::Step(SetupStep::Project)));
        assert!(opened.card.read_with(cx, |c, _| c.skipped.is_empty()));

        let opened = open(SetupStep::Look, "/Users/a/code", cx);
        click(&opened, "setup-back", cx);
        assert_eq!(last(&opened), Some(SetupEvent::Step(SetupStep::Agents)));
    }
}
