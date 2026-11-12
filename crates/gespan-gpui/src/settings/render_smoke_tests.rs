//! Render smoke tests: every Settings page drawn empty and with realistic
//! data, at a wide and a narrow width, with the pointer over the rows the
//! data adds. A page that panics while it builds or paints fails here instead
//! of taking the app down (Settings › Archive's Delete once set its hover
//! twice and crashed on open).
//!
//! Adding a page: add a line to `pages!` below. `every_page_file_has_a_smoke_entry`
//! fails until the new `pages/*.rs` file has one.
use super::*;
use gpui::{
    Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions, point,
    size,
};

/// Fills a view with the data a page draws rows for.
type Seed = fn(&mut SettingsView, &mut Context<SettingsView>);

/// One smoke entry: the page file, the section that draws it, how to seed
/// it, and selectors the seeded page must draw (proof the seed reached the
/// rows that matter).
struct Page {
    section: Section,
    seed: Seed,
    seeded: &'static [&'static str],
}

macro_rules! pages {
    ($($file:ident => $section:ident, $seed:ident, [$($selector:literal),* $(,)?];)*) => {
        /// The `pages/*.rs` files that have a smoke entry.
        const COVERED: &[&str] = &[$(stringify!($file)),*];
        /// Every Section a smoke entry draws.
        const SECTIONS: &[Section] = &[$(Section::$section),*];
        mod smoke {
            use super::*;
            $(
                #[gpui::test]
                fn $file(cx: &mut TestAppContext) {
                    smoke(
                        stringify!($file),
                        Page {
                            section: Section::$section,
                            seed: $seed,
                            seeded: &[$($selector),*],
                        },
                        cx,
                    );
                }
            )*
        }
    };
}

pages! {
    appearance => Appearance, seed_appearance, [];
    archive => Archive, seed_archive, [
        "settings-delete-7",
        "settings-delete-8",
        "settings-delete-project-0",
        "settings-delete-project-1",
    ];
    browser => Browser, seed_nothing, [];
    chat => Chat, seed_nothing, [];
    connections => Devices, seed_connections, [];
    diagnostics => Diagnostics, seed_diagnostics, [];
    dictation => Dictation, seed_nothing, [];
    general => General, seed_general, [];
    herdr_machines => Devices, seed_herdr_machines, ["herdr-machine-m1", "herdr-machine-m2"];
    inbox => Inbox, seed_inbox, [];
    keybindings => Keybindings, seed_keybindings, [];
    mcp => Mcp, seed_mcp, [];
    mcp_catalog => Mcp, seed_mcp_catalog, ["settings-catalog-add-0"];
    mobile => Devices, seed_mobile, [];
    providers => Providers, seed_providers, [];
    services => Services, seed_services, [];
    skills => Skills, seed_skills, [];
    worktrees => Worktrees, seed_worktrees, [];
}

/// Widths the pages draw at: the default window and a narrow one that wraps
/// and truncates rows.
const WIDTHS: [f32; 2] = [1200., 560.];

fn open(section: Section, cx: &mut TestAppContext) -> WindowHandle<SettingsView> {
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(WIDTHS[0]), px(3200.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    let mut view = SettingsView::new(
                        section,
                        Preferences {
                            notifications: false,
                            ..Default::default()
                        },
                        SettingsContext::default(),
                        cx,
                    );
                    // The real discovery reads this machine's CLIs, skills
                    // and Git; the seed stands in for it.
                    view.discovery_task = None;
                    view.discovery_loading = false;
                    view
                })
            },
        )
        .expect("the settings window opens")
    });
    cx.refresh().expect("the first frame draws");
    cx.run_until_parked();
    window
}

fn draw(visual: &mut VisualTestContext, width: f32) {
    visual.simulate_resize(size(px(width), px(3200.)));
    visual.run_until_parked();
    visual.refresh().expect("the page draws");
}

fn smoke(file: &str, page: Page, cx: &mut TestAppContext) {
    // Empty: the page as a fresh install shows it.
    let window = open(page.section, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    for width in WIDTHS {
        draw(visual, width);
    }

    // Seeded: the rows only data brings, hovered, since hover styles are
    // built while drawing.
    let window = open(page.section, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let seed = page.seed;
    window
        .update(visual, |view, _, cx| {
            seed(view, cx);
            cx.notify();
        })
        .expect("the seed applies");
    for width in WIDTHS {
        draw(visual, width);
        for selector in page.seeded {
            let bounds = visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{file}: the seeded page draws {selector}"));
            visual.simulate_mouse_move(bounds.center(), None, Default::default());
            visual.run_until_parked();
            visual.refresh().expect("the hovered page draws");
        }
    }
}

#[test]
fn every_page_file_has_a_smoke_entry() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/settings/pages");
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .expect("the pages folder lists")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "rs")
                .then(|| path.file_stem()?.to_str().map(str::to_owned))
                .flatten()
        })
        .filter(|stem| stem != "mod")
        .collect();
    files.sort();
    let mut covered: Vec<&str> = COVERED.to_vec();
    covered.sort_unstable();
    assert_eq!(
        files, covered,
        "each settings/pages/*.rs needs a line in render_smoke_tests.rs's pages! table"
    );
}

