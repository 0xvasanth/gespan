//! The glass General and Appearance pages: what moved, what each control
//! still writes, and what the preview says.
use super::*;
use gpui::{
    Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions, point,
    size,
};
use std::{cell::RefCell, rc::Rc};

fn window(
    section: Section,
    prefs: Preferences,
    cx: &mut TestAppContext,
) -> (WindowHandle<SettingsView>, VisualTestContext) {
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1200.), px(3200.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    let mut view =
                        SettingsView::new(section, prefs, SettingsContext::default(), cx);
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
    let visual = VisualTestContext::from_window(window.into(), cx);
    (window, visual)
}

/// Every event the view emits, by a short name.
fn record(
    window: WindowHandle<SettingsView>,
    visual: &mut VisualTestContext,
) -> (Rc<RefCell<Vec<String>>>, gpui::Subscription) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let entity = window.root(visual).unwrap();
    let subscription = visual.update(|_, cx| {
        cx.subscribe(&entity, move |_, event: &SettingsEvent, _| {
            log.borrow_mut().push(match event {
                SettingsEvent::WhatsNew => "whats-new".into(),
                SettingsEvent::CheckAppUpdates => "check-app".into(),
                SettingsEvent::CheckHarnessUpdates => "check-harness".into(),
                SettingsEvent::SidebarOption(id) => format!("sidebar:{id}"),
                SettingsEvent::NewLayoutBoxy(boxy) => format!("boxy:{boxy}"),
                SettingsEvent::Changed(_) => "changed".into(),
                _ => "other".into(),
            })
        })
    });
    (seen, subscription)
}

