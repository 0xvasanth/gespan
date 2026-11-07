use super::super::{detect, reads, set_personal, Action, Listing, Personal, SourceError};
use std::path::Path;

fn write(dir: &Path, file: &str, text: &str) {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A project key no other test uses, since personal actions are global.
fn project(dir: &Path) -> String {
    dir.display().to_string()
}

fn rows(listing: &Listing) -> Vec<(&str, &str)> {
    listing
        .actions
        .iter()
        .map(|action| (action.source, action.label.as_str()))
        .collect()
}

fn gespan(tasks: &str) -> String {
    format!(r#"{{ "version": "2.0.0", "tasks": [{tasks}] }}"#)
}

#[test]
fn sources_list_in_priority_order() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "Makefile", "build:\n\tcc main.c\n");
    write(root, "package.json", r#"{ "scripts": { "dev": "vite" } }"#);
    write(
        root,
        ".gespan/tasks.json",
        &gespan(r#"{ "label": "Ship", "command": "echo ship" }"#),
    );
    let listing = detect(&project(root), root);
    assert_eq!(
        rows(&listing),
        [
            ("gespan", "Ship"),
            ("package.json", "dev"),
            ("make", "build")
        ]
    );
    assert_eq!(listing.errors, []);
    let ship = &listing.actions[0];
    assert_eq!(ship.id, "gespan:Ship");
    assert_eq!(ship.file, ".gespan/tasks.json");
    assert_eq!(listing.actions[1].command, "npm run dev");
    assert_eq!(listing.actions[2].file, "Makefile");
}

#[test]
fn same_command_in_two_sources_shows_once() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".vscode/tasks.json",
        &gespan(r#"{ "type": "npm", "script": "test" }"#),
    );
    write(
        root,
        "package.json",
        r#"{ "scripts": { "test": "vitest", "lint": "eslint ." } }"#,
    );
    let listing = detect(&project(root), root);
    assert_eq!(
        rows(&listing),
        [("vscode", "npm: test"), ("package.json", "lint")]
    );
    assert_eq!(listing.actions[0].command, "npm run test");
}

#[test]
fn a_gespan_label_hides_a_detected_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".gespan/tasks.json",
        &gespan(r#"{ "label": "dev", "command": "bun run web" }"#),
    );
    write(root, "package.json", r#"{ "scripts": { "dev": "vite" } }"#);
    write(root, "Makefile", "dev:\n\tmake web\nall:\n\ttrue\n");
    let listing = detect(&project(root), root);
    assert_eq!(rows(&listing), [("gespan", "dev"), ("make", "all")]);
    assert_eq!(listing.actions[0].command, "bun run web");
}

#[test]
fn personal_actions_follow_gespan_and_keep_their_ids() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".gespan/tasks.json",
        &gespan(r#"{ "label": "Ship", "command": "echo ship" }"#),
    );
    write(root, "package.json", r#"{ "scripts": { "dev": "vite" } }"#);
    let key = project(root);
    set_personal(
        &key,
        vec![
            Personal {
                id: "abc".into(),
                name: "Deploy".into(),
                command: "./deploy.sh --env \"$STAGE\"".into(),
                icon: "rocket".into(),
            },
            Personal {
                id: "def".into(),
                name: "Tail".into(),
                command: "tail -f log".into(),
                icon: "not-an-icon".into(),
            },
        ],
    );
    let listing = detect(&key, root);
    assert_eq!(
        rows(&listing),
        [
            ("gespan", "Ship"),
            ("personal", "Deploy"),
            ("personal", "Tail"),
            ("package.json", "dev")
        ]
    );
    let deploy = &listing.actions[1];
    assert_eq!(deploy.id, "personal:abc");
    assert_eq!(deploy.file, "Personal");
    assert_eq!(deploy.cwd, root);
    assert_eq!(deploy.icon, "rocket");
    // A shell line, run as written.
    assert_eq!(deploy.command, "./deploy.sh --env \"$STAGE\"");
    assert_eq!(deploy.blocked, None);
    assert_eq!(listing.actions[2].id, "personal:def");
    assert_eq!(listing.actions[2].icon, "play");

    // Another project doesn't see them, and clearing them removes them.
    let other = tempfile::tempdir().unwrap();
    assert_eq!(detect(&project(other.path()), other.path()).actions, []);
    set_personal(&key, Vec::new());
    assert_eq!(
        rows(&detect(&key, root)),
        [("gespan", "Ship"), ("package.json", "dev")]
    );
}

#[test]
fn a_broken_source_is_one_error_row_and_the_rest_still_list() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // Half saved while the user edits it.
    write(
        root,
        ".vscode/tasks.json",
        r#"{ "version": "2.0.0", "tasks": [ { "label": "#,
    );
    write(root, "package.json", r#"{ "scripts": { "dev": "vite" } }"#);
    write(root, "Makefile", "build:\n\tcc\n");
    let listing = detect(&project(root), root);
    assert_eq!(rows(&listing), [("package.json", "dev"), ("make", "build")]);
    assert_eq!(listing.errors.len(), 1);
    let SourceError { file, reason } = &listing.errors[0];
    assert_eq!(file, ".vscode/tasks.json");
    assert!(!reason.is_empty());
}

#[test]
fn an_unreadable_file_is_an_error_row() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // Not UTF-8, so it can't be read as text.
    std::fs::write(root.join("Procfile"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
    write(root, "Makefile", "build:\n\tcc\n");
    let listing = detect(&project(root), root);
    assert_eq!(rows(&listing), [("make", "build")]);
    assert_eq!(listing.errors.len(), 1);
    assert_eq!(listing.errors[0].file, "Procfile");
}

#[test]
fn a_second_detect_reads_no_files_when_nothing_changed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "package.json", r#"{ "scripts": { "dev": "vite" } }"#);
    write(root, "justfile", "test:\n    cargo test\n");
    write(
        root,
        ".zed/tasks.json",
        r#"[{ "label": "Z", "command": "z" }]"#,
    );
    let before = reads();
    let first = detect(&project(root), root);
    assert_eq!(reads() - before, 3);
    let before = reads();
    let second = detect(&project(root), root);
    assert_eq!(reads() - before, 0);
    assert_eq!(first, second);
}

#[test]
fn an_edited_file_is_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "package.json", r#"{ "scripts": { "dev": "vite" } }"#);
    write(root, "Makefile", "build:\n\tcc\n");
    detect(&project(root), root);
    write(
        root,
        "package.json",
        r#"{ "scripts": { "dev": "vite", "test": "vitest" } }"#,
    );
    let before = reads();
    let listing = detect(&project(root), root);
    // Only the edited file.
    assert_eq!(reads() - before, 1);
    assert_eq!(
        rows(&listing),
        [
            ("package.json", "dev"),
            ("package.json", "test"),
            ("make", "build")
        ]
    );
}

