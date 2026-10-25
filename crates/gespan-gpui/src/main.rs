#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod accounts;
mod activity;
mod arcade;
mod assets;
mod automations;
mod board;
mod changes;
mod composer_completion;
mod composer_motion;
mod composer_runner;
mod context_meter;
mod conversation;
#[cfg(debug_assertions)]
mod dev_reload;
mod diff_view;
mod editor_analysis;
mod editor_scrollbar;
mod editor_syntax;
mod explorer;
mod file_editor;
mod file_icons;
mod fixtures;
mod flows;
mod fonts;
mod fs_watch;
mod git_cache;
mod git_graph;
mod git_picker;
mod git_status;
mod history;
mod image_lightbox;
mod image_memory;
mod inbox;
mod input;
#[cfg(test)]
mod log_capture;
mod markdown;
mod markdown_model;
#[cfg(target_os = "macos")]
mod memory_log;
#[cfg(all(test, target_os = "macos"))]
mod metal_tests;
mod mobile;
mod modal;
mod notes;
mod notifications;
mod pane_layout;
mod pane_motion;
mod particle_text;
mod pickers;
mod pilots;
mod popover;
mod project_background;
mod project_search;
mod project_settings;
mod project_tabs;
mod pulse;
mod questions;
mod quick_composer;
mod quick_open;
mod reflow_motion;
mod report;
mod resource_pressure;
mod response_actions;
mod reveal;
mod review;
mod right_panel;
mod runtime;
mod search;
mod session_mark;
mod session_review;
mod settings;
mod shell;
mod sound;
mod syntax;
mod tab_label;
mod tab_motion;
mod terminal;
mod theme;
mod tickets;
#[cfg(target_os = "macos")]
mod traffic_lights;
mod transcript;
mod usage_footer;
mod visuals;
mod workspace;
mod worktree_switcher;

use gespan_core::{store::Store, transcript::Transcript};
use gpui::{
    App, Bounds, KeyBinding, TitlebarOptions, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, actions, point, prelude::*, px, size,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};
/// libmalloc keeps freed large spans dirty, so startup decodes and image churn
/// stayed in the footprint. mimalloc 2 returns them; 3 retains churn too.
#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

actions!(application, [Quit]);
fn quit(cx: &mut App) {
    if notifications::updater::defer_quit(cx) {
        return;
    }
    let shutdown = cx
        .background_executor()
        .spawn(async { gespan_core::process::shutdown_all() });
    cx.spawn(async move |cx| {
        shutdown.await;
        cx.update(|cx| cx.quit());
    })
    .detach();
}

