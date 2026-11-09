//! The first-run setup card over the shell: it can't be closed, it blocks the
//! shell's shortcuts, it applies picks live and it finishes into a project.
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
fn a_pending_setup_opens_the_card_over_the_shell(cx: &mut TestAppContext) {
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
fn escape_and_scrim_clicks_leave_setup_open(cx: &mut TestAppContext) {
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
    let window = pending(cx, SetupStep::Welcome);
    let count = |cx: &mut TestAppContext| {
        window
            .update(cx, |shell, _, _| shell.session_summaries.len())
            .unwrap()
    };
    let before = count(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-t"
    } else {
        "ctrl-t"
    });
    visual.run_until_parked();
    assert_eq!(count(cx), before);
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
    emit(window, SetupEvent::Layout { classic: true }, cx);
    assert!(
        window
            .update(cx, |shell, _, _| shell.workspace.classic_layout)
            .unwrap()
    );
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
