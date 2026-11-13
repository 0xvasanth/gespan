//! The top bar's three tab layouts: this project's tabs only, every
//! project's side by side, or every project's grouped by project. And
//! opening another project's tab moves the sidebar to that project.
use super::*;
use crate::shell::tests::window;
use crate::strip_groups::StripEntry;
use crate::workspace::TitleTab;
use gpui::{TestAppContext, VisualTestContext, WindowHandle};

fn set_layout(window: WindowHandle<Shell>, layout: &str, visual: &mut VisualTestContext) {
    window
        .update(visual, |shell, window, cx| {
            let mut prefs = shell.workspace.preferences.clone();
            prefs.title_tabs = layout.into();
            shell.set_preferences(prefs, window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual.run_until_parked();
}

fn drawn(visual: &mut VisualTestContext, name: String) -> bool {
    visual
        .debug_bounds(Box::leak(name.into_boxed_str()))
        .is_some()
}

fn any_chip(visual: &mut VisualTestContext) -> bool {
    drawn(visual, "tab-group-chip-gespan".into())
        || drawn(visual, "tab-group-chip-another-project".into())
}

fn all_loose(window: WindowHandle<Shell>, visual: &mut VisualTestContext) -> bool {
    window
        .update(visual, |shell, _, cx| {
            shell
                .strip_entries(cx)
                .iter()
                .all(|entry| matches!(entry, StripEntry::Loose(_)))
        })
        .unwrap()
}

#[gpui::test]
fn grouped_is_the_default_layout(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    visual.run_until_parked();
    window
        .update(visual, |shell, _, _| {
            assert_eq!(
                shell.workspace.preferences.tab_layout(),
                crate::settings::TabLayout::Grouped
            );
        })
        .unwrap();
    assert!(drawn(visual, "tab-group-chip-another-project".into()));
}

#[gpui::test]
fn this_project_layout_shows_only_the_current_projects_tabs(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    set_layout(window, "project", visual);
    assert!(drawn(visual, format!("title-tab-{here}")));
    assert!(
        !drawn(visual, format!("title-tab-{other}")),
        "another project's"
    );
    assert!(!any_chip(visual), "no groups");
    assert!(all_loose(window, visual));

    // Switching project swaps the strip to that project's tabs.
    window
        .update(visual, |shell, window, cx| {
            shell.select_project("/work/another-project".into(), window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual.run_until_parked();
    assert!(drawn(visual, format!("title-tab-{other}")));
    assert!(!drawn(visual, format!("title-tab-{here}")));
}

#[gpui::test]
fn all_projects_layout_shows_every_tab_without_groups(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    set_layout(window, "all", visual);
    assert!(drawn(visual, format!("title-tab-{here}")));
    assert!(
        drawn(visual, format!("title-tab-{other}")),
        "another project's too"
    );
    assert!(!any_chip(visual), "no groups");
    assert!(all_loose(window, visual));

    // Back to grouped, the chips return.
    set_layout(window, "grouped", visual);
    assert!(drawn(visual, "tab-group-chip-another-project".into()));
}

#[gpui::test]
fn reordering_in_a_flat_layout_moves_only_the_dragged_tab(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    set_layout(window, "all", visual);
    window
        .update(visual, |shell, _, cx| {
            let before = shell.global_strip_tabs(cx);
            let last = before.last().unwrap().clone();
            shell.reorder_title_tab(&last, 0, cx);
            let after = shell.global_strip_tabs(cx);
            assert_eq!(after[0], last);
            assert_eq!(after.len(), before.len());
            assert!(after.contains(&TitleTab::Session(other)));
            assert!(after.contains(&TitleTab::Session(here)));
        })
        .unwrap();
}

#[gpui::test]
fn opening_another_projects_tab_moves_the_sidebar_there(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    set_layout(window, "all", visual);
    let far = window
        .update(visual, |shell, _, cx| {
            shell.workspace.add_project("/work/far-project");
            let summary = locked(&shell.store)
                .unwrap()
                .create("codex", "/work/far-project")
                .unwrap();
            let id = summary.id;
            shell.add(summary, cx);
            let active = shell.workspace.active;
            shell.workspace.open(id);
            shell.workspace.active = active;
            shell.workspace.collapsed.insert("/work/far-project".into());
            shell.selected_sessions.insert(here);
            shell.selection_anchor = Some(here);
            shell.invalidate_chrome(cx);
            id
        })
        .unwrap();
    visual.run_until_parked();
    window
        .update(visual, |shell, window, cx| {
            shell.open_title_tab(&TitleTab::Session(far), window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.workspace.project, "/work/far-project");
            assert!(!shell.workspace.collapsed.contains("/work/far-project"));
            assert!(shell.selected_sessions.is_empty(), "the old selection ends");
            assert_eq!(shell.selection_anchor, None);
        })
        .unwrap();
}

#[gpui::test]
fn opening_a_tab_of_the_same_project_keeps_the_selection(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    window
        .update(visual, |shell, window, cx| {
            shell.selected_sessions.insert(here);
            shell.open_title_tab(&TitleTab::Session(here), window, cx);
            assert!(shell.selected_sessions.contains(&here));
        })
        .unwrap();
}

fn git(repo: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Removes the worktrees Gespan made beside the repository.
struct Worktrees(std::path::PathBuf);
impl Drop for Worktrees {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(gespan_core::worktree_management::default_root(&self.0));
    }
}

#[gpui::test]
fn a_chat_that_starts_in_a_worktree_lists_and_opens_under_its_project(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let repo = tempfile::tempdir().unwrap();
    let _cleanup = Worktrees(repo.path().into());
    for args in [
        &["init", "-b", "main"][..],
        &["config", "user.email", "fixture@example.invalid"],
        &["config", "user.name", "Fixture"],
        &["commit", "--allow-empty", "-m", "Initial"],
    ] {
        git(repo.path(), args);
    }
    let tree = gespan_core::git::create_named_worktree(repo.path(), "fix")
        .unwrap()
        .path;
    let root = gespan_core::git::linked_worktree_root(std::path::Path::new(&tree))
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        std::fs::canonicalize(&root).unwrap(),
        std::fs::canonicalize(repo.path()).unwrap()
    );
    let (window, _, _) = window(cx);
    let id = window
        .update(cx, |shell, _, cx| {
            // A run, a Pilot or another app started this chat in the worktree.
            let summary = locked(&shell.store)
                .unwrap()
                .create("codex", &tree)
                .unwrap();
            let id = summary.id;
            shell.add(summary, cx);
            let active = shell.workspace.active;
            shell.workspace.open(id);
            shell.workspace.active = active;
            shell.invalidate_chrome(cx);
            id
        })
        .unwrap();
    window
        .update(cx, |shell, _, cx| {
            assert!(
                !shell.workspace.projects.contains(&tree),
                "a worktree is never a project of its own: {:?}",
                shell.workspace.projects
            );
            assert!(shell.workspace.projects.contains(&root));
            assert_eq!(shell.session_summaries[&id].cwd, root);
            assert_eq!(
                shell.tab_project(&TitleTab::Session(id), cx),
                Some(root.clone())
            );
            // It keeps working in its worktree, now and after a restart.
            assert_eq!(shell.session_worktrees.get(&id), Some(&Some(tree.clone())));
            let store = locked(&shell.store).unwrap();
            let saved = store.sessions().unwrap();
            assert_eq!(saved.iter().find(|s| s.id == id).unwrap().cwd, root);
            assert_eq!(
                store.agent_settings(id).unwrap().unwrap().worktree_cwd,
                Some(tree.clone())
            );
        })
        .unwrap();
    window
        .update(cx, |shell, window, cx| {
            shell.open_title_tab(&TitleTab::Session(id), window, cx);
            assert_eq!(
                shell.workspace.project, root,
                "the sidebar shows its project"
            );
        })
        .unwrap();
}
