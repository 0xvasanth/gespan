//! One title strip for every project: each project's tabs sit together as a
//! group, pins from any project share the deck, and shortcuts still walk
//! only the current project's tabs.
use super::*;
use crate::conversation::SessionChanged;
use crate::shell::tab_groups::PEEK_DELAY;
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

fn selector(name: String) -> &'static str {
    Box::leak(name.into_boxed_str())
}

/// Adds a tab for a new chat in `project` without leaving the current one.
fn add_background_tab(shell: &mut Shell, project: &str, cx: &mut Context<Shell>) -> i64 {
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
    id
}

fn drawn(visual: &mut gpui::VisualTestContext, name: String) -> bool {
    visual.debug_bounds(selector(name)).is_some()
}

fn settle(visual: &mut gpui::VisualTestContext) {
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual.run_until_parked();
}

/// Moves the pointer onto `name` and rests there long enough to peek.
fn hover(visual: &mut gpui::VisualTestContext, name: String) {
    pass_over(visual, name);
    visual.executor().advance_clock(PEEK_DELAY);
    settle(visual);
}

/// Moves the pointer onto `name` without resting.
fn pass_over(visual: &mut gpui::VisualTestContext, name: String) {
    let at = visual
        .debug_bounds(selector(name.clone()))
        .unwrap_or_else(|| panic!("{name} drawn"));
    visual.simulate_mouse_move(at.center(), None, Default::default());
    settle(visual);
}

#[gpui::test]
fn other_projects_draw_collapsed_chips(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    assert!(drawn(visual, "tab-group-chip-another-project".into()));
    assert!(drawn(visual, "tab-group-count-another-project".into()));
    assert!(!drawn(visual, format!("title-tab-{other}")), "collapsed");
    assert!(drawn(visual, format!("title-tab-{here}")));
    // The current project's group is expanded behind its own chip.
    let chip = visual.debug_bounds("tab-group-chip-gespan").unwrap();
    let tab = visual
        .debug_bounds(selector(format!("title-tab-{here}")))
        .unwrap();
    assert!(chip.right() <= tab.left());
    assert!(!drawn(visual, "tab-group-count-gespan".into()));
    let group = visual.debug_bounds("tab-group-gespan").unwrap();
    assert!(group.left() <= chip.left() && group.right() >= tab.right());
}

#[gpui::test]
fn hovering_a_chip_peeks_and_leaving_its_box_collapses(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    hover(visual, "tab-group-chip-another-project".into());
    assert!(drawn(visual, format!("title-tab-{other}")), "peeks");
    window
        .update(visual, |shell, _, _| {
            assert_eq!(
                shell.workspace.project, "/work/gespan",
                "hover never switches"
            );
            assert_eq!(shell.strip_peek.as_deref(), Some("/work/another-project"));
        })
        .unwrap();
    // Onto its own tab: still inside the group's box.
    hover(visual, format!("title-tab-{other}"));
    assert!(
        drawn(visual, format!("title-tab-{other}")),
        "stays while inside"
    );
    hover(visual, format!("title-tab-{here}"));
    assert!(!drawn(visual, format!("title-tab-{other}")), "collapses");
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.strip_peek, None);
            assert_eq!(shell.workspace.project, "/work/gespan");
        })
        .unwrap();
}

#[gpui::test]
fn one_group_peeks_at_a_time(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, _) = window(cx);
    let third = window
        .update(cx, |shell, _, cx| {
            add_background_tab(shell, "/work/third-project", cx)
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    assert!(!drawn(visual, format!("title-tab-{third}")));
    hover(visual, "tab-group-chip-another-project".into());
    assert!(drawn(visual, format!("title-tab-{other}")));
    // From a peeking group the next chip opens without a wait.
    pass_over(visual, "tab-group-chip-third-project".into());
    assert!(drawn(visual, format!("title-tab-{third}")));
    assert!(
        !drawn(visual, format!("title-tab-{other}")),
        "only one peeks"
    );
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.strip_peek.as_deref(), Some("/work/third-project"));
        })
        .unwrap();
}

#[gpui::test]
fn running_project_shows_spinner_on_its_chip(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, _) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    assert!(!drawn(visual, "tab-group-running-another-project".into()));
    for busy in [true, false] {
        window
            .update(visual, |shell, _, cx| {
                shell.sessions[&other].update(cx, |session, cx| {
                    session.busy = busy;
                    cx.emit(SessionChanged);
                });
                shell.invalidate_chrome(cx);
            })
            .unwrap();
        settle(visual);
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(shell.group_running(&[TitleTab::Session(other)], cx), busy);
            })
            .unwrap();
        assert_eq!(
            drawn(visual, "tab-group-running-another-project".into()),
            busy
        );
    }
}

#[gpui::test]
fn worktree_tabs_carry_a_label(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    let worktree = window
        .update(cx, |shell, _, cx| {
            let id = add_background_tab(shell, "/work/gespan", cx);
            shell
                .workspace
                .tab_workspaces
                .insert(id, "/work/gespan-fix-tabs".into());
            id
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    assert!(drawn(visual, format!("title-tab-{worktree}")));
    let label = visual
        .debug_bounds(selector(format!("title-tab-worktree-{worktree}")))
        .expect("worktree label");
    let tab = visual
        .debug_bounds(selector(format!("title-tab-{worktree}")))
        .unwrap();
    assert!(tab.left() <= label.left() && label.right() <= tab.right());
    assert!(!drawn(visual, format!("title-tab-worktree-{here}")));
}

#[gpui::test]
fn current_project_without_tabs_draws_no_group(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.close(here);
            shell.invalidate_chrome(cx);
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    assert!(!drawn(visual, "tab-group-gespan".into()));
    assert!(!drawn(visual, "tab-group-chip-gespan".into()));
    assert!(drawn(visual, "tab-group-chip-another-project".into()));
}

#[gpui::test]
fn a_tab_in_another_project_names_its_branch(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    let project = "/work/branch-label-project";
    let fix = "/work/branch-label-project-fix";
    crate::git_cache::remember(
        project,
        &gespan_core::git::Snapshot {
            worktrees: vec![
                gespan_core::git::Worktree {
                    path: project.into(),
                    branch: Some("main".into()),
                    main: true,
                    ..Default::default()
                },
                gespan_core::git::Worktree {
                    path: fix.into(),
                    branch: Some("fix-tabs".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    );
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.workspace.project, "/work/gespan");
            let id = add_background_tab(shell, project, cx);
            shell.workspace.tab_workspaces.insert(id, fix.into());
            assert_eq!(
                shell.tab_worktree_label(&TitleTab::Session(id), cx),
                Some("fix-tabs".into()),
                "the branch, resolved under the tab's own project"
            );
            assert_eq!(shell.worktree_label_in(project, fix), "fix-tabs");
            // The current project's lookup is unchanged.
            assert_eq!(shell.worktree_label("/work/gespan-other"), "gespan-other");
        })
        .unwrap();
}

fn click(visual: &mut gpui::VisualTestContext, name: String) {
    let at = visual
        .debug_bounds(selector(name.clone()))
        .unwrap_or_else(|| panic!("{name} drawn"));
    visual.simulate_mouse_move(at.center(), None, Default::default());
    settle(visual);
    let at = visual.debug_bounds(selector(name)).unwrap();
    visual.simulate_click(at.center(), Default::default());
    settle(visual);
}

#[gpui::test]
fn clicking_a_peeked_tab_switches_project(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    hover(visual, "tab-group-chip-another-project".into());
    click(visual, format!("title-tab-{other}"));
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.workspace.project, "/work/another-project");
            assert_eq!(shell.workspace.active, Some(other));
            assert_eq!(shell.strip_peek, None);
        })
        .unwrap();
    assert!(
        drawn(visual, format!("title-tab-{other}")),
        "now the open group"
    );
    assert!(drawn(visual, "tab-group-chip-gespan".into()));
    assert!(
        !drawn(visual, format!("title-tab-{here}")),
        "the old group collapses"
    );
    assert!(drawn(visual, "tab-group-count-gespan".into()));
}

