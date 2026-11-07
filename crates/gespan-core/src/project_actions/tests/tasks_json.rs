use super::super::jsonc::strip;
use super::super::tasks_json::{parse, parse_for, resolve};
use super::super::{Action, Ctx, Found};
use std::path::{Path, PathBuf};

const EXAMPLE: &str = r#"{
  "version": "2.0.0",
  "tasks": [
    { "label": "Dev server", "command": "bun run dev", "isBackground": true,
      "detail": "Starts the web app on :5173", "icon": { "id": "rocket" } },
    { "label": "Test", "command": "cargo", "args": ["test", "--workspace"],
      "icon": { "id": "test-tube" } }
  ]
}"#;

fn checkout() -> PathBuf {
    std::env::temp_dir().join("gespan-actions-checkout")
}
fn ctx(checkout: &Path) -> Ctx<'_> {
    Ctx {
        checkout,
        package_manager: "npm",
    }
}
fn one(text: &str) -> Found {
    let checkout = checkout();
    let mut found = parse_for("linux", &ctx(&checkout), text).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    found.remove(0)
}
fn resolved(found: Found) -> Action {
    let checkout = checkout();
    resolve(&ctx(&checkout), "gespan", ".gespan/tasks.json", found)
}
fn task(body: &str) -> String {
    format!(r#"{{ "version": "2.0.0", "tasks": [ {body} ] }}"#)
}

#[test]
fn parses_label_command_args_cwd_env_detail_background_icon() {
    let checkout = checkout();
    let found = parse(&ctx(&checkout), EXAMPLE).unwrap();
    assert_eq!(found.len(), 2);
    let dev = &found[0];
    assert_eq!(dev.label, "Dev server");
    assert_eq!(dev.command, "bun run dev");
    assert!(dev.background);
    assert_eq!(dev.icon.as_deref(), Some("rocket"));
    assert_eq!(dev.detail.as_deref(), Some("Starts the web app on :5173"));
    assert!(dev.shell);
    let test = &found[1];
    assert_eq!(test.label, "Test");
    assert_eq!(test.command, "cargo");
    assert_eq!(test.args, ["test", "--workspace"]);
    assert!(!test.background);
    assert_eq!(test.icon.as_deref(), Some("test-tube"));

    let with_options = one(&task(
        r#"{ "label": "Web", "command": "bun run dev",
             "options": { "cwd": "web", "env": { "PORT": "5173" } } }"#,
    ));
    assert_eq!(with_options.cwd.as_deref(), Some("web"));
    assert_eq!(with_options.env, [("PORT".to_string(), "5173".to_string())]);

    let action = resolved(found[1].clone());
    assert_eq!(action.id, "gespan:Test");
    assert_eq!(action.source, "gespan");
    assert_eq!(action.file, ".gespan/tasks.json");
    assert_eq!(action.command, "cargo test --workspace");
    assert_eq!(action.cwd, checkout);
    assert_eq!(action.icon, "test-tube");
    assert_eq!(action.blocked, None);
}

#[test]
fn jsonc_comments_and_trailing_commas_are_accepted() {
    let text = r#"{
  // Shared actions.
  "version": "2.0.0",
  /* Two of them,
     one block comment. */
  "tasks": [
    { "label": "Lint", "command": "echo // not a comment", },
    { "label": "Quote", "command": "echo \"/* still a string */\"", },
  ],
}"#;
    let stripped = strip(text);
    assert!(stripped.had_comments);
    let checkout = checkout();
    let found = parse(&ctx(&checkout), text).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].command, "echo // not a comment");
    assert_eq!(found[1].command, "echo \"/* still a string */\"");

    let plain = strip(EXAMPLE);
    assert!(!plain.had_comments);
    // Trailing commas alone aren't comments: saving can still rewrite the file.
    assert!(!strip(r#"{ "tasks": [1,], }"#).had_comments);
    assert!(parse(&ctx(&checkout), "{ \"tasks\": [ ").is_err());
}

#[test]
fn os_overrides_replace_command_and_options() {
    let text = task(
        r#"{ "label": "Clean", "command": "rm -rf out", "args": ["-v"],
             "options": { "cwd": "web" },
             "windows": { "command": "rmdir /s /q out", "args": [],
                          "options": { "cwd": "win", "env": { "A": "1" } } },
             "osx": { "command": "trash out" } }"#,
    );
    let checkout = checkout();
    let windows = parse_for("windows", &ctx(&checkout), &text).unwrap();
    assert_eq!(windows[0].command, "rmdir /s /q out");
    assert!(windows[0].args.is_empty());
    assert_eq!(windows[0].cwd.as_deref(), Some("win"));
    assert_eq!(windows[0].env, [("A".to_string(), "1".to_string())]);

    let osx = parse_for("osx", &ctx(&checkout), &text).unwrap();
    assert_eq!(osx[0].command, "trash out");
    assert_eq!(osx[0].args, ["-v"], "args not overridden stay");
    assert_eq!(osx[0].cwd.as_deref(), Some("web"));

    let linux = parse_for("linux", &ctx(&checkout), &text).unwrap();
    assert_eq!(linux[0].command, "rm -rf out");
}

