use super::super::write::{add, relabel, remove, save_detected, update, Draft, WriteError};
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
  { "label": "A", "command": "one", "script": "old", "group": "test", "isBackground": true },
  { "label": "B", "command": "two" }
] }"#,
    );
    let mut next = draft("A2", "three");
    next.icon = "rocket".into();
    update(dir.path(), "A", &next).unwrap();
    let value = read(dir.path());
    // Edited in place: same slot, unknown fields kept, an old script dropped.
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
        task: None,
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
    // None of these can be filled in on any OS.
    for command in ["echo $ZED_WORKTREE_ROOT/x", "echo ${a:-b}", "echo ${a b}"] {
        let mut odd = action(dir.path());
        odd.command = command.into();
        assert!(
            matches!(save_detected(dir.path(), &odd), Err(WriteError::Invalid(_))),
            "{command}"
        );
    }
    // On Windows a `${NAME}` is no reference either; on Unix it is one.
    let mut shell = action(dir.path());
    shell.command = "echo ${HOME_DIR}".into();
    assert_eq!(save_detected(dir.path(), &shell).is_ok(), !cfg!(windows));
    let _ = std::fs::remove_dir_all(dir.path().join(".gespan"));
    let mut env = action(dir.path());
    env.env = vec![("X".into(), "$ZED_WORKTREE_ROOT".into())];
    assert!(matches!(
        save_detected(dir.path(), &env),
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

#[test]
fn add_and_update_check_the_command_like_save_detected() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // The shell's own reference is saved as `${env:…}` and reads back as typed.
    let home = if cfg!(windows) {
        "echo %HOME_DIR%"
    } else {
        "echo ${HOME_DIR}"
    };
    add(root, &draft("Home", home)).unwrap();
    assert_eq!(read(root)["tasks"][0]["command"], "echo ${env:HOME_DIR}");
    assert_eq!(detected(root, "Home").command, home);
    update(root, "Home", &draft("Home", &format!("{home} again"))).unwrap();
    assert_eq!(
        read(root)["tasks"][0]["command"],
        "echo ${env:HOME_DIR} again"
    );

    // Nothing Gespan can't fill in is ever written, by add or by update.
    let before = std::fs::read(file(root)).unwrap();
    for command in ["echo ${a:-b}", "echo ${input:target}", "echo ${a b}"] {
        let added = add(root, &draft("Odd", command));
        assert!(
            matches!(&added, Err(WriteError::Invalid(why)) if why.contains("can't fill in")),
            "{command}: {added:?}"
        );
        let updated = update(root, "Home", &draft("Home", command));
        assert!(matches!(updated, Err(WriteError::Invalid(_))), "{command}");
    }
    assert_eq!(std::fs::read(file(root)).unwrap(), before);
    // No folder is made for a command that can't be saved.
    let fresh = tempfile::tempdir().unwrap();
    assert!(matches!(
        add(fresh.path(), &draft("A", "echo ${a:-b}")),
        Err(WriteError::Invalid(_))
    ));
    assert!(!fresh.path().join(".gespan").exists());
}

#[test]
fn labels_are_trimmed_before_the_duplicate_check() {
    let dir = tempfile::tempdir().unwrap();
    add(dir.path(), &draft(" A ", "a")).unwrap();
    assert_eq!(read(dir.path())["tasks"][0]["label"], "A");
    assert_eq!(
        add(dir.path(), &draft("A  ", "b")),
        Err(WriteError::Duplicate("A".into()))
    );
    add(dir.path(), &draft("B", "b")).unwrap();
    assert_eq!(
        update(dir.path(), "B", &draft("\tA", "b")),
        Err(WriteError::Duplicate("A".into()))
    );
    update(dir.path(), "B", &draft(" C ", "c")).unwrap();
    assert_eq!(labels(dir.path()), ["A", "C"]);
}

#[test]
fn rewriting_keeps_the_file_key_order_and_new_tasks_use_a_set_order() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // Keys in no sorted order, nested ones included, as a person wrote them.
    let lint = r#"    {
      "label": "Lint",
      "command": "eslint",
      "group": "build",
      "presentation": {
        "reveal": "never",
        "panel": 2,
        "ratio": 1.5,
        "focus": false,
        "close": null
      },
      "options": {
        "env": {
          "Z": "1",
          "A": "2"
        }
      },
      "args": [
        "."
      ]
    }"#;
    put(
        root,
        &format!(
            r#"{{
  "tasks": [
{lint},
    {{
      "command": "two",
      "label": "B",
      "type": "shell",
      "group": "test"
    }}
  ],
  "version": "2.0.0",
  "inputs": []
}}
"#
        ),
    );
    let mut b = draft("B", "three");
    b.detail = Some("Third".into());
    update(root, "B", &b).unwrap();
    // Only the edited task changes; a key it gains goes last.
    let edited = format!(
        r#"{{
  "tasks": [
{lint},
    {{
      "command": "three",
      "label": "B",
      "type": "shell",
      "group": "test",
      "detail": "Third"
    }}
  ],
  "version": "2.0.0",
  "inputs": []
}}
"#
    );
    assert_eq!(std::fs::read_to_string(file(root)).unwrap(), edited);

    // A new task's keys come in the set order.
    let mut c = draft("C", "c");
    c.icon = "rocket".into();
    c.detail = Some("See".into());
    add(root, &c).unwrap();
    let c = r#"    {
      "label": "C",
      "type": "shell",
      "command": "c",
      "detail": "See",
      "icon": {
        "id": "rocket"
      }
    }"#;
    let tail = "\n  ],\n  \"version\": \"2.0.0\",\n  \"inputs\": []\n}\n";
    let with_c = format!("{},\n{c}{tail}", edited.strip_suffix(tail).unwrap());
    assert_eq!(std::fs::read_to_string(file(root)).unwrap(), with_c);

    // So does a detected one's, with its folder, environment and background.
    let mut dev = action(root);
    dev.cwd = root.join("web");
    dev.env = vec![("Z".into(), "1".into()), ("A".into(), "2".into())];
    dev.detail = Some("Vite".into());
    dev.background = true;
    dev.icon = "rocket";
    save_detected(root, &dev).unwrap();
    let text = std::fs::read_to_string(file(root)).unwrap();
    let added = &text[text.find(r#""label": "dev""#).unwrap()..];
    let order = [
        "\"label\"",
        "\"type\"",
        "\"command\"",
        "\"options\"",
        "\"cwd\"",
        "\"env\"",
        "\"Z\"",
        "\"A\"",
        "\"detail\"",
        "\"isBackground\"",
        "\"icon\"",
    ];
    let keys: Vec<_> = order.iter().map(|key| added.find(key).unwrap()).collect();
    assert!(keys.is_sorted(), "{added}");
}

#[test]
fn a_new_file_starts_with_its_version() {
    let dir = tempfile::tempdir().unwrap();
    add(dir.path(), &draft("A", "a")).unwrap();
    let text = std::fs::read_to_string(file(dir.path())).unwrap();
    assert!(
        text.starts_with("{\n  \"version\": \"2.0.0\",\n  \"tasks\": ["),
        "{text}"
    );
    // A hand-written file without one gets it first, the rest kept in place.
    put(dir.path(), "{\n  \"tasks\": []\n}\n");
    add(dir.path(), &draft("A", "a")).unwrap();
    let text = std::fs::read_to_string(file(dir.path())).unwrap();
    assert!(
        text.starts_with("{\n  \"version\": \"2.0.0\",\n  \"tasks\": ["),
        "{text}"
    );
}

#[test]
fn errors_read_as_sentences() {
    assert_eq!(
        WriteError::HasComments.to_string(),
        "This tasks.json has comments, so Gespan won't rewrite it."
    );
    assert_eq!(
        WriteError::Duplicate("Test".into()).to_string(),
        "A script named “Test” already exists."
    );
    assert_eq!(WriteError::Io("Disk full".into()).to_string(), "Disk full");
    assert_eq!(
        WriteError::Invalid("Give the script a name".into()).to_string(),
        "Give the script a name"
    );
}

/// A tasks.json with `keys` keys in one object, in reverse order, and a
/// repeated key at the end.
fn huge(dir: &Path, keys: usize) {
    let keys: Vec<String> = (0..keys).rev().map(|n| format!("\"k{n}\": {n}")).collect();
    put(
        dir,
        &format!(
            r#"{{ "big": {{ {}, "k7": "last" }}, "tasks": [] }}"#,
            keys.join(", ")
        ),
    );
}

#[test]
fn a_huge_object_is_read_in_linear_time_and_in_order() {
    let dir = tempfile::tempdir().unwrap();
    huge(dir.path(), 40_000);
    add(dir.path(), &draft("A", "a")).unwrap();
    let text = std::fs::read_to_string(file(dir.path())).unwrap();
    assert!(text.find("\"k39999\"").unwrap() < text.find("\"k0\"").unwrap());
    // A repeated key keeps its first place and its last value.
    assert_eq!(text.matches("\"k7\"").count(), 1);
    assert!(
        text.contains("\"k7\": \"last\",\n    \"k6\": 6"),
        "k7 moved"
    );

    // Four times the keys takes about four times as long; quadratic would be
    // sixteen. Each size's best of three keeps noise out.
    let time = |keys: usize| {
        (0..3)
            .map(|_| {
                let dir = tempfile::tempdir().unwrap();
                huge(dir.path(), keys);
                let start = std::time::Instant::now();
                add(dir.path(), &draft("A", "a")).unwrap();
                start.elapsed()
            })
            .min()
            .unwrap()
    };
    let (small, large) = (time(10_000), time(40_000));
    let floor = std::time::Duration::from_millis(5);
    assert!(
        large < small.max(floor) * 8,
        "10k keys: {small:?}, 40k keys: {large:?}"
    );
}

#[test]
fn typed_workspace_variables_are_kept_as_written() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let line =
        "echo ${workspaceFolder} ${workspaceFolderBasename} ${env:PATH} $ZED_WORKTREE_ROOT/x";
    add(root, &draft("Vars", line)).unwrap();
    assert_eq!(read(root)["tasks"][0]["command"], line);
    // Typed into a plain task's new command, they are kept too.
    add(root, &draft("Plain", "true")).unwrap();
    update(root, "Plain", &draft("Plain", &format!("{line} again"))).unwrap();
    assert_eq!(read(root)["tasks"][1]["command"], format!("{line} again"));
    let back = detected(root, "Vars");
    assert_eq!(back.blocked, None);
    assert!(
        back.command.contains(&root.display().to_string()),
        "{}",
        back.command
    );
    // A `${env:…}` with a name the shell can't take is still refused.
    assert!(matches!(
        add(root, &draft("Bad", "echo ${env:a b}")),
        Err(WriteError::Invalid(_))
    ));
}

