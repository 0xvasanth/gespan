use super::super::write::{add, remove, save_detected, update, Draft, WriteError};
use super::super::{detect, Action};
use serde_json::{json, Value};
use std::path::Path;

fn draft(label: &str, command: &str) -> Draft {
    Draft {
        label: label.into(),
        command: command.into(),
        icon: "play".into(),
        detail: None,
    }
}

fn file(dir: &Path) -> std::path::PathBuf {
    dir.join(".gespan/tasks.json")
}

fn read(dir: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(file(dir)).unwrap()).unwrap()
}

fn put(dir: &Path, text: &str) {
    std::fs::create_dir_all(dir.join(".gespan")).unwrap();
    std::fs::write(file(dir), text).unwrap();
}

fn labels(dir: &Path) -> Vec<String> {
    detect(&dir.display().to_string(), dir)
        .actions
        .into_iter()
        .filter(|action| action.source == "gespan")
        .map(|action| action.label)
        .collect()
}

fn detected(dir: &Path, label: &str) -> Action {
    detect(&dir.display().to_string(), dir)
        .actions
        .into_iter()
        .find(|action| action.source == "gespan" && action.label == label)
        .unwrap()
}

#[test]
fn add_creates_folder_and_file_with_version() {
    let dir = tempfile::tempdir().unwrap();
    let mut test = draft("Test", "cargo test");
    test.icon = "test-tube".into();
    test.detail = Some("All crates".into());
    add(dir.path(), &test).unwrap();

    let text = std::fs::read_to_string(file(dir.path())).unwrap();
    assert!(text.ends_with("}\n"), "{text:?}");
    assert!(text.contains("\n  \"version\": \"2.0.0\""), "{text}");
    let value = read(dir.path());
    assert_eq!(value["version"], "2.0.0");
    assert_eq!(
        value["tasks"],
        json!([{
            "label": "Test",
            "type": "shell",
            "command": "cargo test",
            "detail": "All crates",
            "icon": { "id": "test-tube" },
        }])
    );
    // No temp file is left behind.
    let left: Vec<_> = std::fs::read_dir(dir.path().join(".gespan"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left.len(), 1, "{left:?}");
}

#[test]
fn a_default_icon_and_empty_detail_are_left_out_and_unknown_icons_become_play() {
    let dir = tempfile::tempdir().unwrap();
    let mut odd = draft("Odd", "true");
    odd.icon = "skull".into();
    add(dir.path(), &odd).unwrap();
    let task = &read(dir.path())["tasks"][0];
    assert!(task.get("icon").is_none() && task.get("detail").is_none());
}

#[test]
fn add_keeps_other_tasks_and_unknown_fields() {
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        r#"{
  "version": "2.0.0",
  "inputs": [{ "id": "x" }],
  "tasks": [
    { "label": "Lint", "command": "eslint", "args": ["."], "group": "build", },
  ],
}"#,
    );
    add(dir.path(), &draft("Test", "cargo test")).unwrap();
    let value = read(dir.path());
    assert_eq!(value["inputs"], json!([{ "id": "x" }]));
    assert_eq!(
        value["tasks"][0],
        json!({ "label": "Lint", "command": "eslint", "args": ["."], "group": "build" })
    );
    assert_eq!(value["tasks"][1]["label"], "Test");
    assert_eq!(labels(dir.path()), ["Lint", "Test"]);
}

#[test]
fn add_refuses_a_duplicate_label() {
    let dir = tempfile::tempdir().unwrap();
    add(dir.path(), &draft("Test", "a")).unwrap();
    let before = std::fs::read(file(dir.path())).unwrap();
    assert_eq!(
        add(dir.path(), &draft("Test", "b")),
        Err(WriteError::Duplicate("Test".into()))
    );
    assert_eq!(std::fs::read(file(dir.path())).unwrap(), before);
}

#[test]
fn blank_labels_and_commands_are_invalid() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        add(dir.path(), &draft("  ", "a")),
        Err(WriteError::Invalid(_))
    ));
    assert!(matches!(
        add(dir.path(), &draft("A", " ")),
        Err(WriteError::Invalid(_))
    ));
    assert!(!file(dir.path()).exists());
}