fn data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("GESPAN_NATIVE_DATA_DIR") {
        return path.into();
    }
    #[cfg(target_os = "macos")]
    let base = PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support");
    #[cfg(windows)]
    let base = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default());
    #[cfg(not(any(target_os = "macos", windows)))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        });
    base.join("Gespan Development")
}
// The packaged app runs this binary directly: a launcher that execs it changes the
// process's pid version, and macOS then refuses its menu bar item. The bundle
// records its data mode in Info.plist instead of passing a flag.
fn bundle_uses_gespan_data() -> Option<bool> {
    let exe = std::env::current_exe().ok()?;
    let plist = std::fs::read_to_string(exe.parent()?.parent()?.join("Info.plist")).ok()?;
    plist_data_mode(&plist)
}
fn plist_data_mode(plist: &str) -> Option<bool> {
    let rest = plist.split("<key>GespanDataMode</key>").nth(1)?;
    let mode = rest
        .trim_start()
        .strip_prefix("<string>")?
        .split('<')
        .next()?;
    match mode {
        "gespan-data" => Some(true),
        "dev-data" => Some(false),
        _ => None,
    }
}
/// Development data logs apart from release data, like its database.
fn diagnostics_config(use_gespan: bool) -> gespan_core::diagnostics::Config {
    use gespan_core::diagnostics::{Channel, Config, log_dir};
    let channel = if use_gespan {
        Channel::Release
    } else {
        Channel::Dev
    };
    Config {
        log_dir: log_dir(channel),
        version: notifications::updater::version().into(),
        channel,
    }
}
/// Every quit reaches GPUI's shutdown, whether it starts from Quit, the
/// menu-bar extra, the Dock or the OS; `run` never returns on macOS, so the
/// diagnostics guard is not dropped there.
fn flush_logs_on_quit(cx: &mut App, flush: fn()) {
    cx.on_app_quit(move |_| {
        flush();
        async {}
    })
    .detach();
}
/// `std::process::exit` skips destructors, so buffered log lines are flushed
/// first.
fn exit(code: i32) -> ! {
    gespan_core::diagnostics::flush();
    std::process::exit(code)
}
/// Whether a fatal startup error also needs a stderr copy: the Dev channel's
/// subscriber already prints readable lines there, Release logs only to file.
fn echo_fatal(logging: bool, channel: gespan_core::diagnostics::Channel) -> bool {
    !logging || channel != gespan_core::diagnostics::Channel::Dev
}
/// Logs a startup error that ends the process, and copies it to a terminal
/// when logging isn't already printing it there.
fn fatal(echo: bool, code: i32, message: &str) -> ! {
    tracing::error!("{message}");
    if echo {
        eprintln!("{message}");
    }
    exit(code)
}
fn main() {
    if let Some(code) = gespan_core::remote::ssh_askpass::maybe_run() {
        exit(code);
    }
    let cli_args: Vec<_> = std::env::args().skip(1).collect();
    if cli_args
        .first()
        .is_some_and(|arg| arg == "--version" || arg == "-V")
    {
        println!("Gespan Native {}", notifications::updater::version());
        return;
    }
    if cli_args.first().is_some_and(|a| a == "visual-mcp") {
        if let Err(error) = gespan_core::visual_bridge::mcp_stdio() {
            eprintln!("{error}");
            exit(1);
        }
        return;
    }
    if cli_args.first().is_some_and(|a| a == "visual") {
        match gespan_core::visual_bridge::client(&cli_args[1..]) {
            Ok(value) => {
                println!("{value}");
                if value.get("ok") == Some(&serde_json::Value::Bool(false)) {
                    exit(1);
                }
            }
            Err(error) => {
                println!("{}", serde_json::json!({"ok":false,"error":error}));
                exit(1);
            }
        }
        return;
    }
    if cli_args.first().is_some_and(|a| a == "app") {
        match gespan_core::pilot::bridge::client(&cli_args[1..]) {
            Ok(value) => {
                println!("{value}");
                if value.get("ok") == Some(&serde_json::Value::Bool(false)) {
                    exit(1);
                }
            }
            Err(error) => {
                println!("{}", serde_json::json!({"ok":false,"error":error}));
                exit(1);
            }
        }
        return;
    }
    let mut args = std::env::args().skip(1);
    let mut fixture = false;
    let mut chat_fixture = false;
    let mut visual_fixture = false;
    // Development stays separate; release builds use the installed app's data.
    let mut use_gespan = bundle_uses_gespan_data().unwrap_or(!cfg!(debug_assertions));
    let mut gespan_db = gespan_core::store::gespan_database();
    let mut check_data = false;
    let mut explicit_cwd = false;
    let mut cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fixture" => fixture = true,
            "--chat-fixture" => chat_fixture = true,
            "--visual-fixture" => visual_fixture = true,
            "--dev-data" => use_gespan = false,
            "--gespan-data" => use_gespan = true,
            "--gespan-db" => {
                use_gespan = true;
                gespan_db = PathBuf::from(args.next().expect("--gespan-db needs a path"));
            }
            "--check-data" => check_data = true,
            "--cwd" => {
                explicit_cwd = true;
                cwd = PathBuf::from(args.next().expect("--cwd needs a path"));
            }
            "--help" | "-h" => {
                println!(
                    "gespan-gpui [--cwd PATH] [--dev-data | --gespan-data | --gespan-db PATH]\nDevelopment builds use a separate database; release builds use the installed Gespan database.\n--dev-data opens an isolated development database using Gespan's existing schema.\n--gespan-data opens the existing desktop gespan.db directly.\n--gespan-db PATH opens a specific desktop database.\n--fixture, --chat-fixture and --visual-fixture add local samples to isolated development data only.\nvisual docs prints the native visual SDK reference.\nvisual check/show --input FILE validates or publishes native session visuals.\nvisual-mcp runs the session visual tool server over standard input/output.\n--check-data validates and lists the selected store read-only, then exits."
                );
                return;
            }
            _ => {
                eprintln!("Unknown option: {arg}");
                exit(2);
            }
        }
    }
    if use_gespan && (fixture || chat_fixture || visual_fixture) {
        eprintln!("Fixtures require --dev-data; they cannot be written into your Gespan database.");
        exit(2);
    }
    // Kept for the life of the process; exit paths flush through `exit`.
    let config = diagnostics_config(use_gespan);
    let channel = config.channel;
    let diagnostics = gespan_core::diagnostics::init(config)
        .inspect_err(|error| eprintln!("{error}"))
        .ok();
    let echo = echo_fatal(diagnostics.is_some(), channel);
    let cwd = cwd
        .canonicalize()
        .unwrap_or(cwd)
        .to_string_lossy()
        .to_string();
    let database = if use_gespan {
        gespan_db
    } else {
        data_dir().join("gespan.db")
    };
    if !use_gespan
        && database
            .canonicalize()
            .ok()
            .zip(gespan_core::store::gespan_database().canonicalize().ok())
            .is_some_and(|(development, desktop)| development == desktop)
    {
        fatal(
            echo,
            2,
            "The isolated development directory points at the desktop database. Choose a separate GESPAN_NATIVE_DATA_DIR.",
        );
    }
    let mut db = if check_data {
        Store::read_gespan(&database)
    } else {
        Store::open_gespan(&database)
    }
    .unwrap_or_else(|error| {
        fatal(
            echo,
            1,
            &format!("Cannot open {}: {error}", database.display()),
        )
    });
    if check_data {
        match db.sessions() {
            Ok(sessions) => eprintln!(
                "Database: {}\n{} saved sessions; existing Gespan schema; no import or migration.",
                database.display(),
                sessions.len()
            ),
            Err(error) => fatal(echo, 1, &error),
        }
        return;
    }
    // Logs its own error; the app runs without counters if the file is unusable.
    let _ = gespan_core::diagnostics::metrics::open(
        &gespan_core::diagnostics::metrics::database_path(&database),
        notifications::updater::version(),
    );
    gespan_core::checkpoint::init(
        database
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("checkpoints"),
    );
    let saved = db.ui_state().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "cannot read workspace settings");
        None
    });
    let existing_project = if use_gespan {
        saved
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|state| {
                state["project"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            })
            .or_else(|| {
                db.sessions()
                    .ok()
                    .and_then(|sessions| sessions.into_iter().find(|s| !s.archived).map(|s| s.cwd))
            })
    } else {
        None
    };
    // Opened from the Dock or Finder, macOS starts the app in `/`. Resume the
    // saved project instead of deriving a project from an internal chat path.
    let cwd = if let Some(project) = existing_project.filter(|_| !explicit_cwd) {
        project
    } else if explicit_cwd || std::path::Path::new(&cwd).parent().is_some() {
        cwd
    } else {
        db.sessions()
            .ok()
            .and_then(|sessions| sessions.into_iter().map(|s| s.cwd).find(|c| c != &cwd))
            .unwrap_or_else(|| std::env::var("HOME").unwrap_or(cwd))
    };
    if fixture {
        let s = db.create("codex", &cwd).expect("create fixture");
        db.title(s.id, "Scroll lab · 10,000 messages").unwrap();
        let mut t = Transcript::default();
        let started = Instant::now();
        for i in 0..10_000 {
            t.begin(if i % 2 == 0 { "user" } else { "assistant" });
            t.append(&format!("Message {i:05} · Native transcript stress fixture.\nUnicode stays intact: 🙂 漢字 café. Scroll up while another session streams.\nThis message is local fixture data; sending resumes a new real provider conversation."));
        }
        db.write_rows(s.id, 0, &t.rows).unwrap();
        tracing::info!(
            rows = t.rows.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "seeded the transcript fixture"
        );
    }
    if chat_fixture {
        let s = db.create("codex", &cwd).expect("create chat fixture");
        db.title(s.id, "Native chat · Markdown and tools").unwrap();
        db.write_rows(s.id, 0, &fixtures::chat().rows).unwrap();
    }
    if visual_fixture {
        let s = db.create("codex", &cwd).expect("create visual fixture");
        db.title(s.id, "Native visuals · Controls, charts and 3D")
            .unwrap();
        db.write_rows(s.id, 0, &fixtures::visuals().rows).unwrap();
    }
    let mut sessions = db.sessions().expect("read native sessions");
    if sessions.is_empty() {
        sessions.push(db.create("codex", &cwd).expect("create initial session"));
    }
    // Gather compact sidebar state before entering the platform event loop.
    // The shell does not query thousands of saved agents on the UI thread.
    let metadata = sessions
        .iter()
        .map(|s| (s.id, s.cwd.clone()))
        .collect::<Vec<_>>();
    let workspace = workspace::Workspace::restore(saved.as_deref(), &metadata, &cwd);
    let mut projects = workspace.projects;
    if !projects.contains(&workspace.project) {
        projects.push(workspace.project);
    }
    let sidebar_metadata = db.session_sidebar_metadata(&projects);
    let db = Arc::new(Mutex::new(db));
    let platform = gpui_platform::current_platform(false);
    let title_raster = particle_text::GlyphRaster(platform.text_system());
    gpui::Application::with_platform(platform)
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            flush_logs_on_quit(cx, gespan_core::diagnostics::flush);
            #[cfg(debug_assertions)]
            dev_reload::install(cx);
            cx.set_global(title_raster);
            image_memory::init(cx);
            #[cfg(target_os = "macos")]
            memory_log::start(cx);
            input::bind(cx);
            image_lightbox::bind(cx);
            composer_completion::bind(cx);
            file_editor::bind(cx);
            popover::bind(cx);
            cx.set_prompt_builder(modal::prompt_builder);
            explorer::bind(cx);
            history::bind(cx);
            diff_view::bind(cx);
            terminal::bind(cx);
            let modifier = if cfg!(target_os = "macos") {
                "cmd"
            } else {
                "ctrl"
            };
            cx.bind_keys([KeyBinding::new(&format!("{modifier}-q"), Quit, None)]);
            shell::bind_keys(cx);
            usage_footer::bind(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    quit(cx)
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1180.), px(800.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(820.), px(520.))),
                    window_background: if cfg!(target_os = "macos") {
                        WindowBackgroundAppearance::Blurred
                    } else {
                        WindowBackgroundAppearance::Opaque
                    },
                    // GPUI owns the custom chrome. AppKit must not reinterpret
                    // its controls as native titlebar double-click/drag gestures.
                    app_owns_titlebar_drag: cfg!(target_os = "macos"),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Gespan Native".into()),
                        appears_transparent: cfg!(target_os = "macos"),
                        traffic_light_position: if cfg!(target_os = "macos") {
                            // Original macOS chrome: 12 pt left inset, with
                            // 14 pt native buttons centered in the 40 pt bar.
                            Some(point(px(12.), px((theme::TITLE_BAR_HEIGHT - 14.) / 2.)))
                        } else {
                            None
                        },
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let shell = cx.new(|cx| {
                        shell::Shell::new_with_metadata(
                            db,
                            sessions,
                            cwd,
                            saved,
                            sidebar_metadata,
                            cx,
                        )
                    });
                    shell.update(cx, |shell, cx| {
                        shell.focus_active(window, cx);
                        shell.check_crash_notice(cx);
                    });
                    #[cfg(target_os = "macos")]
                    {
                        traffic_lights::pin(window, theme::TITLE_BAR_HEIGHT);
                        // AppKit rebuilds the buttons on some titlebar changes.
                        shell.update(cx, |_, cx| {
                            cx.observe_window_activation(window, |_, window, _| {
                                traffic_lights::pin(window, theme::TITLE_BAR_HEIGHT)
                            })
                            .detach();
                        });
                    }
                    window.activate_window();
                    shell
                },
            )
            .expect("open GPUI window");
            cx.activate(true);
        });
}