fn click(visual: &mut VisualTestContext, selector: &str) {
    let selector: &'static str = selector.to_owned().leak();
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    visual.simulate_click(bounds.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn prefs(window: WindowHandle<SettingsView>, visual: &mut VisualTestContext) -> Preferences {
    window
        .update(visual, |view, _, _| view.prefs.clone())
        .unwrap()
}

#[gpui::test]
fn the_version_card_leads_general_and_its_actions_reach_the_shell(cx: &mut TestAppContext) {
    let (window, mut visual) = window(Section::General, Preferences::default(), cx);
    let visual = &mut visual;
    let card = visual.debug_bounds("settings-row-update").unwrap();
    let sounds = visual.debug_bounds("settings-row-sounds").unwrap();
    assert!(
        card.bottom() <= sounds.top(),
        "the version comes before Alerts"
    );
    let (seen, _keep) = record(window, visual);
    click(visual, "settings-whats-new");
    click(visual, "settings-check-harness-updates");
    click(visual, "settings-check-updates");
    // Run setup again left this page: nothing here starts setup, and the
    // About group that held it is gone.
    assert_eq!(
        *seen.borrow(),
        ["whats-new", "check-harness", "check-app"],
        "each action emits its own event and nothing else"
    );
}

#[test]
fn the_version_pill_only_claims_up_to_date_after_a_check() {
    use crate::glass::Tone;
    use crate::notifications::updater::Action;
    assert_eq!(super::pages::version_pill(&Action::Check, false), None);
    assert_eq!(
        super::pages::version_pill(&Action::Check, true),
        Some(("Up to date".into(), Tone::Ok))
    );
    assert_eq!(
        super::pages::version_pill(&Action::Install("0.2.0".into()), true),
        Some(("Update available".into(), Tone::Waiting))
    );
    assert_eq!(
        super::pages::version_pill(&Action::Busy("Downloading 40%".into()), false),
        Some(("Downloading 40%".into(), Tone::Running))
    );
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn a_quick_composer_shortcut_conflict_shows_under_its_description(cx: &mut TestAppContext) {
    let (window, mut visual) = window(Section::General, Preferences::default(), cx);
    let visual = &mut visual;
    assert!(
        visual
            .debug_bounds("settings-quick-composer-error")
            .is_none()
    );
    window
        .update(visual, |view, _, cx| {
            view.ctx.quick_error = Some("This shortcut is already in use.".into());
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    let error = visual
        .debug_bounds("settings-quick-composer-error")
        .unwrap();
    let toggle = visual
        .debug_bounds("settings-toggle-quick-composer")
        .unwrap();
    let row = visual.debug_bounds("settings-row-quick-composer").unwrap();
    assert!(
        error.right() < toggle.left(),
        "beside the toggle, not among the controls"
    );
    assert!(row.contains(&error.center()));
}

#[gpui::test]
fn every_workspace_preset_has_a_swatch_and_picking_one_applies_it(cx: &mut TestAppContext) {
    let (window, mut visual) = window(Section::Appearance, Preferences::default(), cx);
    let visual = &mut visual;
    for (id, ..) in palettes::PALETTES {
        assert!(
            visual
                .debug_bounds(format!("settings-workspace-theme-{id}").leak())
                .is_some(),
            "{id} has a swatch"
        );
    }
    let last = palettes::PALETTES.last().unwrap().0;
    click(visual, &format!("settings-workspace-theme-{last}"));
    let mut expected = Preferences::default();
    expected.apply_workspace_theme(last.into());
    let saved = prefs(window, visual);
    assert_eq!(saved.workspace_theme, last);
    // The same tint the old menu applied comes with it.
    assert_eq!(
        (
            saved.theme_hue,
            saved.theme_saturation,
            saved.theme_dark_lightness
        ),
        (
            expected.theme_hue,
            expected.theme_saturation,
            expected.theme_dark_lightness
        )
    );
}

#[gpui::test]
fn restore_defaults_on_the_preview_resets_the_appearance(cx: &mut TestAppContext) {
    let changed = Preferences {
        theme: "light".into(),
        accent_color: Some("#ef4444".into()),
        diff_palette: "colorblind".into(),
        ui_scale: 130,
        sidebar_opacity: 40,
        // Not an appearance setting: it stays.
        sounds: false,
        ..Default::default()
    };
    let (window, mut visual) = window(Section::Appearance, changed.clone(), cx);
    let visual = &mut visual;
    click(visual, "settings-restore-defaults");
    let mut expected = changed;
    expected.restore_appearance();
    let saved = prefs(window, visual);
    assert_eq!(saved.theme, expected.theme);
    assert_eq!(saved.accent_color, None);
    assert_eq!(saved.diff_palette, expected.diff_palette);
    assert_eq!(saved.ui_scale, expected.ui_scale);
    assert_eq!(saved.sidebar_opacity, expected.sidebar_opacity);
    assert!(!saved.sounds, "only appearance resets");
}

#[gpui::test]
fn glass_sliders_write_the_same_preferences(cx: &mut TestAppContext) {
    let (window, mut visual) = window(Section::Appearance, Preferences::default(), cx);
    let visual = &mut visual;
    let track = visual.debug_bounds("settings-slider-saturation").unwrap();
    assert_eq!(track.size.width, px(240.), "the glass track");
    // The 16px knob's centre travels the track less its width.
    let at = |fraction: f32| {
        point(
            track.left() + px(8. + (track.size.width.as_f32() - 16.) * fraction),
            track.center().y,
        )
    };
    let (min, max) = SliderKind::Saturation.range();
    visual.simulate_click(at(0.5), Default::default());
    visual.run_until_parked();
    assert_eq!(
        prefs(window, visual).theme_saturation,
        min + ((max - min) as f32 * 0.5).round() as u32
    );
    visual.simulate_click(at(1.), Default::default());
    visual.run_until_parked();
    assert_eq!(prefs(window, visual).theme_saturation, max);
    visual.simulate_click(at(0.), Default::default());
    visual.run_until_parked();
    assert_eq!(prefs(window, visual).theme_saturation, min);
}

#[gpui::test]
fn layout_pictures_and_sidebar_chips_send_the_shell_events(cx: &mut TestAppContext) {
    let (window, mut visual) = window(Section::Appearance, Preferences::default(), cx);
    let visual = &mut visual;
    let (seen, _keep) = record(window, visual);
    click(visual, "settings-new-layout-style-boxy");
    click(visual, "settings-toggle-sidebar-inbox");
    click(visual, "settings-toggle-sidebar-activity");
    click(visual, "settings-toggle-compact-session-cards");
    assert_eq!(
        *seen.borrow(),
        [
            "boxy:true",
            "sidebar:sidebar-inbox",
            "sidebar:sidebar-activity",
            "sidebar:compact-session-cards"
        ]
    );
}

#[gpui::test]
fn picking_a_diff_palette_saves_it(cx: &mut TestAppContext) {
    let (window, mut visual) = window(Section::Appearance, Preferences::default(), cx);
    let visual = &mut visual;
    assert!(visual.debug_bounds("settings-diff-preview").is_some());
    click(visual, "settings-diff-colors-high-contrast");
    assert_eq!(prefs(window, visual).diff_palette, "high-contrast");
}

#[test]
fn the_preview_line_names_each_choice() {
    use super::pages::appearance_summary;
    let mut p = Preferences {
        theme: "dark".into(),
        workspace_theme: "harbor".into(),
        accent_color: Some("#8b5cf6".into()),
        inset_glass: true,
        sidebar_opacity: 85,
        sidebar_blur: 24,
        ..Default::default()
    };
    assert_eq!(
        appearance_summary(&p, false, false, true, true),
        "Dark · Harbor preset · Violet accent · Glass on, 85% opacity, 24px blur · Rounded workspace"
    );
    // Blur only where the platform honours it.
    assert!(
        appearance_summary(&p, false, true, true, false)
            .ends_with("Glass on, 85% opacity · Boxy workspace")
    );
    // Light mode and no platform glass both keep the window opaque.
    assert!(appearance_summary(&p, true, false, true, true).contains("Opaque in light mode"));
    assert!(appearance_summary(&p, false, false, false, true).contains("Opaque window"));
    p.inset_glass = false;
    p.accent_color = Some("#123456".into());
    p.theme = "system".into();
    let line = appearance_summary(&p, false, false, true, true);
    assert!(line.starts_with("System · Harbor preset · Custom accent · Glass off"));
    p.accent_color = None;
    assert!(appearance_summary(&p, false, false, true, true).contains("Default accent"));
}
