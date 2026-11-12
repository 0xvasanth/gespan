//! One title strip for every project: each project's tabs sit together as a
//! group, pins from any project share the deck, and shortcuts still walk
//! only the current project's tabs.
use super::*;
use crate::conversation::SessionChanged;
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

fn hover(visual: &mut gpui::VisualTestContext, name: String) {
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
    hover(visual, "tab-group-chip-third-project".into());
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
    let ids = window
        .update(visual, |shell, _, cx| {
            (0..20)
                .map(|_| add_background_tab(shell, "/work/gespan", cx))
                .collect::<Vec<_>>()
        })
        .unwrap();
    settle(visual);
    for id in ids.iter().chain([&here]) {
        let tab = visual
            .debug_bounds(selector(format!("title-tab-{id}")))
            .unwrap_or_else(|| panic!("tab {id} drawn"));
        assert!(
            tab.size.width >= px(SELECTED_TAB_MIN_WIDTH),
            "tab {id} squeezed to {:?}",
            tab.size.width
        );
    }
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