#[test]
fn relabel_changes_only_the_name_icon_and_detail() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let before = r#"{ "version": "2.0.0", "tasks": [
  { "label": "Build", "type": "process", "command": "${workspaceFolder}/build.sh",
    "args": ["--fast", "a b"], "options": { "env": { "A": "1" } },
    "windows": { "command": "build.cmd" }, "detail": "Old", "group": "build" }
] }"#;
    put(root, before);
    // The command given is ignored: it is the resolved one, as listed.
    let mut next = draft(" Build all ", "/abs/path/build.sh --fast 'a b'");
    next.icon = "hammer".into();
    next.detail = Some("New".into());
    relabel(root, "Build", &next).unwrap();
    assert_eq!(
        read(root)["tasks"][0],
        json!({
            "label": "Build all", "type": "process", "command": "${workspaceFolder}/build.sh",
            "args": ["--fast", "a b"], "options": { "env": { "A": "1" } },
            "windows": { "command": "build.cmd" }, "detail": "New", "group": "build",
            "icon": { "id": "hammer" },
        })
    );
    // Clearing the detail and icon removes them; names still can't clash.
    relabel(root, "Build all", &draft("Build all", "")).unwrap();
    let task = &read(root)["tasks"][0];
    assert!(task.get("detail").is_none() && task.get("icon").is_none());
    add(root, &draft("Other", "true")).unwrap();
    assert_eq!(
        relabel(root, "Other", &draft("Build all", "")),
        Err(WriteError::Duplicate("Build all".into()))
    );
    assert!(matches!(
        relabel(root, "Other", &draft(" ", "")),
        Err(WriteError::Invalid(_))
    ));
}

