use super::*;
use crate::tool::edit_preview::LineKind;
use serde_json::json;

const HOME: &str = "/Users/dev";
const SESSION: &str = "/Users/dev/code/app";

fn call(name: &str, input: Value) -> ToolCall {
    let mut call = ToolCall::new("call-1", name);
    call.input = input.to_string();
    call
}

fn view(name: &str, input: Value) -> ApprovalView {
    approval_view(Some(&call(name, input)), "", Some(SESSION), Some(HOME))
}

fn field(view: &ApprovalView, key: &str) -> Option<String> {
    view.fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| field.value.clone())
}

/// Sorted: whether arguments keep the provider's order depends on how
/// serde_json is built, which other crates in the workspace decide.
fn keys(view: &ApprovalView) -> Vec<&str> {
    let mut keys: Vec<&str> = view.fields.iter().map(|field| field.key.as_str()).collect();
    keys.sort_unstable();
    keys
}

#[test]
fn claude_bash_reads_as_its_description_command_and_folder() {
    // The request from the report: a scratch folder and a multi-line script
    // that the raw JSON showed with escaped quotes and literal `\n`.
    let script = "no=\"zero one two\".split()\ndef w(n):\n  return o[n]\nwith open(\"count.txt\",\"w\") as f:\n  f.write(w(1))";
    let command = format!(
        "cd \"/private/tmp/claude-501/-Users-dev-code-app--claude-worktrees-import-f1d65c/ce7a5034/scratchpad\" && python3 -I -c '{script}'"
    );
    let shown = view(
        "Bash",
        json!({
            "command": command,
            "description": "Write the numbers 1 to 3000 as words\nsecond line",
            "timeout": 120000,
        }),
    );
    assert_eq!(
        shown.summary.as_deref(),
        Some("Write the numbers 1 to 3000 as words")
    );
    assert_eq!(
        shown.body,
        Body::Command(format!("python3 -I -c '{script}'"))
    );
    let Body::Command(body) = &shown.body else {
        unreachable!()
    };
    assert!(body.contains("\n  return o[n]\n"), "real line breaks");
    assert!(!body.contains("\\n") && !body.contains("\\\""), "no escapes");
    assert_eq!(
        shown.cwd.as_deref(),
        Some("/private/tmp/…/ce7a5034/scratchpad")
    );
    assert_eq!(keys(&shown), ["timeout"]);
    assert_eq!(field(&shown, "timeout").as_deref(), Some("120000"));
    assert_eq!(shown.mcp, None);
}

#[test]
fn a_command_without_a_leading_cd_keeps_the_whole_command() {
    let shown = view("Bash", json!({"command": "cargo test -p gespan-core"}));
    assert_eq!(shown.body, Body::Command("cargo test -p gespan-core".into()));
    assert_eq!(shown.cwd, None);
    assert_eq!(shown.summary, None);
    assert!(shown.fields.is_empty());
}

#[test]
fn a_sandbox_override_stays_visible_as_a_field() {
    let shown = view(
        "Bash",
        json!({"command": "curl example.com", "dangerouslyDisableSandbox": true}),
    );
    assert_eq!(
        field(&shown, "dangerouslyDisableSandbox").as_deref(),
        Some("true")
    );
}

#[test]
fn codex_commands_lose_their_launcher_and_the_session_folder() {
    let mut codex = call(
        "commandExecution",
        json!({
            "command": "/bin/zsh -lc 'git status --short'",
            "cwd": SESSION,
            "commandActions": [{"type": "unknown"}],
        }),
    );
    let shown = approval_view(Some(&codex), "", Some(SESSION), Some(HOME));
    assert_eq!(shown.body, Body::Command("git status --short".into()));
    assert_eq!(shown.cwd, None, "the session's own folder is implied");
    assert!(shown.fields.is_empty(), "{:?}", shown.fields);

    codex.input = json!({"command": "ls", "cwd": "/Users/dev/other"}).to_string();
    let shown = approval_view(Some(&codex), "", Some(SESSION), Some(HOME));
    assert_eq!(shown.cwd.as_deref(), Some("~/other"));
}

