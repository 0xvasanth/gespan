//! Render smoke tests for the shell: the main layouts drawn at a wide and a
//! narrow width, and the tab tile stack drawn for every focus and leaving
//! index it can be handed, including past the end. A layout that panics while
//! it builds or paints fails here instead of taking the app down
//! (`pane_tile_stack` once indexed a tab with no panes left).
use super::*;
use gpui::{TestAppContext, VisualTestContext, WindowHandle, size};
use std::time::{Duration, Instant};

/// Widths the shell draws at: roomy, and narrow enough to collapse tabs.
const WIDTHS: [f32; 2] = [1400., 700.];

fn draw(window: WindowHandle<Shell>, cx: &mut TestAppContext) {
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    for width in WIDTHS {
        visual.simulate_resize(size(px(width), px(900.)));
        visual.run_until_parked();
        visual.refresh().expect("the shell draws");
    }
}

/// A new chat in the shell, not yet open in a tab.
fn chat(shell: &mut Shell, provider: &str, cx: &mut Context<Shell>) -> i64 {
    let summary = locked(&shell.store)
        .expect("the store opens")
        .create(provider, "/work/gespan")
        .expect("the chat saves");
    let id = summary.id;
    shell.add(summary, cx);
    id
}

/// Puts the active tab mid-shuffle, its tiles leaving `previous` for the
/// focused pane, and holds it there however slow the machine is.
fn shuffle_from(shell: &mut Shell, previous: i64, cx: &mut Context<Shell>) {
    let tab = shell.workspace.active_tab().expect("a tab is active");
    shell.pane_ticks.insert(
        tab,
        PaneTick {
            shown: (shell.workspace.tab_focus(tab), false),
            from: Some((previous, false)),
            dir: 1.,
            start: Instant::now() + Duration::from_secs(600),
        },
    );
    // The title strip is cached chrome; redraw it next frame.
    shell.invalidate_chrome(cx);
}

fn shuffling(shell: &Shell) -> bool {
    shell.pane_ticks.values().any(|tick| tick.from.is_some())
}

type Setup = fn(&mut Shell, i64, i64, &mut Window, &mut Context<Shell>);

/// Each layout, set up from the shared two-chat window (`a` in another
/// project, `b` in this one, both open).
const LAYOUTS: &[(&str, Setup)] = &[
    ("no tabs", |shell, _, _, window, cx| {
        // Closing every tab leaves one fresh blank tab.
        for id in shell.workspace.tabs.clone() {
            shell.close(id, window, cx);
        }
    }),
    ("one tab", |shell, a, _, window, cx| {
        shell.close(a, window, cx);
    }),
    ("many tabs", |shell, _, _, window, cx| {
        for ix in 0..18 {
            let id = chat(shell, if ix % 2 == 0 { "codex" } else { "claude" }, cx);
            shell.workspace.open(id);
        }
        shell.invalidate_chrome(cx);
        let last = *shell.workspace.tabs.last().expect("a tab is open");
        shell.open(last, window, cx);
    }),
    ("split panes", |shell, a, _, window, cx| {
        shell.open(a, window, cx);
        let right = chat(shell, "claude", cx);
        shell.workspace.split(right, SplitDir::Right);
        let below = chat(shell, "codex", cx);
        shell.workspace.split(below, SplitDir::Down);
        shell.open(a, window, cx);
    }),
    ("settings over many tabs", |shell, _, _, window, cx| {
        for _ in 0..6 {
            let id = chat(shell, "claude", cx);
            shell.workspace.open(id);
        }
        shell.open_settings(crate::settings::Section::Archive, window, cx);
    }),
];

#[gpui::test]
fn every_shell_layout_draws(cx: &mut TestAppContext) {
    for (name, setup) in LAYOUTS {
        let (window, a, b) = super::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                setup(shell, a, b, window, cx);
                cx.notify();
            })
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        draw(window, cx);
        window
            .update(cx, |shell, _, _| {
                assert!(
                    !shell.workspace.tabs.is_empty(),
                    "{name}: the shell keeps a tab"
                );
            })
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

/// Closes panes one at a time, down to the last tab, each while the tab's
/// tiles are mid-shuffle from a focus change.
#[gpui::test]
fn closing_panes_and_the_last_tab_mid_shuffle_draws(cx: &mut TestAppContext) {
    let (window, a, b) = super::tests::window(cx);
    let (c, d) = window
        .update(cx, |shell, window, cx| {
            shell.close(b, window, cx);
            shell.open(a, window, cx);
            let c = chat(shell, "claude", cx);
            shell.workspace.split(c, SplitDir::Right);
            let d = chat(shell, "codex", cx);
            shell.workspace.split(d, SplitDir::Down);
            (c, d)
        })
        .expect("the split opens");
    draw(window, cx);

    // Each close lands mid-shuffle: a pane behind, then the pane the tiles
    // are leaving, then the tab's last pane.
    for (focus, leaving, close) in [(a, c, d), (c, a, a), (c, c, c)] {
        window
            .update(cx, |shell, window, cx| {
                shell.focus_pane(focus, window, cx);
                shuffle_from(shell, leaving, cx);
            })
            .expect("focus moves");
        draw(window, cx);
        window
            .update(cx, |shell, window, cx| {
                if leaving != focus {
                    assert!(shuffling(shell), "the tiles leave {leaving} for {focus}");
                }
                shuffle_from(shell, leaving, cx);
                shell.close_pane(close, window, cx);
            })
            .expect("the pane closes");
        draw(window, cx);
    }
    window
        .update(cx, |shell, window, cx| {
            for id in shell.workspace.tabs.clone() {
                shell.close(id, window, cx);
            }
        })
        .expect("the tabs close");
    draw(window, cx);
}

/// `pane_tile_stack` for every pane count, focus and leaving index,
/// including the frame a tab draws with no panes left and indexes past the
/// end (index out of bounds in 0.0.7 and 0.1.1).
#[gpui::test]
fn the_tile_stack_draws_for_any_focus_and_leaving_index(cx: &mut TestAppContext) {
    let (window, _, _) = super::tests::window(cx);
    let shell = window.root(cx).expect("the shell is open");
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let marks = || {
        [
            PaneMark::Session("claude".into(), None),
            PaneMark::Session("codex".into(), Some(1)),
            PaneMark::File(Some("main.rs".into())),
            PaneMark::File(None),
            PaneMark::Pilot("Delivery".into(), 0x4da3f5),
            PaneMark::Terminal,
        ]
    };
    for panes in 0..=marks().len() {
        for focus in 0..=panes + 1 {
            for leaving in [
                None,
                Some(0),
                Some(panes.saturating_sub(1)),
                Some(panes + 3),
            ] {
                for shuffle in [0., 0.5, 1.] {
                    let copy = TabCopy {
                        headline: "Headline".into(),
                        panes: marks()
                            .into_iter()
                            .take(panes)
                            .enumerate()
                            .map(|(ix, mark)| TabPane {
                                key: (ix as i64, false),
                                title: format!("Pane {ix}"),
                                mark,
                            })
                            .collect(),
                        focus,
                    };
                    visual.draw(
                        gpui::point(px(0.), px(0.)),
                        size(px(120.), px(40.)),
                        |_, cx| shell.read(cx).pane_tile_stack(&copy, leaving, shuffle),
                    );
                }
            }
        }
    }
}