#[test]
fn update_and_remove_by_label() {
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        r#"{ "version": "2.0.0", "tasks": [
  { "label": "A", "command": "one", "args": ["x"], "group": "test", "isBackground": true },
  { "label": "B", "command": "two" }
] }"#,
    );
    let mut next = draft("A2", "three");
    next.icon = "rocket".into();
    update(dir.path(), "A", &next).unwrap();
    let value = read(dir.path());
    // Edited in place: same slot, unknown fields kept, old args dropped.
    assert_eq!(
        value["tasks"][0],
        json!({
            "label": "A2", "command": "three", "type": "shell", "group": "test",
            "isBackground": true, "icon": { "id": "rocket" },
        })
    );
    assert_eq!(detected(dir.path(), "A2").command, "three");

    // An edit that clears the icon and detail removes them.
    update(dir.path(), "A2", &draft("A2", "three")).unwrap();
    assert!(read(dir.path())["tasks"][0].get("icon").is_none());

    assert_eq!(
        update(dir.path(), "A2", &draft("B", "x")),
        Err(WriteError::Duplicate("B".into()))
    );
    assert!(matches!(
        update(dir.path(), "Nope", &draft("N", "x")),
        Err(WriteError::Invalid(_))
    ));

    remove(dir.path(), "A2").unwrap();
    assert_eq!(labels(dir.path()), ["B"]);
    assert!(matches!(
        remove(dir.path(), "A2"),
        Err(WriteError::Invalid(_))
    ));
}

#[test]
fn a_file_with_comments_is_never_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let text = "{\n  // my tasks\n  \"version\": \"2.0.0\",\n  \"tasks\": [\n    { \"label\": \"A\", \"command\": \"a\" /* keep */ }\n  ]\n}\n";
    put(dir.path(), text);
    let cases = [
        add(dir.path(), &draft("B", "b")),
        update(dir.path(), "A", &draft("A", "z")),
        remove(dir.path(), "A"),
    ];
    for result in cases {
        assert_eq!(result, Err(WriteError::HasComments));
    }
    assert_eq!(std::fs::read(file(dir.path())).unwrap(), text.as_bytes());
}

#[test]
fn a_comment_marker_inside_a_string_is_not_a_comment() {
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        r#"{ "tasks": [ { "label": "Open", "command": "open http://x.test" } ] }"#,
    );
    add(dir.path(), &draft("B", "b")).unwrap();
    assert_eq!(
        read(dir.path())["tasks"][0]["command"],
        "open http://x.test"
    );
    // The version is added when a hand-written file lacks it.
    assert_eq!(read(dir.path())["version"], "2.0.0");
}

#[test]
fn a_broken_or_odd_file_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    for text in ["{ \"tasks\": [", "[]", r#"{ "tasks": {} }"#] {
        put(dir.path(), text);
        assert!(
            matches!(
                add(dir.path(), &draft("B", "b")),
                Err(WriteError::Invalid(_))
            ),
            "{text}"
        );
        assert_eq!(std::fs::read_to_string(file(dir.path())).unwrap(), text);
    }
}

#[test]
fn add_then_detect_lists_it_even_within_the_same_second() {
    let dir = tempfile::tempdir().unwrap();
    add(dir.path(), &draft("A", "a")).unwrap();
    assert_eq!(labels(dir.path()), ["A"]);
    // Same length and, below, the same mtime: only a dropped cache entry
    // can show the change.
    let before = std::fs::metadata(file(dir.path()))
        .unwrap()
        .modified()
        .unwrap();
    update(dir.path(), "A", &draft("A", "b")).unwrap();
    std::fs::File::options()
        .write(true)
        .open(file(dir.path()))
        .unwrap()
        .set_modified(before)
        .unwrap();
    assert_eq!(detected(dir.path(), "A").command, "b");
    update(dir.path(), "A", &draft("B", "b")).unwrap();
    assert_eq!(labels(dir.path()), ["B"]);
    remove(dir.path(), "B").unwrap();
    assert!(labels(dir.path()).is_empty());
}

fn action(dir: &Path) -> Action {
    Action {
        id: "package.json:dev".into(),
        label: "dev".into(),
        detail: None,
        source: "package.json",
        file: "package.json".into(),
        command: "bun run dev".into(),
        cwd: dir.to_path_buf(),
        env: Vec::new(),
        background: false,
        icon: "play",
        blocked: None,
        note: None,
    }
}

