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
    classic: bool,
    change: impl FnOnce(&mut Preferences),
) {
    window
        .update(visual, |view, _, cx| {
            view.ctx.classic = classic;
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

    appearance_prefs(window, visual, true, |p| p.theme = "dark".into());
    assert!(visual.debug_bounds(TOGGLE).is_none(), "classic has no row");

    appearance_prefs(window, visual, false, |_| {});
    let toggle = visual.debug_bounds(TOGGLE).unwrap();
    visual.simulate_click(toggle.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    let saved = window
        .update(visual, |view, _, _| view.prefs.inset_glass)
        .unwrap();
    assert_eq!(saved, !crate::theme::GLASS_PLATFORM);

    // With the window opaque, the glass controls under it do nothing.
    appearance_prefs(window, visual, false, |p| {
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
    appearance_prefs(window, visual, false, |p| {
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
fn blur_radius_is_only_offered_where_the_platform_honors_it(cx: &mut TestAppContext) {
    let window = window(Section::Appearance, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    appearance_prefs(window, visual, true, |p| p.theme = "dark".into());
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
    assert_eq!(SliderKind::Blur.get(&prefs), 24);
    SliderKind::Blur.set(&mut prefs, 60);
    assert_eq!(prefs.sidebar_blur, 60);
}

#[gpui::test]
fn workspace_style_is_only_offered_under_the_new_layout(cx: &mut TestAppContext) {
    let window = window(Section::Appearance, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    for classic in [false, true, false] {
        window
            .update(visual, |view, _, cx| {
                view.ctx.classic = classic;
                view.ctx.new_layout_boxy = true;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(
            visual
                .debug_bounds("settings-new-layout-style-boxy")
                .is_some(),
            !classic
        );
        assert_eq!(
            visual
                .debug_bounds("settings-new-layout-style-rounded")
                .is_some(),
            !classic
        );
        assert!(
            visual
                .debug_bounds("settings-collapsed-project-rail-hidden")
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
fn pilot_mascot_is_beside_name_and_reset_stays_visible(cx: &mut TestAppContext) {
    let window = window(Section::Pilots, cx);
    window
        .update(cx, |view, _, cx| {
            view.ctx.pilots = vec![PilotSetting {
                id: "layout-pilot".into(),
                name: "Big Boss".into(),
                mascot: "ghost".into(),
                color: 0xee99bb,
                projects: vec!["gespan".into(), "pilot-web".into()],
            }];
            cx.notify();
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let mascot = visual.debug_bounds("settings-pilot-mascot").unwrap();
    let reset = visual.debug_bounds("settings-pilot-reset").unwrap();
    assert_eq!(mascot.size, size(px(16.), px(16.)));
    let name = visual.debug_bounds("settings-pilot-name").unwrap();
    assert!(mascot.right() < name.left());
    assert!(mascot.left() < px(200.));
    assert!(name.right() <= reset.left());
    assert!(reset.left() > px(800.));
    assert!(reset.right() <= px(1200.));
    assert!((mascot.top() - reset.top()).abs() < px(8.));
    // Filing a Pilot's sessions in its folder is no longer a setting.
    assert!(
        visual
            .debug_bounds("settings-toggle-pilot-sidebar-show")
            .is_none()
    );
    assert!(
        visual
            .debug_bounds("settings-toggle-pilot-sidebar-folders")
            .is_none()
    );
    visual.simulate_click(reset.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("settings-management-dialog").is_some());
    window
        .update(visual, |view, window, cx| {
            view.close_management(window, cx);
            cx.notify();
        })
        .unwrap();
    visual.simulate_resize(size(px(700.), px(800.)));
    visual.run_until_parked();
    let reset = visual.debug_bounds("settings-pilot-reset").unwrap();
    assert!(reset.right() <= px(700.));
}
