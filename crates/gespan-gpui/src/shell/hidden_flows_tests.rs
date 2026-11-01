//! With `flows::FLOWS_VISIBLE` off, nothing in the shell leads to Flows.
//! Delete this file when turning it back on.
use super::*;
use crate::workspace::{AppView, TitleTab};
use gpui::{TestAppContext, VisualTestContext, WindowHandle};

/// A window whose rail is `compact` or full, in the original or new layout.
fn setup(cx: &mut TestAppContext, classic: bool, compact: bool) -> WindowHandle<Shell> {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, session) = super::tests::window(cx);
    cx.simulate_window_resize(window.into(), gpui::size(px(1500.), px(900.)));
    window
        .update(cx, |shell, window, cx| {
            shell.workspace.classic_layout = classic;
            shell.workspace.sidebar_visible = !compact;
            shell.workspace.project_rail_open = !compact;
            shell.workspace.project_rail_width = 220.;
            shell.open(session, window, cx);
            shell.layout_changed(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
}

/// Whether `prefix`'s Automations and Flows entries are on screen.
fn entries(prefix: &str, visual: &mut VisualTestContext) -> (bool, bool) {
    visual.refresh().unwrap();
    let shown = |visual: &mut VisualTestContext, label: &str| {
        let selector = Box::leak(format!("{prefix}-{label}").into_boxed_str());
        visual.debug_bounds(selector).is_some()
    };
    (shown(visual, "Automations"), shown(visual, "Flows"))
}

const LAYOUTS: [(&str, bool, bool); 3] = [
    ("sidebar-navigation", true, false),
    ("sidebar-navigation", false, false),
    ("rail-navigation", true, true),
];

#[gpui::test]
fn no_navigation_list_offers_flows(cx: &mut TestAppContext) {
    for (prefix, classic, compact) in LAYOUTS {
        let window = setup(cx, classic, compact);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        assert_eq!(
            entries(prefix, visual),
            (true, false),
            "{prefix} classic={classic}"
        );
    }
}

/// The lists above do draw a Flows entry once the switch allows it, so the
/// test above fails if a selector goes stale instead of passing for nothing.
#[gpui::test]
fn every_navigation_list_offers_flows_when_the_switch_is_on(cx: &mut TestAppContext) {
    cx.update(crate::flows::show_for_test);
    for (prefix, classic, compact) in LAYOUTS {
        let window = setup(cx, classic, compact);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        assert_eq!(
            entries(prefix, visual),
            (true, true),
            "{prefix} classic={classic}"
        );
    }
}

#[gpui::test]
fn no_route_opens_the_flows_view(cx: &mut TestAppContext) {
    let window = setup(cx, false, false);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let focused = window
        .read_with(visual, |shell, _| shell.workspace.active)
        .unwrap();
    window
        .update(visual, |shell, window, cx| {
            shell.open_flows(window, cx);
            shell.open_title_tab(&TitleTab::App(AppView::Flows), window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    window
        .read_with(visual, |shell, _| {
            assert_eq!(shell.active_app_view, None);
            assert!(shell.flows.is_none());
            assert!(
                !shell
                    .workspace
                    .title_order
                    .contains(&TitleTab::App(AppView::Flows))
            );
            assert_eq!(shell.workspace.active, focused, "the session stays open");
        })
        .unwrap();
    assert!(visual.debug_bounds("flows-view").is_none());
}
