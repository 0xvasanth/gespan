//! The look card: theme, accent and glass, each applied live behind
//! the card as it is picked. Built to the Look board of the design canvas.
use super::{SetupStep, card::SetupCard, ui};
use crate::glass;
use crate::settings::ACCENTS;
use crate::theme::*;
use gpui::{AnyElement, Context, Div, Role, SharedString, Window, div, prelude::*, px, rgb};

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
    let skip_setup = ui::skip_link("setup-skip-setup")
        .debug_selector(|| "setup-skip-setup".into())
        .on_click(cx.listener(|card, _, _, cx| card.skip_setup(cx)));
    let back = ui::ghost("setup-back", "Back", 32.)
        .debug_selector(|| "setup-back".into())
        .on_click(cx.listener(|card, _, _, cx| card.go(SetupStep::Agents, cx)));
    let skip = ui::ghost("setup-skip", "Skip", 34.)
        .debug_selector(|| "setup-skip".into())
        .on_click(cx.listener(|card, _, _, cx| card.skip(SetupStep::Project, cx)));
    let next = ui::primary("setup-continue", "Continue", true)
        .debug_selector(|| "setup-continue".into())
        .on_click(cx.listener(|card, _, _, cx| card.go(SetupStep::Project, cx)));
    ui::card()
        .child(ui::header(ui::steps(2), Some(skip_setup)))
        .child(ui::intro(
            "Make it yours.",
            "Changes show behind this card as you pick.",
        ))
        .child(
            ui::list()
                .child(
                    ui::row(true)
                        .child(label("Theme"))
                        .child(div().flex_1())
                        .child(theme_control(&card.prefs.theme, cx)),
                )
                .child(
                    ui::row(false)
                        .child(label("Accent color"))
                        .child(div().flex_1())
                        .child(accents(card.prefs.accent_color.as_deref(), cx)),
                )
                .child(
                    ui::row(false)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(ui::t2("Glass"))
                                .child(ui::t3("Let the desktop show through the window.")),
                        )
                        .child(div().flex_1())
                        .child(glass_switch(card.prefs.inset_glass, cx)),
                ),
        )
        .child(ui::footer(
            Some(back.into_any_element()),
            vec![skip.into_any_element(), next.into_any_element()],
        ))
        .into_any_element()
}

/// A row's 110pt label.
fn label(text: &'static str) -> Div {
    ui::t2(text).w(px(110.)).flex_shrink_0()
}

fn theme_control(current: &str, cx: &mut Context<SetupCard>) -> impl IntoElement {
    glass::segmented()
        .id("setup-theme")
        .role(Role::RadioGroup)
        .aria_label("Theme")
        .children(THEMES.iter().map(|&(id, label)| {
            let selected = current == id;
            glass::segment(
                SharedString::from(format!("setup-theme-{id}")),
                label,
                selected,
            )
            .debug_selector(move || format!("setup-theme-{id}"))
            .role(Role::RadioButton)
            .aria_label(label)
            .aria_selected(selected)
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
        .gap(px(12.))
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
                .size(px(18.))
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

/// The glass switch: 30×18 with a 14pt knob, the accent when on.
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
        .w(px(30.))
        .h(px(18.))
        .rounded_full()
        .bg(if on { accent_color() } else { content(0.14) })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(if on { 14. } else { 2. }))
                .size(px(14.))
                .rounded_full()
                .bg(rgb(0xffffff))
                .shadow(vec![
                    gpui::BoxShadow::new(px(0.), px(1.), black(0.4)).blur_radius(px(3.)),
                ]),
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
    fn look_card_has_no_layout_row(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Look, "/Users/a/code", cx);
        assert!(!super::super::test_support::has(
            &opened,
            "setup-layout",
            cx
        ));
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