#[cfg(test)]
mod startup_log_tests {
    use super::*;
    use gespan_core::diagnostics::Channel;

    #[test]
    fn fatal_errors_reach_a_terminal_exactly_once() {
        // Without a subscriber, stderr is the only place the error can go.
        assert!(echo_fatal(false, Channel::Dev));
        assert!(echo_fatal(false, Channel::Release));
        // Release logging writes only the file, so the terminal needs a copy.
        assert!(echo_fatal(true, Channel::Release));
        // Dev logging already prints readable lines on stderr.
        assert!(!echo_fatal(true, Channel::Dev));
    }

    /// Error paths log through `tracing`; stderr is kept only for messages
    /// written before logging starts, the `--check-data` report, `fatal`'s
    /// terminal copy, and benchmark output in tests.
    #[test]
    fn no_new_eprintln_in_gespan_gpui() {
        let allowed = [
            ("main.rs", 6),
            ("shell.rs", 1),
            ("shell/sidebar_list.rs", 1),
        ];
        let needle = concat!("eprint", "ln!(");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut pending = vec![root.clone()];
        let mut found = Vec::new();
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let count = std::fs::read_to_string(&path)
                        .unwrap()
                        .matches(needle)
                        .count();
                    if count > 0 {
                        let relative = path.strip_prefix(&root).unwrap();
                        found.push((relative.to_string_lossy().replace('\\', "/"), count));
                    }
                }
            }
        }
        found.sort();
        let mut allowed: Vec<_> = allowed.iter().map(|(f, n)| (f.to_string(), *n)).collect();
        allowed.sort();
        assert_eq!(
            found, allowed,
            "log error paths with tracing instead of stderr"
        );
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use crate::{input, transcript::TranscriptView};
    use gespan_core::transcript::Row;
    use gpui::{Context, Entity, Render, StyleRefinement, Window, div};
    use gpui::{EntityInputHandler, ListOffset, TestAppContext, point};
    struct Surface {
        transcript: Entity<TranscriptView>,
        input: Entity<input::Input>,
    }
    impl Render for Surface {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .flex_col()
                .size_full()
                .child(
                    div().flex_1().min_h_0().child(
                        self.transcript
                            .clone()
                            .cached(StyleRefinement::default().size_full()),
                    ),
                )
                .child(
                    self.input
                        .clone()
                        .cached(StyleRefinement::default().w_full().h(px(76.))),
                )
        }
    }
    static FLUSHES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    fn count_flush() {
        FLUSHES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    #[gpui::test]
    fn every_quit_path_flushes_logs(cx: &mut TestAppContext) {
        cx.update(|cx| flush_logs_on_quit(cx, count_flush));
        assert_eq!(FLUSHES.load(std::sync::atomic::Ordering::SeqCst), 0);
        // GPUI's shutdown, which menu, Dock and AppKit quits all reach.
        cx.quit();
        assert_eq!(FLUSHES.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
    #[test]
    fn development_data_logs_to_its_own_folder() {
        use gespan_core::diagnostics::Channel;
        let release = diagnostics_config(true);
        let dev = diagnostics_config(false);
        assert_eq!(release.channel, Channel::Release);
        assert_eq!(dev.channel, Channel::Dev);
        assert_eq!(
            release.log_dir,
            gespan_core::diagnostics::log_dir(Channel::Release)
        );
        assert_eq!(dev.log_dir, gespan_core::diagnostics::log_dir(Channel::Dev));
        assert_ne!(release.log_dir, dev.log_dir);
        assert_eq!(release.version, notifications::updater::version());
    }
    #[test]
    fn bundle_plist_selects_the_data_mode() {
        let plist = |mode: &str| {
            format!("<dict>\n<key>GespanDataMode</key><string>{mode}</string>\n</dict>")
        };
        assert_eq!(plist_data_mode(&plist("gespan-data")), Some(true));
        assert_eq!(plist_data_mode(&plist("dev-data")), Some(false));
        assert_eq!(plist_data_mode(&plist("other")), None);
        assert_eq!(plist_data_mode("<dict></dict>"), None);
    }
    #[gpui::test]
    fn long_history_virtualizes_and_typing_does_not_rebuild_it(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(900.), px(700.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        let transcript = cx.new(|_| TranscriptView::new());
                        let mut history = Transcript::default();
                        for _ in 0..10_000 {
                            history.begin("assistant");
                            history.append("A bounded block of text.\nA second line.");
                        }
                        transcript.update(cx, |v, cx| v.patch(0, history.rows, cx));
                        Surface {
                            transcript,
                            input: cx.new(input::Input::new),
                        }
                    })
                },
            )
            .unwrap()
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let root = window.root(cx).unwrap();
        let transcript = root.read_with(cx, |v, _| v.transcript.clone());
        let (passes, rendered) = transcript.read_with(cx, |t, _| (t.render_passes, t.rendered));
        assert!(
            rendered > 0 && rendered < 100,
            "expected viewport-sized work, rendered {rendered} rows"
        );
        window
            .update(cx, |v, w, cx| {
                v.input.update(cx, |i, cx| {
                    i.replace_text_in_range(None, "Typing 🙂 stays local", w, cx)
                })
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            transcript.read_with(cx, |t, _| t.render_passes),
            passes,
            "typing must not rerender the transcript entity"
        );
        window
            .update(cx, |v, _, cx| {
                v.transcript.update(cx, |t, cx| {
                    t.list.pause_following_tail();
                    t.list.scroll_to(ListOffset {
                        item_ix: 5000,
                        offset_in_item: px(7.),
                    });
                    let anchor = t.list.logical_scroll_top();
                    t.patch(
                        9999,
                        vec![Row {
                            role: "assistant".into(),
                            text: "Changed streaming tail".into(),
                            first: true,
                            sent_at: None,
                        }],
                        cx,
                    );
                    assert_eq!(t.list.logical_scroll_top().item_ix, anchor.item_ix);
                    assert_eq!(
                        t.list.logical_scroll_top().offset_in_item,
                        anchor.offset_in_item
                    );
                });
            })
            .unwrap();
    }
}
