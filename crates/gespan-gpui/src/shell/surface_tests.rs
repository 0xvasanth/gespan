//! Destinations open as workspace tabs beside the project sessions without
//! changing the remembered sidebar or destroying the conversation workspace.
use super::*;
use gpui::{TestAppContext, VisualTestContext, WindowHandle};

#[derive(Clone, Copy, Debug)]
enum Surface {
    Inbox,
    Notes,
    Automations,
    Flows,
    Board,
    Runs,
    Pilot,
}

impl Surface {
    fn open(self, shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
        match self {
            Self::Inbox => shell.open_inbox(window, cx),
            Self::Notes => shell.open_notes(window, cx),
            Self::Automations => {
                // An empty in-memory service keeps this navigation test
                // independent of asynchronous service initialization.
                shell.automation_state.service = Some(Arc::new(Mutex::new(
                    gespan_core::automations::Service::open(std::path::Path::new(":memory:"))
                        .unwrap(),
                )));
                shell.open_automations(window, cx);
            }
            Self::Flows => shell.open_flows(window, cx),
            Self::Board => {
                shell.workspace.preferences.board = true;
                shell.open_board(window, cx);
            }
            Self::Runs => {
                shell.automation_state.service = Some(Arc::new(Mutex::new(
                    gespan_core::automations::Service::open(std::path::Path::new(":memory:"))
                        .unwrap(),
                )));
                shell.open_runs(Default::default(), window, cx);
            }
            Self::Pilot => {
                let project = shell.workspace.project.clone();
                let session = shell.workspace.active;
                shell.pilot_manager().update(cx, |pilots, cx| {
                    let id = pilots.roster.create(vec![project]).id.clone();
                    let pilot = pilots.roster.get_mut(&id).unwrap();
                    pilot.name = "Parity Pilot".into();
                    pilot.session_id = session;
                    pilots.open(&id, window, cx);
                });
            }
        }
    }

    fn close(self, shell: &mut Shell, session: i64, window: &mut Window, cx: &mut Context<Shell>) {
        match self {
            Self::Inbox
            | Self::Notes
            | Self::Automations
            | Self::Flows
            | Self::Board
            | Self::Runs => shell.close_app_view(window, cx),
            // Returning to a conversation is the same route used by opening
            // a Pilot's launched session.
            Self::Pilot => shell.open(session, window, cx),
        }
    }
}

fn setup(cx: &mut TestAppContext, boxy: bool) -> (WindowHandle<Shell>, i64) {
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        crate::flows::show_for_test(cx);
    });
    let (window, _, session) = super::tests::window(cx);
    cx.simulate_window_resize(window.into(), gpui::size(px(1500.), px(900.)));
    window
        .update(cx, |shell, window, cx| {
            shell.workspace.new_layout_boxy = boxy;
            shell.workspace.sidebar_visible = true;
            shell.workspace.sidebar_width = 260.;
            shell.workspace.right_panel_open = false;
            shell.open(session, window, cx);
            shell.layout_changed(cx);
        })
        .unwrap();
    cx.run_until_parked();
    (window, session)
}