#[gpui::test]
fn clicking_a_chip_switches_project(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, _) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    click(visual, "tab-group-chip-another-project".into());
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.workspace.project, "/work/another-project");
            assert_eq!(shell.workspace.active, Some(other), "its last tab");
            assert_eq!(shell.strip_peek, None);
        })
        .unwrap();
    assert!(!drawn(visual, "tab-group-count-another-project".into()));
}

#[gpui::test]
fn closing_the_active_group_opens_the_nearest_tab(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            assert_eq!(shell.workspace.active, Some(here));
            shell.close_group("/work/gespan", w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.workspace.active, Some(other), "the tab to its left");
            assert_eq!(shell.workspace.project, "/work/another-project");
            assert!(!shell.workspace.tabs.contains(&here));
            assert!(
                !shell
                    .global_strip_tabs(cx)
                    .contains(&TitleTab::Session(here))
            );
            let sessions = locked(&shell.store).unwrap().sessions().unwrap();
            let closed = sessions.iter().find(|s| s.id == here).expect("kept");
            assert!(!closed.archived, "closing a tab never archives its chat");
        })
        .unwrap();
}

#[gpui::test]
fn closing_a_group_prefers_the_tab_to_its_right(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    let third = window
        .update(cx, |shell, _, cx| {
            add_background_tab(shell, "/work/third-project", cx)
        })
        .unwrap();
    window
        .update(cx, |shell, w, cx| {
            shell.close_group("/work/gespan", w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.workspace.active, Some(third));
            assert_eq!(shell.workspace.project, "/work/third-project");
            assert!(!shell.workspace.tabs.contains(&here));
            assert!(
                shell
                    .global_strip_tabs(cx)
                    .contains(&TitleTab::Session(other))
            );
        })
        .unwrap();
}

#[gpui::test]
fn closing_a_collapsed_group_from_its_chip_leaves_the_current_tab(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    click(visual, "close-tab-group-another-project".into());
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.workspace.project, "/work/gespan", "no switch");
            assert_eq!(shell.workspace.active, Some(here));
            assert!(!shell.workspace.tabs.contains(&other));
        })
        .unwrap();
    assert!(!drawn(visual, "tab-group-another-project".into()));
}

#[gpui::test]
fn closing_a_group_keeps_its_pins(cx: &mut TestAppContext) {
    let (window, _, here) = window(cx);
    let pinned = window.update(cx, add_session).unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            shell.toggle_pin_tab(TitleTab::Session(pinned), cx);
            shell.close_group("/work/gespan", w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert!(
                shell
                    .workspace
                    .title_order
                    .contains(&TitleTab::Session(pinned))
            );
            assert_eq!(shell.workspace.pinned_tabs, [TitleTab::Session(pinned)]);
            assert!(!shell.workspace.tabs.contains(&here));
        })
        .unwrap();
}

#[gpui::test]
fn crowded_strip_scrolls_instead_of_squeezing(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(900.), px(700.)));
    // More tabs than fit even as select-only plates.
    let ids = window
        .update(visual, |shell, _, cx| {
            (0..40)
                .map(|_| add_background_tab(shell, "/work/gespan", cx))
                .collect::<Vec<_>>()
        })
        .unwrap();
    settle(visual);
    let width = |visual: &mut gpui::VisualTestContext, id: i64| {
        visual
            .debug_bounds(selector(format!("title-tab-{id}")))
            .unwrap_or_else(|| panic!("tab {id} drawn"))
            .size
            .width
    };
    for id in &ids {
        let w = width(visual, *id);
        assert!(w >= px(TAB_MIN_WIDTH), "tab {id} squeezed to {w:?}");
        assert!(w < px(TAB_CLOSE_MIN_WIDTH), "tab {id} gave way: {w:?}");
    }
    // The selected tab is a plate like the rest.
    assert!(
        (width(visual, here) - px(TAB_MIN_WIDTH)).abs() <= px(1.),
        "selected"
    );
    let max = window
        .update(visual, |shell, _, _| shell.tab_scroll.max_offset().x)
        .unwrap();
    assert!(max > px(0.), "the strip scrolls");
    // The new-tab button stays in sight at the strip's end, outside the
    // scrolled tabs.
    let new_tab = visual.debug_bounds("new-tab").expect("new-tab drawn");
    let strip = visual.debug_bounds("title-tab-strip").unwrap();
    let view = window
        .update(visual, |shell, _, _| shell.tab_scroll.bounds())
        .unwrap();
    assert!(strip.left() <= new_tab.left() && new_tab.right() <= strip.right());
    assert!(new_tab.left() >= view.right(), "{new_tab:?} {view:?}");
    // A plain vertical wheel scrolls the strip sideways.
    let before = window
        .update(visual, |shell, _, _| shell.tab_scroll.offset().x)
        .unwrap();
    let strip = visual.debug_bounds("title-tab-strip").unwrap();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: strip.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(120.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: strip.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-120.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    settle(visual);
    let after = window
        .update(visual, |shell, _, _| shell.tab_scroll.offset().x)
        .unwrap();
    assert_ne!(before, after, "the wheel moved the strip");
}

#[gpui::test]
fn the_active_tab_scrolls_into_view(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(900.), px(700.)));
    let ids = window
        .update(visual, |shell, _, cx| {
            (0..20)
                .map(|_| add_background_tab(shell, "/work/gespan", cx))
                .collect::<Vec<_>>()
        })
        .unwrap();
    settle(visual);
    let in_view = |visual: &mut gpui::VisualTestContext, id: i64| {
        let tab = visual
            .debug_bounds(selector(format!("title-tab-{id}")))
            .unwrap();
        let view = window
            .update(visual, |shell, _, _| shell.tab_scroll.bounds())
            .unwrap();
        tab.left() >= view.left() && tab.right() <= view.right()
    };
    let last = *ids.last().unwrap();
    assert!(!in_view(visual, last), "starts past the edge");
    window
        .update(visual, |shell, w, cx| shell.open(last, w, cx))
        .unwrap();
    settle(visual);
    settle(visual);
    assert!(
        in_view(visual, last),
        "the last tab, inside its group, shows"
    );
    window
        .update(visual, |shell, w, cx| shell.open(here, w, cx))
        .unwrap();
    settle(visual);
    settle(visual);
    assert!(in_view(visual, here), "and back to the first");
}