#[test]
fn acp_execute_calls_from_kiro_and_cursor_read_as_commands() {
    // Kiro and Cursor name a call by its title and mark it `execute`.
    let mut kiro = call("ls -la src", json!({"command": "ls -la src"}));
    kiro.display = Some(json!({"title": "ls -la src", "kind": "execute"}));
    let shown = approval_view(Some(&kiro), "", Some(SESSION), Some(HOME));
    assert_eq!(shown.body, Body::Command("ls -la src".into()));

    let mut cursor = call("Terminal", json!({"command": "bun run check", "cwd": "/srv/app"}));
    cursor.display = Some(json!({"title": "Terminal", "kind": "execute"}));
    let shown = approval_view(Some(&cursor), "", Some(SESSION), Some(HOME));
    assert_eq!(shown.body, Body::Command("bun run check".into()));
    assert_eq!(shown.cwd.as_deref(), Some("/srv/app"));
}

#[test]
fn edits_show_the_file_and_its_change() {
    let shown = view(
        "Edit",
        json!({
            "file_path": "/Users/dev/code/app/src/main.rs",
            "old_string": "let a = 1;",
            "new_string": "let a = 2;\nlet b = 3;",
            "replace_all": false,
        }),
    );
    let Body::Change { path, preview } = &shown.body else {
        panic!("{:?}", shown.body)
    };
    assert_eq!(path, "src/main.rs");
    let preview = preview.as_ref().expect("the edit carries its change");
    assert_eq!((preview.additions, preview.deletions), (2, 1));
    assert!(preview
        .lines
        .iter()
        .any(|line| line.kind == LineKind::Del && line.text == "let a = 1;"));
    assert_eq!(keys(&shown), ["replace_all"], "texts are in the diff");
}

#[test]
fn writes_preview_their_content_and_paths_outside_the_session_use_home() {
    let shown = view(
        "Write",
        json!({"file_path": "/Users/dev/notes/todo.md", "content": "# Todo\n- ship"}),
    );
    let Body::Change { path, preview } = &shown.body else {
        panic!("{:?}", shown.body)
    };
    assert_eq!(path, "~/notes/todo.md");
    let preview = preview.as_ref().unwrap();
    assert!(preview.content_only);
    assert_eq!(preview.lines[0].text, "# Todo");
    assert!(shown.fields.is_empty());
}

#[test]
fn reads_and_searches_show_their_path_or_pattern() {
    let read = view(
        "Read",
        json!({"file_path": "/Users/dev/code/app/README.md", "offset": 10}),
    );
    assert_eq!(
        read.body,
        Body::Target {
            verb: "Read".into(),
            target: "README.md".into()
        }
    );
    assert_eq!(keys(&read), ["offset"]);

    let grep = view(
        "Grep",
        json!({"pattern": "fn approval", "path": "/Users/dev/code/app/crates"}),
    );
    assert_eq!(
        grep.body,
        Body::Target {
            verb: "Find".into(),
            target: "fn approval".into()
        }
    );
    assert_eq!(keys(&grep), ["path"], "the folder searched stays visible");
    assert_eq!(field(&grep, "path").as_deref(), Some("crates"), "and reads short");

    let glob = view(
        "Glob",
        json!({"pattern": "**/*.rs", "path": "/Users/dev/code/app"}),
    );
    assert_eq!(
        glob.body,
        Body::Target {
            verb: "List".into(),
            target: "**/*.rs".into()
        }
    );
}

#[test]
fn web_fetch_shows_the_url_and_its_prompt() {
    let shown = view(
        "WebFetch",
        json!({"url": "https://docs.rs/gpui", "prompt": "Summarize the API"}),
    );
    assert_eq!(shown.body, Body::Url("https://docs.rs/gpui".into()));
    assert_eq!(field(&shown, "prompt").as_deref(), Some("Summarize the API"));
    assert_eq!(keys(&shown), ["prompt"]);
}