#[gpui::test]
fn turning_board_off_closes_the_board_and_drops_its_tab(cx: &mut TestAppContext) {
    let (window, _) = setup(cx, false);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    window
        .update(visual, |shell, window, cx| {
            Surface::Board.open(shell, window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    window
        .update(visual, |shell, window, cx| {
            assert_eq!(shell.active_app_view, Some(app_views::AppView::Board));
            let mut prefs = shell.workspace.preferences.clone();
            prefs.board = false;
            shell.set_preferences(prefs, window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    window
        .read_with(visual, |shell, cx| {
            assert_eq!(shell.active_app_view, None);
            assert!(
                !shell
                    .visible_tabs(cx)
                    .contains(&TitleTab::App(app_views::AppView::Board))
            );
        })
        .unwrap();
}

#[gpui::test]
fn new_layout_projects_restore_the_last_session_or_pilot_tab(cx: &mut TestAppContext) {
    for boxy in [false, true] {
        let (window, _) = setup(cx, boxy);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        let stable = |value: String| Box::leak(value.into_boxed_str()) as &'static str;
        let mut targets = Vec::new();
        for path in ["/work/gespan", "/work/another-project"] {
            let target = window
                .update(visual, |shell, window, cx| {
                    shell.select_project(path.into(), window, cx);
                    let session = shell.workspace.active.unwrap();
                    Surface::Pilot.open(shell, window, cx);
                    let pilot = shell.pilot_manager().read(cx).active.clone().unwrap();
                    let ix = shell
                        .workspace
                        .projects
                        .iter()
                        .position(|p| p == path)
                        .unwrap();
                    (stable(format!("project-space-{ix}")), session, pilot)
                })
                .unwrap();
            visual.run_until_parked();
            targets.push(target);
        }
        let click = |selector: &'static str, visual: &mut VisualTestContext| {
            visual.refresh().unwrap();
            let bounds = visual.debug_bounds(selector).unwrap();
            visual.simulate_click(bounds.center(), Default::default());
            visual.run_until_parked();
            visual.refresh().unwrap();
        };
        let assert_pilot = |ix: usize, visual: &mut VisualTestContext| {
            window
                .read_with(visual, |shell, cx| {
                    assert_eq!(
                        shell.pilot_manager().read(cx).active.as_ref(),
                        Some(&targets[ix].2)
                    );
                    assert!(
                        shell
                            .visible_tabs(cx)
                            .contains(&shell.pilot_tab(targets[ix].2.clone()))
                    );
                })
                .unwrap();
            assert!(
                visual
                    .debug_bounds(stable(format!("pilot-title-tab-{}", targets[ix].2)))
                    .is_some()
            );
        };
        click(targets[0].0, visual);
        assert_pilot(0, visual);
        click(stable(format!("title-tab-{}", targets[0].1)), visual);
        click(targets[1].0, visual);
        assert_pilot(1, visual);
        click(targets[0].0, visual);
        window
            .read_with(visual, |shell, cx| {
                assert!(!shell.pilot_active(cx));
                assert_eq!(shell.workspace.active, Some(targets[0].1));
            })
            .unwrap();
        click(stable(format!("pilot-rail-{}", targets[0].2)), visual);
        assert_pilot(0, visual);
        // Both same-project clicks and returning from another project restore
        // a selected Pilot in either new frame style.
        click(targets[0].0, visual);
        assert_pilot(0, visual);
        click(targets[1].0, visual);
        assert_pilot(1, visual);
        click(targets[0].0, visual);
        assert_pilot(0, visual);
    }
}

#[gpui::test]
fn app_surfaces_stay_tabbed_in_both_workspace_styles(cx: &mut TestAppContext) {
    for boxy in [false, true] {
        let (window, session) = setup(cx, boxy);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        for surface in [
            Surface::Inbox,
            Surface::Notes,
            Surface::Automations,
            Surface::Flows,
            Surface::Board,
            Surface::Runs,
            Surface::Pilot,
        ] {
            window
                .update(visual, |shell, window, cx| surface.open(shell, window, cx))
                .unwrap();
            visual.run_until_parked();
            visual.refresh().unwrap();
            assert!(
                visual.debug_bounds("sidebar-content").is_some(),
                "{surface:?} keeps the new layout sidebar (boxy={boxy})"
            );
            assert!(
                visual.debug_bounds("title-tab-strip").is_some(),
                "{surface:?} stays a workspace tab (boxy={boxy})"
            );
            let app = match surface {
                Surface::Inbox => Some(app_views::AppView::Inbox),
                Surface::Notes => Some(app_views::AppView::Notes),
                Surface::Automations => Some(app_views::AppView::Automations),
                Surface::Flows => Some(app_views::AppView::Flows),
                Surface::Board => Some(app_views::AppView::Board),
                Surface::Runs => Some(app_views::AppView::Runs),
                _ => None,
            };
            if let Some(app) = app {
                let selector = Box::leak(format!("app-title-tab-{}", app.label()).into_boxed_str());
                assert!(
                    visual.debug_bounds(selector).is_some(),
                    "the destination itself has a tab (boxy={boxy})"
                );
                window
                    .read_with(visual, |shell, cx| {
                        assert!(shell.visible_tabs(cx).contains(&TitleTab::App(app)));
                        assert_eq!(shell.active_title_tab(cx), Some(TitleTab::App(app)));
                    })
                    .unwrap();
            }
            window
                .read_with(visual, |shell, _| {
                    assert_eq!(shell.workspace.new_layout_boxy, boxy);
                    assert!(shell.workspace.sidebar_visible);
                })
                .unwrap();
            window
                .update(visual, |shell, window, cx| {
                    surface.close(shell, session, window, cx)
                })
                .unwrap();
            visual.run_until_parked();
        }
    }
}

#[gpui::test]
fn new_layout_app_tabs_survive_frame_changes_and_close_like_session_tabs(cx: &mut TestAppContext) {
    cx.update(super::bind_keys);
    for boxy in [false, true] {
        let (window, session) = setup(cx, boxy);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        for surface in [
            Surface::Inbox,
            Surface::Notes,
            Surface::Automations,
            Surface::Flows,
            Surface::Board,
            Surface::Runs,
        ] {
            window
                .update(visual, |shell, window, cx| surface.open(shell, window, cx))
                .unwrap();
            visual.run_until_parked();
        }
        let order = window
            .update(visual, |shell, _, cx| {
                for app in [
                    app_views::AppView::Inbox,
                    app_views::AppView::Notes,
                    app_views::AppView::Automations,
                    app_views::AppView::Flows,
                    app_views::AppView::Board,
                    app_views::AppView::Runs,
                ] {
                    assert_eq!(
                        shell
                            .workspace
                            .title_order
                            .iter()
                            .filter(|tab| **tab == TitleTab::App(app))
                            .count(),
                        1
                    );
                }
                let order = shell.workspace.title_order.clone();
                shell.workspace.new_layout_boxy = !boxy;
                shell.layout_changed(cx);
                order
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        for app in [
            app_views::AppView::Inbox,
            app_views::AppView::Notes,
            app_views::AppView::Automations,
            app_views::AppView::Flows,
            app_views::AppView::Board,
            app_views::AppView::Runs,
        ] {
            let selector = Box::leak(format!("app-title-tab-{}", app.label()).into_boxed_str());
            let tab = visual.debug_bounds(selector).unwrap();
            visual.simulate_click(tab.center(), Default::default());
            visual.run_until_parked();
            window
                .read_with(visual, |shell, _| {
                    assert_eq!(shell.active_app_view, Some(app));
                    assert_eq!(shell.workspace.title_order, order);
                })
                .unwrap();
        }
        // Close an inactive tab through its button, then the selected tabs
        // through Cmd-W, preserving the underlying conversation.
        visual.refresh().unwrap();
        let close = visual.debug_bounds("close-app-tab-Inbox").unwrap();
        visual.simulate_click(close.center(), Default::default());
        visual.run_until_parked();
        for _ in 0..5 {
            visual.simulate_keystrokes("secondary-w");
            visual.run_until_parked();
        }
        window
            .read_with(visual, |shell, cx| {
                assert!(!shell.app_view_open());
                assert_eq!(shell.workspace.active, Some(session));
                assert!(
                    shell
                        .workspace
                        .title_order
                        .iter()
                        .all(|tab| !matches!(tab, TitleTab::App(_)))
                );
                assert_eq!(shell.active_title_tab(cx), Some(TitleTab::Session(session)));
            })
            .unwrap();
    }
}

#[gpui::test]
fn the_sidebar_board_entry_follows_the_setting_and_opens_the_board(cx: &mut TestAppContext) {
    let (window, _) = setup(cx, false);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    let entry = visual
        .debug_bounds("sidebar-navigation-Board")
        .expect("Board is on by default");
    visual.simulate_click(entry.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, window, cx| {
            assert_eq!(shell.active_app_view, Some(app_views::AppView::Board));
            let mut prefs = shell.workspace.preferences.clone();
            prefs.board = false;
            shell.set_preferences(prefs, window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("sidebar-navigation-Board").is_none());
    window
        .update(visual, |shell, window, cx| {
            shell.open_board(window, cx);
            assert_ne!(shell.active_app_view, Some(app_views::AppView::Board));
        })
        .unwrap();
}

#[gpui::test]
fn switching_board_off_in_settings_saves_it_and_keeps_focus_on_settings(cx: &mut TestAppContext) {
    let (window, _) = setup(cx, false);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    window
        .update(visual, |shell, window, cx| {
            Surface::Board.open(shell, window, cx);
            shell.open_settings(crate::settings::Section::Appearance, window, cx);
        })
        .unwrap();
    visual.simulate_resize(gpui::size(px(1200.), px(2600.)));
    visual.run_until_parked();
    visual.refresh().unwrap();
    let toggle = visual
        .debug_bounds("settings-toggle-sidebar-board")
        .expect("the Board switch is in the Appearance sidebar group");
    visual.simulate_click(toggle.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, window, cx| {
            assert!(!shell.workspace.preferences.board);
            assert_eq!(shell.active_app_view, None);
            let settings = shell.settings.as_ref().expect("Settings stays open");
            assert!(
                settings.focus_handle(cx).contains_focused(window, cx),
                "focus stays on Settings, not the chat behind it"
            );
            let saved: serde_json::Value =
                serde_json::from_str(&shell.latest_state.lock().unwrap()).unwrap();
            assert_eq!(saved["preferences"]["board"], serde_json::json!(false));
        })
        .unwrap();
}

#[gpui::test]
fn a_project_added_while_the_board_is_open_is_offered_as_a_choice_at_once(cx: &mut TestAppContext) {
    let (window, _) = setup(cx, false);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    window
        .update(visual, |shell, window, cx| {
            Surface::Board.open(shell, window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    window
        .update(visual, |shell, _, cx| {
            shell.workspace.projects.push("/work/added-later".into());
            shell.persist(cx);
            let board = shell.board.as_ref().unwrap().read(cx);
            assert!(
                board
                    .project_choices()
                    .contains(&("/work/added-later".into(), "added-later".into())),
                "{:?}",
                board.project_choices()
            );
        })
        .unwrap();
}

#[gpui::test]
fn opening_a_pilot_from_the_board_leaves_the_board(cx: &mut TestAppContext) {
    let (window, _) = setup(cx, false);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    window
        .update(visual, |shell, window, cx| {
            Surface::Board.open(shell, window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    window
        .update(visual, |shell, _, cx| {
            let project = shell.workspace.project.clone();
            let id = shell.pilot_manager().update(cx, |pilots, _| {
                pilots.roster.create(vec![project]).id.clone()
            });
            shell
                .board
                .as_ref()
                .unwrap()
                .update(cx, |_, cx| cx.emit(crate::board::Event::OpenPilot(id)));
        })
        .unwrap();
    visual.run_until_parked();
    window
        .read_with(visual, |shell, _| {
            assert_eq!(
                shell.active_app_view, None,
                "the Pilot is not behind the Board"
            )
        })
        .unwrap();
}

#[gpui::test]
fn a_session_started_in_the_background_offers_its_project_to_the_board(cx: &mut TestAppContext) {
    let (window, _) = setup(cx, false);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    window
        .update(visual, |shell, window, cx| {
            Surface::Board.open(shell, window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    window
        .update(visual, |shell, _, cx| {
            let summary = locked(&shell.store)
                .unwrap()
                .create("codex", "/work/background")
                .unwrap();
            // What a background start does: add the session, no persist.
            shell.add(summary, cx);
            let board = shell.board.as_ref().unwrap().read(cx);
            assert!(
                board
                    .project_choices()
                    .contains(&("/work/background".into(), "background".into()))
            );
        })
        .unwrap();
}

#[gpui::test]
fn settings_back_and_sidebar_toggle(cx: &mut TestAppContext) {
    let (window, session) = setup(cx, false);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    window
        .update(visual, |shell, window, cx| {
            shell.open_settings(crate::settings::Section::General, window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();

    // The toggle steps aside but keeps its slot, so the title bar holds
    // still, and the shortcut can't hide what Settings put in the sidebar's
    // place.
    let toggle = visual.debug_bounds("toggle-sidebar").unwrap();
    assert_eq!(toggle.size.width, px(24.));
    visual.dispatch_action(ToggleSidebar);
    visual.run_until_parked();
    window
        .read_with(visual, |shell, _| {
            assert!(shell.settings.is_some());
            assert!(shell.workspace.sidebar_visible);
        })
        .unwrap();

    visual.refresh().unwrap();
    let back = visual.debug_bounds("visit-back").unwrap();
    visual.simulate_click(back.center(), Default::default());
    visual.run_until_parked();
    window
        .read_with(visual, |shell, _| {
            assert!(shell.settings.is_none(), "Back leaves Settings");
            assert_eq!(shell.workspace.active, Some(session));
        })
        .unwrap();
}

#[gpui::test]
fn escape_steps_back_inside_the_board_and_never_leaves_it(cx: &mut TestAppContext) {
    let (window, _) = setup(cx, false);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    window
        .update(visual, |shell, window, cx| {
            Surface::Board.open(shell, window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    let tap = |visual: &mut VisualTestContext, target: &'static str| {
        visual.refresh().unwrap();
        let at = visual
            .debug_bounds(target)
            .unwrap_or_else(|| panic!("{target} is not on screen"))
            .center();
        visual.simulate_click(at, Default::default());
        visual.run_until_parked();
    };
    let on_board = |visual: &mut VisualTestContext| {
        window
            .read_with(visual, |shell, _| shell.active_app_view)
            .unwrap()
            == Some(app_views::AppView::Board)
    };
    window
        .update(visual, |shell, _, cx| {
            shell
                .board
                .as_ref()
                .unwrap()
                .update(cx, |board, cx| board.add_test_card("Escape test", cx))
        })
        .unwrap();
    visual.run_until_parked();
    tap(visual, "board-card-1");
    tap(visual, "board-subtask-add");
    visual.simulate_input("First step");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    visual.simulate_input("half typed");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(
        on_board(visual),
        "Escape in the subtask field stays on the board"
    );
    let drawer = |visual: &mut VisualTestContext| {
        window
            .read_with(visual, |shell, cx| {
                shell.board.as_ref().unwrap().read(cx).card_detail.is_some()
            })
            .unwrap()
    };
    assert!(drawer(visual), "the field closes first, the drawer stays");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(!drawer(visual), "the next Escape closes the drawer");
    assert!(on_board(visual));
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(
        on_board(visual),
        "with nothing open Escape does nothing, as in Tauri"
    );
    // The same after a click on the board's empty space, which focuses it.
    visual.refresh().unwrap();
    let body = visual.debug_bounds("board-column-body-0").unwrap();
    visual.simulate_click(
        gpui::point(body.center().x, body.bottom() - gpui::px(4.)),
        Default::default(),
    );
    visual.run_until_parked();
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(on_board(visual), "Escape on the focused board stays on it");
}

mod inbox_settings {
    use super::*;
    use crate::settings::Section;
    use gespan_core::inbox::{OptionRow, Status};

    fn inbox_page(cx: &mut TestAppContext) -> (WindowHandle<Shell>, VisualTestContext) {
        settings_page(Section::Inbox, cx)
    }

    fn connections_page(cx: &mut TestAppContext) -> (WindowHandle<Shell>, VisualTestContext) {
        settings_page(Section::Services, cx)
    }

    fn settings_page(
        section: Section,
        cx: &mut TestAppContext,
    ) -> (WindowHandle<Shell>, VisualTestContext) {
        cx.update(crate::shell::bind_keys);
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = crate::shell::tests::window(cx);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(gpui::size(px(1500.), px(2200.)));
        window
            .update(&mut visual, |shell, w, cx| {
                shell.open_settings(section, w, cx)
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        (window, visual)
    }

    fn click(visual: &mut VisualTestContext, target: String) {
        let target: &'static str = target.leak();
        visual.refresh().unwrap();
        let bounds = visual
            .debug_bounds(target)
            .unwrap_or_else(|| panic!("{target} is not on screen"));
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
    }

    #[gpui::test]
    fn settings_opens_the_connect_dialog_in_place_for_each_provider(cx: &mut TestAppContext) {
        let (window, mut visual) = connections_page(cx);
        let visual = &mut visual;
        for provider in ["gitlab", "azuredevops", "jira", "plane", "linear"] {
            // Nothing is connected: the empty page's Add connection opens
            // the picker, and its Connect closes it for the dialog.
            click(visual, "settings-add-connection".into());
            assert!(visual.debug_bounds("settings-services-picker").is_some());
            click(visual, format!("settings-picker-connect-{provider}"));
            assert!(visual.debug_bounds("settings-services-picker").is_none());
            assert!(
                visual.debug_bounds("inbox-connect-dialog").is_some(),
                "{provider} opens the dialog"
            );
            window
                .update(visual, |shell, _, cx| {
                    assert!(shell.settings.is_some(), "{provider} stays in Settings");
                    assert_eq!(shell.active_app_view, None);
                    assert!(
                        shell
                            .inbox
                            .as_ref()
                            .unwrap()
                            .read(cx)
                            .connecting_to(provider)
                    );
                })
                .unwrap();
            // Escape closes the dialog alone, in the field that has focus.
            visual.simulate_keystrokes("escape");
            visual.run_until_parked();
            visual.refresh().unwrap();
            assert!(visual.debug_bounds("inbox-connect-dialog").is_none());
            window
                .update(visual, |shell, _, _| assert!(shell.settings.is_some()))
                .unwrap();
        }
    }

    #[gpui::test]
    fn the_inbox_view_does_not_show_a_dialog_opened_from_settings(cx: &mut TestAppContext) {
        let (window, mut visual) = connections_page(cx);
        let visual = &mut visual;
        click(visual, "settings-add-connection".into());
        click(visual, "settings-picker-connect-plane".into());
        window
            .update(visual, |shell, w, cx| {
                shell.close_settings(w, cx);
                shell.open_inbox(w, cx);
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("inbox-connect-dialog").is_none());
    }

    #[gpui::test]
    fn linear_teams_are_picked_in_settings_once_connected(cx: &mut TestAppContext) {
        let (window, mut visual) = connections_page(cx);
        let visual = &mut visual;
        assert!(visual.debug_bounds("settings-linear-teams").is_none());
        // A toggle refetches, and this test has no Linear to answer: seed again.
        let seed = |visual: &mut VisualTestContext| {
            window
                .update(visual, |shell, _, cx| {
                    shell.inbox.as_ref().unwrap().update(cx, |inbox, cx| {
                        inbox.seed_linear_teams(
                            Status {
                                connected: true,
                                ..Default::default()
                            },
                            ["Web", "Mobile"]
                                .map(|name| OptionRow {
                                    id: name.to_lowercase(),
                                    name: name.into(),
                                    key: name[..3].to_uppercase(),
                                })
                                .to_vec(),
                            cx,
                        )
                    });
                    shell.sync_settings(cx);
                })
                .unwrap();
            visual.run_until_parked();
        };
        seed(visual);
        click(visual, "settings-linear-team-1".into());
        let hidden = |window: WindowHandle<Shell>, visual: &mut VisualTestContext| {
            window
                .update(visual, |shell, _, cx| {
                    shell
                        .inbox
                        .as_ref()
                        .unwrap()
                        .read(cx)
                        .hidden_linear_teams()
                        .to_vec()
                })
                .unwrap()
        };
        assert_eq!(hidden(window, visual), ["mobile"]);
        seed(visual);
        click(visual, "settings-linear-team-1".into());
        assert!(hidden(window, visual).is_empty());
    }

    #[gpui::test]
    fn settings_inbox_holds_no_services_and_points_to_connections(cx: &mut TestAppContext) {
        let (window, mut visual) = inbox_page(cx);
        let visual = &mut visual;
        for provider in ["github", "gitlab", "azuredevops", "jira", "plane", "linear"] {
            assert!(
                visual
                    .debug_bounds(format!("settings-connect-{provider}").leak())
                    .is_none(),
                "{provider} is set up in Connections now"
            );
        }
        assert!(visual.debug_bounds("settings-add-connection").is_none());
        click(visual, "settings-open-connections".into());
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(
                    shell.settings.as_ref().unwrap().read(cx).section,
                    Section::Services
                )
            })
            .unwrap();
        assert!(visual.debug_bounds("settings-services-empty").is_some());
    }

    #[gpui::test]
    fn the_inbox_add_connection_button_opens_settings_connections(cx: &mut TestAppContext) {
        cx.update(crate::shell::bind_keys);
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = crate::shell::tests::window(cx);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let visual = &mut visual;
        visual.simulate_resize(gpui::size(px(1500.), px(1000.)));
        window
            .update(visual, |shell, w, cx| shell.open_inbox(w, cx))
            .unwrap();
        visual.run_until_parked();
        click(visual, "inbox-add-connection".into());
        window
            .update(visual, |shell, _, cx| {
                let settings = shell.settings.as_ref().expect("Settings opens");
                assert_eq!(settings.read(cx).section, Section::Services);
                assert!(
                    !shell
                        .inbox
                        .as_ref()
                        .unwrap()
                        .read(cx)
                        .connections_menu_open()
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn the_picker_filters_by_chip_and_closes_on_escape(cx: &mut TestAppContext) {
        let (_, mut visual) = connections_page(cx);
        let visual = &mut visual;
        click(visual, "settings-add-connection".into());
        assert!(visual.debug_bounds("settings-picker-github").is_some());
        click(visual, "settings-services-chip-issues".into());
        assert!(visual.debug_bounds("settings-picker-github").is_none());
        assert!(visual.debug_bounds("settings-picker-linear").is_some());
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("settings-services-picker").is_none());
    }
}

/// With the shell's real key bindings, which the Board's own fixture lacks:
/// its Escape binding runs before any view's key handler.
mod real_keys {
    use super::*;

    fn board(cx: &mut TestAppContext) -> (WindowHandle<Shell>, VisualTestContext) {
        cx.update(crate::shell::bind_keys);
        cx.update(crate::input::bind);
        cx.update(crate::popover::bind);
        cx.update(crate::board::bind);
        let (window, _) = setup(cx, false);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        window
            .update(&mut visual, |shell, window, cx| {
                Surface::Board.open(shell, window, cx);
                shell
                    .board
                    .as_ref()
                    .unwrap()
                    .update(cx, |board, cx| board.add_test_card("Task", cx));
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        (window, visual)
    }

    fn view(
        window: WindowHandle<Shell>,
        visual: &mut VisualTestContext,
    ) -> Option<app_views::AppView> {
        window
            .read_with(visual, |shell, _| shell.active_app_view)
            .unwrap()
    }

    fn escape(visual: &mut VisualTestContext) {
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        visual.refresh().unwrap();
    }

    fn tap(visual: &mut VisualTestContext, target: &'static str) {
        visual.refresh().unwrap();
        let at = visual
            .debug_bounds(target)
            .unwrap_or_else(|| panic!("{target} is not on screen"))
            .center();
        visual.simulate_click(at, Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
    }

    #[gpui::test]
    fn escape_closes_the_new_card_box_and_stays_on_the_board(cx: &mut TestAppContext) {
        let (window, mut visual) = board(cx);
        let visual = &mut visual;
        window
            .update(visual, |shell, window, cx| {
                shell
                    .board
                    .as_ref()
                    .unwrap()
                    .update(cx, |board, cx| board.open_new_card(window, cx))
            })
            .unwrap();
        visual.run_until_parked();
        visual.simulate_input("half a title");
        escape(visual);
        window
            .update(visual, |shell, _, cx| {
                assert!(!shell.board.as_ref().unwrap().read(cx).new_card_open());
            })
            .unwrap();
        assert_eq!(view(window, visual), Some(app_views::AppView::Board));
    }

    #[gpui::test]
    fn escape_closes_a_board_menu_and_nothing_else_ever_leaves(cx: &mut TestAppContext) {
        let (window, mut visual) = board(cx);
        let visual = &mut visual;
        tap(visual, "board-sort");
        assert!(visual.debug_bounds("board-sort-manual").is_some());
        escape(visual);
        assert!(visual.debug_bounds("board-sort-manual").is_none());
        assert_eq!(view(window, visual), Some(app_views::AppView::Board));
        // With nothing left to close the Board still stays open.
        escape(visual);
        assert_eq!(view(window, visual), Some(app_views::AppView::Board));
    }

    #[gpui::test]
    fn escape_steps_out_of_the_inbox_one_layer_at_a_time(cx: &mut TestAppContext) {
        cx.update(crate::shell::bind_keys);
        cx.update(crate::popover::bind);
        let (window, _) = setup(cx, false);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |shell, window, cx| {
                Surface::Inbox.open(shell, window, cx)
            })
            .unwrap();
        visual.run_until_parked();
        let inbox = window
            .read_with(visual, |shell, _| shell.inbox.clone().unwrap())
            .unwrap();
        // No tracker is connected here, so the header has no filter button.
        inbox.update(visual, |inbox, cx| inbox.test_open_filters(cx));
        visual.run_until_parked();
        inbox.read_with(visual, |inbox, _| assert!(inbox.filters_open()));
        escape(visual);
        inbox.read_with(visual, |inbox, _| assert!(!inbox.filters_open()));
        assert_eq!(view(window, visual), Some(app_views::AppView::Inbox));
        escape(visual);
        assert_eq!(
            view(window, visual),
            None,
            "the last Escape leaves the Inbox"
        );
    }

    #[gpui::test]
    fn alt_arrows_move_a_focused_subtask(cx: &mut TestAppContext) {
        let (window, mut visual) = board(cx);
        let visual = &mut visual;
        tap(visual, "board-card-1");
        tap(visual, "board-subtask-add");
        for title in ["one", "two"] {
            visual.simulate_input(title);
            visual.simulate_keystrokes("enter");
            visual.run_until_parked();
        }
        escape(visual);
        let board = window
            .read_with(visual, |shell, _| shell.board.clone().unwrap())
            .unwrap();
        window
            .update(visual, |_, window, cx| {
                board.update(cx, |board, cx| {
                    board.test_focus_subtask(1, window, cx);
                })
            })
            .unwrap();
        visual.run_until_parked();
        visual.simulate_keystrokes("alt-up");
        visual.run_until_parked();
        board.read_with(visual, |board, _| {
            assert_eq!(board.test_subtask_titles(), ["two", "one"])
        });
    }

    #[gpui::test]
    fn tab_reaches_a_list_row_and_enter_opens_it(cx: &mut TestAppContext) {
        let (window, mut visual) = board(cx);
        let visual = &mut visual;
        tap(visual, "board-layout-list");
        let board = window
            .read_with(visual, |shell, _| shell.board.clone().unwrap())
            .unwrap();
        let reached = (0..40).any(|_| {
            visual.simulate_keystrokes("tab");
            visual.run_until_parked();
            window
                .update(visual, |_, window, cx| {
                    board.read(cx).test_focused_card(window, cx).is_some()
                })
                .unwrap()
        });
        assert!(reached, "Tab reaches the row");
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        board.read_with(visual, |board, _| assert!(board.card_detail.is_some()));
    }

    #[gpui::test]
    fn pick_a_date_saves_from_the_card_face(cx: &mut TestAppContext) {
        let (window, mut visual) = board(cx);
        let visual = &mut visual;
        let board = window
            .read_with(visual, |shell, _| shell.board.clone().unwrap())
            .unwrap();
        window
            .update(visual, |_, window, cx| {
                board.update(cx, |board, cx| board.test_open_due_menu(window, cx))
            })
            .unwrap();
        visual.run_until_parked();
        tap(visual, "menu-action-due-pick");
        assert!(visual.debug_bounds("board-due-entry").is_some());
        // Escape backs out of the field, and the Board stays.
        escape(visual);
        assert!(visual.debug_bounds("board-due-entry").is_none());
        assert_eq!(view(window, visual), Some(app_views::AppView::Board));

        window
            .update(visual, |_, window, cx| {
                board.update(cx, |board, cx| board.test_open_due_menu(window, cx))
            })
            .unwrap();
        visual.run_until_parked();
        tap(visual, "menu-action-due-pick");
        visual.simulate_input("2030-03-14");
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("board-due-entry").is_none());
        assert!(visual.debug_bounds("board-card-due-1").is_some());
    }

    #[gpui::test]
    fn the_focused_layout_shows_the_meet_pilots_card(cx: &mut TestAppContext) {
        cx.update(crate::shell::bind_keys);
        let (window, _) = setup(cx, false);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        let card = visual.debug_bounds("pilot-intro").unwrap();
        let list = visual.debug_bounds("sidebar-sessions").unwrap();
        let row = visual.debug_bounds("sidebar-new-thread").unwrap();
        // Centred in the sidebar, on the same edges as the rows around it.
        let (left, right) = (card.left() - list.left(), list.right() - card.right());
        assert!(left > gpui::px(0.), "{card:?} is inset from {list:?}");
        assert!(
            (left - right).abs() < gpui::px(0.5),
            "{left:?} vs {right:?}"
        );
        assert!(
            (card.left() - row.left()).abs() < gpui::px(0.5),
            "{card:?} vs {row:?}"
        );
        assert!(
            (card.right() - row.right()).abs() < gpui::px(0.5),
            "{card:?} vs {row:?}"
        );
        assert!(visual.debug_bounds("new-pilot").is_none());
        tap(visual, "pilot-intro-create");
        assert!(visual.debug_bounds("pilot-intro").is_none());
        window
            .update(visual, |shell, _, cx| {
                let project = shell.workspace.project.clone();
                let manager = shell.pilot_manager();
                let manager = manager.read(cx);
                assert_eq!(
                    manager.roster.pilots[0].projects,
                    [project],
                    "a Pilot made in a project starts on that project"
                );
            })
            .unwrap();
    }
}

#[gpui::test]
fn a_gradient_backdrop_is_painted_behind_the_shell(cx: &mut TestAppContext) {
    let (window, _, _) = crate::shell::tests::window(cx);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let visual = &mut visual;
    let set = |visual: &mut VisualTestContext, backdrop: &str| {
        window
            .update(visual, |shell, w, cx| {
                let mut p = shell.workspace.preferences.clone();
                p.theme = "dark".into();
                p.inset_glass = true;
                p.backdrop = backdrop.into();
                shell.set_preferences(p, w, cx);
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        visual.debug_bounds("shell-backdrop").is_some()
    };
    assert!(!set(visual, "desktop"), "Desktop leaves the native glass");
    assert!(set(visual, "gradient"));
    assert!(!set(visual, "desktop"));
}