#[test]
fn a_deleted_file_stops_listing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "package.json", r#"{ "scripts": { "dev": "vite" } }"#);
    write(root, "Makefile", "build:\n\tcc\n");
    assert_eq!(detect(&project(root), root).actions.len(), 2);
    std::fs::remove_file(root.join("package.json")).unwrap();
    assert_eq!(rows(&detect(&project(root), root)), [("make", "build")]);
    // Back again, it is read afresh rather than served from the old entry.
    write(root, "package.json", r#"{ "scripts": { "web": "vite" } }"#);
    let before = reads();
    let listing = detect(&project(root), root);
    assert_eq!(reads() - before, 1);
    assert_eq!(listing.actions[0].label, "web");
}

#[test]
fn a_new_lockfile_changes_cached_commands() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "package.json", r#"{ "scripts": { "dev": "vite" } }"#);
    assert_eq!(
        detect(&project(root), root).actions[0].command,
        "npm run dev"
    );
    write(root, "bun.lock", "");
    assert_eq!(
        detect(&project(root), root).actions[0].command,
        "bun run dev"
    );
}

#[test]
fn hidden_actions_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".vscode/tasks.json",
        &gespan(
            r#"{ "label": "Shown", "command": "a" },
               { "label": "Secret", "command": "b", "hide": true }"#,
        ),
    );
    write(
        root,
        ".zed/tasks.json",
        r#"[{ "label": "Gone", "command": "c", "hide": "always" },
            { "label": "Kept", "command": "d", "hide": "on_success" }]"#,
    );
    let listing = detect(&project(root), root);
    assert_eq!(rows(&listing), [("vscode", "Shown"), ("zed", "Kept")]);
}

#[test]
fn a_label_twice_in_one_source_gets_a_number() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "Procfile", "web: a\nweb: b\nweb: c\n");
    write(
        root,
        ".zed/tasks.json",
        r#"[{ "label": "web", "command": "z" }]"#,
    );
    let listing = detect(&project(root), root);
    let ids: Vec<&str> = listing.actions.iter().map(|a| a.id.as_str()).collect();
    // Other sources keep their own ids, so only clashes within one count.
    assert_eq!(
        ids,
        [
            "zed:web",
            "procfile:web",
            "procfile:web (2)",
            "procfile:web (3)"
        ]
    );
    assert_eq!(listing.actions[2].label, "web (2)");
}

#[test]
fn detect_reads_the_given_checkout_not_the_project_root() {
    let main = tempfile::tempdir().unwrap();
    let worktree = tempfile::tempdir().unwrap();
    write(
        main.path(),
        "package.json",
        r#"{ "scripts": { "main-only": "x" } }"#,
    );
    write(
        worktree.path(),
        "package.json",
        r#"{ "scripts": { "branch-only": "y" } }"#,
    );
    let listing = detect(&project(main.path()), worktree.path());
    assert_eq!(rows(&listing), [("package.json", "branch-only")]);
    let action: &Action = &listing.actions[0];
    assert_eq!(action.cwd, worktree.path());
}