#[test]
fn a_new_command_for_a_task_gespan_cant_rewrite_is_refused() {
    let tasks = [
        r#"{ "label": "T", "command": "eslint", "args": ["."] }"#,
        r#"{ "label": "T", "command": "make", "osx": { "command": "gmake" } }"#,
        r#"{ "label": "T", "command": "make", "linux": { "args": ["-j"] } }"#,
        r#"{ "label": "T", "command": "${workspaceFolder}/run.sh" }"#,
        r#"{ "label": "T", "command": "echo ${workspaceFolderBasename}" }"#,
        r#"{ "label": "T", "command": "$ZED_WORKTREE_ROOT/run.sh" }"#,
    ];
    for task in tasks {
        let dir = tempfile::tempdir().unwrap();
        let text = format!(r#"{{ "version": "2.0.0", "tasks": [ {task} ] }}"#);
        put(dir.path(), &text);
        let result = update(dir.path(), "T", &draft("T", "echo new"));
        assert_eq!(result, Err(WriteError::NeedsHandEdit), "{task}");
        assert_eq!(std::fs::read_to_string(file(dir.path())).unwrap(), text);
    }
    assert_eq!(
        WriteError::NeedsHandEdit.to_string(),
        "This script uses arguments, per-OS commands or workspace variables; edit it in tasks.json"
    );
}

/// The detected action `label` from any source but Gespan's own file.
fn found_in(dir: &Path, source: &str, label: &str) -> Action {
    detect(&dir.display().to_string(), dir)
        .actions
        .into_iter()
        .find(|action| action.source == source && action.label == label)
        .unwrap()
}

#[test]
fn save_detected_spells_the_checkout_as_workspace_folder() {
    let dir = tempfile::tempdir().unwrap();
    // A folder that needs quoting, as a worktree under "My Projects" would.
    for root in [dir.path().join("plain"), dir.path().join("my repo")] {
        let root = root.as_path();
        std::fs::create_dir_all(root.join(".vscode")).unwrap();
        std::fs::write(
            root.join(".vscode/tasks.json"),
            r#"{ "version": "2.0.0", "tasks": [
                 { "label": "Web", "command": "cd ${workspaceFolder}/web && make",
                   "options": { "env": { "OUT": "${workspaceFolder}/dist" } } },
                 { "label": "Make web", "command": "make",
                   "args": ["-C", "${workspaceFolder}/web"] }
               ] }"#,
        )
        .unwrap();
        let checkout = root.display().to_string();
        for label in ["Web", "Make web"] {
            let from = found_in(root, "vscode", label);
            assert!(from.command.contains(&checkout), "{}", from.command);
            save_detected(root, &from).unwrap();

            let text = std::fs::read_to_string(file(root)).unwrap();
            assert!(text.contains("${workspaceFolder}/web"), "{text}");
            assert!(!text.contains(&checkout), "{text}");
            // It reads back as the very same command, env and folder.
            let back = detected(root, label);
            assert_eq!(back.blocked, None);
            assert_eq!(back.command, from.command);
            assert_eq!(back.env, from.env);
            assert_eq!(back.cwd, from.cwd);
        }
        assert_eq!(
            read(root)["tasks"][0]["options"]["env"]["OUT"],
            "${workspaceFolder}/dist"
        );
    }
}

