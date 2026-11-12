use super::*;
use gpui::{
    Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions, point,
    size,
};

fn window(section: Section, cx: &mut TestAppContext) -> WindowHandle<SettingsView> {
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1200.), px(800.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    let mut view = SettingsView::new(
                        section,
                        Preferences::default(),
                        SettingsContext::default(),
                        cx,
                    );
                    view.discovery_task = None;
                    view.discovery_loading = false;
                    view
                })
            },
        )
        .unwrap()
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
}

fn appearance_prefs(
    window: WindowHandle<SettingsView>,
    visual: &mut VisualTestContext,
    change: impl FnOnce(&mut Preferences),
) {
    window
        .update(visual, |view, _, cx| {
            change(&mut view.prefs);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
}

#[gpui::test]
fn transparent_window_is_an_inset_row_and_turning_it_off_disables_glass_controls(
    cx: &mut TestAppContext,
) {
    let window = window(Section::Appearance, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    // Tall enough to click the rows below the fold.
    visual.simulate_resize(size(px(1200.), px(2400.)));
    const TOGGLE: &str = "settings-toggle-inset-glass";

    appearance_prefs(window, visual, |p| p.theme = "dark".into());
    let toggle = visual.debug_bounds(TOGGLE).unwrap();
    visual.simulate_click(toggle.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    let saved = window
        .update(visual, |view, _, _| view.prefs.inset_glass)
        .unwrap();
    assert_eq!(saved, !crate::theme::GLASS_PLATFORM);

    // With the window opaque, the glass controls under it do nothing.
    appearance_prefs(window, visual, |p| {
        p.inset_glass = false;
        p.body_glass = true;
    });
    let main_pane = visual
        .debug_bounds("settings-toggle-main-pane-glass")
        .unwrap();
    visual.simulate_click(main_pane.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |view, _, _| assert!(view.prefs.body_glass))
        .unwrap();

    // Light mode keeps the dark-mode choice and won't change it.
    appearance_prefs(window, visual, |p| {
        p.theme = "light".into();
        p.inset_glass = true;
    });
    let toggle = visual.debug_bounds(TOGGLE).unwrap();
    visual.simulate_click(toggle.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |view, _, _| assert!(view.prefs.inset_glass))
        .unwrap();
}

#[gpui::test]
fn restore_defaults_keeps_the_focused_layout(cx: &mut TestAppContext) {
    let window = window(Section::Appearance, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    window
        .update(visual, |view, _, cx| {
            view.ctx.new_layout_boxy = true;
            cx.notify();
        })
        .unwrap();
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = seen.clone();
    let entity = window.root(visual).unwrap();
    let _subscription = visual.update(|_, cx| {
        cx.subscribe(&entity, move |_, event: &SettingsEvent, _| {
            log.borrow_mut().push(match event {
                SettingsEvent::Changed(_) => "changed",
                SettingsEvent::NewLayoutBoxy(_) => "workspace style",
                _ => "other",
            })
        })
    });
    window
        .update(visual, |view, _, cx| view.restore_defaults(cx))
        .unwrap();
    visual.run_until_parked();
    // The style lives in the workspace, so only the shell could change it, and
    // only by hearing a layout event.
    assert_eq!(*seen.borrow(), ["changed"], "no layout event");
}

#[gpui::test]
fn interface_scale_slider_steps_through_every_percent(cx: &mut TestAppContext) {
    let window = window(Section::Appearance, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1200.), px(2400.)));
    visual.run_until_parked();
    visual.refresh().unwrap();
    let track = visual
        .debug_bounds("settings-slider-interface-scale")
        .unwrap();
    // The thumb travels the track less its own 12px width.
    let travel = track.size.width.as_f32() - 12.;
    let last = (UI_SCALE_PERCENTS.len() - 1) as f32;
    let mut seen = Vec::new();
    for ix in 0..UI_SCALE_PERCENTS.len() {
        let x = track.left() + px(6. + travel * ix as f32 / last);
        visual.simulate_click(point(x, track.center().y), Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
        seen.push(
            window
                .update(visual, |view, _, _| view.prefs.ui_scale)
                .unwrap(),
        );
    }
    assert_eq!(seen, UI_SCALE_PERCENTS);
}

#[gpui::test]
fn interface_scale_slider_applies_only_on_release(cx: &mut TestAppContext) {
    let window = window(Section::Appearance, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1200.), px(2400.)));
    visual.run_until_parked();
    visual.refresh().unwrap();
    let track = visual
        .debug_bounds("settings-slider-interface-scale")
        .unwrap();
    let travel = track.size.width.as_f32() - 12.;
    let last = (UI_SCALE_PERCENTS.len() - 1) as f32;
    let at = |ix: usize| {
        point(
            track.left() + px(6. + travel * ix as f32 / last),
            track.center().y,
        )
    };
    let saved = window
        .update(visual, |view, _, _| view.prefs.ui_scale)
        .unwrap();
    let changes = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = changes.clone();
    let entity = window.root(visual).unwrap();
    let _subscription = visual.update(|_, cx| {
        cx.subscribe(&entity, move |_, event: &SettingsEvent, _| {
            if matches!(event, SettingsEvent::Changed(_)) {
                count.set(count.get() + 1);
            }
        })
    });
    let state = |visual: &mut VisualTestContext| {
        window
            .update(visual, |view, _, _| {
                (view.prefs.ui_scale, view.shown_ui_scale())
            })
            .unwrap()
    };

    visual.simulate_mouse_down(at(3), MouseButton::Left, Default::default());
    visual.simulate_mouse_move(at(9), MouseButton::Left, Default::default());
    visual.run_until_parked();
    // Mid-drag the slider shows the pending step, but the window is not zoomed.
    assert_eq!(state(visual), (saved, UI_SCALE_PERCENTS[9]));
    assert_eq!(changes.get(), 0, "no preference change mid-drag");
    visual.simulate_mouse_move(at(7), MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert_eq!(state(visual), (saved, UI_SCALE_PERCENTS[7]));
    assert_eq!(changes.get(), 0);

    visual.simulate_mouse_up(at(7), MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert_eq!(state(visual), (UI_SCALE_PERCENTS[7], UI_SCALE_PERCENTS[7]));
    assert_eq!(changes.get(), 1, "one change on release");
}

#[gpui::test]
fn other_sliders_still_apply_while_dragging(cx: &mut TestAppContext) {
    let window = window(Section::Appearance, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1200.), px(2400.)));
    visual.run_until_parked();
    visual.refresh().unwrap();
    let track = visual.debug_bounds("settings-slider-hue").unwrap();
    let before = window
        .update(visual, |view, _, _| view.prefs.theme_hue)
        .unwrap();
    visual.simulate_mouse_down(
        point(track.left() + px(6.), track.center().y),
        MouseButton::Left,
        Default::default(),
    );
    visual.simulate_mouse_move(
        point(track.right() - px(6.), track.center().y),
        MouseButton::Left,
        Default::default(),
    );
    visual.run_until_parked();
    let during = window
        .update(visual, |view, _, _| view.prefs.theme_hue)
        .unwrap();
    assert_ne!(during, before, "hue is live mid-drag");
}

#[test]
fn interface_scale_slider_maps_positions_to_percents() {
    let kind = SliderKind::InterfaceScale;
    assert_eq!(kind.range(), (0, UI_SCALE_PERCENTS.len() as u32 - 1));
    let mut prefs = Preferences::default();
    assert_eq!(UI_SCALE_PERCENTS[kind.get(&prefs) as usize], prefs.ui_scale);
    kind.set(&mut prefs, 0);
    assert_eq!(prefs.ui_scale, 50);
    kind.set(&mut prefs, 15);
    assert_eq!(prefs.ui_scale, 200);
    // A saved scale off the list lands on the nearest step.
    prefs.ui_scale = 117;
    assert_eq!(UI_SCALE_PERCENTS[kind.get(&prefs) as usize], 120);
    prefs.ui_scale = 5000;
    assert_eq!(kind.get(&prefs), 15);
}

#[gpui::test]
fn editor_group_lists_the_moved_rows(cx: &mut TestAppContext) {
    let window = window(Section::Chat, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1200.), px(2400.)));
    visual.run_until_parked();
    visual.refresh().unwrap();
    let ys: Vec<_> = [
        "settings-row-file-tabs",
        "settings-row-file-word-wrap",
        "settings-row-show-excluded-files",
        "settings-row-format-on-save",
    ]
    .into_iter()
    .map(|selector| {
        visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is on the Chat page"))
            .origin
            .y
    })
    .collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "{ys:?}");
}

