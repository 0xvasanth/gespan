//! Tabs a run opened, not you: an automation's chat shows in the strip only
//! while its run works and leaves once it is done, unless you open it.
use super::automation_runs::Running;
use super::*;
use crate::conversation::SessionChanged;
use crate::shell::tests::window;
use crate::workspace::TitleTab;
use gpui::TestAppContext;

/// A chat in `project` that a run started, with its tab added the way a
/// run adds it and the run marked as working.
fn start_run(shell: &mut Shell, project: &str, cx: &mut Context<Shell>) -> i64 {
    let summary = locked(&shell.store)
        .unwrap()
        .create("codex", project)
        .unwrap();
    let id = summary.id;
    shell.add(summary, cx);
    shell.automation_state.running.insert(
        id,
        Running::submitting_for_test(&format!("run-{id}"), "automation", project),
    );
    shell
        .automation_state
        .session_automations
        .insert(id, "automation".into());
    shell.add_run_tab(id);
    shell.invalidate_chrome(cx);
    id
}

/// The run ends the way a chat reports it: the run lets go of the chat and
/// the chat says it changed.
fn finish_run(window: gpui::WindowHandle<Shell>, id: i64, cx: &mut TestAppContext) {
    window
        .update(cx, |shell, _, cx| {
            shell.automation_state.running.remove(&id);
            shell.sessions[&id].update(cx, |view, cx| {
                view.busy = false;
                cx.emit(SessionChanged);
            });
        })
        .unwrap();
    cx.run_until_parked();
}

fn on_strip(window: gpui::WindowHandle<Shell>, id: i64, cx: &mut TestAppContext) -> bool {
    window
        .update(cx, |shell, _, cx| {
            shell.global_strip_tabs(cx).contains(&TitleTab::Session(id))
        })
        .unwrap()
}

#[gpui::test]
fn a_run_tab_shows_while_its_run_works_and_leaves_when_done(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let project = window
        .update(cx, |shell, _, _| shell.workspace.project.clone())
        .unwrap();
    let id = window
        .update(cx, |shell, _, cx| start_run(shell, &project, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(on_strip(window, id, cx), "a working run shows its tab");

    finish_run(window, id, cx);
    window
        .update(cx, |shell, _, _| {
            assert!(!shell.workspace.tabs.contains(&id), "done, its tab closed");
            assert!(!shell.workspace.title_order.contains(&TitleTab::Session(id)));
            assert!(shell.workspace.background_tabs.is_empty());
            // The chat itself stays, listed in the sidebar.
            assert!(shell.session_summaries.contains_key(&id));
        })
        .unwrap();
}

#[gpui::test]
fn a_busy_run_chat_keeps_its_tab_until_its_turn_ends(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let id = window
        .update(cx, |shell, _, cx| {
            start_run(shell, "/work/another-project", cx)
        })
        .unwrap();
    // The run let go of the chat, but its turn still works.
    window
        .update(cx, |shell, _, cx| {
            shell.automation_state.running.remove(&id);
            shell.sessions[&id].update(cx, |view, cx| {
                view.busy = true;
                cx.emit(SessionChanged);
            });
        })
        .unwrap();
    cx.run_until_parked();
    assert!(on_strip(window, id, cx), "busy, it stays");
    finish_run(window, id, cx);
    assert!(!on_strip(window, id, cx), "idle, it leaves");
}

#[gpui::test]
fn a_run_tab_you_open_stays_after_the_run(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let id = window
        .update(cx, |shell, _, cx| {
            start_run(shell, "/work/another-project", cx)
        })
        .unwrap();
    window
        .update(cx, |shell, window, cx| {
            shell.open_title_tab(&TitleTab::Session(id), window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert!(!shell.workspace.background_tabs.contains(&id), "yours now");
        })
        .unwrap();
    finish_run(window, id, cx);
    assert!(on_strip(window, id, cx), "opened by you, it stays");
}

#[gpui::test]
fn a_done_run_tab_never_draws_before_it_is_retired(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let id = window
        .update(cx, |shell, _, cx| {
            start_run(shell, "/work/another-project", cx)
        })
        .unwrap();
    // The run ends without the chat reporting a change yet.
    window
        .update(cx, |shell, _, _| {
            shell.automation_state.running.remove(&id);
        })
        .unwrap();
    assert!(!on_strip(window, id, cx));
}

#[gpui::test]
fn saved_automation_tabs_are_sorted_once(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let (left_over, on_screen, pinned, mine) = window
        .update(cx, |shell, window, cx| {
            let project = shell.workspace.project.clone();
            let open = |shell: &mut Shell, cx: &mut Context<Shell>| {
                let summary = locked(&shell.store)
                    .unwrap()
                    .create("codex", &project)
                    .unwrap();
                let id = summary.id;
                shell.add(summary, cx);
                shell.workspace.open(id);
                id
            };
            let left_over = open(shell, cx);
            let pinned = open(shell, cx);
            let mine = open(shell, cx);
            let on_screen = open(shell, cx);
            shell.toggle_pin_tab(TitleTab::Session(pinned), cx);
            shell.open(on_screen, window, cx);
            // Saved by a build before background tabs existed.
            shell.workspace.background_tabs.clear();
            shell.workspace.background_tabs_sorted = false;
            for id in [left_over, on_screen, pinned] {
                shell
                    .automation_state
                    .session_automations
                    .insert(id, "automation".into());
            }
            shell.sort_saved_automation_tabs(cx);
            (left_over, on_screen, pinned, mine)
        })
        .unwrap();
    window
        .update(cx, |shell, _, _| {
            let tabs = &shell.workspace.tabs;
            assert!(!tabs.contains(&left_over), "a finished run's tab goes");
            assert!(tabs.contains(&on_screen), "the one on screen stays");
            assert!(tabs.contains(&pinned), "a pinned one stays");
            assert!(tabs.contains(&mine), "your own chat stays");
            assert!(shell.workspace.background_tabs_sorted);
        })
        .unwrap();
    // Once sorted, tabs an automation chat holds later are left alone.
    window
        .update(cx, |shell, _, cx| {
            shell
                .automation_state
                .session_automations
                .insert(mine, "automation".into());
            shell.sort_saved_automation_tabs(cx);
            assert!(shell.workspace.tabs.contains(&mine));
        })
        .unwrap();
}

#[test]
fn a_workspace_saved_before_run_tabs_reads_as_unsorted() {
    let saved = serde_json::json!({ "tabs": [4, 7] });
    let workspace: crate::workspace::Workspace = serde_json::from_value(saved).unwrap();
    assert!(workspace.background_tabs.is_empty());
    assert!(!workspace.background_tabs_sorted);
    let mut workspace = workspace;
    workspace.background_tabs.insert(7);
    workspace.background_tabs_sorted = true;
    let round: crate::workspace::Workspace =
        serde_json::from_value(serde_json::to_value(&workspace).unwrap()).unwrap();
    assert!(round.background_tabs.contains(&7));
    assert!(round.background_tabs_sorted);
}