#[gpui::test]
fn moving_from_one_chip_to_the_next_keeps_the_next_peeked(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(1400.), px(700.)));
    // A full current group makes the strip scroll; small groups follow.
    let (wide, third) = window
        .update(visual, |shell, _, cx| {
            for _ in 0..20 {
                add_background_tab(shell, "/work/gespan", cx);
            }
            let wide: Vec<i64> = (0..3)
                .map(|_| add_background_tab(shell, "/work/wide-project", cx))
                .collect();
            let third = add_background_tab(shell, "/work/third-project", cx);
            // Collapsed groups after them leave room to scroll either way.
            for ix in 0..8 {
                add_background_tab(shell, &format!("/work/after-{ix}"), cx);
            }
            (wide, third)
        })
        .unwrap();
    settle(visual);
    // Scroll the first small group's chip to near the strip's left edge.
    let chip = visual.debug_bounds("tab-group-chip-wide-project").unwrap();
    window
        .update(visual, |shell, _, cx| {
            let view = shell.tab_scroll.bounds();
            let offset = shell.tab_scroll.offset();
            let x = offset.x - (chip.left() - view.left() - px(40.));
            assert!(x < px(0.));
            shell.tab_scroll.set_offset(gpui::point(x, offset.y));
            shell.tabs_frame.update(cx, |_, cx| cx.notify());
        })
        .unwrap();
    settle(visual);
    hover(visual, "tab-group-chip-wide-project".into());
    for id in &wide {
        assert!(drawn(visual, format!("title-tab-{id}")), "peeks");
    }
    let chip = visual.debug_bounds("tab-group-chip-third-project").unwrap();
    hover(visual, "tab-group-chip-third-project".into());
    // Settle again, so any hover change the new layout causes has landed.
    settle(visual);
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.strip_peek.as_deref(), Some("/work/third-project"));
        })
        .unwrap();
    assert!(drawn(visual, format!("title-tab-{third}")));
    for id in &wide {
        assert!(
            !drawn(visual, format!("title-tab-{id}")),
            "the first closed"
        );
    }
    // Closed fully: its box is its chip and insets, no gap.
    let wide_box = visual.debug_bounds("tab-group-wide-project").unwrap();
    let wide_chip = visual.debug_bounds("tab-group-chip-wide-project").unwrap();
    assert!(
        wide_box.size.width <= wide_chip.size.width + px(9.),
        "{wide_box:?} {wide_chip:?}"
    );
    // The strip scrolled back by what closed, so the chip stayed put.
    let held = visual.debug_bounds("tab-group-chip-third-project").unwrap();
    assert!(
        (held.left() - chip.left()).abs() <= px(3.),
        "{held:?} vs {chip:?}"
    );
    assert!(held.contains(&chip.center()));
}

#[gpui::test]
fn a_pilot_gone_from_the_roster_opens_its_project(cx: &mut TestAppContext) {
    let (window, other, _) = window(cx);
    let pilot = window
        .update(cx, |shell, w, cx| {
            shell.select_project("/work/another-project".into(), w, cx);
            let manager = shell.pilot_manager();
            manager.update(cx, |m, cx| m.create(Vec::new(), w, cx));
            manager.read(cx).active.clone().unwrap()
        })
        .unwrap();
    cx.run_until_parked();
    let tab = window
        .update(cx, |shell, w, cx| {
            let tab = shell.pilot_tab(pilot.clone());
            shell.select_project("/work/gespan".into(), w, cx);
            shell
                .pilot_manager()
                .update(cx, |m, _| m.roster.pilots.retain(|p| p.id != pilot));
            tab
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| shell.open_title_tab(&tab, w, cx))
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.workspace.project, "/work/another-project");
            assert_eq!(shell.workspace.active, Some(other), "its chat, not nothing");
            assert_eq!(shell.pilot_manager().read(cx).active, None);
        })
        .unwrap();
}

#[gpui::test]
fn pilot_tabs_in_a_worktree_carry_a_label(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let pilot = window
        .update(cx, |shell, w, cx| {
            let manager = shell.pilot_manager();
            manager.update(cx, |m, cx| m.create(Vec::new(), w, cx));
            manager.read(cx).active.clone().unwrap()
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            let tab = shell.pilot_tab(pilot.clone());
            let at = shell
                .workspace
                .title_order
                .iter()
                .position(|t| *t == tab)
                .unwrap();
            shell.workspace.title_order[at] = TitleTab::Pilot {
                id: pilot.clone(),
                project: "/work/gespan".into(),
                worktree: "/work/gespan-pilot-fix".into(),
            };
            shell.invalidate_chrome(cx);
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    let tab = visual
        .debug_bounds(selector(format!("pilot-title-tab-{pilot}")))
        .expect("pilot tab drawn");
    let label = visual
        .debug_bounds(selector(format!("title-tab-worktree-{pilot}")))
        .expect("worktree label");
    assert!(tab.left() <= label.left() && label.right() <= tab.right());
}

#[gpui::test]
fn a_pinned_session_of_another_project_switches_to_it(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, _) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.toggle_pin_tab(TitleTab::Session(other), cx)
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    click(visual, "pinned-tab-0".into());
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.workspace.project, "/work/another-project");
            assert_eq!(shell.workspace.active, Some(other));
        })
        .unwrap();
}