#[test]
fn editing_a_renumbered_duplicate_changes_that_task() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        r#"{ "version": "2.0.0", "tasks": [
             { "label": "Build", "command": "make one" },
             { "label": "Build", "command": "make two" },
             { "label": "Other", "command": "make other" }
           ] }"#,
    );
    let second = detected(root, "Build (2)");
    assert_eq!(second.command, "make two");
    update(root, &second, &draft("Build web", "make web")).unwrap();
    let tasks = read(root)["tasks"].clone();
    assert_eq!(tasks[0]["command"], "make one");
    assert_eq!(tasks[0]["label"], "Build");
    assert_eq!(tasks[1]["label"], "Build web");
    assert_eq!(tasks[1]["command"], "make web");

    // Renaming and removing find it the same way.
    put(
        root,
        r#"{ "version": "2.0.0", "tasks": [
             { "label": "Build", "command": "make one" },
             { "label": "Build", "command": "make two" }
           ] }"#,
    );
    let second = detected(root, "Build (2)");
    relabel(root, &second, &draft("Build two", "")).unwrap();
    let tasks = read(root)["tasks"].clone();
    assert_eq!(tasks[0]["label"], "Build");
    assert_eq!(tasks[1]["label"], "Build two");
    assert_eq!(tasks[1]["command"], "make two");
    let second = detected(root, "Build two");
    remove(root, &second).unwrap();
    let tasks = read(root)["tasks"].clone();
    assert_eq!(tasks.as_array().unwrap().len(), 1);
    assert_eq!(tasks[0]["command"], "make one");

    // A file that changed since it was listed isn't edited by a stale place.
    put(
        root,
        r#"{ "version": "2.0.0", "tasks": [
             { "label": "Build", "command": "make one" },
             { "label": "Build", "command": "make two" }
           ] }"#,
    );
    let second = detected(root, "Build (2)");
    put(
        root,
        r#"{ "version": "2.0.0", "tasks": [ { "label": "Build", "command": "make one" } ] }"#,
    );
    assert!(matches!(remove(root, &second), Err(WriteError::Invalid(_))));
    assert_eq!(read(root)["tasks"][0]["command"], "make one");
}

