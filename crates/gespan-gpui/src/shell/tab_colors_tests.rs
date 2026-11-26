//! Every project tab in the ungrouped strip carries its project's colour,
//! each project's colour is saved and distinct from the others', and the
//! active or hovered tab names its project.
use super::*;
use crate::shell::tests::window;
use crate::workspace::{TitleTab, Workspace};
use gpui::{TestAppContext, VisualTestContext};

fn selector(name: String) -> &'static str {
    Box::leak(name.into_boxed_str())
}

fn drawn(visual: &mut VisualTestContext, name: String) -> bool {
    visual.debug_bounds(selector(name)).is_some()
}

fn settle(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual.run_until_parked();
}

/// The colour `project` is drawn in.
fn color_of(
    window: gpui::WindowHandle<Shell>,
    project: &str,
    visual: &mut VisualTestContext,
) -> u32 {
    window
        .update(visual, |shell, _, _| {
            shell.appearance(project).color(project)
        })
        .unwrap()
}

#[gpui::test]
fn every_project_tab_carries_its_projects_colour(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    let gespan = color_of(window, "/work/gespan", visual);
    let another = color_of(window, "/work/another-project", visual);
    assert_ne!(gespan, another, "two projects, two colours");
    window
        .update(visual, |shell, _, cx| {
            assert_eq!(shell.tab_color(&TitleTab::Session(here), cx), Some(gespan));
            assert_eq!(
                shell.tab_color(&TitleTab::Session(other), cx),
                Some(another)
            );
        })
        .unwrap();
    // Drawn: each tab leads with its project's mark in that colour.
    assert!(drawn(
        visual,
        format!("title-tab-project-{here}-{gespan:06x}")
    ));
    assert!(drawn(
        visual,
        format!("title-tab-project-{other}-{another:06x}")
    ));
    assert!(!drawn(
        visual,
        format!("title-tab-project-{here}-{another:06x}")
    ));
}

#[gpui::test]
fn an_app_tab_has_no_project_colour(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let board = TitleTab::App(app_views::AppView::Board);
    window
        .update(visual, |shell, w, cx| {
            shell.workspace.preferences.board = true;
            shell.open_title_tab(&board, w, cx);
        })
        .unwrap();
    settle(visual);
    window
        .update(visual, |shell, _, cx| {
            assert!(shell.global_strip_tabs(cx).contains(&board));
            assert_eq!(shell.tab_color(&board, cx), None);
        })
        .unwrap();
}

#[gpui::test]
fn a_colour_picked_in_the_project_menu_recolours_its_tabs(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    let another = color_of(window, "/work/another-project", visual);
    let picked = 0x37beb4;
    assert_ne!(color_of(window, "/work/gespan", visual), picked);
    window
        .update(visual, |shell, _, cx| {
            let mut appearance = shell.appearance("/work/gespan");
            appearance.color = Some(picked);
            shell.save_project_appearance("/work/gespan".into(), appearance, cx);
        })
        .unwrap();
    settle(visual);
    assert!(drawn(
        visual,
        format!("title-tab-project-{here}-{picked:06x}")
    ));
    // Other projects keep theirs.
    assert!(drawn(
        visual,
        format!("title-tab-project-{other}-{another:06x}")
    ));
}

#[gpui::test]
fn the_active_tab_names_its_project_and_a_hovered_one_does_too(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(1400.), px(800.)));
    settle(visual);
    let active = window
        .update(visual, |shell, _, _| shell.workspace.active)
        .unwrap();
    assert_eq!(active, Some(here));
    assert!(drawn(visual, format!("title-tab-project-name-{here}")));
    assert!(
        !drawn(visual, format!("title-tab-project-name-{other}")),
        "inactive"
    );
    // Resting on the other tab names its project.
    let at = visual
        .debug_bounds(selector(format!("title-tab-{other}")))
        .unwrap();
    visual.simulate_mouse_move(at.center(), None, Default::default());
    settle(visual);
    assert!(drawn(visual, format!("title-tab-project-name-{other}")));
}

#[gpui::test]
fn a_tab_at_its_narrowest_still_shows_its_projects_mark(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(900.), px(700.)));
    let crowd = window
        .update(visual, |shell, _, cx| {
            (0..40)
                .map(|ix| {
                    let project = if ix % 2 == 0 {
                        "/work/gespan"
                    } else {
                        "/work/another-project"
                    };
                    let summary = locked(&shell.store)
                        .unwrap()
                        .create("codex", project)
                        .unwrap();
                    let id = summary.id;
                    shell.add(summary, cx);
                    let active = shell.workspace.active;
                    shell.workspace.open(id);
                    shell.workspace.active = active;
                    shell.invalidate_chrome(cx);
                    (id, project)
                })
                .collect::<Vec<_>>()
        })
        .unwrap();
    settle(visual);
    let gespan = color_of(window, "/work/gespan", visual);
    let another = color_of(window, "/work/another-project", visual);
    // Every drawn tab, squeezed to a plate, still leads with its mark.
    let mut checked = 0;
    for (id, project) in crowd
        .iter()
        .copied()
        .chain([(here, "/work/gespan"), (other, "/work/another-project")])
    {
        let Some(tab) = visual.debug_bounds(selector(format!("title-tab-{id}"))) else {
            continue;
        };
        assert!(tab.size.width < px(NARROW_TAB), "tab {id} is narrow");
        let color = if project == "/work/gespan" {
            gespan
        } else {
            another
        };
        assert!(
            drawn(visual, format!("title-tab-project-{id}-{color:06x}")),
            "tab {id} of {project}"
        );
        assert!(!drawn(visual, format!("title-tab-project-name-{id}")));
        checked += 1;
    }
    assert!(checked > 10, "{checked} tabs on screen");
}

#[test]
fn projects_get_distinct_colours_that_survive_a_restart() {
    // These two paths hash to the same colour.
    let site = "/Users/me/work/gespan-site";
    let mobile = "/Users/me/work/mobile";
    let hashed = |p: &str| gespan_core::project_appearance::ProjectAppearance::default().color(p);
    assert_eq!(hashed(site), hashed(mobile));
    let mut state = Workspace::default();
    state.add_project(site);
    state.add_project(mobile);
    let colors = |state: &Workspace| {
        [site, mobile].map(|p| state.project_appearance[p].color.expect("saved"))
    };
    let first = colors(&state);
    assert_ne!(first[0], first[1]);
    assert_eq!(first[0], hashed(site), "the first keeps its own");
    let saved = serde_json::to_string(&state).unwrap();
    let restored = Workspace::restore(Some(&saved), &[], site);
    assert_eq!(colors(&restored), first, "stable across a restart");
}

#[test]
fn a_workspace_saved_before_colours_were_saved_gets_them_on_restore() {
    let saved = serde_json::json!({
        "projects": ["/Users/me/work/gespan-site", "/Users/me/work/mobile", "/Users/me/work/docs"],
        "project": "/Users/me/work/docs",
    })
    .to_string();
    let restored = Workspace::restore(Some(&saved), &[], "/Users/me/work/docs");
    let colors: std::collections::HashSet<u32> = restored
        .projects
        .iter()
        .map(|p| restored.project_appearance[p].color.expect("saved"))
        .collect();
    assert_eq!(colors.len(), 3, "{colors:x?}");
}
