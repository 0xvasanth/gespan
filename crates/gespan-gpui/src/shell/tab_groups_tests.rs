//! One title strip for every project: each project's tabs sit together as a
//! group, pins from any project share the deck, and shortcuts still walk
//! only the current project's tabs.
use super::*;
use crate::shell::tests::window;
use crate::strip_groups::StripEntry;
use crate::workspace::TitleTab;
use gpui::TestAppContext;

fn add_session(shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) -> i64 {
    let summary = locked(&shell.store)
        .unwrap()
        .create("codex", &shell.workspace.project)
        .unwrap();
    let id = summary.id;
    shell.add(summary, cx);
    let focus = shell.worktree_focus.get(&shell.workspace.project).cloned();
    shell.seed_worktree(id, focus.as_ref(), cx);
    shell.open(id, window, cx);
    id
}

fn groups(entries: Vec<StripEntry<TitleTab>>) -> Vec<(String, Vec<TitleTab>)> {
    entries
        .into_iter()
        .filter_map(|entry| match entry {
            StripEntry::Group { project, tabs } => Some((project, tabs)),
            StripEntry::Loose(_) => None,
        })
        .collect()
}

#[gpui::test]
fn strip_groups_every_project(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(
                groups(shell.strip_entries(cx)),
                [
                    (
                        "/work/another-project".to_string(),
                        vec![TitleTab::Session(other)]
                    ),
                    ("/work/gespan".to_string(), vec![TitleTab::Session(here)]),
                ]
            );
        })
        .unwrap();
}

#[gpui::test]
fn opening_a_tab_joins_its_project_group(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    let board = TitleTab::App(app_views::AppView::Board);
    window
        .update(cx, |shell, w, cx| {
            shell.workspace.preferences.board = true;
            shell.open_title_tab(&board, w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let second = window.update(cx, add_session).unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            shell.persist(cx);
            let order = &shell.workspace.title_order;
            let at = |tab: &TitleTab| order.iter().position(|t| t == tab).unwrap();
            let (first, second, board) = (
                at(&TitleTab::Session(here)),
                at(&TitleTab::Session(second)),
                at(&board),
            );
            assert_eq!(second, first + 1, "gespan's sessions sit together");
            assert!(
                board > second,
                "Board stays after the group it was opened behind"
            );
            assert!(at(&TitleTab::Session(other)) < first);
            // The session list follows the strip's new order.
            let sessions: Vec<i64> = order.iter().filter_map(TitleTab::session).collect();
            assert_eq!(shell.workspace.tabs, sessions);
        })
        .unwrap();
}

#[gpui::test]
fn pins_from_every_project_show_in_the_deck(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.workspace.project, "/work/gespan");
            shell.toggle_pin_tab(TitleTab::Session(other), cx);
            assert_eq!(shell.pinned_visible(cx), [TitleTab::Session(other)]);
            let groups = groups(shell.strip_entries(cx));
            assert_eq!(
                groups,
                [("/work/gespan".to_string(), vec![TitleTab::Session(here)])],
                "a pinned tab leaves its project's group"
            );
        })
        .unwrap();
}

#[gpui::test]
fn unloaded_sessions_are_not_grouped(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.session_summaries.remove(&other);
            let entries = shell.strip_entries(cx);
            let tabs: Vec<TitleTab> = entries
                .iter()
                .flat_map(|entry| match entry {
                    StripEntry::Loose(tab) => vec![tab.clone()],
                    StripEntry::Group { tabs, .. } => tabs.clone(),
                })
                .collect();
            assert!(!tabs.contains(&TitleTab::Session(other)));
            assert!(tabs.contains(&TitleTab::Session(here)));
            assert!(
                entries.iter().all(
                    |entry| !matches!(entry, StripEntry::Group { project, .. } if project.is_empty())
                ),
                "no group without a project"
            );
            assert_eq!(shell.tab_project(&TitleTab::Session(other), cx), None);
        })
        .unwrap();
}

#[gpui::test]
fn shortcut_scope_is_unchanged(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            let sessions: Vec<TitleTab> = shell
                .visible_tabs(cx)
                .into_iter()
                .filter(|tab| tab.session().is_some())
                .collect();
            assert_eq!(sessions, [TitleTab::Session(here)]);
            assert!(!shell.strip_tabs(cx).contains(&TitleTab::Session(other)));
            assert!(
                shell
                    .global_strip_tabs(cx)
                    .contains(&TitleTab::Session(other))
            );
        })
        .unwrap();
}

#[gpui::test]
fn a_tab_in_a_worktree_names_it_and_one_in_the_project_folder_does_not(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.tab_worktree_label(&TitleTab::Session(here), cx), None);
            // A trailing slash is the same folder.
            shell
                .workspace
                .tab_workspaces
                .insert(here, "/work/gespan/".into());
            assert_eq!(shell.tab_worktree_label(&TitleTab::Session(here), cx), None);
            shell
                .workspace
                .tab_workspaces
                .insert(here, "/work/gespan-fix-tabs".into());
            assert_eq!(
                shell.tab_worktree_label(&TitleTab::Session(here), cx),
                Some("gespan-fix-tabs".into())
            );
            assert_eq!(
                shell.tab_worktree_label(&TitleTab::Session(other), cx),
                None
            );
            let pilot = TitleTab::Pilot {
                id: "p".into(),
                project: "/work/gespan".into(),
                worktree: "/work/gespan-pilot".into(),
            };
            assert_eq!(
                shell.tab_worktree_label(&pilot, cx),
                Some("gespan-pilot".into())
            );
            assert_eq!(shell.tab_project(&pilot, cx), Some("/work/gespan".into()));
            assert_eq!(
                shell.tab_project(&TitleTab::App(app_views::AppView::Board), cx),
                None
            );
        })
        .unwrap();
}
