use super::super::sources::agent_apps::{ClaudeLaunch, Codex, Conductor, Cursor, Superset};
use super::super::sources::editors::{Gespan, VsCode, Zed};
use super::super::sources::Source;
use super::super::{Ctx, Found};
use std::path::Path;

fn parse(source: &dyn Source, file: &str, text: &str) -> Vec<Found> {
    let checkout = Path::new("/work/app");
    let ctx = Ctx {
        checkout,
        package_manager: "npm",
    };
    assert!(
        source.files().contains(&file),
        "{file} isn't a {} file",
        source.id()
    );
    source.parse(&ctx, file, text).unwrap()
}

#[test]
fn source_ids_and_files_are_stable() {
    let all: [(&dyn Source, &str, &[&str]); 8] = [
        (&Gespan, "gespan", &[".gespan/tasks.json"]),
        (&VsCode, "vscode", &[".vscode/tasks.json"]),
        (&Zed, "zed", &[".zed/tasks.json"]),
        (
            &Conductor,
            "conductor",
            &[".conductor/settings.toml", "conductor.json"],
        ),
        (
            &Superset,
            "superset",
            &[".paradev/config.json", ".superset/config.json"],
        ),
        (&Cursor, "cursor", &[".cursor/environment.json"]),
        (&Codex, "codex", &[".codex/environments/environment.toml"]),
        (&ClaudeLaunch, "claude", &[".claude/launch.json"]),
    ];
    for (source, id, files) in all {
        assert_eq!(source.id(), id);
        assert_eq!(source.files(), files);
    }
}

#[test]
fn gespan_and_vscode_read_tasks_json() {
    let text =
        r#"{ "version": "2.0.0", "tasks": [ { "label": "Test", "command": "cargo test" } ] }"#;
    for (source, file) in [
        (&Gespan as &dyn Source, ".gespan/tasks.json"),
        (&VsCode, ".vscode/tasks.json"),
    ] {
        let found = parse(source, file, text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "Test");
        assert_eq!(found[0].command, "cargo test");
    }
}

#[test]
fn conductor_toml_named_run_scripts() {
    let text = r#"
[scripts]
setup = "bun install"
archive = "rm -rf node_modules"

[scripts.run.dev]
command = "bun"
args = ["run", "dev"]
options = { cwd = "web" }

[scripts.run.secret]
command = "make secret"
hide = true

[scripts.run.empty]
args = ["x"]
"#;
    let found = parse(&Conductor, ".conductor/settings.toml", text);
    let labels: Vec<_> = found.iter().map(|f| f.label.as_str()).collect();
    // A script with no command isn't an action.
    assert_eq!(labels, ["dev", "secret"]);
    assert_eq!(found[0].command, "bun");
    assert_eq!(found[0].args, ["run", "dev"]);
    assert_eq!(found[0].cwd.as_deref(), Some("web"));
    assert!(found[0].shell);
    assert!(!found[0].hide);
    assert!(found[1].hide);
}

#[test]
fn conductor_json_legacy_run() {
    let text = r#"{ "scripts": { "setup": "npm i", "run": "npm start", "archive": "x" },
                    "run_mode": "nonconcurrent" }"#;
    let found = parse(&Conductor, "conductor.json", text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].label, "Run");
    assert_eq!(found[0].command, "npm start");
    // Neither is a table form, nor does the blank string count.
    let blank = r#"{ "scripts": { "run": "  " } }"#;
    assert!(parse(&Conductor, "conductor.json", blank).is_empty());
    // A plain `run` string works in the TOML file too.
    let toml = "[scripts]\nrun = \"npm start\"\n";
    let found = parse(&Conductor, ".conductor/settings.toml", toml);
    assert_eq!(found[0].label, "Run");
}

#[test]
fn superset_run_entries_get_numbered_labels_and_cwd() {
    let text = r#"{
        "setup": ["bun install"], "teardown": ["rm -rf dist"],
        "cwd": "app",
        "run": ["bun run dev", "bun run worker", "bun run docs"]
    }"#;
    for file in [".paradev/config.json", ".superset/config.json"] {
        let found = parse(&Superset, file, text);
        let rows: Vec<_> = found
            .iter()
            .map(|f| (f.label.as_str(), f.command.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("Run", "bun run dev"),
                ("Run 2", "bun run worker"),
                ("Run 3", "bun run docs")
            ]
        );
        assert!(found
            .iter()
            .all(|f| f.cwd.as_deref() == Some("app") && f.shell));
    }
    let one = parse(&Superset, ".superset/config.json", r#"{ "run": ["make"] }"#);
    assert_eq!(one[0].label, "Run");
    assert_eq!(one[0].cwd, None);
}

#[test]
fn cursor_terminals_are_background_with_description_as_detail() {
    let text = r#"{
        "install": "bun install",
        "start": "bun run dev",
        "terminals": [
            { "name": "Web", "command": "bun run web", "description": "The web app" },
            { "name": "No command" },
            { "command": "no name" }
        ]
    }"#;
    let found = parse(&Cursor, ".cursor/environment.json", text);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].label, "Web");
    assert_eq!(found[0].command, "bun run web");
    assert_eq!(found[0].detail.as_deref(), Some("The web app"));
    assert_eq!(found[1].label, "Start");
    assert_eq!(found[1].command, "bun run dev");
    assert!(found.iter().all(|f| f.background && f.shell));
}

