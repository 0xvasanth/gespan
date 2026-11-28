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
        if id == here {
            // The selected tab stays expanded, its mark leading.
            assert!(drawn(
                visual,
                format!("title-tab-project-{id}-{gespan:06x}")
            ));
            continue;
        }
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

/// Rests the pointer on tab `id` for `wait`.
fn rest_on(visual: &mut VisualTestContext, id: i64, wait: std::time::Duration) {
    let tab = visual
        .debug_bounds(selector(format!("title-tab-{id}")))
        .unwrap_or_else(|| panic!("tab {id} drawn"));
    visual.simulate_mouse_move(tab.center(), None, Default::default());
    visual.executor().advance_clock(wait);
    settle(visual);
}

#[gpui::test]
fn resting_on_a_tab_hangs_the_designed_tooltip_under_it(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(1400.), px(800.)));
    settle(visual);
    let tip = format!("title-tab-tooltip-{other}");
    // Passing over a tab shows nothing yet.
    rest_on(visual, other, tab_colors::TAB_TIP_DELAY / 2);
    assert!(!drawn(visual, tip.clone()), "not before the delay");
    // Resting on it hangs the card 6px under the plate, from its left edge.
    visual.executor().advance_clock(tab_colors::TAB_TIP_DELAY);
    settle(visual);
    let card = visual
        .debug_bounds(selector(tip.clone()))
        .expect("the tooltip shows after the delay");
    let tab = visual
        .debug_bounds(selector(format!("title-tab-{other}")))
        .unwrap();
    let plate_bottom =
        tab.top() + px((crate::theme::TITLE_BAR_HEIGHT + crate::theme::TAB_PLATE_HEIGHT) / 2.);
    assert!(
        (card.top() - plate_bottom - px(6.)).abs() <= px(1.),
        "under the plate: {card:?} {tab:?}"
    );
    assert!(
        (card.left() - tab.left()).abs() <= px(1.),
        "from its left edge"
    );
    assert!(
        (card.size.height - px(26.)).abs() <= px(1.),
        "one row: {card:?}"
    );
    // Its row: the project's mark and name in its colour, then the status.
    let shown = window
        .update(visual, |shell, _, cx| {
            shell.tab_tooltip(other, Some("/work/another-project"), cx)
        })
        .unwrap();
    let another = color_of(window, "/work/another-project", visual);
    assert_eq!(
        shown.project,
        Some(("another-project".into(), another)),
        "the project, in its colour"
    );
    assert_eq!(shown.detail(), "· idle");
    // Moving to another tab takes it down.
    rest_on(visual, here, std::time::Duration::ZERO);
    assert!(!drawn(visual, tip), "gone once the pointer leaves");
}

#[gpui::test]
fn the_tooltip_says_whether_a_tab_needs_you_works_or_rests(cx: &mut TestAppContext) {
    let (window, other, _) = window(cx);
    let status = |window: gpui::WindowHandle<Shell>, cx: &mut TestAppContext| {
        window
            .update(cx, |shell, _, cx| shell.tab_status(other, cx))
            .unwrap()
    };
    assert_eq!(status(window, cx), tab_colors::TabStatus::Idle);
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&other].update(cx, |view, cx| {
                view.busy = true;
                cx.emit(crate::conversation::SessionChanged);
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(status(window, cx), tab_colors::TabStatus::Working);
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&other].update(cx, |view, cx| {
                view.set_approvals_for_test(vec![gespan_core::agent::Approval {
                    id: "a1".into(),
                    title: "Run a command?".into(),
                    detail: "git status".into(),
                    can_remember: false,
                    tool: None,
                }]);
                cx.emit(crate::conversation::SessionChanged);
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(status(window, cx), tab_colors::TabStatus::NeedsYou);
}

#[test]
fn the_tooltip_row_reads_branch_then_status() {
    let tip = |branch: Option<&str>, status| tab_colors::TabTooltip::for_test(branch, status);
    assert_eq!(
        tip(Some("main"), tab_colors::TabStatus::Working).detail(),
        "· main · working"
    );
    assert_eq!(
        tip(Some("feat/limits"), tab_colors::TabStatus::NeedsYou).detail(),
        "· feat/limits · needs you"
    );
    assert_eq!(tip(None, tab_colors::TabStatus::Idle).detail(), "· idle");
}