#[gpui::test]
fn a_pinned_pilot_of_another_project_switches_to_it(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    let pilot = window
        .update(cx, |shell, w, cx| {
            shell.select_project("/work/another-project".into(), w, cx);
            let manager = shell.pilot_manager();
            manager.update(cx, |m, cx| m.create(Vec::new(), w, cx));
            manager.read(cx).active.clone().unwrap()
        })
        .unwrap();
    cx.run_until_parked();
    let tab = window
        .update(cx, |shell, w, cx| {
            let tab = shell.pilot_tab(pilot.clone());
            assert!(shell.workspace.title_order.contains(&tab));
            shell.select_project("/work/gespan".into(), w, cx);
            shell.toggle_pin_tab(tab.clone(), cx);
            tab
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert_eq!(shell.workspace.project, "/work/gespan");
            assert_eq!(shell.workspace.active, Some(here));
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    click(visual, "pinned-tab-0".into());
    window
        .update(visual, |shell, _, cx| {
            assert_eq!(shell.workspace.project, "/work/another-project");
            assert_eq!(
                shell.pilot_manager().read(cx).active.as_deref(),
                Some(pilot.as_str())
            );
            assert_eq!(shell.active_title_tab(cx), Some(tab.clone()));
            let pilots = shell
                .workspace
                .title_order
                .iter()
                .filter(|t| matches!(t, TitleTab::Pilot { id, .. } if *id == pilot))
                .count();
            assert_eq!(pilots, 1, "no second tab for the same Pilot");
        })
        .unwrap();
}

/// Each project's tabs sit side by side in `title_order`.
fn assert_groups_contiguous(shell: &Shell, cx: &App) {
    let order = &shell.workspace.title_order;
    let mut seen: Vec<String> = Vec::new();
    let mut last: Option<String> = None;
    for tab in order {
        let project = shell.tab_project(tab, cx);
        if project != last
            && let Some(project) = &project
        {
            assert!(!seen.contains(project), "{project} split in {order:?}");
            seen.push(project.clone());
        }
        last = project;
    }
}

fn entry_names(entries: Vec<StripEntry<TitleTab>>) -> Vec<String> {
    entries
        .into_iter()
        .map(|entry| match entry {
            StripEntry::Loose(tab) => format!("{tab:?}"),
            StripEntry::Group { project, .. } => project,
        })
        .collect()
}

#[gpui::test]
fn reordering_inside_a_group_stays_in_the_group(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    let board = TitleTab::App(app_views::AppView::Board);
    window
        .update(cx, |shell, w, cx| {
            shell.workspace.preferences.board = true;
            shell.open_title_tab(&board, w, cx);
        })
        .unwrap();
    let second = window
        .update(cx, |shell, _, cx| {
            add_background_tab(shell, "/work/gespan", cx)
        })
        .unwrap();
    window
        .update(cx, |shell, _, cx| {
            // Board sits between the two groups, so a strip slot and a slot
            // in the group differ.
            shell.workspace.title_order = vec![
                TitleTab::Session(other),
                board.clone(),
                TitleTab::Session(here),
                TitleTab::Session(second),
            ];
            shell.reorder_title_tab(&TitleTab::Session(second), 0, cx);
            assert_eq!(
                shell.workspace.title_order,
                [
                    TitleTab::Session(other),
                    board.clone(),
                    TitleTab::Session(second),
                    TitleTab::Session(here),
                ],
                "the group's order swaps and Board keeps its place"
            );
            assert_eq!(shell.workspace.tabs, [other, second, here]);
        })
        .unwrap();
}

#[gpui::test]
fn dragging_a_chip_moves_the_whole_group(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    let board = TitleTab::App(app_views::AppView::Board);
    window
        .update(cx, |shell, w, cx| {
            shell.workspace.preferences.board = true;
            shell.open_title_tab(&board, w, cx);
        })
        .unwrap();
    window
        .update(cx, |shell, _, cx| {
            let other2 = add_background_tab(shell, "/work/another-project", cx);
            shell.workspace.title_order = vec![
                TitleTab::Session(other),
                TitleTab::Session(other2),
                TitleTab::Session(here),
                board.clone(),
            ];
            assert_eq!(
                entry_names(shell.strip_entries(cx)),
                ["/work/another-project", "/work/gespan", "App(Board)"]
            );
            shell.reorder_entry(0, 2, cx);
            assert_eq!(
                entry_names(shell.strip_entries(cx)),
                ["/work/gespan", "App(Board)", "/work/another-project"]
            );
            assert_eq!(
                shell.workspace.title_order,
                [
                    TitleTab::Session(here),
                    board.clone(),
                    TitleTab::Session(other),
                    TitleTab::Session(other2),
                ],
                "the group's tabs move together, in their order"
            );
            assert_eq!(shell.workspace.tabs, [here, other, other2]);
            // And back to the front.
            shell.reorder_entry(2, 0, cx);
            assert_eq!(
                entry_names(shell.strip_entries(cx)),
                ["/work/another-project", "/work/gespan", "App(Board)"]
            );
        })
        .unwrap();
}

#[gpui::test]
fn app_tab_cannot_land_inside_a_group(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    let board = TitleTab::App(app_views::AppView::Board);
    window
        .update(cx, |shell, w, cx| {
            shell.workspace.preferences.board = true;
            shell.open_title_tab(&board, w, cx);
        })
        .unwrap();
    window
        .update(cx, |shell, _, cx| {
            let second = add_background_tab(shell, "/work/gespan", cx);
            let start = vec![
                TitleTab::Session(other),
                TitleTab::Session(here),
                TitleTab::Session(second),
                board.clone(),
            ];
            let expected = [
                ["App(Board)", "/work/another-project", "/work/gespan"],
                ["/work/another-project", "App(Board)", "/work/gespan"],
                ["/work/another-project", "/work/gespan", "App(Board)"],
                ["/work/another-project", "/work/gespan", "App(Board)"],
            ];
            for (to, expected) in expected.iter().enumerate() {
                shell.workspace.title_order = start.clone();
                shell.reorder_title_tab(&board, to, cx);
                assert_groups_contiguous(shell, cx);
                assert_eq!(entry_names(shell.strip_entries(cx)), expected, "to {to}");
            }
        })
        .unwrap();
}

/// Presses at `from`, drags to `to` and lets go there.
fn drag(visual: &mut gpui::VisualTestContext, from: gpui::Point<Pixels>, to: gpui::Point<Pixels>) {
    use gpui::Modifiers;
    visual.simulate_mouse_move(from, None, Modifiers::default());
    visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    let step = if to.x > from.x { px(8.) } else { px(-8.) };
    visual.simulate_mouse_move(
        gpui::point(from.x + step, from.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.run_until_parked();
    visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    settle(visual);
}

fn group_tabs(shell: &Shell, project: &str, cx: &App) -> Vec<TitleTab> {
    groups(shell.strip_entries(cx))
        .into_iter()
        .find(|(p, _)| p == project)
        .map(|(_, tabs)| tabs)
        .unwrap_or_default()
}

#[gpui::test]
fn a_tab_in_another_worktree_drags_within_the_active_group(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    let worktree = window
        .update(cx, |shell, _, cx| {
            let id = add_background_tab(shell, "/work/gespan", cx);
            shell
                .workspace
                .tab_workspaces
                .insert(id, "/work/gespan-fix-tabs".into());
            id
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    window
        .update(visual, |shell, _, cx| {
            assert!(
                !shell.strip_tabs(cx).contains(&TitleTab::Session(worktree)),
                "outside the focused worktree's scope"
            );
        })
        .unwrap();
    let first = visual
        .debug_bounds(selector(format!("title-tab-{here}")))
        .unwrap();
    let moved = visual
        .debug_bounds(selector(format!("title-tab-{worktree}")))
        .unwrap();
    drag(visual, moved.center(), first.center());
    window
        .update(visual, |shell, _, cx| {
            assert_eq!(
                group_tabs(shell, "/work/gespan", cx),
                [TitleTab::Session(worktree), TitleTab::Session(here)]
            );
            assert_eq!(shell.workspace.active, Some(here), "a drag never opens");
            assert_eq!(shell.workspace.project, "/work/gespan");
        })
        .unwrap();
}

#[gpui::test]
fn a_peeking_groups_tabs_drag_within_it(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let other2 = window
        .update(cx, |shell, _, cx| {
            add_background_tab(shell, "/work/another-project", cx)
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    hover(visual, "tab-group-chip-another-project".into());
    let first = visual
        .debug_bounds(selector(format!("title-tab-{other}")))
        .expect("peeks");
    let moved = visual
        .debug_bounds(selector(format!("title-tab-{other2}")))
        .unwrap();
    drag(visual, moved.center(), first.center());
    window
        .update(visual, |shell, _, cx| {
            assert_eq!(
                group_tabs(shell, "/work/another-project", cx),
                [TitleTab::Session(other2), TitleTab::Session(other)]
            );
            assert_eq!(shell.workspace.project, "/work/gespan", "never switches");
            assert_eq!(shell.workspace.active, Some(here));
            assert_eq!(
                shell.strip_peek.as_deref(),
                Some("/work/another-project"),
                "still under the pointer, it still peeks"
            );
        })
        .unwrap();
    // Leaving it afterwards closes the peek as usual.
    hover(visual, format!("title-tab-{here}"));
    window
        .update(visual, |shell, _, _| assert_eq!(shell.strip_peek, None))
        .unwrap();
}

#[gpui::test]
fn dragging_a_chip_by_the_pointer_moves_its_group(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    let chip = visual
        .debug_bounds("tab-group-chip-another-project")
        .unwrap();
    let gespan = visual.debug_bounds("tab-group-gespan").unwrap();
    drag(
        visual,
        chip.center(),
        gpui::point(gespan.right() + px(40.), chip.center().y),
    );
    window
        .update(visual, |shell, _, cx| {
            assert_eq!(
                entry_names(shell.strip_entries(cx)),
                ["/work/gespan", "/work/another-project"]
            );
            let order = &shell.workspace.title_order;
            let at = |id: i64| order.iter().position(|t| *t == TitleTab::Session(id));
            assert!(at(here) < at(other));
            assert_eq!(
                shell.workspace.project, "/work/gespan",
                "a drag never selects"
            );
        })
        .unwrap();
}

#[gpui::test]
fn hovering_a_chip_mid_drag_does_not_peek(cx: &mut TestAppContext) {
    use gpui::Modifiers;
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    let second = window
        .update(cx, |shell, _, cx| {
            add_background_tab(shell, "/work/gespan", cx)
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    let tab = visual
        .debug_bounds(selector(format!("title-tab-{here}")))
        .unwrap()
        .center();
    let chip = visual
        .debug_bounds("tab-group-chip-another-project")
        .unwrap()
        .center();
    visual.simulate_mouse_move(tab, None, Modifiers::default());
    visual.simulate_mouse_down(tab, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(
        tab + gpui::point(px(-8.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.simulate_mouse_move(chip, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(
        chip + gpui::point(px(2.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.run_until_parked();
    window
        .update(visual, |shell, _, _| {
            assert!(shell.tab_drag.as_ref().is_some_and(|d| d.active));
            assert_eq!(shell.strip_peek, None, "the strip holds still under a drag");
        })
        .unwrap();
    visual.simulate_mouse_up(chip, MouseButton::Left, Modifiers::default());
    settle(visual);
    window
        .update(visual, |shell, _, cx| {
            assert_eq!(shell.strip_peek, None);
            assert_eq!(
                group_tabs(shell, "/work/gespan", cx),
                [TitleTab::Session(here), TitleTab::Session(second)],
                "dropped short of the group's start, it stays first"
            );
        })
        .unwrap();
    // Without a drag, the same chip peeks again once the pointer rests.
    visual.simulate_mouse_move(
        chip + gpui::point(px(-2.), px(0.)),
        None,
        Modifiers::default(),
    );
    visual.executor().advance_clock(PEEK_DELAY);
    settle(visual);
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.strip_peek.as_deref(), Some("/work/another-project"));
        })
        .unwrap();
}

#[gpui::test]
fn dropping_over_the_new_tab_button_lands_at_the_end(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    let (second, third) = window
        .update(cx, |shell, _, cx| {
            (
                add_background_tab(shell, "/work/gespan", cx),
                add_background_tab(shell, "/work/gespan", cx),
            )
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    let new_tab = visual.debug_bounds("new-tab").unwrap();
    // A pane dragged over the button is dropped at the strip's end.
    window
        .update(visual, |shell, _, cx| {
            assert!(!shell.tab_scroll.bounds().contains(&new_tab.center()));
            assert_eq!(
                shell.tab_insert_at(new_tab.center(), cx),
                Some(shell.strip_tabs(cx).len())
            );
        })
        .unwrap();
    let tab = visual
        .debug_bounds(selector(format!("title-tab-{here}")))
        .unwrap();
    drag(
        visual,
        tab.center(),
        gpui::point(new_tab.center().x, tab.center().y),
    );
    window
        .update(visual, |shell, _, cx| {
            assert_eq!(
                group_tabs(shell, "/work/gespan", cx),
                [
                    TitleTab::Session(second),
                    TitleTab::Session(third),
                    TitleTab::Session(here)
                ]
            );
        })
        .unwrap();
}

#[gpui::test]
fn dragging_in_a_scrolled_strip_lands_where_dropped(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(900.), px(700.)));
    let ids = window
        .update(visual, |shell, _, cx| {
            // Enough select-only plates to scroll well past the first.
            (0..40)
                .map(|_| add_background_tab(shell, "/work/gespan", cx))
                .collect::<Vec<_>>()
        })
        .unwrap();
    settle(visual);
    window
        .update(visual, |shell, _, cx| {
            let max = shell.tab_scroll.max_offset().x;
            assert!(max > px(200.), "the strip scrolls");
            let offset = shell.tab_scroll.offset();
            shell
                .tab_scroll
                .set_offset(gpui::point(-(max / 2.), offset.y));
            shell.tabs_frame.update(cx, |_, cx| cx.notify());
        })
        .unwrap();
    settle(visual);
    let view = window
        .update(visual, |shell, _, _| {
            assert!(shell.tab_scroll.offset().x < px(-100.), "scrolled");
            shell.tab_scroll.bounds()
        })
        .unwrap();
    // Two tabs fully in view, two slots apart.
    let order: Vec<i64> = std::iter::once(here).chain(ids.iter().copied()).collect();
    let visible: Vec<usize> = order
        .iter()
        .enumerate()
        .filter(|(_, id)| {
            visual
                .debug_bounds(selector(format!("title-tab-{id}")))
                .is_some_and(|b| b.left() >= view.left() && b.right() <= view.right())
        })
        .map(|(ix, _)| ix)
        .collect();
    let from = visible[0];
    assert!(from > 0, "the first tabs scrolled out of view");
    let to = from + 2;
    assert!(visible.contains(&to));
    let a = visual
        .debug_bounds(selector(format!("title-tab-{}", order[from])))
        .unwrap();
    let b = visual
        .debug_bounds(selector(format!("title-tab-{}", order[to])))
        .unwrap();
    drag(visual, a.center(), b.center());
    window
        .update(visual, |shell, _, cx| {
            let tabs = group_tabs(shell, "/work/gespan", cx);
            assert_eq!(
                tabs.iter()
                    .position(|t| *t == TitleTab::Session(order[from])),
                Some(to),
                "lands on the slot under the pointer"
            );
        })
        .unwrap();
}

/// Seconds past the idle limit, so a tab focused this long ago is idle.
const LONG_AGO: i64 = crate::strip_groups::IDLE_AFTER_SECS + 1;

fn now() -> i64 {
    super::title_tabs::unix_now()
}

#[gpui::test]
fn idle_tab_leaves_the_strip_but_stays_in_the_sidebar(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            let now = now();
            let tab = TitleTab::Session(other);
            shell
                .workspace
                .tab_focused_at
                .insert(tab.clone(), now - LONG_AGO);
            let closed = shell.close_idle_tabs(now, w, cx);
            assert_eq!(closed, std::slice::from_ref(&tab));
            assert!(!shell.workspace.title_order.contains(&tab));
            assert!(!shell.workspace.tabs.contains(&other));
            assert!(!shell.workspace.tab_focused_at.contains_key(&tab));
            // The chat itself is untouched: still listed, not archived.
            assert!(shell.session_summaries.contains_key(&other));
            assert!(!shell.session_metadata(other, cx).summary.archived);
            assert!(!shell.session_summaries[&other].archived);
            assert_eq!(shell.workspace.active, Some(here));
        })
        .unwrap();
}

#[gpui::test]
fn recent_and_active_tabs_stay(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            let now = now();
            assert_eq!(shell.active_title_tab(cx), Some(TitleTab::Session(here)));
            let focused = &mut shell.workspace.tab_focused_at;
            focused.insert(TitleTab::Session(here), now - 50_000);
            focused.insert(TitleTab::Session(other), now - 100);
            assert!(shell.close_idle_tabs(now, w, cx).is_empty());
            assert_eq!(
                shell.workspace.title_order,
                [TitleTab::Session(other), TitleTab::Session(here)]
            );
            // Just under the limit is still recent.
            let limit = crate::strip_groups::IDLE_AFTER_SECS;
            shell
                .workspace
                .tab_focused_at
                .insert(TitleTab::Session(other), now - limit);
            assert!(shell.close_idle_tabs(now, w, cx).is_empty());
        })
        .unwrap();
}

#[gpui::test]
fn pinned_and_unsent_tabs_stay(cx: &mut TestAppContext) {
    let (window, other, _) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            let now = now();
            let tab = TitleTab::Session(other);
            shell.toggle_pin_tab(tab.clone(), cx);
            shell
                .workspace
                .tab_focused_at
                .insert(tab.clone(), now - LONG_AGO);
            assert!(shell.close_idle_tabs(now, w, cx).is_empty(), "pinned");
            shell.toggle_pin_tab(tab.clone(), cx);
            shell.sessions[&other].update(cx, |chat, cx| {
                chat.input
                    .update(cx, |input, cx| input.set_text("half a thought".into(), cx));
            });
            shell
                .workspace
                .tab_focused_at
                .insert(tab.clone(), now - LONG_AGO);
            assert!(shell.close_idle_tabs(now, w, cx).is_empty(), "unsent input");
            shell.sessions[&other].update(cx, |chat, cx| {
                chat.input
                    .update(cx, |input, cx| input.set_text("".into(), cx));
            });
            assert_eq!(shell.close_idle_tabs(now, w, cx), [tab]);
        })
        .unwrap();
}

#[gpui::test]
fn busy_split_pane_keeps_its_tab(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            let now = now();
            let beside = add_background_tab(shell, "/work/another-project", cx);
            shell.ensure_session(beside, cx);
            shell
                .workspace
                .place(beside, other, crate::pane_layout::Edge::Right);
            shell.workspace.active = Some(here);
            let key = shell.workspace.tab_of(other).unwrap();
            assert_eq!(shell.workspace.layout(key).leaf_ids(), [other, beside]);
            // The tab shows `other`; the pane beside it is the busy one.
            shell.workspace.focused.insert(key, other);
            shell.sessions[&beside].update(cx, |chat, _| chat.busy = true);
            let tab = TitleTab::Session(key);
            shell
                .workspace
                .tab_focused_at
                .insert(tab.clone(), now - LONG_AGO);
            assert!(shell.close_idle_tabs(now, w, cx).is_empty());
            assert!(shell.workspace.title_order.contains(&tab));
            shell.sessions[&beside].update(cx, |chat, _| chat.busy = false);
            assert_eq!(shell.close_idle_tabs(now, w, cx), [tab]);
        })
        .unwrap();
}

#[gpui::test]
fn tabs_with_terminal_panes_are_kept(cx: &mut TestAppContext) {
    let (window, other, _) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            let now = now();
            let terminal = -7;
            shell.workspace.terminal_panes.insert(
                terminal,
                crate::workspace::TerminalPane {
                    owner: other,
                    cwd: "/work/another-project".into(),
                },
            );
            shell.workspace.layouts.insert(
                other,
                crate::pane_layout::leaf(other).split_pane(
                    other,
                    crate::pane_layout::SplitDir::Right,
                    terminal,
                ),
            );
            let tab = TitleTab::Session(other);
            shell
                .workspace
                .tab_focused_at
                .insert(tab.clone(), now - LONG_AGO);
            assert!(shell.close_idle_tabs(now, w, cx).is_empty());
            assert!(shell.workspace.title_order.contains(&tab));
            assert!(shell.workspace.terminal_panes.contains_key(&terminal));
        })
        .unwrap();
}

#[gpui::test]
fn upgrade_without_focus_times_closes_nothing(cx: &mut TestAppContext) {
    let store = gespan_core::store::Store::open(std::path::Path::new(":memory:")).unwrap();
    let a = store.create("codex", "/work/another-project").unwrap();
    let b = store.create("claude", "/work/gespan").unwrap();
    let mut state = crate::workspace::Workspace::default();
    state.add_project("/work/another-project");
    state.add_project("/work/gespan");
    state.open(a.id);
    state.open(b.id);
    // A workspace saved before tabs kept focus times.
    let mut saved = serde_json::to_value(&state).unwrap();
    saved.as_object_mut().unwrap().remove("tab_focused_at");
    let saved = saved.to_string();
    assert!(!saved.contains("tab_focused_at"));
    let (a_id, b_id) = (a.id, b.id);
    let store = Arc::new(Mutex::new(store));
    let before = now();
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| Shell::new(store, vec![b, a], "/work/gespan".into(), Some(saved), cx))
        })
        .unwrap()
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            let order = shell.workspace.title_order.clone();
            assert_eq!(order.len(), 2);
            for tab in &order {
                let at = shell.workspace.tab_focused_at.get(tab).copied();
                assert!(at.is_some_and(|at| at >= before), "{tab:?} timed at launch");
            }
            assert!(shell.close_idle_tabs(now(), w, cx).is_empty());
            // Their clock started at launch, so they go once that's long ago.
            let later = now() + LONG_AGO;
            let closed = shell.close_idle_tabs(later, w, cx);
            assert_eq!(closed, [TitleTab::Session(a_id)]);
            assert_eq!(shell.workspace.title_order, [TitleTab::Session(b_id)]);
        })
        .unwrap();
}