#[test]
fn codex_actions() {
    let text = r#"
[setup]
script = "bun install"

[[actions]]
name = "Run"
icon = "play"
command = "bun run dev"

[[actions]]
name = "Test"
icon = "test-tube"
command = "bun test"

[[actions]]
name = "Broken"
"#;
    let found = parse(&Codex, ".codex/environments/environment.toml", text);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].label, "Run");
    assert_eq!(found[0].command, "bun run dev");
    assert_eq!(found[0].icon.as_deref(), Some("play"));
    assert_eq!(found[1].label, "Test");
    assert_eq!(found[1].icon.as_deref(), Some("test-tube"));
    assert!(found.iter().all(|f| !f.background));
}

#[test]
fn claude_launch_configurations() {
    let text = r#"{
        "version": "0.0.1",
        // Dev servers
        "configurations": [
            { "name": "web", "runtimeExecutable": "bun", "runtimeArgs": ["run", "dev"],
              "port": 5173, "cwd": "web", "env": { "PORT": "5173" } },
            { "name": "no program", "port": 1 },
        ]
    }"#;
    let found = parse(&ClaudeLaunch, ".claude/launch.json", text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].label, "web");
    assert_eq!(found[0].command, "bun");
    assert_eq!(found[0].args, ["run", "dev"]);
    assert_eq!(found[0].cwd.as_deref(), Some("web"));
    assert_eq!(found[0].env, [("PORT".to_string(), "5173".to_string())]);
    assert!(found[0].background);
    assert!(!found[0].shell, "a program and its args, not a shell line");
}

#[test]
fn zed_tasks() {
    let text = r#"[
        // Zed's own file
        { "label": "Test", "command": "cargo", "args": ["test", "-p", "app"],
          "cwd": "$ZED_WORKTREE_ROOT/crates", "env": { "RUST_LOG": "debug" } },
        { "label": "Hidden", "command": "make", "hide": "always" },
        { "label": "On success", "command": "make x", "hide": "on_success" },
        { "label": "No command" },
    ]"#;
    let found = parse(&Zed, ".zed/tasks.json", text);
    let rows: Vec<_> = found.iter().map(|f| (f.label.as_str(), f.hide)).collect();
    assert_eq!(
        rows,
        [("Test", false), ("Hidden", true), ("On success", false)]
    );
    assert_eq!(found[0].command, "cargo");
    assert_eq!(found[0].args, ["test", "-p", "app"]);
    assert_eq!(found[0].cwd.as_deref(), Some("$ZED_WORKTREE_ROOT/crates"));
    assert_eq!(
        found[0].env,
        [("RUST_LOG".to_string(), "debug".to_string())]
    );
    assert!(found[0].shell);
}

#[test]
fn setup_and_teardown_keys_are_never_actions() {
    let checkout = Path::new("/work/app");
    let ctx = Ctx {
        checkout,
        package_manager: "npm",
    };
    let cases: [(&dyn Source, &str, &str); 6] = [
        (
            &Conductor,
            ".conductor/settings.toml",
            "[scripts]\nsetup = \"a\"\narchive = \"b\"\nteardown = \"c\"\n",
        ),
        (
            &Conductor,
            "conductor.json",
            r#"{ "scripts": { "setup": "a", "archive": "b" } }"#,
        ),
        (
            &Superset,
            ".superset/config.json",
            r#"{ "setup": ["a"], "teardown": ["b"], "archive": ["c"] }"#,
        ),
        (
            &Cursor,
            ".cursor/environment.json",
            r#"{ "install": "a", "setup": "b", "teardown": "c" }"#,
        ),
        (
            &Codex,
            ".codex/environments/environment.toml",
            "[setup]\nscript = \"a\"\n[teardown]\nscript = \"b\"\n",
        ),
        (
            &ClaudeLaunch,
            ".claude/launch.json",
            r#"{ "setup": "a", "teardown": "b", "configurations": [] }"#,
        ),
    ];
    for (source, file, text) in cases {
        let found = source.parse(&ctx, file, text).unwrap();
        assert!(found.is_empty(), "{} read {found:?}", source.id());
    }
}

#[test]
fn a_half_saved_file_is_an_error_not_a_panic() {
    let checkout = Path::new("/work/app");
    let ctx = Ctx {
        checkout,
        package_manager: "npm",
    };
    let broken: [(&dyn Source, &str, &str); 6] = [
        (&Zed, ".zed/tasks.json", "[ { \"label\": "),
        (&Zed, ".zed/tasks.json", r#"{ "label": "not a list" }"#),
        (&Conductor, ".conductor/settings.toml", "[scripts"),
        (&Superset, ".superset/config.json", "{ \"run\": [ "),
        (&Cursor, ".cursor/environment.json", "[]"),
        (&Codex, ".codex/environments/environment.toml", "[[actions]"),
    ];
    for (source, file, text) in broken {
        assert!(
            source.parse(&ctx, file, text).is_err(),
            "{} {text}",
            source.id()
        );
    }
}