#[test]
fn every_section_in_the_rail_is_drawn() {
    for (group, sections) in model::GROUPS {
        for section in sections {
            assert!(
                SECTIONS.contains(section),
                "{group} › {section:?} has no smoke entry"
            );
        }
    }
}

fn seed_nothing(_: &mut SettingsView, _: &mut Context<SettingsView>) {}

fn seed_appearance(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    view.ctx.new_layout_boxy = true;
    view.ctx.sidebar = SidebarVisibility {
        automations: true,
        inbox: true,
        compact: true,
    };
    view.prefs.theme = "dark".into();
    view.accent_custom_open = true;
}

fn seed_archive(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    view.ctx.cwd = "/work/api".into();
    view.ctx.project_label = "api".into();
    view.ctx.archived = vec![
        ArchivedSession {
            id: 7,
            title: "Old spike".into(),
            provider: "claude".into(),
            updated_at: 1_760_000_000_000,
        },
        ArchivedSession {
            id: 8,
            title: "A title long enough to truncate in a narrow window, and then some more".into(),
            provider: "codex".into(),
            // No date: the row draws without one.
            updated_at: 0,
        },
    ];
    view.ctx.archived_projects = vec![
        ("/work/old".into(), "old".into()),
        ("/work/older".into(), "older".into()),
    ];
}

fn seed_connections(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    use gespan_core::remote::{Machine, SshTarget};
    view.connections.seed_for_test(
        vec![
            Machine {
                id: "studio".into(),
                name: "Studio".into(),
                endpoint: "http://127.0.0.1:47000".into(),
                environment_id: "env-studio".into(),
                ssh: Some(SshTarget {
                    target: "me@studio".into(),
                    port: Some(2222),
                    remote_port: 47000,
                }),
            },
            Machine {
                id: "cloud".into(),
                name: "Cloud box".into(),
                endpoint: "https://cloud.example".into(),
                environment_id: "env-cloud".into(),
                ssh: None,
            },
        ],
        "Connected",
        "cloud",
    );
}

fn seed_diagnostics(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    view.diagnostics.counters = Some(Ok(vec![
        pages::CounterRow {
            name: "provider.launch".into(),
            kind: "timeout",
            today: 2,
            week: 9,
        },
        pages::CounterRow {
            name: "git.snapshot".into(),
            kind: "io",
            today: 0,
            week: 1,
        },
    ]));
}

fn seed_general(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    view.ctx.quick_error = Some("The quick composer shortcut is taken by another app.".into());
}

fn seed_herdr_machines(_: &mut SettingsView, cx: &mut Context<SettingsView>) {
    use gespan_core::machines::{HERDR_VERSION, Machine, MachineView, Online, Snapshot};
    let machine = |id: &str, online: Online, version: Option<&str>, enabled: bool| {
        let needs_install = matches!(online, Online::HerdrMissing | Online::HerdrTooOld);
        MachineView {
            machine: Machine {
                id: id.into(),
                label: format!("{id} box"),
                target: format!("me@{id}"),
                session: "default".into(),
                enabled,
            },
            version_mismatch: version.filter(|v| *v != HERDR_VERSION).map(str::to_owned),
            needs_install,
            snapshot: Snapshot {
                online,
                version: version.map(str::to_owned),
                ..Default::default()
            },
        }
    };
    crate::machines::show_for_test(cx);
    crate::machines::install_for_test(
        vec![
            machine("m1", Online::Online, Some(HERDR_VERSION), true),
            machine("m2", Online::HerdrTooOld, Some("0.0.1"), true),
            machine("m3", Online::HerdrMissing, None, false),
        ],
        cx,
    );
}

fn seed_inbox(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    view.ctx.projects = vec![
        ("/work/api".into(), "api".into()),
        ("/work/web".into(), "web".into()),
    ];
    view.ctx.linear_teams = vec![(
        gespan_core::inbox::OptionRow {
            id: "team-1".into(),
            name: "Platform".into(),
            key: "PLT".into(),
        },
        true,
    )];
}

fn seed_keybindings(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    view.recording = Some("quick-composer".into());
    view.recording_preview = "⌘⇧".into();
    view.recording_error = Some("Already used by New chat".into());
}