#[gpui::test]
fn focus_time_round_trips_through_saved_workspace(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    let saved = window
        .update(cx, |shell, _, _| {
            let board = TitleTab::App(app_views::AppView::Board);
            let flows = TitleTab::App(app_views::AppView::Flows);
            let focused = &mut shell.workspace.tab_focused_at;
            focused.insert(TitleTab::Session(other), 1_000);
            focused.insert(TitleTab::Session(here), 2_000);
            focused.insert(board.clone(), 3_000);
            focused.insert(flows.clone(), 4_000);
            // A time for a tab no longer on the strip is dropped.
            focused.insert(TitleTab::Session(9_999), 5_000);
            shell.workspace.title_order.push(board);
            shell.workspace.title_order.push(flows);
            shell.workspace.normalize_title_order();
            assert!(
                !shell
                    .workspace
                    .tab_focused_at
                    .contains_key(&TitleTab::Session(9_999))
            );
            serde_json::to_string(&shell.workspace).unwrap()
        })
        .unwrap();
    let sessions = [
        (other, "/work/another-project".to_string()),
        (here, "/work/gespan".to_string()),
    ];
    let restored = crate::workspace::Workspace::restore(Some(&saved), &sessions, "/work/gespan");
    let times: HashMap<TitleTab, i64> = restored.tab_focused_at;
    assert_eq!(
        times,
        HashMap::from([
            (TitleTab::Session(other), 1_000),
            (TitleTab::Session(here), 2_000),
            (TitleTab::App(app_views::AppView::Board), 3_000),
        ]),
        "Flows tabs aren't saved, so neither is their time"
    );
}

