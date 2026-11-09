//! An agent's browser tab beside its thread: "Show in Gespan" opens it in
//! the thread's review pane (one pane a tab), the first tab an agent opens
//! in the thread the user is looking at shows on its own once, and closing
//! the pane's tab leaves the agent's tab alone.
use super::*;
use crate::{browser_test_rig::Rig, review::ReviewTab, shell::tests::window};
use gespan_core::browser::{TabId, TabState, session_key, view::session_tabs};
use gpui::{TestAppContext, VisualTestContext, WindowHandle};

fn shell(rig: &Rig, boxy: bool, cx: &mut TestAppContext) -> (WindowHandle<Shell>, i64, i64) {
    let host = rig.host.clone();
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        crate::browser::install_host(host, cx);
    });
    let (window, other, session) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.workspace.new_layout_boxy = boxy;
            shell.open(session, w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    (window, other, session)
}

/// The host changed: the app reads it again, as following it would.
fn refresh(window: WindowHandle<Shell>, cx: &mut TestAppContext) {
    cx.update(crate::browser::refresh);
    cx.run_until_parked();
    window.update(cx, |_, w, _| w.refresh()).unwrap();
    cx.run_until_parked();
}

fn browser_tabs(shell: &Shell, session: i64, cx: &App) -> Vec<ReviewTab> {
    shell.reviews.get(&session).map_or_else(Vec::new, |review| {
        review
            .read(cx)
            .state
            .tabs
            .iter()
            .filter(|tab| matches!(tab, ReviewTab::Browser { .. }))
            .cloned()
            .collect()
    })
}

fn browser_tab(session: i64, tab: &TabId) -> ReviewTab {
    ReviewTab::Browser {
        session,
        tab: tab.0.clone(),
    }
}

#[gpui::test]
fn show_in_gespan_opens_the_tab_once_in_both_layouts(cx: &mut TestAppContext) {
    for boxy in [false, true] {
        let rig = Rig::start();
        let (window, _, session) = shell(&rig, boxy, cx);
        let tab = rig.open_tab(session);
        refresh(window, cx);
        // Shown on its own already (the first tab): close it, then ask.
        window
            .update(cx, |shell, _, cx| {
                if let Some(review) = shell.reviews.get(&session) {
                    review.update(cx, |r, cx| r.close_active(cx));
                }
            })
            .unwrap();
        cx.run_until_parked();
        let show = |cx: &mut TestAppContext| {
            window
                .update(cx, |shell, _, cx| {
                    shell.sessions[&session]
                        .update(cx, |chat, cx| chat.show_browser_tab(tab.0.clone(), cx));
                })
                .unwrap();
            cx.run_until_parked();
            window.update(cx, |_, w, _| w.refresh()).unwrap();
            cx.run_until_parked();
        };
        show(cx);
        let first = window
            .update(cx, |shell, _, cx| {
                assert_eq!(
                    browser_tabs(shell, session, cx),
                    [browser_tab(session, &tab)],
                    "boxy={boxy}"
                );
                shell.reviews[&session]
                    .read(cx)
                    .browser_pane_for_tests(&tab.0)
            })
            .unwrap()
            .expect("the review pane holds the tab's live pane");
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        let stage = visual
            .debug_bounds("browser-pane-stage")
            .expect("the pane is drawn beside the thread");
        assert!(
            stage.size.width > px(100.) && stage.size.height > px(100.),
            "boxy={boxy}: {stage:?}"
        );
        // Again: the same pane takes focus; no second view of the tab,
        // which would stop the first.
        show(cx);
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(browser_tabs(shell, session, cx).len(), 1, "boxy={boxy}");
                let again = shell.reviews[&session]
                    .read(cx)
                    .browser_pane_for_tests(&tab.0)
                    .unwrap();
                assert_eq!(again, first);
                let review = shell.reviews[&session].read(cx);
                assert_eq!(
                    review.state.tabs[review.state.active],
                    browser_tab(session, &tab)
                );
            })
            .unwrap();
        cx.update(|cx| first.update(cx, |pane, cx| pane.pull_for_tests(cx)));
        cx.update(|cx| {
            assert_ne!(
                first.read(cx).ended,
                Some(gespan_core::browser::Ended::Stopped),
                "the one view keeps going"
            );
        });
    }
}