#[test]
fn npm_type_uses_the_detected_package_manager() {
    let checkout = checkout();
    let bun = Ctx {
        checkout: &checkout,
        package_manager: "bun",
    };
    let text = task(r#"{ "type": "npm", "script": "test", "label": "Unit tests" }"#);
    let found = parse_for("linux", &bun, &text).unwrap();
    assert_eq!(found[0].command, "bun run test");
    assert_eq!(found[0].label, "Unit tests");

    // VS Code names an unlabelled npm task after its script.
    let unlabelled = parse_for(
        "linux",
        &bun,
        &task(r#"{ "type": "npm", "script": "lint" }"#),
    );
    assert_eq!(unlabelled.unwrap()[0].label, "npm: lint");
}

#[test]
fn unsupported_variable_blocks_with_reason() {
    let action = resolved(one(&task(
        r#"{ "label": "Format", "command": "prettier", "args": ["${file}"] }"#,
    )));
    let blocked = action.blocked.expect("blocked");
    assert!(blocked.contains("${file}"), "{blocked}");
    assert_eq!(blocked, "Uses ${file}, which Gespan can't fill in");

    let in_cwd = resolved(one(&task(
        r#"{ "label": "Here", "command": "ls", "options": { "cwd": "${fileDirname}" } }"#,
    )));
    assert!(in_cwd.blocked.unwrap().contains("${fileDirname}"));
}

#[test]
fn supported_variables_are_filled() {
    let root = if cfg!(windows) {
        r"C:\work\app"
    } else {
        "/work/app"
    };
    let fixed = Ctx {
        checkout: Path::new(root),
        package_manager: "npm",
    };
    let resolve_fixed = |body: &str| {
        let found = parse_for("linux", &fixed, &task(body)).unwrap().remove(0);
        resolve(&fixed, "gespan", ".gespan/tasks.json", found)
    };

    let args = resolve_fixed(
        r#"{ "label": "Args", "command": "echo",
             "args": ["${workspaceFolder}/x", "${workspaceFolderBasename}", "$ZED_WORKTREE_ROOT",
                      "${env:HOME}/bin", "${env:GESPAN_X}"],
             "options": { "env": { "H": "${env:HOME}" } } }"#,
    );
    assert_eq!(args.blocked, None);
    // `${env:…}` is left for the shell, so a listing never shows the host's values.
    #[cfg(unix)]
    {
        assert_eq!(
            args.command,
            r#"echo /work/app/x app /work/app "${HOME}"/bin "${GESPAN_X}""#
        );
        assert_eq!(args.env, [("H".to_string(), "${HOME}".to_string())]);
    }
    #[cfg(windows)]
    {
        assert_eq!(
            args.command,
            r#"echo C:\work\app/x app C:\work\app "%HOME%/bin" "%GESPAN_X%""#
        );
        assert_eq!(args.env, [("H".to_string(), "%HOME%".to_string())]);
    }

    let line = resolve_fixed(
        r#"{ "label": "Line", "command": "echo ${env:HOME} ${workspaceFolderBasename} $ZED_WORKTREE_ROOT" }"#,
    );
    #[cfg(unix)]
    assert_eq!(line.command, "echo ${HOME} app /work/app");
    #[cfg(windows)]
    assert_eq!(line.command, r"echo %HOME% app C:\work\app");

    for name in ["A-B", "", "1X", "PATH)"] {
        let bad = resolve_fixed(&format!(
            r#"{{ "label": "Bad", "command": "echo ${{env:{name}}}" }}"#
        ));
        assert_eq!(
            bad.blocked,
            Some(format!(
                "Uses ${{env:{name}}} with a name Gespan can't pass to the shell"
            ))
        );
    }

    // A folder isn't read by the shell, so an env variable there can't be filled.
    let cwd = resolve_fixed(
        r#"{ "label": "Cwd", "command": "ls", "options": { "cwd": "${env:HOME}" } }"#,
    );
    assert_eq!(
        cwd.blocked.as_deref(),
        Some("Uses ${env:HOME}, which Gespan can't fill in")
    );
}

#[test]
fn args_are_quoted_for_the_shell() {
    let action = resolved(one(&task(
        r#"{ "label": "Say", "command": "echo", "args": ["a b", "plain"] }"#,
    )));
    #[cfg(unix)]
    assert_eq!(action.command, "echo 'a b' plain");
    #[cfg(windows)]
    assert_eq!(action.command, r#"echo "a b" plain"#);

    let empty = resolved(one(&task(
        r#"{ "label": "Empty", "command": "echo", "args": [""] }"#,
    )));
    #[cfg(unix)]
    assert_eq!(empty.command, "echo ''");
    #[cfg(windows)]
    assert_eq!(empty.command, r#"echo """#);

    let quote = resolved(one(&task(
        r#"{ "label": "Quote", "command": "echo", "args": ["say \"hi\" & bye"] }"#,
    )));
    #[cfg(unix)]
    assert_eq!(quote.command, r#"echo 'say "hi" & bye'"#);
    #[cfg(windows)]
    assert_eq!(quote.command, r#"echo "say ""hi"" & bye""#);

    // A `process` task's program is an argument too, so a spaced path stays whole.
    let process = resolved(one(&task(
        r#"{ "label": "Tool", "type": "process", "command": "my tool", "args": ["x"] }"#,
    )));
    #[cfg(unix)]
    assert_eq!(process.command, "'my tool' x");
    #[cfg(windows)]
    assert_eq!(process.command, r#""my tool" x"#);
}

#[test]
fn a_cwd_outside_the_checkout_is_blocked() {
    let checkout = checkout();
    let inside = resolved(one(&task(
        r#"{ "label": "In", "command": "ls", "options": { "cwd": "web/../api" } }"#,
    )));
    assert_eq!(inside.blocked, None);
    assert_eq!(inside.cwd, checkout.join("api"));

    let filled = resolved(one(&task(
        r#"{ "label": "Var", "command": "ls", "options": { "cwd": "${workspaceFolder}/web" } }"#,
    )));
    assert_eq!(filled.blocked, None);
    assert_eq!(filled.cwd, checkout.join("web"));

    let up = resolved(one(&task(
        r#"{ "label": "Up", "command": "ls", "options": { "cwd": "../.." } }"#,
    )));
    assert_eq!(up.blocked.as_deref(), Some("Runs outside the project"));

    let elsewhere = std::env::temp_dir().join("somewhere-else");
    let absolute = serde_json::json!({
        "tasks": [{ "label": "Abs", "command": "ls", "options": { "cwd": elsewhere } }]
    });
    let abs = resolved(one(&absolute.to_string()));
    assert_eq!(abs.blocked.as_deref(), Some("Runs outside the project"));

    // A sibling whose name starts with the checkout's isn't inside it.
    let sibling = format!("{}-other", checkout.display());
    let sibling = serde_json::json!({
        "tasks": [{ "label": "Sib", "command": "ls", "options": { "cwd": sibling } }]
    });
    let sib = resolved(one(&sibling.to_string()));
    assert_eq!(sib.blocked.as_deref(), Some("Runs outside the project"));
}

#[test]
fn depends_on_only_is_blocked_and_with_command_gets_a_note() {
    let only = one(&task(
        r#"{ "label": "All", "dependsOn": ["Build", "Test"] }"#,
    ));
    assert_eq!(
        only.blocked.as_deref(),
        Some("Only runs other tasks, which Gespan doesn't do yet")
    );
    let action = resolved(only);
    assert_eq!(
        action.blocked.as_deref(),
        Some("Only runs other tasks, which Gespan doesn't do yet")
    );

    let with_command = one(&task(
        r#"{ "label": "Ship", "command": "make ship", "dependsOn": "Build" }"#,
    ));
    assert_eq!(with_command.blocked, None);
    assert_eq!(
        with_command.note.as_deref(),
        Some("Dependencies aren't run")
    );
    assert_eq!(
        resolved(with_command).note.as_deref(),
        Some("Dependencies aren't run")
    );

    let plain = one(&task(r#"{ "label": "Plain", "command": "make" }"#));
    assert_eq!(plain.note, None);
}

#[test]
fn hidden_tasks_are_marked() {
    let found = parse_for(
        "linux",
        &ctx(&checkout()),
        &task(
            r#"{ "label": "Secret", "command": "x", "hide": true },
               { "label": "Shown", "command": "y" }"#,
        ),
    )
    .unwrap();
    assert!(found[0].hide);
    assert!(!found[1].hide);
}

#[test]
fn unknown_icon_falls_back_to_play() {
    let unknown = resolved(one(&task(
        r#"{ "label": "Odd", "command": "x", "icon": { "id": "flame" } }"#,
    )));
    assert_eq!(unknown.icon, "play");
    let none = resolved(one(&task(r#"{ "label": "None", "command": "x" }"#)));
    assert_eq!(none.icon, "play");
    let known = resolved(one(&task(
        r#"{ "label": "Bug", "command": "x", "icon": { "id": "bug" } }"#,
    )));
    assert_eq!(known.icon, "bug");
}

#[test]
fn a_task_without_a_command_is_blocked() {
    for body in [
        r#"{ "label": "Gulp", "type": "gulp", "task": "build" }"#,
        r#"{ "label": "Tsc", "type": "typescript", "tsconfig": "tsconfig.json" }"#,
        r#"{ "label": "Blank", "command": "  " }"#,
    ] {
        let found = one(&task(body));
        assert_eq!(
            found.blocked.as_deref(),
            Some("Has no command Gespan can run")
        );
        let action = resolved(found);
        assert_eq!(
            action.blocked.as_deref(),
            Some("Has no command Gespan can run")
        );
    }
    // The dependsOn reason is the more useful one, so it isn't replaced.
    let only = one(&task(r#"{ "label": "All", "dependsOn": ["Build"] }"#));
    assert_eq!(
        only.blocked.as_deref(),
        Some("Only runs other tasks, which Gespan doesn't do yet")
    );
    // An npm task's command comes from its script.
    assert_eq!(
        one(&task(r#"{ "type": "npm", "script": "test" }"#)).blocked,
        None
    );
}

#[test]
fn a_workspace_path_with_spaces_stays_one_word() {
    let checkout = std::env::temp_dir()
        .join("Application Support")
        .join("my app");
    let ctx = ctx(&checkout);
    let line = |command: &str| {
        let found = Found {
            label: "Run".into(),
            command: command.into(),
            shell: true,
            ..Found::default()
        };
        resolve(&ctx, "gespan", ".gespan/tasks.json", found).command
    };
    let root = checkout.display().to_string();
    let quoted = line("${workspaceFolder}/bin/run --fast");
    let zed = line("$ZED_WORKTREE_ROOT/bin/run");
    let base = line("echo ${workspaceFolderBasename}");
    #[cfg(not(windows))]
    {
        assert_eq!(quoted, format!("'{root}'/bin/run --fast"));
        assert_eq!(zed, format!("'{root}'/bin/run"));
        assert_eq!(base, "echo 'my app'");
    }
    #[cfg(windows)]
    {
        assert_eq!(quoted, format!("\"{root}\"/bin/run --fast"));
        assert_eq!(zed, format!("\"{root}\"/bin/run"));
        assert_eq!(base, "echo \"my app\"");
    }
    // A path with nothing to quote stays as written.
    let plain = std::env::temp_dir().join("plain");
    let found = Found {
        label: "Run".into(),
        command: "ls ${workspaceFolder}".into(),
        shell: true,
        ..Found::default()
    };
    let action = resolve(
        &super::super::Ctx {
            checkout: &plain,
            package_manager: "npm",
        },
        "gespan",
        "f",
        found,
    );
    assert_eq!(action.command, format!("ls {}", plain.display()));
}