#[gpui::test]
fn a_group_left_empty_by_cleanup_disappears(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, _) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    assert!(drawn(visual, "tab-group-chip-another-project".into()));
    window
        .update(visual, |shell, w, cx| {
            let now = now();
            let tab = TitleTab::Session(other);
            shell
                .workspace
                .tab_focused_at
                .insert(tab.clone(), now - LONG_AGO);
            assert_eq!(shell.close_idle_tabs(now, w, cx), [tab]);
        })
        .unwrap();
    settle(visual);
    assert!(!drawn(visual, "tab-group-chip-another-project".into()));
    assert!(drawn(visual, "tab-group-chip-gespan".into()));
}

#[gpui::test]
fn a_running_pilot_tab_stays_until_it_is_idle(cx: &mut TestAppContext) {
    let (window, _, here) = window(cx);
    let pilot = window
        .update(cx, |shell, w, cx| {
            shell.select_project("/work/another-project".into(), w, cx);
            let manager = shell.pilot_manager();
            manager.update(cx, |m, cx| m.create(Vec::new(), w, cx));
            manager.read(cx).active.clone().unwrap()
        })
        .unwrap();
    cx.run_until_parked();
    let tab = window
        .update(cx, |shell, w, cx| {
            let tab = shell.pilot_tab(pilot.clone());
            shell.select_project("/work/gespan".into(), w, cx);
            tab
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            let now = now();
            assert_eq!(shell.active_title_tab(cx), Some(TitleTab::Session(here)));
            let chat = shell.pilot_manager().read(cx).chat_for(&pilot).unwrap();
            chat.update(cx, |chat, _| chat.busy = true);
            shell
                .workspace
                .tab_focused_at
                .insert(tab.clone(), now - LONG_AGO);
            assert!(shell.close_idle_tabs(now, w, cx).is_empty(), "running");
            assert!(shell.workspace.title_order.contains(&tab));
            chat.update(cx, |chat, _| chat.busy = false);
            assert_eq!(
                shell.close_idle_tabs(now, w, cx),
                std::slice::from_ref(&tab)
            );
            assert!(!shell.workspace.title_order.contains(&tab));
            assert_eq!(shell.active_title_tab(cx), Some(TitleTab::Session(here)));
        })
        .unwrap();
}

/// Starts a Pilot in `project`, then goes back to /work/gespan. Returns the
/// Pilot's id and its tab.
fn pilot_in(
    window: gpui::WindowHandle<Shell>,
    project: &str,
    cx: &mut TestAppContext,
) -> (String, TitleTab) {
    let pilot = window
        .update(cx, |shell, w, cx| {
            shell.select_project(project.into(), w, cx);
            let manager = shell.pilot_manager();
            manager.update(cx, |m, cx| m.create(Vec::new(), w, cx));
            manager.read(cx).active.clone().unwrap()
        })
        .unwrap();
    cx.run_until_parked();
    let tab = window
        .update(cx, |shell, w, cx| {
            let tab = shell.pilot_tab(pilot.clone());
            shell.select_project("/work/gespan".into(), w, cx);
            tab
        })
        .unwrap();
    cx.run_until_parked();
    (pilot, tab)
}