#[gpui::test]
fn a_browser_tab_behind_another_stops_its_view_until_shown(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (window, _, session) = shell(&rig, false, cx);
    let tab = rig.open_tab(session);
    refresh(window, cx);
    let pane = window
        .update(cx, |shell, _, cx| {
            shell.reviews[&session]
                .read(cx)
                .browser_pane_for_tests(&tab.0)
                .unwrap()
        })
        .unwrap();
    cx.update(|cx| assert!(pane.read(cx).is_live(), "shown, it streams"));
    let project = window
        .update(cx, |shell, _, _| shell.workspace.project.clone())
        .unwrap();
    window
        .update(cx, |shell, w, cx| {
            shell.open_review(
                ReviewTab::Changes {
                    cwd: project,
                    path: None,
                    staged: None,
                },
                w,
                cx,
            );
        })
        .unwrap();
    refresh(window, cx);
    cx.update(|cx| assert!(!pane.read(cx).is_live(), "behind another tab, it stops"));
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&session]
                .update(cx, |chat, cx| chat.show_browser_tab(tab.0.clone(), cx));
        })
        .unwrap();
    refresh(window, cx);
    cx.update(|cx| assert!(pane.read(cx).is_live(), "in front again, it streams again"));
    // Every view the pane started was its own: none stopped another.
    cx.update(|cx| pane.update(cx, |pane, cx| pane.pull_for_tests(cx)));
    cx.update(|cx| assert_eq!(pane.read(cx).ended, None));
}

#[gpui::test]
fn the_first_agent_tab_shows_on_its_own_once_per_thread(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (window, other, session) = shell(&rig, false, cx);
    window
        .update(cx, |shell, w, cx| {
            let input = shell.sessions[&session].read(cx).input.clone();
            w.focus(&input.focus_handle(cx), cx);
        })
        .unwrap();
    cx.run_until_parked();
    let first = rig.open_tab(session);
    refresh(window, cx);
    window
        .update(cx, |shell, w, cx| {
            assert_eq!(
                browser_tabs(shell, session, cx),
                [browser_tab(session, &first)]
            );
            let input = shell.sessions[&session].read(cx).input.clone();
            assert!(
                input.focus_handle(cx).is_focused(w),
                "the composer keeps the keys: typing never lands in the page"
            );
            shell.reviews[&session].update(cx, |r, cx| r.close_active(cx));
        })
        .unwrap();
    cx.run_until_parked();
    // Once a thread: its next tab stays in the globe's list.
    let second = rig.open_tab(session);
    refresh(window, cx);
    window
        .update(cx, |shell, _, cx| {
            assert!(browser_tabs(shell, session, cx).is_empty());
            assert!(
                session_tabs(&rig.host.state(), &session_key(session)).any(|t| t.tab.tab == second)
            );
        })
        .unwrap();
    // A thread the user is not looking at shows nothing; it still has its
    // once for when they are.
    rig.open_tab(other);
    refresh(window, cx);
    window
        .update(cx, |shell, w, cx| {
            assert!(browser_tabs(shell, other, cx).is_empty());
            assert!(browser_tabs(shell, session, cx).is_empty());
            shell.open(other, w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let theirs = rig.open_tab(other);
    refresh(window, cx);
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(
                browser_tabs(shell, other, cx),
                [browser_tab(other, &theirs)]
            );
        })
        .unwrap();
}

#[gpui::test]
fn closing_the_pane_tab_leaves_the_agents_tab_open(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (window, _, session) = shell(&rig, false, cx);
    let tab = rig.open_tab(session);
    refresh(window, cx);
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(browser_tabs(shell, session, cx).len(), 1);
            shell.reviews[&session].update(cx, |r, cx| r.close_active(cx));
        })
        .unwrap();
    cx.run_until_parked();
    window.update(cx, |_, w, _| w.refresh()).unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert!(browser_tabs(shell, session, cx).is_empty());
        })
        .unwrap();
    let (state, key) = (rig.host.state(), session_key(session));
    let left = session_tabs(&state, &key)
        .find(|t| t.tab.tab == tab)
        .expect("the agent still has its tab");
    assert_eq!(left.tab.state, TabState::Open);
    assert!(
        !rig.server
            .received()
            .iter()
            .any(|r| r.method == "Target.closeTarget"),
        "nothing closed the tab"
    );
}