#[test]
fn a_checkout_path_inside_a_longer_name_is_not_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let checkout = root.display().to_string();
    let sep = std::path::MAIN_SEPARATOR;
    let mut from = action(&root);
    from.label = "copy".into();
    // `repo-old` beside the checkout, then the checkout itself.
    from.command = format!("cp {checkout}-old{sep}x '{checkout}{sep}y' {checkout}");
    from.cwd = root.clone();
    from.env = vec![("OLD".into(), format!("{checkout}old"))];
    save_detected(&root, &from).unwrap();
    let saved = read(&root);
    assert_eq!(
        saved["tasks"][0]["command"],
        format!("cp {checkout}-old{sep}x '${{workspaceFolder}}{sep}y' ${{workspaceFolder}}")
    );
    assert_eq!(
        saved["tasks"][0]["options"]["env"]["OLD"],
        format!("{checkout}old")
    );
    let back = detected(&root, "copy");
    assert_eq!(back.command, from.command);
}

#[test]
fn a_numbered_label_edits_only_the_task_with_that_many_before_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        r#"{ "version": "2.0.0", "tasks": [
             { "label": "Build", "command": "make one" },
             { "label": "Build", "command": "make two" }
           ] }"#,
    );
    let second = detected(root, "Build (2)");
    // Rewritten since: a "Build" sits at the same place, but it is now the
    // first of its name, not the second.
    put(
        root,
        r#"{ "version": "2.0.0", "tasks": [
             { "label": "Other", "command": "make other" },
             { "label": "Build", "command": "make one" },
             { "label": "Build", "command": "make two" }
           ] }"#,
    );
    assert!(matches!(
        update(root, &second, &draft("Build", "make changed")),
        Err(WriteError::Invalid(_))
    ));
    let tasks = read(root)["tasks"].clone();
    assert_eq!(tasks[1]["command"], "make one");
    assert_eq!(tasks[2]["command"], "make two");
}
