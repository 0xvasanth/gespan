//! The first-run setup screen: it can't be closed, it blocks the shell's
//! shortcuts, it locks both navs shut, it applies picks live and it finishes
//! into a project with the navs open again.
use super::*;
use crate::setup::{SetupStep, card::SetupEvent};
use crate::workspace::Workspace;
use gpui::{TestAppContext, VisualTestContext, WindowHandle};

fn pending(cx: &mut TestAppContext, step: SetupStep) -> WindowHandle<Shell> {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = super::tests::window(cx);
    cx.simulate_window_resize(window.into(), gpui::size(px(1440.), px(900.)));
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.setup = Some(step);
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
}

fn emit(window: WindowHandle<Shell>, event: SetupEvent, cx: &mut TestAppContext) {
    window
        .update(cx, |shell, _, cx| {
            let card = shell.setup.as_ref().expect("setup is open").view.clone();
            card.update(cx, |_, cx| cx.emit(event));
        })
        .unwrap();
    cx.run_until_parked();
}

fn open(window: WindowHandle<Shell>, cx: &mut TestAppContext) -> bool {
    window
        .update(cx, |shell, _, _| {
            shell.setup.is_some() && shell.workspace.setup.is_some()
        })
        .unwrap()
}

#[gpui::test]
fn a_pending_setup_opens_the_card(cx: &mut TestAppContext) {
    let window = pending(cx, SetupStep::Look);
    assert!(open(window, cx));
    let step = window
        .update(cx, |shell, _, cx| {
            shell.setup.as_ref().unwrap().view.read(cx).step()
        })
        .unwrap();
    assert_eq!(step, SetupStep::Look);
}

#[gpui::test]
fn escape_and_clicks_beside_the_card_leave_setup_open(cx: &mut TestAppContext) {
    let window = pending(cx, SetupStep::Welcome);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    visual.simulate_click(gpui::point(px(8.), px(450.)), Default::default());
    visual.run_until_parked();
    assert!(open(window, cx));
}

#[gpui::test]
fn shell_shortcuts_are_blocked_while_setup_is_open(cx: &mut TestAppContext) {
    cx.update(super::bind_keys);
    let window = pending(cx, SetupStep::Welcome);
    let state = |cx: &mut TestAppContext| {
        window
            .update(cx, |shell, _, _| {
                (
                    shell.session_summaries.len(),
                    shell.settings.is_some(),
                    shell.workspace.projects.clone(),
                )
            })
            .unwrap()
    };
    let before = state(cx);
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    for keys in ["t", "n", ",", "o", "w"] {
        visual.simulate_keystrokes(&format!("{m}-{keys}"));
        visual.run_until_parked();
    }
    assert_eq!(
        state(cx),
        before,
        "no shortcut reaches the shell under the card"
    );
    assert!(open(window, cx));
}

#[gpui::test]
fn look_picks_apply_to_the_shell_live(cx: &mut TestAppContext) {
    let window = pending(cx, SetupStep::Look);
    let mut prefs = window
        .update(cx, |shell, _, _| shell.workspace.preferences.clone())
        .unwrap();
    prefs.theme = "light".into();
    emit(window, SetupEvent::Preferences(Box::new(prefs)), cx);
    let theme = window
        .update(cx, |shell, _, _| shell.workspace.preferences.theme.clone())
        .unwrap();
    assert_eq!(theme, "light");
}

#[gpui::test]
fn picking_a_default_agent_saves_it(cx: &mut TestAppContext) {
    let window = pending(cx, SetupStep::Agents);
    emit(window, SetupEvent::DefaultAgent(Kind::Claude), cx);
    let last = window
        .update(cx, |shell, _, _| {
            shell.workspace.preferences.last_used_provider()
        })
        .unwrap();
    assert_eq!(last, Some(Kind::Claude));
}

#[gpui::test]
fn quitting_on_project_resumes_on_project(cx: &mut TestAppContext) {
    let window = pending(cx, SetupStep::Look);
    emit(window, SetupEvent::Step(SetupStep::Project), cx);
    let saved = window
        .update(cx, |shell, _, _| shell.latest_state.lock().unwrap().clone())
        .unwrap();
    let restored = Workspace::restore(Some(&saved), &[], "/work/gespan");
    assert_eq!(restored.setup, Some(SetupStep::Project));
}

#[gpui::test]
fn finishing_setup_selects_the_project_and_opens_a_thread(cx: &mut TestAppContext) {
    let window = pending(cx, SetupStep::Project);
    emit(window, SetupEvent::Finish("/work/new-project".into()), cx);
    window
        .update(cx, |shell, _, _| {
            assert!(shell.setup.is_none());
            assert_eq!(shell.workspace.setup, None);
            assert_eq!(shell.workspace.project, "/work/new-project");
            assert!(
                shell
                    .workspace
                    .projects
                    .contains(&"/work/new-project".to_string())
            );
            assert!(
                shell
                    .session_summaries
                    .values()
                    .any(|s| s.cwd == "/work/new-project"),
                "a new thread opens in the picked project"
            );
        })
        .unwrap();
}