#[test]
fn save_detected_writes_a_relative_cwd_and_background() {
    let dir = tempfile::tempdir().unwrap();
    let mut dev = action(dir.path());
    dev.cwd = dir.path().join("web/app");
    dev.env = vec![("PORT".into(), "3000".into())];
    dev.detail = Some("Vite".into());
    dev.background = true;
    dev.icon = "rocket";
    save_detected(dir.path(), &dev).unwrap();
    assert_eq!(
        read(dir.path())["tasks"][0],
        json!({
            "label": "dev", "type": "shell", "command": "bun run dev", "detail": "Vite",
            "isBackground": true, "icon": { "id": "rocket" },
            "options": { "cwd": "web/app", "env": { "PORT": "3000" } },
        })
    );

    // A plain action at the root writes no options and no background flag.
    save_detected(dir.path(), &{
        let mut plain = action(dir.path());
        plain.label = "plain".into();
        plain
    })
    .unwrap();
    let plain = &read(dir.path())["tasks"][1];
    assert!(plain.get("options").is_none() && plain.get("isBackground").is_none());
    assert_eq!(
        save_detected(dir.path(), &dev),
        Err(WriteError::Duplicate("dev".into()))
    );
}

#[test]
fn a_saved_action_detects_back_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (home, shown) = if cfg!(windows) {
        ("%HOME_DIR%", "%HOME_DIR%")
    } else {
        ("${HOME_DIR}", "${HOME_DIR}")
    };
    let mut from = action(root);
    from.label = "serve".into();
    from.command = format!("echo \"{home}\" 'a b' && run --dir \"{}\"", root.display());
    from.cwd = root.join("sub");
    from.env = vec![
        ("DIR".into(), format!("{shown}/x")),
        ("N".into(), "1".into()),
    ];
    from.background = true;
    save_detected(root, &from).unwrap();

    let file = std::fs::read_to_string(file(root)).unwrap();
    assert!(file.contains("${env:HOME_DIR}/x"), "{file}");
    let back = detected(root, "serve");
    assert_eq!(back.blocked, None);
    assert_eq!(back.command, from.command);
    assert_eq!(back.env, from.env);
    assert_eq!(back.cwd, from.cwd);
    assert!(back.background);
}

#[test]
fn a_blocked_action_or_an_outside_cwd_is_not_saved() {
    let dir = tempfile::tempdir().unwrap();
    let mut blocked = action(dir.path());
    blocked.blocked = Some("Runs outside the project".into());
    assert!(matches!(
        save_detected(dir.path(), &blocked),
        Err(WriteError::Invalid(_))
    ));
    let mut outside = action(dir.path());
    outside.cwd = dir.path().parent().unwrap().to_path_buf();
    assert!(matches!(
        save_detected(dir.path(), &outside),
        Err(WriteError::Invalid(_))
    ));
    let mut odd = action(dir.path());
    odd.command = "echo ${a:-b}".into();
    assert!(matches!(
        save_detected(dir.path(), &odd),
        Err(WriteError::Invalid(_))
    ));
    assert!(!file(dir.path()).exists());
}

#[cfg(unix)]
#[test]
fn writing_never_follows_a_link_out_of_the_checkout() {
    use std::os::unix::fs::symlink;
    let outside = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();

    // `.gespan` is a link to a folder elsewhere.
    symlink(outside.path(), dir.path().join(".gespan")).unwrap();
    let result = add(dir.path(), &draft("A", "a"));
    assert!(matches!(&result, Err(WriteError::Invalid(why)) if why.contains("regular folder")));
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);

    // `tasks.json` is a link to a file elsewhere.
    std::fs::remove_file(dir.path().join(".gespan")).unwrap();
    std::fs::create_dir(dir.path().join(".gespan")).unwrap();
    let target = outside.path().join("target.json");
    std::fs::write(&target, r#"{ "tasks": [] }"#).unwrap();
    symlink(&target, file(dir.path())).unwrap();
    for result in [add(dir.path(), &draft("A", "a")), remove(dir.path(), "A")] {
        assert!(matches!(&result, Err(WriteError::Invalid(why)) if why.contains("regular file")));
    }
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        r#"{ "tasks": [] }"#
    );
}

#[test]
fn rewriting_keeps_the_file_mode() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), r#"{ "tasks": [] }"#);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        std::fs::set_permissions(file(dir.path()), std::fs::Permissions::from_mode(0o664)).unwrap();
        add(dir.path(), &draft("A", "a")).unwrap();
        assert_eq!(mode(&file(dir.path())), 0o664);
        let fresh = tempfile::tempdir().unwrap();
        add(fresh.path(), &draft("A", "a")).unwrap();
        assert_eq!(mode(&file(fresh.path())), 0o644);
    }
}
