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
    // The name sits left of the swatches on their line, and each accent dot
    // is centred in its swatch.
    let name = visual
        .debug_bounds("settings-workspace-theme-name")
        .unwrap();
    let first = visual
        .debug_bounds("settings-workspace-theme-default")
        .unwrap();
    assert!(name.right() <= first.left());
    assert!((name.center().y - first.center().y).abs() < px(1.));
    for (id, ..) in palettes::PALETTES {
        let swatch = visual
            .debug_bounds(format!("settings-workspace-theme-{id}").leak())
            .unwrap();
        let dot = visual
            .debug_bounds(format!("settings-workspace-theme-dot-{id}").leak())
            .unwrap();
        assert!((dot.center().x - swatch.center().x).abs() < px(0.5), "{id}");
        assert!((dot.center().y - swatch.center().y).abs() < px(0.5), "{id}");
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

#[test]
fn the_inbox_filter_shows_muted_custom_or_matching_projects() {
    use super::notification_preferences::project_shown;
    // name, detail, muted, custom
    let projects = [
        ("acme/api", "~/code/api", false, false),
        ("acme/dashboard", "~/code/dash", false, true),
        ("notes", "~/notes", true, false),
    ];
    let names = |show: &str, query: &str| {
        projects
            .iter()
            .filter(|(n, d, m, c)| project_shown(show, query, n, d, *m, *c))
            .map(|p| p.0)
            .collect::<Vec<_>>()
    };
    assert_eq!(names("all", "").len(), 3);
    assert_eq!(names("muted", ""), ["notes"]);
    assert_eq!(names("custom", ""), ["acme/dashboard"]);
    assert_eq!(names("all", " ACME "), ["acme/api", "acme/dashboard"]);
    assert_eq!(names("all", "code/dash"), ["acme/dashboard"], "by path");
    assert!(names("muted", "acme").is_empty(), "both must hold");
}

#[gpui::test]
fn providers_draw_one_card_per_agent_and_hide_controls_an_absent_cli_cannot_use(
    cx: &mut TestAppContext,
) {
    use gespan_core::provider::Kind;
    let (window, mut visual) = window(Section::Providers, Preferences::default(), cx);
    let visual = &mut visual;
    // While the check runs, no agent is called installed or not.
    window
        .update(visual, |view, _, cx| {
            view.discovery_loading = true;
            view.discovery.binaries = vec![(Kind::Codex, Some("/bin/codex".into()))];
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(
        visual
            .debug_bounds("settings-agent-installed-codex")
            .is_none()
    );
    window
        .update(visual, |view, _, cx| {
            view.discovery_loading = false;
            view.discovery.binaries = Kind::ALL
                .into_iter()
                .map(|kind| (kind, (kind == Kind::Codex).then(|| "/bin/codex".into())))
                .collect();
            view.ctx.catalogs.insert(
                Kind::Codex.id().into(),
                vec![("gpt-5".into(), "GPT-5".into())],
            );
            view.ctx.catalogs.insert(
                Kind::Claude.id().into(),
                vec![("opus".into(), "Opus".into())],
            );
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("settings-agents-missing").is_some());
    for kind in Kind::ALL {
        let id = kind.id();
        let installed = kind == Kind::Codex;
        // An installed CLI has a full card; the rest a compact one in the
        // Not installed grid, with the command to install it where known.
        assert_eq!(
            visual
                .debug_bounds(format!("settings-agent-{id}").leak())
                .is_some(),
            installed,
            "{id} full card"
        );
        assert_eq!(
            visual
                .debug_bounds(format!("settings-agent-missing-{id}").leak())
                .is_some(),
            !installed,
            "{id} in the Not installed grid"
        );
        if !installed {
            assert_eq!(
                visual
                    .debug_bounds(format!("settings-agent-install-{id}").leak())
                    .is_some(),
                super::pages::install_command(&kind.unavailable_hint()).is_some(),
                "{id} install command"
            );
        }
        let select = visual
            .debug_bounds(format!("settings-select-{id}").leak())
            .is_some();
        let picker = visual
            .debug_bounds(format!("settings-toggle-{id}").leak())
            .is_some();
        // Only the installed CLI offers a model and the picker switch, even
        // where a model catalogue exists for one that isn't installed.
        assert_eq!(select, kind == Kind::Codex, "{id} model select");
        assert_eq!(
            visual
                .debug_bounds(format!("settings-agent-installed-{id}").leak())
                .is_some(),
            kind == Kind::Codex,
            "{id} installed pill"
        );
        assert_eq!(picker, kind == Kind::Codex, "{id} picker toggle");
    }
}

#[test]
fn keybindings_group_by_area_and_split_into_caps() {
    use super::pages::{BINDINGS, by_area, key_caps};
    let areas = by_area(BINDINGS);
    let names: Vec<_> = areas.iter().map(|(a, _)| *a).collect();
    assert_eq!(
        names.first(),
        Some(&"App"),
        "in the order they first appear"
    );
    let total: usize = areas.iter().map(|(_, rows)| rows.len()).sum();
    assert_eq!(total, BINDINGS.len(), "every binding lands in one area");
    for (area, rows) in &areas {
        assert!(
            rows.iter()
                .all(|r| r.command.starts_with(&format!("{area}: ")))
        );
    }
    assert_eq!(key_caps("⌘⇧P"), ["⌘", "⇧", "P"]);
    assert_eq!(key_caps("⌘⇧Space"), ["⌘", "⇧", "Space"]);
    assert_eq!(key_caps("Ctrl+Shift+P"), ["Ctrl", "Shift", "P"]);
    // The plus key itself is a cap, not a separator.
    assert_eq!(key_caps("⌘+"), ["⌘", "+"]);
    assert_eq!(key_caps("Ctrl++"), ["Ctrl", "+"]);
    assert!(key_caps("").is_empty());
}

#[gpui::test]
fn the_keybinding_filter_narrows_areas_and_a_chip_jumps_to_one(cx: &mut TestAppContext) {
    let (window, mut visual) = window(Section::Keybindings, Preferences::default(), cx);
    let visual = &mut visual;
    assert!(
        visual
            .debug_bounds("settings-keybinding-jump-App")
            .is_some()
    );
    assert!(
        visual
            .debug_bounds("settings-keybinding-jump-Terminal")
            .is_some()
    );
    click(visual, "settings-keybinding-jump-Terminal");
    window
        .update(visual, |view, _, _| {
            assert_eq!(
                view.revealed.as_deref(),
                Some("Terminal"),
                "the area flashes"
            );
            assert_eq!(
                view.keybinding_area,
                Some("Terminal"),
                "and its chip stays selected"
            );
        })
        .unwrap();
    window
        .update(visual, |view, _, cx| {
            view.keybinding_filter
                .update(cx, |i, cx| i.set_text("terminal".into(), cx))
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(
        visual
            .debug_bounds("settings-keybinding-jump-Terminal")
            .is_some()
    );
    assert!(
        visual
            .debug_bounds("settings-keybinding-jump-Tab")
            .is_none(),
        "areas with no match drop out"
    );
    assert!(visual.debug_bounds("shortcut-Terminal: New").is_some());
    assert!(visual.debug_bounds("shortcut-App: Settings").is_none());
}

#[gpui::test]
fn dictation_starts_with_the_speech_model_and_has_no_shortcut_card(cx: &mut TestAppContext) {
    let (_, mut visual) = window(Section::Dictation, Preferences::default(), cx);
    let visual = &mut visual;
    // The shortcut lives on Keybindings now.
    assert!(
        visual
            .debug_bounds("settings-row-dictation-shortcut")
            .is_none()
    );
    assert!(visual.debug_bounds("dictation-shortcut-keys").is_none());
    let header = visual.debug_bounds("settings-page-header").unwrap();
    let model = visual.debug_bounds("dictation-model-group").unwrap();
    assert!(header.bottom() <= model.top());
}

#[gpui::test]
fn skills_list_in_project_personal_and_gespan_cards(cx: &mut TestAppContext) {
    let (window, mut visual) = window(Section::Skills, Preferences::default(), cx);
    let visual = &mut visual;
    window
        .update(visual, |view, _, cx| {
            view.discovery.skills = [("review", "project"), ("notes", "user")]
                .map(|(name, scope)| data::Skill {
                    name: name.into(),
                    description: String::new(),
                    path: format!("/skills/{name}/SKILL.md").into(),
                    scope: scope.into(),
                    provider: "agents".into(),
                })
                .to_vec();
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    let project = visual.debug_bounds("settings-skills-project").unwrap();
    let personal = visual.debug_bounds("settings-skills-personal").unwrap();
    assert!(project.bottom() <= personal.top(), "Project first");
    // A kind with no skills shows no empty card.
    assert!(visual.debug_bounds("settings-skills-gespan").is_none());
    assert_eq!(super::pages::skill_bucket("builtin"), 2);
}

#[gpui::test]
fn archive_lists_its_projects_and_conversations_and_deletes_them(cx: &mut TestAppContext) {
    let (window, mut visual) = window(Section::Archive, Preferences::default(), cx);
    let visual = &mut visual;
    window
        .update(visual, |view, _, cx| {
            view.ctx.cwd = "/work/api".into();
            view.ctx.project_label = "api".into();
            view.ctx.archived = vec![ArchivedSession {
                id: 7,
                title: "Old spike".into(),
                provider: "claude".into(),
                updated_at: 1_760_000_000_000,
            }];
            view.ctx.archived_projects = vec![("/work/old".into(), "old".into())];
            cx.notify();
        })
        .unwrap();
    // Drawing both kinds of row once is the regression: Delete once set its
    // hover twice and took the app down.
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("settings-archived-count").is_some());
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let entity = window.root(visual).unwrap();
    let _keep = visual.update(|_, cx| {
        cx.subscribe(&entity, move |_, event: &SettingsEvent, _| {
            log.borrow_mut().push(match event {
                SettingsEvent::Delete(id) => format!("delete:{id}"),
                SettingsEvent::DeleteProject(path) => format!("delete-project:{path}"),
                _ => "other".into(),
            })
        })
    });
    click(visual, "settings-delete-7");
    click(visual, "settings-delete-project-0");
    assert_eq!(*seen.borrow(), ["delete:7", "delete-project:/work/old"]);
}

#[gpui::test]
fn the_mcp_registry_lists_results_as_cards_even_while_providers_load(cx: &mut TestAppContext) {
    use gespan_core::mcp_registry::{Install, Listing};
    let (window, mut visual) = window(Section::Mcp, Preferences::default(), cx);
    let visual = &mut visual;
    window
        .update(visual, |view, _, cx| {
            view.discovery_loading = true;
            view.mcp_listings = [
                "postgres-tools",
                "sqlite-explorer",
                "mysql-mcp",
                "duckdb-local",
            ]
            .map(|name| Listing {
                name: name.into(),
                description: format!("{name} for your data"),
                version: "1.0.0".into(),
                install: Install::Command {
                    command: "npx".into(),
                    args: vec![name.into()],
                },
                inputs: Vec::new(),
            })
            .to_vec();
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(
        visual
            .debug_bounds("settings-catalog-registry-results")
            .is_some()
    );
    let first = visual.debug_bounds("settings-catalog-listing-0").unwrap();
    let second = visual.debug_bounds("settings-catalog-listing-1").unwrap();
    let fourth = visual.debug_bounds("settings-catalog-listing-3").unwrap();
    // Three to a row: the second sits beside the first, the fourth below it.
    assert_eq!(first.top(), second.top());
    assert!(second.left() > first.right());
    assert!(fourth.top() > first.bottom());
    assert!(visual.debug_bounds("settings-catalog-add-3").is_some());
    // The managed switch now sits in the Gespan servers card with a title.
    assert!(
        visual
            .debug_bounds("settings-row-catalog-managed")
            .is_some()
    );
}

#[test]
fn an_install_hint_yields_its_command() {
    use super::pages::install_command;
    assert_eq!(
        install_command(
            "Grok Build CLI not found (`curl -fsSL https://x.ai/cli/install.sh | bash`). Install it."
        ),
        Some("curl -fsSL https://x.ai/cli/install.sh | bash")
    );
    assert_eq!(
        install_command("Antigravity ACP server not found. Install it."),
        None
    );
    assert_eq!(install_command("broken `unterminated"), None);
    assert_eq!(install_command("empty ``"), None);
    // Guidance in backticks stays prose.
    assert_eq!(
        install_command("Hermes not found (`Install from example.com, then run hermes model`)."),
        None
    );
}

#[gpui::test]
fn accept_connections_shows_once_a_phone_is_paired_and_reachable_only_while_listening(
    cx: &mut TestAppContext,
) {
    let (window, mut visual) = window(Section::Devices, Preferences::default(), cx);
    let visual = &mut visual;
    let set = |visual: &mut VisualTestContext, phones: usize, listening: bool| {
        window
            .update(visual, |view, _, cx| {
                view.ctx.mobile = Some(gespan_core::mobile::Status {
                    enabled: true,
                    listening,
                    name: "desk".into(),
                    fingerprint: None,
                    addresses: Vec::new(),
                    devices: (0..phones)
                        .map(|ix| gespan_core::mobile::Device {
                            id: format!("p{ix}"),
                            name: format!("Phone {ix}"),
                            fingerprint: String::new(),
                        })
                        .collect(),
                    open_router: false,
                    remote_address: String::new(),
                    router_status: String::new(),
                    error: None,
                });
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        (
            visual.debug_bounds("mobile-access-card").is_some(),
            visual.debug_bounds("mobile-reachable").is_some(),
        )
    };
    // Nothing paired: there is nothing to accept yet.
    assert_eq!(set(visual, 0, true), (false, false));
    assert_eq!(set(visual, 1, true), (true, true));
    // Paired but not listening: the switch stays, Reachable doesn't.
    assert_eq!(set(visual, 1, false), (true, false));
}

#[gpui::test]
fn diagnostics_kind_chips_hug_their_text_and_a_panic_is_red(cx: &mut TestAppContext) {
    use super::pages::kind_tone;
    use crate::glass::Tone;
    assert_eq!(kind_tone("panic"), Tone::Bad);
    assert_eq!(kind_tone("timeout"), Tone::Muted);
    let (window, mut visual) = window(Section::Diagnostics, Preferences::default(), cx);
    let visual = &mut visual;
    window
        .update(visual, |view, _, cx| {
            view.diagnostics.counters = Some(Ok(vec![
                super::pages::CounterRow {
                    name: "git.command".into(),
                    kind: "exit_status",
                    today: 3,
                    week: 9,
                },
                super::pages::CounterRow {
                    name: "panic".into(),
                    kind: "panic",
                    today: 0,
                    week: 1,
                },
            ]));
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    let long = visual
        .debug_bounds("settings-counter-kind-exit_status")
        .unwrap();
    let short = visual.debug_bounds("settings-counter-kind-panic").unwrap();
    // Each chip is as wide as its word, not the column.
    assert!(short.size.width < long.size.width);
    assert!(long.size.width < px(120.));
    // The report tile sits centred against its card's text.
    assert!(visual.debug_bounds("settings-report-tile").is_some());
    assert!(
        visual.debug_bounds("settings-counters-today").is_some(),
        "3 today"
    );
}

#[test]
fn the_settings_rail_tints_each_page_as_designed() {
    let dark = |s: Section| s.tint().map(|(d, _)| d);
    assert_eq!(dark(Section::Appearance), Some(0xf9a8d4));
    assert_eq!(dark(Section::Services), Some(0xa5b4fc));
    assert_eq!(dark(Section::Devices), Some(0x5eead4));
    assert_eq!(dark(Section::Chat), Some(0xc0affc));
    assert_eq!(dark(Section::Providers), Some(0xd97757));
    assert_eq!(dark(Section::Browser), Some(0x7dd3fc));
    assert_eq!(dark(Section::Skills), Some(0xe8c547));
    assert_eq!(dark(Section::Inbox), Some(0xa5b4fc));
    assert_eq!(dark(Section::Worktrees), Some(0x6ee7b7));
    // The rest stay grey, as in the design.
    for quiet in [
        Section::General,
        Section::Keybindings,
        Section::Dictation,
        Section::Diagnostics,
        Section::Mcp,
        Section::Archive,
    ] {
        assert_eq!(quiet.tint(), None, "{quiet:?}");
    }
}

#[test]
fn a_gradient_backdrop_paints_only_behind_a_see_through_window() {
    use super::backdrop_colors;
    let mut p = Preferences {
        backdrop: "gradient".into(),
        backdrop_gradient: "dusk".into(),
        ..Default::default()
    };
    assert_eq!(backdrop_colors(&p, true), Some((0xf2a6ba, 0x8f64f6)));
    // An opaque window (light theme, Transparent window off) keeps its look.
    assert_eq!(backdrop_colors(&p, false), None);
    p.backdrop_gradient = "gone".into();
    assert_eq!(
        backdrop_colors(&p, true),
        Some((0x8f64f6, 0x459bf7)),
        "falls back to Aurora"
    );
    p.backdrop = "desktop".into();
    assert_eq!(backdrop_colors(&p, true), None);
}

#[gpui::test]
fn picking_a_backdrop_gradient_saves_it_and_desktop_switches_back(cx: &mut TestAppContext) {
    let prefs = Preferences {
        theme: "dark".into(),
        inset_glass: true,
        ..Default::default()
    };
    let (window, mut visual) = window(Section::Appearance, prefs, cx);
    let visual = &mut visual;
    click(visual, "settings-backdrop-dune");
    let saved = super::glass_tests::prefs(window, visual);
    assert_eq!(
        (saved.backdrop.as_str(), saved.backdrop_gradient.as_str()),
        ("gradient", "dune")
    );
    click(visual, "settings-backdrop-desktop");
    let saved = super::glass_tests::prefs(window, visual);
    assert_eq!(saved.backdrop, "desktop");
    assert_eq!(
        saved.backdrop_gradient, "dune",
        "the gradient is remembered"
    );
    // Restore defaults puts the desktop back.
    window
        .update(visual, |view, _, cx| {
            view.update(cx, |p| p.backdrop = "gradient".into());
            view.restore_defaults(cx)
        })
        .unwrap();
    assert_eq!(
        super::glass_tests::prefs(window, visual).backdrop,
        "desktop"
    );
}

#[gpui::test]
fn chat_background_controls_write_their_preferences(cx: &mut TestAppContext) {
    let (window, mut visual) = window(Section::Appearance, Preferences::default(), cx);
    let visual = &mut visual;
    // Every field shows before an image is picked.
    for selector in [
        "settings-segmented-background-effect",
        "settings-segmented-background-scope",
        "settings-slider-background-empty",
        "settings-slider-background-session",
    ] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    click(visual, "settings-background-effect-halftone");
    assert_eq!(prefs(window, visual).chat_background_effect, "halftone");
    click(visual, "settings-background-scope-all");
    assert_eq!(prefs(window, visual).chat_background_scope, "all");
    let track = visual
        .debug_bounds("settings-slider-background-empty")
        .unwrap();
    let (min, max) = SliderKind::BackgroundEmpty.range();
    visual.simulate_click(
        point(track.right() - px(8.), track.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert_eq!(prefs(window, visual).chat_background_empty, max);
    visual.simulate_click(
        point(track.left() + px(8.), track.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert_eq!(prefs(window, visual).chat_background_empty, min);
    // With an image, Remove clears it.
    let image = tempfile::NamedTempFile::new().unwrap();
    window
        .update(visual, |view, _, cx| {
            view.prefs.chat_background = Some(image.path().display().to_string());
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    click(visual, "settings-remove-chat-background");
    assert_eq!(prefs(window, visual).chat_background, None);
}

#[gpui::test]
fn devices_show_machine_states_the_remove_card_and_away_from_home_in_its_card(
    cx: &mut TestAppContext,
) {
    use gespan_core::remote::{Machine, SshTarget};
    let (window, mut visual) = window(Section::Devices, Preferences::default(), cx);
    let visual = &mut visual;
    let machine = |id: &str, name: &str| Machine {
        id: id.into(),
        name: name.into(),
        endpoint: String::new(),
        environment_id: String::new(),
        ssh: Some(SshTarget {
            target: format!("me@{id}"),
            port: None,
            remote_port: 3774,
        }),
    };
    window
        .update(visual, |view, _, cx| {
            view.seed_machines_for_test(
                vec![
                    machine("mini", "Home Mac mini"),
                    machine("box", "Build box"),
                ],
                &[
                    ("mini", "Connected"),
                    (
                        "box",
                        "Connected · host update needed for Explorer and Changes",
                    ),
                ],
                Some("box"),
            );
            view.ctx.mobile = Some(gespan_core::mobile::Status {
                enabled: true,
                listening: true,
                name: "desk".into(),
                fingerprint: Some("7f3a9c21d4e5".into()),
                addresses: Vec::new(),
                devices: Vec::new(),
                open_router: true,
                remote_address: String::new(),
                router_status: String::new(),
                error: None,
            });
            view.open_away_for_test();
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    // One pill per machine, matching what its host reported.
    assert!(visual.debug_bounds("remote-remove-confirmation").is_some());
    let card = visual.debug_bounds("mobile-away-card").unwrap();
    let details = visual.debug_bounds("mobile-away-details").unwrap();
    assert!(
        card.contains(&details.center()),
        "the details open inside the card"
    );
    let router = visual
        .debug_bounds("settings-toggle-mobile-open-router")
        .unwrap();
    assert!(
        router.left() > card.center().x,
        "the router switch sits on the right"
    );
}