fn seed_mcp(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    view.selected_project = "/work/api".into();
    view.mcp_show_all = true;
    view.discovery.mcp = ["claude", "codex", "gemini"]
        .into_iter()
        .enumerate()
        .map(|(ix, provider)| data::McpServer {
            name: format!("server-{ix}"),
            provider: provider.into(),
            scope: if ix == 0 { "project" } else { "user" }.into(),
            path: format!("/work/api/.{provider}/mcp.json").into(),
            transport: if ix == 1 { "http" } else { "stdio" }.into(),
            status: if ix == 2 { "Disabled" } else { "Configured" }.into(),
        })
        .collect();
}

fn seed_mcp_catalog(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    use gespan_core::mcp_registry::{Input, Install, Listing};
    view.mcp_catalog = gespan_core::mcp_catalog::Catalog {
        entries: vec![
            gespan_core::mcp_catalog::Entry {
                id: "linear".into(),
                name: "Linear".into(),
                url: Some("https://mcp.linear.app/mcp".into()),
                enabled: true,
                ..Default::default()
            },
            gespan_core::mcp_catalog::Entry {
                id: "search".into(),
                name: "Search".into(),
                command: "npx".into(),
                args: vec!["-y".into(), "acme@1.2.0".into()],
                env: vec!["API_KEY".into()],
                required: vec!["API_KEY".into()],
                needed: vec!["API_KEY".into()],
                source: Some("~/.claude.json".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    view.mcp_catalog_loaded = true;
    view.mcp_catalog_error = Some("Couldn't reach the registry.".into());
    view.mcp_listings = vec![Listing {
        name: "com.acme/search".into(),
        description: "Search the web".into(),
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
}

fn seed_mobile(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    use gespan_core::mobile::{Device, Status};
    view.ctx.mobile = Some(Status {
        enabled: true,
        listening: true,
        name: "Studio".into(),
        fingerprint: Some("AB:CD:EF".into()),
        devices: vec![
            Device {
                id: "phone-1".into(),
                name: "Pixel".into(),
                fingerprint: "11:22".into(),
            },
            Device {
                id: "phone-2".into(),
                name: "iPhone".into(),
                fingerprint: "33:44".into(),
            },
        ],
        open_router: true,
        remote_address: "studio.example:47100".into(),
        router_status: "Forwarded".into(),
        ..Default::default()
    });
}

fn seed_providers(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    use gespan_core::provider::Kind;
    view.ctx.projects = vec![("/work/api".into(), "api".into())];
    // Half installed, half missing.
    view.discovery.binaries = Kind::ALL
        .into_iter()
        .enumerate()
        .map(|(ix, kind)| {
            (
                kind,
                (ix % 2 == 0).then(|| format!("/usr/local/bin/{}", kind.id()).into()),
            )
        })
        .collect();
}

fn seed_services(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    view.ctx.inbox_connections = [
        ("github".to_owned(), (true, "octocat".to_owned())),
        ("linear".to_owned(), (false, String::new())),
    ]
    .into_iter()
    .collect();
    view.ctx.linear_teams = vec![(
        gespan_core::inbox::OptionRow {
            id: "team-1".into(),
            name: "Platform".into(),
            key: "PLT".into(),
        },
        false,
    )];
    view.services_picker = true;
}

fn seed_skills(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    view.selected_project = "/work/api".into();
    view.discovery.skills = [
        ("review", "project", "claude"),
        ("notes", "user", "codex"),
        ("plan", "builtin", "gespan"),
    ]
    .map(|(name, scope, provider)| data::Skill {
        name: name.into(),
        description: format!("The {name} skill"),
        path: format!("/work/api/.claude/skills/{name}/SKILL.md").into(),
        scope: scope.into(),
        provider: provider.into(),
    })
    .into();
}

fn seed_worktrees(view: &mut SettingsView, _: &mut Context<SettingsView>) {
    let tree = |name: &str, locked: bool, dirty: Option<bool>, unpushed: usize| data::Tree {
        tree: gespan_core::git::Worktree {
            path: format!("/work/api-worktrees/{name}"),
            branch: Some(name.into()),
            ..Default::default()
        },
        locked,
        dirty,
        unpushed,
        merged: None,
        setup: None,
    };
    view.ctx.projects = vec![("/work/api".into(), "api".into())];
    view.selected_project = "/work/api".into();
    view.discovery.worktrees = data::ResultTrees {
        trees: vec![
            tree("feature", false, Some(false), 0),
            tree("dirty", false, Some(true), 3),
            tree("locked", true, None, 0),
        ],
        branches: Vec::new(),
        default_root: "/work/api-worktrees".into(),
        main: "/work/api".into(),
        error: None,
    };
}
