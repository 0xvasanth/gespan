//! The look card: theme, layout, accent and glass, each applied live behind
//! the card as it is picked. Built to the Look board of the design canvas.
use super::{SetupStep, card::SetupCard, ui};
use crate::settings::ACCENTS;
use crate::theme::*;
use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, Role, SharedString, Window, div, linear_color_stop,
    linear_gradient, prelude::*, px, rgb, rgba,
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
        .child(row("Theme", true).child(theme_control(&card.prefs.theme, cx)))
        .child(row("Layout", false).child(layout_cards(card.classic, cx)))
        .child(row("Accent color", true).child(accents(card.prefs.accent_color.as_deref(), cx)))
        .child(
            row("Glass", true)
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
/// Layout's label sits at the top with 8pt above it; the rest centre.
fn row(label: &'static str, centred: bool) -> Div {
    div()
        .flex()
        .gap(px(16.))
        .when(centred, |d| d.items_center())
        .child(
            div()
                .w(px(120.))
                .flex_shrink_0()
                .when(!centred, |d| d.pt(px(8.)))
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

fn layout_cards(classic: bool, cx: &mut Context<SetupCard>) -> impl IntoElement {
    let option = |id: &'static str,
                  on: bool,
                  pick: bool,
                  sketch: Div,
                  name: &'static str,
                  line: &'static str,
                  cx: &mut Context<SetupCard>| {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::RadioButton)
            .aria_label(name)
            .aria_selected(on)
            .cursor_pointer()
            .flex_1()
            .min_w_0()
            .p(px(12.))
            .rounded(px(12.))
            .border_1()
            .border_color(if on {
                ui::selected_edge()
            } else {
                ui::glass_edge()
            })
            .bg(if on { ui::selected_fill() } else { ui::well() })
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(sketch)
            .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(ui::muted())
                    .child(line),
            )
            .on_click(cx.listener(move |card, _, _, cx| card.set_layout(pick, cx)))
    };
    div()
        .id("setup-layout")
        .role(Role::RadioGroup)
        .aria_label("Layout")
        .flex_1()
        .flex()
        .gap(px(10.))
        .child(option(
            "setup-layout-focused",
            !classic,
            false,
            focused_sketch(),
            "Focused",
            "One sidebar: All projects on top, the open project below.",
            cx,
        ))
        .child(option(
            "setup-layout-classic",
            classic,
            true,
            classic_sketch(),
            "Classic",
            "A rail of project icons beside the session list.",
            cx,
        ))
}

/// The 74pt miniature window both layout sketches draw into.
fn sketch() -> Div {
    div()
        .h(px(74.))
        .rounded(px(8.))
        .bg(shell_fill())
        .flex()
        .gap(px(6.))
        .p(px(6.))
}

/// A 4pt sidebar line in the sketch.
fn bar(width: f32, color: Hsla) -> Div {
    div().h(px(4.)).w(px(width)).rounded(px(2.)).bg(color)
}

/// The sketch's main pane: #060606 on dark, the app's pane fill otherwise.
fn sketch_pane() -> Div {
    div().flex_1().rounded(px(5.)).bg(if is_light() {
        pane_fill()
    } else {
        rgb(0x060606).into()
    })
}

fn focused_sketch() -> Div {
    let line = content(0.2);
    sketch()
        .child(
            div()
                .w(px(56.))
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .pt(px(2.))
                .child(bar(30., rgb(0xc0affc).into()))
                .child(bar(44., line))
                .child(bar(38., line))
                .child(div().h(px(1.)).w(px(50.)).my(px(2.)).bg(content(0.12)))
                .child(
                    div()
                        .h(px(8.))
                        .w(px(50.))
                        .rounded(px(2.))
                        .border_1()
                        .border_color(content(0.25)),
                )
                .child(bar(40., line)),
        )
        .child(
            // The canvas's violet glow from the top right corner; GPUI has
            // no radial gradient, so a diagonal linear one stands in.
            sketch_pane()
                .overflow_hidden()
                .child(div().size_full().rounded(px(5.)).bg(linear_gradient(
                    225.,
                    linear_color_stop(rgba(0x8f64f659), 0.),
                    linear_color_stop(rgba(0x8f64f600), 0.6),
                ))),
        )
}

fn classic_sketch() -> Div {
    let line = content(0.2);
    let tile = |color: u32| div().size(px(10.)).rounded(px(3.)).bg(rgb(color));
    sketch()
        .child(
            div()
                .w(px(14.))
                .flex_shrink_0()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .pt(px(2.))
                .child(tile(0xe8c547))
                .child(tile(0x459bf7))
                .child(tile(0xf9a8c9)),
        )
        .child(
            div()
                .w(px(46.))
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .pt(px(2.))
                .child(bar(40., line))
                .child(bar(34., line))
                .child(bar(38., line)),
        )
        .child(sketch_pane())
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
    fn layout_pick_emits_classic(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Look, "/Users/a/code", cx);
        click(&opened, "setup-layout-classic", cx);
        assert_eq!(last(&opened), Some(SetupEvent::Layout { classic: true }));
        let count = opened.events.borrow().len();
        click(&opened, "setup-layout-classic", cx);
        assert_eq!(
            opened.events.borrow().len(),
            count,
            "re-picking Classic emits nothing"
        );
        click(&opened, "setup-layout-focused", cx);
        assert_eq!(last(&opened), Some(SetupEvent::Layout { classic: false }));
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
