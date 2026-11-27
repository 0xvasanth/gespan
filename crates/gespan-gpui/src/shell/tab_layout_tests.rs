//! The top bar's tabs: always every project's side by side, ungrouped,
//! whatever layout was saved, with the grouped and this-project strips still
//! drawable for their own tests. And opening another project's tab moves the
//! sidebar to that project.
use super::*;
use crate::settings::TabLayout;
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

/// Drives the strip in a layout the app no longer offers, as tests of the
/// grouped and this-project strips do.
fn force_layout(window: WindowHandle<Shell>, layout: TabLayout, visual: &mut VisualTestContext) {
    window
        .update(visual, |shell, _, cx| {
            shell.set_tab_layout_override(Some(layout), cx)
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual.run_until_parked();
}

#[gpui::test]
fn the_strip_is_ungrouped_by_default(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    visual.run_until_parked();
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.workspace.preferences.tab_layout(), TabLayout::All);
        })
        .unwrap();
    assert!(drawn(visual, format!("title-tab-{here}")));
    assert!(drawn(visual, format!("title-tab-{other}")));
    assert!(!any_chip(visual), "no groups");
    assert!(all_loose(window, visual));
}

#[gpui::test]
fn a_saved_grouped_or_this_project_choice_still_draws_every_tab_ungrouped(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    for saved in ["grouped", "project", "anything else"] {
        set_layout(window, saved, visual);
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.tab_layout(), TabLayout::All, "{saved}");
            })
            .unwrap();
        assert!(drawn(visual, format!("title-tab-{here}")), "{saved}");
        assert!(
            drawn(visual, format!("title-tab-{other}")),
            "{saved}: another project's tab too"
        );
        assert!(!any_chip(visual), "{saved}: no groups");
        assert!(all_loose(window, visual), "{saved}");
    }
}

#[gpui::test]
fn every_projects_tabs_share_the_strip(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(1200.), px(700.)));
    // Many tabs of another project: they take their share, so the strip
    // narrows every tab instead of running past its end.
    let away = window
        .update(visual, |shell, _, cx| {
            (0..8)
                .map(|_| {
                    let summary = locked(&shell.store)
                        .unwrap()
                        .create("codex", "/work/another-project")
                        .unwrap();
                    let id = summary.id;
                    shell.add(summary, cx);
                    let active = shell.workspace.active;
                    shell.workspace.open(id);
                    shell.workspace.active = active;
                    shell.invalidate_chrome(cx);
                    id
                })
                .collect::<Vec<_>>()
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual.run_until_parked();
    let width = |visual: &mut VisualTestContext, id: i64| {
        visual
            .debug_bounds(Box::leak(format!("title-tab-{id}").into_boxed_str()))
            .unwrap_or_else(|| panic!("tab {id} drawn"))
            .size
            .width
    };
    // The selected tab stays expanded; every other tab, this project's or
    // another's, takes the same share of the rest.
    assert!((width(visual, here) - px(SELECTED_TAB_WIDTH)).abs() <= px(1.));
    let theirs = width(visual, other);
    assert!(
        theirs < px(SELECTED_TAB_WIDTH),
        "the strip is shared: {theirs:?}"
    );
    for id in &away {
        assert!((width(visual, *id) - theirs).abs() <= px(1.), "tab {id}");
    }
    let max = window
        .update(visual, |shell, _, _| shell.tab_scroll.max_offset().x)
        .unwrap();
    assert_eq!(max, px(0.), "ten tabs fit without scrolling");
}

#[gpui::test]
fn this_project_strip_still_shows_only_the_current_projects_tabs(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    force_layout(window, TabLayout::Project, visual);
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
fn the_grouped_strip_still_draws_its_chips(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    assert!(!any_chip(visual));
    force_layout(window, TabLayout::Grouped, visual);
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

#[gpui::test]
fn a_full_strip_keeps_the_selected_tab_open_and_closable(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(900.), px(700.)));
    // So many tabs that every other one is squeezed to its mark.
    let crowd = window
        .update(visual, |shell, _, cx| {
            (0..40)
                .map(|_| {
                    let summary = locked(&shell.store)
                        .unwrap()
                        .create("codex", "/work/gespan")
                        .unwrap();
                    let id = summary.id;
                    shell.add(summary, cx);
                    let active = shell.workspace.active;
                    shell.workspace.open(id);
                    shell.workspace.active = active;
                    shell.invalidate_chrome(cx);
                    id
                })
                .collect::<Vec<_>>()
        })
        .unwrap();
    let settle = |visual: &mut VisualTestContext| {
        visual.run_until_parked();
        visual.refresh().unwrap();
        visual.run_until_parked();
    };
    settle(visual);
    let bounds = |visual: &mut VisualTestContext, name: String| {
        visual.debug_bounds(Box::leak(name.into_boxed_str()))
    };
    // The selected tab stays expanded: its project, title and close button.
    let tab = bounds(visual, format!("title-tab-{here}")).expect("the selected tab is drawn");
    assert!(
        (tab.size.width - px(SELECTED_TAB_WIDTH)).abs() <= px(2.),
        "expanded: {tab:?}"
    );
    assert!(bounds(visual, format!("title-tab-project-name-{here}")).is_some());
    let close = bounds(visual, format!("close-title-tab-{here}")).expect("selected tab closes");
    assert!(
        tab.contains(&close.center()),
        "{close:?} sits on its tab {tab:?}"
    );
    // The others are select-only plates, so a click on one never closes it.
    let mut others = 0;
    for id in &crowd {
        if let Some(other) = bounds(visual, format!("title-tab-{id}")) {
            assert!(
                other.size.width < px(TAB_CLOSE_MIN_WIDTH),
                "tab {id}: {other:?}"
            );
            assert!(
                bounds(visual, format!("close-title-tab-{id}")).is_none(),
                "tab {id}"
            );
            others += 1;
        }
    }
    assert!(others > 10, "{others} squeezed tabs on screen");
    // Closing it selects another, which expands in its place.
    visual.simulate_mouse_move(close.center(), None, Default::default());
    visual.simulate_click(close.center(), Default::default());
    settle(visual);
    let next = window
        .update(visual, |shell, _, _| {
            assert!(!shell.workspace.tabs.contains(&here), "closed");
            shell
                .workspace
                .active_tab()
                .expect("another tab is selected")
        })
        .unwrap();
    let next_tab = bounds(visual, format!("title-tab-{next}")).expect("drawn");
    assert!(
        (next_tab.size.width - px(SELECTED_TAB_WIDTH)).abs() <= px(2.),
        "{next_tab:?}"
    );
    assert!(bounds(visual, format!("close-title-tab-{next}")).is_some());
}

#[gpui::test]
fn a_roomy_strip_still_gives_every_tab_the_same_width(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(1600.), px(800.)));
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual.run_until_parked();
    let width = |visual: &mut VisualTestContext, id: i64| {
        visual
            .debug_bounds(Box::leak(format!("title-tab-{id}").into_boxed_str()))
            .unwrap()
            .size
            .width
    };
    // Two tabs fit at full width: the selected one is no wider.
    assert_eq!(width(visual, here), px(crate::theme::TITLE_TAB_MAX_WIDTH));
    assert_eq!(width(visual, other), width(visual, here));
}