fn has_group(shell: &Shell, project: &str, cx: &App) -> bool {
    groups(shell.strip_entries(cx))
        .iter()
        .any(|(p, _)| p == project)
}

#[gpui::test]
fn archiving_a_project_drops_its_pilot_tabs(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let (_, tab) = pilot_in(window, "/work/another-project", cx);
    window
        .update(cx, |shell, w, cx| {
            assert!(shell.workspace.title_order.contains(&tab));
            shell.toggle_pin_tab(tab.clone(), cx);
            shell.archive_project("/work/another-project", w, cx);
            assert!(!shell.workspace.title_order.contains(&tab));
            assert!(!shell.workspace.pinned_tabs.contains(&tab));
            assert!(!shell.workspace.tab_focused_at.contains_key(&tab));
            assert!(!has_group(shell, "/work/another-project", cx));
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    assert!(!drawn(visual, "tab-group-chip-another-project".into()));
    assert!(drawn(visual, "tab-group-chip-gespan".into()));
}

#[gpui::test]
fn a_saved_pilot_tab_of_an_unknown_project_draws_no_group(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let (pilot, _) = pilot_in(window, "/work/gespan", cx);
    // An older build kept a removed project's Pilot tab, pinned and not.
    let stale = |project: &str| TitleTab::Pilot {
        id: pilot.clone(),
        project: project.into(),
        worktree: project.into(),
    };
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.title_order.push(stale("/work/gone"));
            shell.workspace.title_order.push(stale("/work/pinned-gone"));
            shell.workspace.pinned_tabs.push(stale("/work/pinned-gone"));
            assert!(!shell.workspace.projects.iter().any(|p| p.contains("gone")));
            assert!(!has_group(shell, "/work/gone", cx));
            assert!(!shell.tab_drawable(&stale("/work/gone"), cx));
            assert!(
                !shell
                    .pinned_visible(cx)
                    .contains(&stale("/work/pinned-gone"))
            );
            assert!(!shell.visible_tabs(cx).contains(&stale("/work/gone")));
            shell.invalidate_chrome(cx);
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    assert!(!drawn(visual, "tab-group-chip-gone".into()));
    assert!(drawn(visual, "tab-group-chip-another-project".into()));
}

#[gpui::test]
fn pins_of_another_windows_tabs_stay_out_of_this_deck(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    let (_, pilot) = pilot_in(window, "/work/gespan", cx);
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.preferences.board = true;
            let board = TitleTab::App(app_views::AppView::Board);
            // Another window pinned these; this window's strip has neither.
            shell
                .workspace
                .title_order
                .retain(|t| *t != pilot && *t != board);
            shell.workspace.pinned_tabs.push(pilot.clone());
            shell.workspace.pinned_tabs.push(board.clone());
            assert!(shell.tab_drawable(&pilot, cx), "the Pilot itself exists");
            let deck = shell.pinned_visible(cx);
            assert!(!deck.contains(&pilot), "{deck:?}");
            assert!(!deck.contains(&board), "{deck:?}");
            // Once this window opens it too, its pin shows.
            shell.workspace.title_order.push(board.clone());
            assert!(shell.pinned_visible(cx).contains(&board));
        })
        .unwrap();
}