#[gpui::test]
fn the_general_page_no_longer_holds_editor_rows(cx: &mut TestAppContext) {
    let window = window(Section::General, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1200.), px(2400.)));
    visual.run_until_parked();
    visual.refresh().unwrap();
    for selector in [
        "settings-row-file-tabs",
        "settings-row-file-word-wrap",
        "settings-row-tab-animations",
    ] {
        assert!(visual.debug_bounds(selector).is_none(), "{selector}");
    }
}

#[gpui::test]
fn blur_radius_is_only_offered_where_the_platform_honors_it(cx: &mut TestAppContext) {
    let window = window(Section::Appearance, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    appearance_prefs(window, visual, |p| p.theme = "dark".into());
    assert_eq!(
        visual.debug_bounds("settings-row-blur").is_some(),
        crate::window_blur::BLUR_ADJUSTABLE
    );
    assert_eq!(
        model::search("blur radius", "")
            .iter()
            .any(|r| r.setting == Some("blur")),
        crate::window_blur::BLUR_ADJUSTABLE
    );
}

#[test]
fn blur_slider_covers_the_saved_range() {
    assert_eq!(SliderKind::Blur.range(), (1, 64));
    let mut prefs = Preferences::default();
    assert_eq!(SliderKind::Blur.get(&prefs), 7);
    SliderKind::Blur.set(&mut prefs, 60);
    assert_eq!(prefs.sidebar_blur, 60);
}

#[gpui::test]
fn workspace_style_is_always_offered_and_the_layout_row_is_gone(cx: &mut TestAppContext) {
    let window = window(Section::Appearance, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    for boxy in [false, true] {
        window
            .update(visual, |view, _, cx| {
                view.ctx.new_layout_boxy = boxy;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(
            visual
                .debug_bounds("settings-new-layout-style-boxy")
                .is_some()
        );
        assert!(
            visual
                .debug_bounds("settings-new-layout-style-rounded")
                .is_some()
        );
        assert!(visual.debug_bounds("settings-row-inset-layout").is_none());
        assert!(
            visual
                .debug_bounds("settings-inset-layout-classic")
                .is_none()
        );
    }
}

#[gpui::test]
fn skills_document_has_its_own_full_height_pane_and_compact_actions(cx: &mut TestAppContext) {
    let window = window(Section::Skills, cx);
    window
        .update(cx, |view, _, cx| {
            let path = std::path::PathBuf::from("/project/.agents/skills/review/SKILL.md");
            view.discovery.skills = vec![data::Skill {
                name: "review".into(),
                description: "Review changes".into(),
                path: path.clone(),
                scope: "project".into(),
                provider: "agents".into(),
            }];
            view.skill_preview = Some(path);
            view.skill_preview_text = Some(Ok(skill_document::DocumentData::new(format!(
                "---\nname: review\n---\n# Review\n\n{}",
                "Instructions.\n\n".repeat(150)
            ))));
            cx.notify();
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let left = visual.debug_bounds("skills-list-scroll").unwrap();
    let right = visual.debug_bounds("skill-preview-pane").unwrap();
    let body = visual.debug_bounds("skill-document").unwrap();
    assert_eq!(left.size.height, px(800.));
    assert_eq!(right.size.height, px(800.));
    assert!(right.left() >= left.right());
    assert!(body.bottom() <= right.bottom());
    assert!(body.size.height > px(600.));
    assert!(visual.debug_bounds("transcript").is_none());
    for id in [
        "skill-preview-action",
        "skill-copy-action",
        "skill-reveal-action",
    ] {
        assert_eq!(
            visual.debug_bounds(id).unwrap().size,
            size(px(20.), px(20.))
        );
    }
    let source = visual
        .debug_bounds("settings-skill-document-source")
        .unwrap();
    assert!(source.size.width < px(100.));
    visual.simulate_click(source.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |view, _, _| {
            assert_eq!(view.prefs.skill_source_paths.len(), 1)
        })
        .unwrap();
    visual.simulate_resize(size(px(700.), px(800.)));
    visual.run_until_parked();
    // Deliver the frame that applies the measured Skills container breakpoint.
    visual.update(|window, cx| {
        window.simulate_next_frame(cx);
    });
    visual.run_until_parked();
    let left = visual.debug_bounds("skills-list-scroll").unwrap();
    let right = visual.debug_bounds("skill-preview-pane").unwrap();
    assert!(
        right.top() >= left.bottom(),
        "list={left:?}, preview={right:?}"
    );
    assert_eq!(left.size.width, px(700.));
    assert_eq!(right.size.width, px(700.));
    assert!(visual.debug_bounds("skill-document").unwrap().size.height > px(200.));
}

#[gpui::test]
fn the_mcp_page_shows_what_each_server_would_run_and_whether_it_needs_approval(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let (_, visual) = &mut catalog_window(&dir, cx);
    assert!(visual.debug_bounds("settings-catalog-detail-0").is_some());
    assert!(visual.debug_bounds("settings-catalog-approve-0").is_some());
    assert!(
        visual
            .debug_bounds("settings-segmented-catalog-class")
            .is_some()
    );
    assert!(
        visual.debug_bounds("settings-catalog-off-0").is_none(),
        "not on yet"
    );
}

const APPROVE: &str = "settings-catalog-approve-0";

fn saved(dir: &tempfile::TempDir) -> gespan_core::mcp_catalog::Catalog {
    gespan_core::mcp_catalog::Catalog::load(dir.path()).unwrap()
}

/// The MCP page over a catalogue of one server, loaded from disk as the app
/// does.
fn catalog_window(
    dir: &tempfile::TempDir,
    cx: &mut TestAppContext,
) -> (WindowHandle<SettingsView>, VisualTestContext) {
    catalog_window_of(
        dir,
        vec![gespan_core::mcp_catalog::Entry {
            id: "docs".into(),
            name: "Docs search".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "docs-mcp".into()],
            source: Some("claude (user)".into()),
            ..Default::default()
        }],
        cx,
    )
}

fn catalog_window_of(
    dir: &tempfile::TempDir,
    entries: Vec<gespan_core::mcp_catalog::Entry>,
    cx: &mut TestAppContext,
) -> (WindowHandle<SettingsView>, VisualTestContext) {
    gespan_tracker_secrets::testing::use_memory_keychain();
    let catalog = gespan_core::mcp_catalog::Catalog {
        entries,
        ..Default::default()
    };
    catalog.save(dir.path()).unwrap();
    let window = window(Section::Mcp, cx);
    let path = dir.path().to_owned();
    window
        .update(cx, |view, _, cx| {
            view.mcp_catalog_dir = Some(path);
            view.load_catalog(cx);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    // Tall enough that every catalogue control stays on screen to click.
    visual.simulate_resize(size(px(1200.), px(1600.)));
    settle(&mut visual);
    (window, visual)
}

fn remote_entry(url: &str) -> gespan_core::mcp_catalog::Entry {
    gespan_core::mcp_catalog::Entry {
        id: "linear".into(),
        name: "Linear".into(),
        url: Some(url.into()),
        ..Default::default()
    }
}

#[gpui::test]
fn a_failed_sign_in_stays_on_screen_and_the_button_comes_back(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let entries = vec![remote_entry("http://mcp.example/mcp")];
    let (window, visual) = &mut catalog_window_of(&dir, entries, cx);
    assert!(
        visual
            .debug_bounds("settings-segmented-catalog-class")
            .is_none(),
        "a remote connection has no process choice"
    );
    click(visual, "settings-catalog-sign-in-0");
    assert!(
        visual.debug_bounds("settings-catalog-error").is_some(),
        "the reason is gone"
    );
    window
        .update(visual, |view, _, _| {
            assert!(view.mcp_signing_in.is_empty());
            assert!(
                view.mcp_catalog_error
                    .as_deref()
                    .is_some_and(|error| error.contains("https"))
            );
        })
        .unwrap();
}

#[gpui::test]
fn a_second_sign_in_waits_for_the_first(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let entries = vec![remote_entry("http://mcp.example/mcp")];
    let (window, visual) = &mut catalog_window_of(&dir, entries, cx);
    window
        .update(visual, |view, _, cx| {
            view.mcp_signing_in.insert("linear".into());
            view.catalog_sign_in("linear".into(), "http://mcp.example/mcp".into(), cx);
        })
        .unwrap();
    settle(visual);
    window
        .update(visual, |view, _, _| {
            assert!(view.mcp_signing_in.contains("linear"), "it's still waiting");
            assert!(view.mcp_catalog_error.is_none(), "no second flow ran");
        })
        .unwrap();
}

#[gpui::test]
fn adding_from_the_registry_lands_off_and_settings_unlock_approval(cx: &mut TestAppContext) {
    use gespan_core::mcp_registry::{Input, Install, Listing};
    let dir = tempfile::tempdir().unwrap();
    let (window, visual) = &mut catalog_window_of(&dir, Vec::new(), cx);
    visual.simulate_resize(size(px(1000.), px(1400.)));
    window
        .update(visual, |view, _, cx| {
            view.mcp_listings = vec![Listing {
                name: "com.acme/search".into(),
                description: String::new(),
                version: "1.2.0".into(),
                install: Install::Command {
                    command: "npx".into(),
                    args: vec!["-y".into(), "acme@1.2.0".into()],
                },
                inputs: vec![Input {
                    name: "API_KEY".into(),
                    description: String::new(),
                    required: true,
                }],
            }];
            cx.notify();
        })
        .unwrap();
    settle(visual);
    click(visual, "settings-catalog-add-0");
    let entry = saved(&dir).entries.remove(0);
    assert!(!entry.enabled && entry.approved.is_none());
    assert_eq!(entry.needed, ["API_KEY"]);

    click(visual, "settings-catalog-settings-0");
    window
        .update(visual, |view, _, cx| {
            let (_, input) = view.mcp_secrets.clone().unwrap();
            input.update(cx, |input, _| {
                assert!(input.is_secret(), "a key stays off the screen");
                input.content = "API_KEY=k\nLOG=info".into();
            });
        })
        .unwrap();
    click(visual, "settings-catalog-settings-save");
    let entry = saved(&dir).entries.remove(0);
    assert!(entry.needed.is_empty());
    assert_eq!(entry.env, ["API_KEY", "LOG"]);

    click(visual, APPROVE);
    answer(visual, "Approve and turn on");
    let entry = saved(&dir).entries.remove(0);
    assert!(entry.enabled && entry.approved.is_some());
}

#[gpui::test]
fn a_mistyped_setting_is_reported_without_repeating_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (window, visual) = &mut catalog_window(&dir, cx);
    click(visual, "settings-catalog-settings-0");
    window
        .update(visual, |view, _, cx| {
            let (_, input) = view.mcp_secrets.clone().unwrap();
            input.update(cx, |input, _| input.content = "s3cret-no-name".into());
        })
        .unwrap();
    click(visual, "settings-catalog-settings-save");
    window
        .update(visual, |view, _, _| {
            let error = view.mcp_catalog_error.clone().unwrap();
            assert!(!error.contains("s3cret"), "{error}");
        })
        .unwrap();
}

#[gpui::test]
fn a_failure_stays_on_the_page_after_its_server_is_removed(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (window, visual) = &mut catalog_window(&dir, cx);
    window
        .update(visual, |view, _, cx| {
            view.mcp_server_errors = vec![gespan_core::tool_servers::ServerFailure {
                server: "docs".into(),
                message: "The server exited".into(),
                at: std::time::SystemTime::now(),
            }];
            cx.notify();
        })
        .unwrap();
    settle(visual);
    assert!(visual.debug_bounds("settings-catalog-failure-0").is_some());
    click(visual, "settings-catalog-remove-0");
    assert!(saved(&dir).entries.is_empty());
    assert!(
        visual.debug_bounds("settings-catalog-failure-0").is_some(),
        "the empty page hid it"
    );
}

fn settle(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual.run_until_parked();
}

fn reload(window: WindowHandle<SettingsView>, visual: &mut VisualTestContext) {
    window
        .update(visual, |view, _, cx| view.load_catalog(cx))
        .unwrap();
    settle(visual);
}

fn answer(visual: &mut VisualTestContext, button: &str) {
    visual.simulate_prompt_answer(button);
    settle(visual);
}

fn click(visual: &mut VisualTestContext, selector: &'static str) {
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("no {selector}"));
    visual.simulate_click(bounds.center(), Default::default());
    settle(visual);
}

#[gpui::test]
fn the_catalogue_page_shows_what_is_saved_and_why_it_cant_load(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (window, visual) = &mut catalog_window(&dir, cx);
    assert!(visual.debug_bounds(APPROVE).is_some(), "the saved entry");
    assert!(visual.debug_bounds("settings-catalog-error").is_none());
    std::fs::write(dir.path().join("mcp-catalog.json"), "not json").unwrap();
    reload(*window, visual);
    assert!(visual.debug_bounds("settings-catalog-error").is_some());
}

#[gpui::test]
fn approving_a_server_turns_it_on_and_cancelling_does_not(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (_, visual) = &mut catalog_window(&dir, cx);
    click(visual, APPROVE);
    answer(visual, "Cancel");
    assert!(!saved(&dir).entries[0].enabled);
    click(visual, APPROVE);
    answer(visual, "Approve and turn on");
    let entry = saved(&dir).entries.remove(0);
    assert!(entry.enabled && entry.approved.is_some());
    assert!(visual.debug_bounds("settings-catalog-off-0").is_some());
    assert!(visual.debug_bounds(APPROVE).is_none());
    click(visual, "settings-catalog-off-0");
    assert!(!saved(&dir).entries[0].enabled);
    assert!(visual.debug_bounds(APPROVE).is_some());
}

#[gpui::test]
fn a_server_edited_after_approval_asks_again_and_can_be_approved(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (window, visual) = &mut catalog_window(&dir, cx);
    click(visual, APPROVE);
    answer(visual, "Approve and turn on");
    let mut catalog = saved(&dir);
    catalog.entries[0].args.push("--more".into());
    catalog.save(dir.path()).unwrap();
    reload(*window, visual);
    assert!(
        visual.debug_bounds(APPROVE).is_some(),
        "on, but it won't run until reviewed"
    );
    click(visual, APPROVE);
    answer(visual, "Approve and turn on");
    assert!(visual.debug_bounds(APPROVE).is_none());
    assert_eq!(saved(&dir).specs(dir.path()).len(), 1);
}

#[gpui::test]
fn managed_mode_and_removal_reach_the_saved_catalogue(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (_, visual) = &mut catalog_window(&dir, cx);
    click(visual, "settings-toggle-catalog-managed");
    assert!(saved(&dir).managed);
    click(visual, "settings-toggle-catalog-managed");
    assert!(!saved(&dir).managed);
    click(visual, "settings-catalog-remove-0");
    assert!(saved(&dir).entries.is_empty());
    assert!(visual.debug_bounds(APPROVE).is_none());
}

fn a_tree() -> data::Tree {
    data::Tree {
        tree: gespan_core::git::Worktree {
            path: "/work/app-worktrees/feature".into(),
            branch: Some("feature".into()),
            ..Default::default()
        },
        locked: false,
        dirty: Some(false),
        unpushed: 0,
        merged: None,
        setup: None,
    }
}

#[gpui::test]
fn the_delete_dialog_starts_from_the_project_then_the_app(cx: &mut TestAppContext) {
    let window = window(Section::Worktrees, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let opened = |visual: &mut VisualTestContext, project: &str| {
        window
            .update(visual, |view, window, cx| {
                view.selected_project = project.into();
                view.management = None;
                view.open_management(management::Target::DeleteTree(a_tree()), window, cx);
                let editor = view.management.as_ref().unwrap();
                (editor.delete_branch, editor.delete_sessions)
            })
            .unwrap()
    };
    assert_eq!(
        opened(visual, "/a"),
        (false, false),
        "keeps things at first"
    );
    window
        .update(visual, |view, _, _| {
            view.prefs.worktree_delete_branch = true;
            view.prefs.project_worktrees.insert(
                "/a".into(),
                preferences::WorktreeDefaults {
                    delete_branch: Some(false),
                    delete_sessions: Some(true),
                },
            );
        })
        .unwrap();
    assert_eq!(opened(visual, "/a"), (false, true));
    assert_eq!(opened(visual, "/b"), (true, false));
}

#[gpui::test]
fn scripts_save_and_unsaved_text_survives_picking_another_project(cx: &mut TestAppContext) {
    let one = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let main = one.path().to_string_lossy().into_owned();
    let second = other.path().to_string_lossy().into_owned();
    let window = window(Section::Worktrees, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let (first, in_second) = (main.clone(), second.clone());
    window
        .update(visual, move |view, _, cx| {
            view.scripts_for(&first, cx);
            let setup = view.worktree_scripts[&first].setup.clone();
            setup.update(cx, |input, cx| input.set_text("bun install".into(), cx));
            view.save_worktree_scripts(&first, cx);
            setup.update(cx, |input, cx| input.set_text("not saved".into(), cx));
            view.scripts_for(&in_second, cx);
            view.scripts_for(&first, cx);
            let kept = view.worktree_scripts[&first]
                .setup
                .read(cx)
                .content
                .to_string();
            assert_eq!(kept, "not saved");
        })
        .unwrap();
    let saved = gespan_core::worktree_scripts::scripts(std::path::Path::new(&main));
    assert_eq!(saved.setup, "bun install");
}

#[gpui::test]
fn the_delete_default_controls_set_the_app_and_the_project(cx: &mut TestAppContext) {
    let repo = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(repo.path())
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}");
    };
    git(&["init", "-b", "main"]);
    let window = window(Section::Worktrees, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    // Tall enough to click the rows below the fold.
    visual.simulate_resize(size(px(1200.), px(2400.)));
    let project = repo.path().to_string_lossy().into_owned();
    let main = project.clone();
    window
        .update(visual, move |view, _, cx| {
            view.selected_project = main.clone();
            view.discovery.worktrees.main = main;
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    for target in [
        "settings-worktree-delete-branch-on",
        "settings-worktree-delete-sessions-project-on",
    ] {
        let control = visual
            .debug_bounds(target)
            .unwrap_or_else(|| panic!("{target} is not on screen"));
        visual.simulate_click(control.center(), Default::default());
        visual.run_until_parked();
    }
    window
        .read_with(visual, |view, _| {
            assert!(view.prefs.deletes_merged_branch("/other"));
            assert!(view.prefs.deletes_worktree_sessions(&project));
            assert!(!view.prefs.deletes_worktree_sessions("/other"));
        })
        .unwrap();
}

#[gpui::test]
fn related_by_meaning_row_is_shown_only_while_the_decision_log_is_on(cx: &mut TestAppContext) {
    let window = window(Section::Providers, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1200.), px(6000.)));
    for log in [false, true, false] {
        appearance_prefs(window, visual, |p| {
            p.decision_log = log;
            p.decision_meaning = true;
        });
        assert!(visual.debug_bounds("settings-row-decision-log").is_some());
        assert_eq!(
            visual
                .debug_bounds("settings-row-decision-meaning")
                .is_some(),
            log,
            "decision log {log}"
        );
    }
}