#[test]
fn mcp_tools_name_their_server_and_list_arguments_for_every_provider() {
    let input = json!({
        "title": "Crash on start",
        "labels": ["bug", "p1"],
        "body": "Steps:\n1. open\n2. crash",
        "command": "not a shell command",
    });
    for name in [
        "mcp__github_tools__create_issue",
        "github_tools:create_issue",
        "@github_tools/create_issue",
    ] {
        let shown = view(name, input.clone());
        assert_eq!(
            shown.mcp,
            Some(McpTool {
                server: "github_tools".into(),
                tool: "create_issue".into()
            }),
            "{name}"
        );
        assert_eq!(shown.body, Body::None, "{name}: an argument named command");
        assert_eq!(keys(&shown), ["body", "command", "labels", "title"]);
        assert_eq!(field(&shown, "labels").as_deref(), Some("bug, p1"));
        assert_eq!(
            field(&shown, "body").as_deref(),
            Some("Steps:\n1. open\n2. crash")
        );
    }
}

#[test]
fn tool_names_that_only_look_like_mcp_are_not_split() {
    assert_eq!(mcp_tool("Bash"), None);
    assert_eq!(mcp_tool("Run: ls -la"), None);
    assert_eq!(mcp_tool("https://example.com"), None);
    assert_eq!(mcp_tool("mcp__onlyserver"), None);
    assert_eq!(mcp_tool(":tool"), None);
}

#[test]
fn unknown_tools_fall_back_to_readable_arguments() {
    let shown = view(
        "NotebookEdit",
        json!({"cell": {"id": 3, "source": "print(1)\nprint(2)"}, "notebook_path": "/tmp/a.ipynb"}),
    );
    assert_eq!(shown.body, Body::None);
    assert_eq!(keys(&shown), ["cell", "notebook_path"]);
    assert_eq!(field(&shown, "notebook_path").as_deref(), Some("/tmp/a.ipynb"));
    let inside = view("NotebookEdit", json!({"notebook_path": "/Users/dev/code/app/a.ipynb", "note": "/not/a/path field"}));
    assert_eq!(field(&inside, "notebook_path").as_deref(), Some("a.ipynb"));
    assert_eq!(field(&inside, "note").as_deref(), Some("/not/a/path field"), "only path arguments");
    assert_eq!(
        field(&shown, "cell").as_deref(),
        Some("id: 3\nsource:\n  print(1)\n  print(2)")
    );

    let text = approval_view(
        Some(&ToolCall {
            input: "plain words, not JSON".into(),
            ..ToolCall::new("x", "Mystery")
        }),
        "",
        None,
        None,
    );
    assert_eq!(text.body, Body::Text("plain words, not JSON".into()));
}

#[test]
fn desktop_envelopes_show_the_raw_input_not_the_envelope() {
    let shown = view(
        "Mystery",
        json!({"__desktop": {"title": "Mystery"}, "rawInput": {"answer": 42}}),
    );
    assert_eq!(keys(&shown), ["answer"]);
}

#[test]
fn text_only_approvals_unescape_json_and_keep_plain_text() {
    let json_detail = approval_view(
        None,
        "{\n  \"permission\": \"bash\",\n  \"patterns\": [\"git *\"]\n}",
        None,
        None,
    );
    assert_eq!(keys(&json_detail), ["patterns", "permission"]);
    assert_eq!(field(&json_detail, "patterns").as_deref(), Some("git *"));

    let quoted = approval_view(None, "\"line one\\nline two\"", None, None);
    assert_eq!(quoted.body, Body::Text("line one\nline two".into()));

    let plain = approval_view(None, "  The agent wants to continue.\r\n ", None, None);
    assert_eq!(plain.body, Body::Text("The agent wants to continue.".into()));

    let empty = approval_view(None, "   ", None, None);
    assert_eq!((empty.body, empty.fields.len()), (Body::None, 0));
}