#[gpui::test]
fn a_fresh_install_starts_with_no_sessions_and_the_welcome_card(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let store = gespan_core::store::Store::open(std::path::Path::new(":memory:")).unwrap();
    let store = Arc::new(Mutex::new(store));
    let saved = crate::setup::rules::with_pending_setup(None);
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| Shell::new(store, vec![], "/Users/a".into(), Some(saved), cx))
        })
        .unwrap()
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert!(shell.session_summaries.is_empty());
            let card = shell.setup.as_ref().expect("setup opens").view.read(cx);
            assert_eq!(card.step(), SetupStep::Welcome);
        })
        .unwrap();
}

#[gpui::test]
fn run_setup_again_opens_welcome_with_current_values_and_keeps_projects(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = super::tests::window(cx);
    window
        .update(cx, |shell, window, cx| {
            shell.workspace.preferences.theme = "dark".into();
            shell.run_setup_again(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            let card = shell.setup.as_ref().expect("setup opens").view.read(cx);
            assert_eq!(card.step(), SetupStep::Welcome);
            assert_eq!(card.prefs().theme, "dark");
        })
        .unwrap();
    emit(window, SetupEvent::Finish("/work/gespan".into()), cx);
    let projects = window
        .update(cx, |shell, _, _| shell.workspace.projects.clone())
        .unwrap();
    assert!(projects.contains(&"/work/gespan".to_string()));
    assert!(projects.contains(&"/work/another-project".to_string()));
}

#[gpui::test]
fn run_setup_again_never_removes_a_project_without_sessions(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = super::tests::window(cx);
    window
        .update(cx, |shell, window, cx| {
            shell.setup_launch = "/work/unused".into();
            shell.workspace.add_project("/work/unused");
            shell.run_setup_again(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    emit(window, SetupEvent::Finish("/work/gespan".into()), cx);
    let projects = window
        .update(cx, |shell, _, _| shell.workspace.projects.clone())
        .unwrap();
    assert!(projects.contains(&"/work/unused".to_string()));
}

#[gpui::test]
fn a_sign_in_that_never_starts_does_not_leave_the_card_waiting(cx: &mut TestAppContext) {
    let window = pending(cx, SetupStep::Agents);
    let card = window
        .update(cx, |shell, _, _| shell.setup.as_ref().unwrap().view.clone())
        .unwrap();
    card.update(cx, |card, cx| card.sign_in(Kind::Codex, cx));
    cx.run_until_parked();
    assert_eq!(card.read_with(cx, |card, _| card.signing_in()), None);
}

#[gpui::test]
fn a_second_window_does_not_show_setup(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = super::tests::window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.secondary = true;
            shell.workspace.setup = Some(SetupStep::Welcome);
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    assert!(
        window
            .update(cx, |shell, _, _| shell.setup.is_none())
            .unwrap()
    );
}

/// The sidebar's and right panel's saved open state.
fn panels(window: WindowHandle<Shell>, cx: &mut TestAppContext) -> (bool, bool) {
    window
        .update(cx, |shell, _, _| {
            (
                shell.workspace.sidebar_visible,
                shell.workspace.right_panel_open,
            )
        })
        .unwrap()
}

/// Whether the shell drew `selector` on its last frame.
fn drawn(window: WindowHandle<Shell>, selector: &'static str, cx: &mut TestAppContext) -> bool {
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.debug_bounds(selector).is_some()
}

/// Every way to open or close a nav: the title bar toggles, ⌘B and ⌘⇧B,
/// the actions the command palette and the app menu dispatch, and hovering
/// the sidebar toggle, which peeks a closed sidebar out.
fn try_every_toggle(window: WindowHandle<Shell>, cx: &mut TestAppContext) {
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    for selector in ["toggle-sidebar", "toggle-right-panel"] {
        let bounds = visual.debug_bounds(selector).expect("the toggle is drawn");
        visual.simulate_mouse_move(bounds.center(), None, Default::default());
        visual.run_until_parked();
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
    }
    for keys in ["b", "shift-b"] {
        visual.simulate_keystrokes(&format!("{m}-{keys}"));
        visual.run_until_parked();
    }
    visual.update(|window, cx| {
        window.dispatch_action(Box::new(ToggleSidebar), cx);
        window.dispatch_action(Box::new(ToggleRightPanel), cx);
        window.dispatch_action(Box::new(SearchFiles), cx);
    });
    visual.run_until_parked();
}

#[gpui::test]
fn pending_setup_shows_the_card_alone_with_both_navs_closed(cx: &mut TestAppContext) {
    let window = pending(cx, SetupStep::Welcome);
    // The saved state has the sidebar open; setup draws it closed anyway.
    assert!(panels(window, cx).0);
    assert!(drawn(window, "setup-card", cx));
    assert!(drawn(window, "setup-title-bar", cx));
    assert!(!drawn(window, "sidebar-frame", cx), "no sidebar");
    assert!(!drawn(window, "rounded-main-pane", cx), "no workspace");
    assert!(!drawn(window, "title-tab-strip", cx), "no tabs");

    // Centred on the content plane.
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let plane = visual.debug_bounds("setup-plane").unwrap();
    let card = visual.debug_bounds("setup-card").unwrap();
    let off = |a: gpui::Pixels, b: gpui::Pixels| (a - b).as_f32().abs();
    assert!(
        off(plane.center().x, card.center().x) < 1.,
        "{plane:?} {card:?}"
    );
    assert!(
        off(plane.center().y, card.center().y) < 1.,
        "{plane:?} {card:?}"
    );
    assert_eq!(card.size.width, px(crate::setup::ui::WIDTH));
}

#[gpui::test]
fn toggles_shortcuts_and_commands_leave_both_navs_closed_during_setup(cx: &mut TestAppContext) {
    cx.update(super::bind_keys);
    let window = pending(cx, SetupStep::Agents);
    let before = panels(window, cx);
    try_every_toggle(window, cx);
    assert_eq!(panels(window, cx), before, "the saved state is untouched");
    assert!(!drawn(window, "sidebar-frame", cx));
    let peeking = window
        .update(cx, |shell, _, _| shell.sidebar_peek.is_some())
        .unwrap();
    assert!(!peeking, "hovering the locked toggle peeks nothing out");
    assert!(open(window, cx));

    // What a relaunch would load still has the user's own layout.
    emit(window, SetupEvent::Step(SetupStep::Look), cx);
    let saved = window
        .update(cx, |shell, _, _| shell.latest_state.lock().unwrap().clone())
        .unwrap();
    let restored = Workspace::restore(Some(&saved), &[], "/work/gespan");
    assert_eq!(
        (restored.sidebar_visible, restored.right_panel_open),
        before
    );
}

#[gpui::test]
fn the_locked_toggles_say_why_they_do_nothing(cx: &mut TestAppContext) {
    let window = pending(cx, SetupStep::Welcome);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let bar = visual.debug_bounds("setup-title-bar").unwrap();
    for selector in ["toggle-sidebar", "toggle-right-panel"] {
        let toggle = visual.debug_bounds(selector).unwrap();
        assert!(
            bar.contains(&toggle.center()),
            "{selector} is in the title bar"
        );
    }
    assert_eq!(
        super::first_run::SIDEBAR_LOCKED,
        "Finish setup to open the sidebar"
    );
    assert_eq!(
        super::first_run::PANEL_LOCKED,
        "Finish setup to open the panel"
    );
}

#[gpui::test]
fn finishing_a_fresh_setup_opens_both_navs(cx: &mut TestAppContext) {
    let window = pending(cx, SetupStep::Project);
    window
        .update(cx, |shell, _, _| {
            shell.workspace.sidebar_visible = false;
            shell.workspace.right_panel_open = false;
        })
        .unwrap();
    emit(window, SetupEvent::Finish("/work/gespan".into()), cx);
    assert_eq!(panels(window, cx), (true, true));
    assert!(!drawn(window, "setup-card", cx));
    assert!(drawn(window, "sidebar-frame", cx));

    // Unlocked: the toggles work again.
    cx.update(super::bind_keys);
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_keystrokes(&format!("{m}-b"));
    visual.run_until_parked();
    assert!(!panels(window, cx).0, "⌘B closes the sidebar again");
}

#[gpui::test]
fn run_setup_again_locks_the_navs_then_restores_them(cx: &mut TestAppContext) {
    cx.update(super::bind_keys);
    for before in [(false, true), (true, false)] {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = super::tests::window(cx);
        cx.simulate_window_resize(window.into(), gpui::size(px(1440.), px(900.)));
        window
            .update(cx, |shell, window, cx| {
                shell.workspace.sidebar_visible = before.0;
                shell.workspace.right_panel_open = before.1;
                shell.run_setup_again(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(drawn(window, "setup-card", cx));
        assert!(!drawn(window, "sidebar-frame", cx));
        try_every_toggle(window, cx);
        assert_eq!(panels(window, cx), before, "locked during setup");

        emit(window, SetupEvent::Finish("/work/gespan".into()), cx);
        assert_eq!(panels(window, cx), before, "restored as they were");
        assert_eq!(drawn(window, "sidebar-frame", cx), before.0);
    }
}