#[gpui::test]
fn idle_cleanup_waits_out_a_tab_drag(cx: &mut TestAppContext) {
    use gpui::Modifiers;
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            add_background_tab(shell, "/work/gespan", cx);
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    let tab = visual
        .debug_bounds(selector(format!("title-tab-{here}")))
        .unwrap()
        .center();
    visual.simulate_mouse_move(tab, None, Modifiers::default());
    visual.simulate_mouse_down(tab, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(
        tab + gpui::point(px(24.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.run_until_parked();
    let idle = TitleTab::Session(other);
    window
        .update(visual, |shell, w, cx| {
            assert!(shell.tab_drag.as_ref().is_some_and(|d| d.active));
            let now = now();
            shell
                .workspace
                .tab_focused_at
                .insert(idle.clone(), now - LONG_AGO);
            assert!(shell.close_idle_tabs(now, w, cx).is_empty(), "mid-drag");
            assert!(shell.workspace.title_order.contains(&idle));
            assert!(
                shell.tab_drag.as_ref().is_some_and(|d| d.active),
                "the drag carries on"
            );
        })
        .unwrap();
    visual.simulate_mouse_up(
        tab + gpui::point(px(24.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    settle(visual);
    window
        .update(visual, |shell, w, cx| {
            assert!(shell.tab_drag.is_none());
            // The next tick, with the drag over, closes it.
            assert_eq!(
                shell.close_idle_tabs(now(), w, cx),
                std::slice::from_ref(&idle)
            );
        })
        .unwrap();
}

#[gpui::test]
fn idle_cleanup_waits_out_an_open_menu_and_keeps_focus(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    let idle = TitleTab::Session(other);
    window
        .update(cx, |shell, w, cx| {
            let anchor = Anchor::Point(gpui::point(px(400.), px(30.)));
            shell.open_menu(MenuTarget::Tab(here), anchor, w, cx);
            let now = now();
            shell
                .workspace
                .tab_focused_at
                .insert(idle.clone(), now - LONG_AGO);
            assert!(shell.close_idle_tabs(now, w, cx).is_empty(), "menu open");
            assert!(shell.menu_open(&MenuTarget::Tab(here)), "the menu stays");
            shell.close_menu(cx);
            let input = shell.sessions[&here].read(cx).input.focus_handle(cx);
            w.focus(&input, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            let input = shell.sessions[&here].read(cx).input.focus_handle(cx);
            assert!(input.is_focused(w));
            assert_eq!(
                shell.close_idle_tabs(now(), w, cx),
                std::slice::from_ref(&idle)
            );
            assert!(input.is_focused(w), "closing a background tab keeps focus");
            assert_eq!(shell.workspace.active, Some(here));
        })
        .unwrap();
}

#[gpui::test]
fn a_busy_pilot_pane_in_a_split_keeps_its_tab(cx: &mut TestAppContext) {
    let (window, other, here) = window(cx);
    let (pilot, pilot_tab) = pilot_in(window, "/work/gespan", cx);
    window
        .update(cx, |shell, w, cx| {
            shell.open_title_tab(&TitleTab::Session(here), w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            let pane = shell
                .workspace
                .place_pilot(pilot_tab, other, crate::pane_layout::Edge::Right)
                .unwrap();
            shell.workspace.active = Some(here);
            let key = shell.workspace.tab_of(other).unwrap();
            assert_eq!(shell.workspace.layout(key).leaf_ids(), [other, pane]);
            assert_eq!(shell.active_title_tab(cx), Some(TitleTab::Session(here)));
            let chat = shell.pilot_manager().read(cx).chat_for(&pilot).unwrap();
            chat.update(cx, |chat, _| chat.busy = true);
            let now = now();
            let tab = TitleTab::Session(key);
            shell
                .workspace
                .tab_focused_at
                .insert(tab.clone(), now - LONG_AGO);
            assert!(
                shell.close_idle_tabs(now, w, cx).is_empty(),
                "Pilot working"
            );
            assert!(shell.workspace.title_order.contains(&tab));
            chat.update(cx, |chat, _| chat.busy = false);
            assert_eq!(shell.close_idle_tabs(now, w, cx), [tab]);
        })
        .unwrap();
}

#[gpui::test]
fn a_pilot_in_the_current_folder_draws_before_the_sidebar_lists_it(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let pilot = window
        .update(cx, |shell, w, cx| {
            shell.select_project("/work/loose-folder".into(), w, cx);
            let manager = shell.pilot_manager();
            manager.update(cx, |m, cx| m.create(Vec::new(), w, cx));
            manager.read(cx).active.clone().unwrap()
        })
        .unwrap();
    cx.run_until_parked();
    let tab = window
        .update(cx, |shell, _, cx| {
            let tab = shell.pilot_tab(pilot.clone());
            // Entering a folder lists it, so its Pilot draws.
            assert!(shell.tab_drawable(&tab, cx));
            assert!(has_group(shell, "/work/loose-folder", cx));
            // The current folder isn't listed yet (first-run setup, or before
            // its blank chat lands): its Pilot still draws.
            shell
                .workspace
                .projects
                .retain(|p| !p.contains("loose-folder"));
            assert_eq!(shell.workspace.project, "/work/loose-folder");
            assert!(shell.tab_drawable(&tab, cx));
            assert!(has_group(shell, "/work/loose-folder", cx));
            assert!(shell.visible_tabs(cx).contains(&tab));
            shell.invalidate_chrome(cx);
            tab
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    assert!(drawn(visual, "tab-group-chip-loose-folder".into()));
    // Left behind unlisted, it is a stale tab and hides like one.
    window
        .update(cx, |shell, w, cx| {
            shell.select_project("/work/gespan".into(), w, cx);
            assert!(!shell.tab_drawable(&tab, cx));
            assert!(!has_group(shell, "/work/loose-folder", cx));
        })
        .unwrap();
}

/// Draws the next frame of a moving strip: tab motion runs on the wall
/// clock, as the window's frames do.
fn next_frame(window: gpui::WindowHandle<Shell>, visual: &mut gpui::VisualTestContext, ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
    window
        .update(visual, |shell, _, cx| {
            shell.tabs_frame.update(cx, |_, cx| cx.notify())
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual.run_until_parked();
}

fn animated(window: gpui::WindowHandle<Shell>, cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(false));
    window
        .update(cx, |shell, _, _| {
            shell.workspace.preferences.tab_animations = true
        })
        .unwrap();
}

#[gpui::test]
fn resting_on_a_peeking_chips_count_holds_the_peek_while_it_opens(cx: &mut TestAppContext) {
    let (window, other, _) = window(cx);
    animated(window, cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    let chip = visual
        .debug_bounds("tab-group-chip-another-project")
        .unwrap();
    let count = visual
        .debug_bounds("tab-group-count-another-project")
        .unwrap();
    // The pointer comes to rest on the count, at the chip's far end.
    let at = gpui::point(count.right() - px(2.), count.center().y);
    assert!(chip.contains(&at));
    visual.simulate_mouse_move(at, None, Default::default());
    visual.run_until_parked();
    visual.executor().advance_clock(PEEK_DELAY);
    visual.run_until_parked();
    // While the group's tabs ease open, the chip holds still under the
    // pointer and the peek never drops.
    for _ in 0..8 {
        next_frame(window, visual, 30);
        let now = visual
            .debug_bounds("tab-group-chip-another-project")
            .unwrap();
        assert_eq!(now.size.width, chip.size.width, "the chip keeps its size");
        assert_eq!(now.left(), chip.left());
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.strip_peek.as_deref(), Some("/work/another-project"));
            })
            .unwrap();
    }
    assert!(drawn(visual, format!("title-tab-{other}")), "opened");
}

#[gpui::test]
fn moving_on_from_a_peek_holds_the_next_chip_still_while_the_first_eases_shut(
    cx: &mut TestAppContext,
) {
    let (window, _, _) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(1400.), px(700.)));
    window
        .update(visual, |shell, _, cx| {
            for _ in 0..20 {
                add_background_tab(shell, "/work/gespan", cx);
            }
            for _ in 0..4 {
                add_background_tab(shell, "/work/wide-project", cx);
            }
            add_background_tab(shell, "/work/third-project", cx);
            for ix in 0..8 {
                add_background_tab(shell, &format!("/work/after-{ix}"), cx);
            }
        })
        .unwrap();
    settle(visual);
    let chip = visual.debug_bounds("tab-group-chip-wide-project").unwrap();
    window
        .update(visual, |shell, _, cx| {
            let view = shell.tab_scroll.bounds();
            let offset = shell.tab_scroll.offset();
            let x = offset.x - (chip.left() - view.left() - px(40.));
            shell.tab_scroll.set_offset(gpui::point(x, offset.y));
            shell.tabs_frame.update(cx, |_, cx| cx.notify());
        })
        .unwrap();
    settle(visual);
    animated(window, visual);
    hover(visual, "tab-group-chip-wide-project".into());
    next_frame(window, visual, 260);
    let held = visual.debug_bounds("tab-group-chip-third-project").unwrap();
    visual.simulate_mouse_move(held.center(), None, Default::default());
    visual.run_until_parked();
    // Every frame of the first group easing shut, the chip under the
    // pointer stays where it was.
    for _ in 0..9 {
        next_frame(window, visual, 25);
        let now = visual.debug_bounds("tab-group-chip-third-project").unwrap();
        assert!(
            (now.left() - held.left()).abs() <= px(1.),
            "{now:?} moved from {held:?}"
        );
    }
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.strip_peek.as_deref(), Some("/work/third-project"));
        })
        .unwrap();
}

#[gpui::test]
fn passing_over_a_chip_does_not_peek(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, other, here) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    settle(visual);
    let before = visual
        .debug_bounds(selector(format!("title-tab-{here}")))
        .unwrap();
    // Across the chip and on before the wait runs out: nothing opens.
    pass_over(visual, "tab-group-chip-another-project".into());
    visual
        .executor()
        .advance_clock(PEEK_DELAY - std::time::Duration::from_millis(50));
    settle(visual);
    assert!(!drawn(visual, format!("title-tab-{other}")), "not yet");
    pass_over(visual, format!("title-tab-{here}"));
    visual.executor().advance_clock(PEEK_DELAY * 2);
    settle(visual);
    assert!(!drawn(visual, format!("title-tab-{other}")), "passed over");
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.strip_peek, None);
            assert!(shell.peek_pending.is_none());
        })
        .unwrap();
    assert_eq!(
        visual
            .debug_bounds(selector(format!("title-tab-{here}")))
            .unwrap(),
        before,
        "the strip stayed still"
    );
    // Moving within the chip keeps one wait, then it peeks.
    pass_over(visual, "tab-group-chip-another-project".into());
    let chip = visual
        .debug_bounds("tab-group-chip-another-project")
        .unwrap();
    visual
        .executor()
        .advance_clock(PEEK_DELAY - std::time::Duration::from_millis(50));
    visual.simulate_mouse_move(
        chip.center() + gpui::point(px(2.), px(0.)),
        None,
        Default::default(),
    );
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(60));
    settle(visual);
    assert!(drawn(visual, format!("title-tab-{other}")), "rested, peeks");
}

#[gpui::test]
fn switching_tabs_in_a_crowded_strip_moves_nothing(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, here) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(900.), px(700.)));
    let ids = window
        .update(visual, |shell, _, cx| {
            (0..20)
                .map(|_| add_background_tab(shell, "/work/gespan", cx))
                .collect::<Vec<_>>()
        })
        .unwrap();
    settle(visual);
    let places = |visual: &mut gpui::VisualTestContext| {
        std::iter::once(here)
            .chain(ids.iter().copied())
            .map(|id| {
                visual
                    .debug_bounds(selector(format!("title-tab-{id}")))
                    .unwrap_or_else(|| panic!("tab {id} drawn"))
            })
            .collect::<Vec<_>>()
    };
    let before = places(visual);
    assert!(
        before[0].size.width < px(TAB_CLOSE_MIN_WIDTH),
        "crowded: {:?}",
        before[0].size.width
    );
    click(visual, format!("title-tab-{}", ids[3]));
    window
        .update(visual, |shell, _, cx| {
            assert_eq!(shell.active_title_tab(cx), Some(TitleTab::Session(ids[3])));
        })
        .unwrap();
    assert_eq!(places(visual), before, "no tab moved or resized");
}