#[test]
fn pretty_prints_nested_values_without_quotes_or_escapes() {
    // In alphabetical order, so the result is the same with or without
    // serde_json's preserve_order.
    let value = json!({
        "empty": [],
        "flag": true,
        "list": [{"a": 1}, "two"],
        "nothing": null,
        "text": "say \"hi\"",
    });
    assert_eq!(
        pretty(&value, 0),
        "empty: []\nflag: true\nlist:\n  - a: 1\n  - two\nnothing: null\ntext: say \"hi\""
    );
}

#[test]
fn leading_cd_splits_only_a_plain_cd_joined_by_and() {
    assert_eq!(
        split_leading_cd("cd \"/a b/c\" && make test"),
        Some(("/a b/c".into(), "make test"))
    );
    assert_eq!(
        split_leading_cd("cd /x && a && b"),
        Some(("/x".into(), "a && b"))
    );
    assert_eq!(
        split_leading_cd("cd 'it''s' && ls"),
        Some(("its".into(), "ls"))
    );
    for kept in [
        "cd /x; rm -rf build",
        "cd /x || exit 1",
        "cd /x &",
        "cd /x && ",
        "cd /x y && ls",
        "cd $(pwd) && ls",
        "cd /x\nls",
        "cdx /x && ls",
        "echo cd /x && ls",
        "cd \"/unterminated && ls",
    ] {
        assert_eq!(split_leading_cd(kept), None, "{kept:?}");
    }
}

#[test]
fn short_paths_are_relative_home_based_or_cut_in_the_middle() {
    let short = |path| short_path(path, Some(SESSION), Some(HOME));
    assert_eq!(short("/Users/dev/code/app/src/lib.rs"), "src/lib.rs");
    assert_eq!(short("/Users/dev/code/app"), ".");
    assert_eq!(short("/Users/dev/.zshrc"), "~/.zshrc");
    assert_eq!(short("/Users/dev"), "~");
    assert_eq!(short("/Users/devops/x"), "/Users/devops/x", "not a home prefix");
    assert_eq!(short("file:///etc/hosts"), "/etc/hosts");
    let long = "/var/folders/xy/very-long-generated-folder-name-0123456789/T/build/out.log";
    let cut = short(long);
    assert!(cut.chars().count() <= MAX_PATH_CHARS, "{cut}");
    assert!(cut.starts_with("/var/") && cut.ends_with("build/out.log"), "{cut}");
    // Long middle parts: the root stays, so the path still says where it is.
    let scratch = "/private/tmp/claude-501/-private-tmp-claude-501--Users-dev-Developer-work-app--worktrees-f1d65c/ce7a5034-9016-4e32-a664-46eee6b1abea/scratchpad";
    assert_eq!(short(scratch), "/private/tmp/…/scratchpad");
    let one_part = format!("/{}", "x".repeat(80));
    let cut = short(&one_part);
    assert_eq!(cut.chars().count(), MAX_PATH_CHARS - 1);
    assert!(cut.contains('…'));
}

#[test]
fn collapse_keeps_short_text_and_counts_what_it_hides() {
    assert_eq!(collapse("one\ntwo", 3, 80), None);
    let many = (1..=10).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
    let cut = collapse(&many, 4, 80).unwrap();
    assert_eq!(cut.text, "line 1\nline 2\nline 3\nline 4");
    assert_eq!(cut.hidden_lines, 6);

    let wide = format!("short\n{}", "y".repeat(30));
    let cut = collapse(&wide, 4, 10).unwrap();
    assert_eq!(cut.text, format!("short\n{}…", "y".repeat(9)));
    assert_eq!(cut.hidden_lines, 1, "a cut line counts as hidden");

    let multibyte = "é".repeat(12);
    let cut = collapse(&multibyte, 4, 10).unwrap();
    assert_eq!(cut.text.chars().count(), 10, "cut by characters, not bytes");
}
