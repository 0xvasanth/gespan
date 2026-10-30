use super::*;
use crate::tool::ToolState;
use serde_json::json;

fn call(name: &str, input: Value) -> ToolCall {
    let mut call = ToolCall::new("t", name);
    call.input = input.to_string();
    call
}

fn kinds(preview: &EditPreview) -> Vec<LineKind> {
    preview.lines.iter().map(|line| line.kind).collect()
}

#[test]
fn a_claude_edit_previews_its_replacement_without_line_numbers() {
    let preview = call(
        "Edit",
        json!({"file_path":"/repo/src/a.rs","old_string":"let a = 1;\nlet b = 2;\nlet c = 3;","new_string":"let a = 1;\nlet b = 20;\nlet c = 3;"}),
    )
    .edit_preview()
    .unwrap();
    assert_eq!(
        kinds(&preview),
        [
            LineKind::Context,
            LineKind::Del,
            LineKind::Add,
            LineKind::Context
        ]
    );
    assert_eq!(preview.lines[2].text, "let b = 20;");
    assert!(
        preview.lines.iter().all(|line| line.number.is_none()),
        "an excerpt"
    );
    assert_eq!((preview.additions, preview.deletions), (1, 1));
    assert!(preview.shows_change());
}

#[test]
fn a_write_shows_its_first_lines_as_content_not_additions() {
    let content: String = (1..=10).map(|n| format!("line {n}\r\n")).collect();
    let preview = call(
        "Write",
        json!({"file_path":"/repo/notes.md","content":content}),
    )
    .edit_preview()
    .unwrap();
    assert!(preview.content_only);
    assert_eq!(preview.lines.len(), MAX_LINES);
    assert_eq!(preview.lines[0].text, "line 1", "CRLF is not shown");
    assert_eq!(preview.lines[5].number, Some(6));
    assert!(kinds(&preview)
        .iter()
        .all(|kind| *kind == LineKind::Context));
    assert_eq!((preview.additions, preview.deletions), (0, 0));

    let empty = call("Write", json!({"file_path":"/repo/empty","content":""}))
        .edit_preview()
        .unwrap();
    assert!(empty.content_only && empty.lines.is_empty() && empty.shows_change());
}

#[test]
fn multi_edit_and_acp_argument_shapes_preview_their_first_edit() {
    let multi = call(
        "MultiEdit",
        json!({"file_path":"/repo/a.rs","edits":[{"old_string":"a","new_string":"b"},{"old_string":"c","new_string":"d"}]}),
    );
    let mut acp = call(
        "Edit src/a.rs",
        json!({"path":"src/a.rs","oldText":"x","newText":"y"}),
    );
    acp.display = Some(json!({"kind":"edit","title":"Edit src/a.rs"}));
    for (call, removed, added) in [(multi, "a", "b"), (acp, "x", "y")] {
        let preview = call
            .edit_preview()
            .unwrap_or_else(|| panic!("{}", call.name));
        assert_eq!(preview.lines[0].text, removed);
        assert_eq!(preview.lines[1].text, added);
    }
}

#[test]
fn only_edit_and_write_calls_have_a_preview() {
    let read = call("Read", json!({"file_path":"/repo/a.rs","content":"x"}));
    let bash = call(
        "Bash",
        json!({"command":"sed -i s/a/b/ a.rs","old_string":"a","new_string":"b"}),
    );
    let edit_without_change = call("Edit", json!({"file_path":"/repo/a.rs"}));
    // An Edit's content is not a Write's whole file.
    let edit_with_content = call("Edit", json!({"file_path":"/repo/a.rs","content":"x"}));
    for call in [read, bash, edit_without_change, edit_with_content] {
        assert_eq!(call.edit_preview(), None, "{}", call.input);
    }
    let unchanged = call(
        "Edit",
        json!({"file_path":"/a","old_string":"same","new_string":"same"}),
    )
    .edit_preview()
    .unwrap();
    assert!(!unchanged.shows_change(), "nothing to show");
}

#[test]
fn a_long_edit_starts_just_before_its_first_change_and_counts_all_of_it() {
    let old: String = (1..=40).map(|n| format!("{n}\n")).collect();
    let new = old
        .replace("20\n", "twenty\n")
        .replace("35\n", "thirty-five\n");
    let preview = EditPreview::from_texts(Some(&old), &new);
    assert_eq!(preview.lines.len(), MAX_LINES);
    assert_eq!(preview.lines[0].text, "19");
    assert_eq!(preview.lines[0].number, Some(19));
    assert_eq!(preview.lines[1].kind, LineKind::Del);
    assert_eq!((preview.additions, preview.deletions), (2, 2));
}

#[test]
fn a_new_file_is_all_additions_and_long_lines_are_capped() {
    let long = "x".repeat(300);
    let preview = EditPreview::from_texts(None, &format!("{long}\nshort\n"));
    assert_eq!(kinds(&preview), [LineKind::Add, LineKind::Add]);
    assert_eq!(preview.lines[0].text.chars().count(), MAX_LINE_CHARS);
    assert!(preview.lines[0].text.ends_with('…'));
    assert_eq!(preview.lines[1].number, Some(2));
    // Multibyte text is cut on a character, never inside one.
    let wide = EditPreview::from_texts(None, &"é".repeat(200));
    assert!(wide.lines[0].text.ends_with('…'));
}

#[test]
fn a_git_patch_keeps_new_file_numbers_and_skips_headers() {
    let patch = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -10,3 +10,3 @@ fn main\n keep\n-old\n+new\n\\ No newline at end of file\n";
    let preview = EditPreview::from_patch(patch).unwrap();
    let shown: Vec<_> = preview
        .lines
        .iter()
        .map(|line| (line.number, line.kind, line.text.as_str()))
        .collect();
    assert_eq!(
        shown[..3],
        [
            (Some(10), LineKind::Context, "keep"),
            (Some(11), LineKind::Del, "old"),
            (Some(11), LineKind::Add, "new"),
        ]
    );
    assert_eq!((preview.additions, preview.deletions), (1, 1));
    assert_eq!(EditPreview::from_patch("plain text"), None);
}

#[test]
fn a_saved_preview_wins_and_survives_a_round_trip() {
    let mut edit = call("fileChange", json!({"path":"/repo/a.rs"}));
    edit.state = ToolState::Completed;
    let built = EditPreview::from_texts(Some("a\n"), "b\n");
    edit.display = Some(json!({ DISPLAY_KEY: built }));
    let saved = serde_json::to_string(&edit).unwrap();
    let loaded: ToolCall = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded.edit_preview(), Some(built));
}
