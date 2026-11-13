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
