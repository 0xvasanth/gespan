use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use super::*;
use crate::outside_turns::TurnRole;

fn user(uuid: &str, parent: Option<&str>, text: &str) -> String {
    serde_json::json!({
        "type": "user", "uuid": uuid, "parentUuid": parent,
        "cwd": "/tmp/p", "gitBranch": "main",
        "message": {"role": "user", "content": text},
    })
    .to_string()
}

fn assistant(uuid: &str, parent: &str, text: &str) -> String {
    serde_json::json!({
        "type": "assistant", "uuid": uuid, "parentUuid": parent,
        "message": {"role": "assistant", "id": uuid,
                    "content": [{"type": "text", "text": text}]},
    })
    .to_string()
}

fn snapshot() -> String {
    r#"{"type":"file-history-snapshot","messageId":"m","snapshot":{}}"#.to_string()
}

fn write(home: &Path, rel: &str, lines: &[String]) -> PathBuf {
    let path = home.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    path
}

fn set_mtime(path: &Path, secs: u64) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

#[test]
fn claude_lists_title_cwd_branch_and_id() {
    let home = tempfile::tempdir().unwrap();
    let path = write(
        home.path(),
        "projects/-tmp-p/abc.jsonl",
        &[
            snapshot(),
            r#"{"type":"user","cwd":"/tmp/p","gitBranch":"main","message":{"role":"user","content":"Fix the login redirect\nmore"},"uuid":"u1"}"#.to_string(),
            assistant("a1", "u1", "On it."),
        ],
    );
    let found = list(Kind::Claude, home.path());
    assert_eq!(found.len(), 1);
    let c = &found[0];
    assert_eq!(c.kind, Kind::Claude);
    assert_eq!(c.provider_id, "abc");
    assert_eq!(c.cwd, "/tmp/p");
    assert_eq!(c.branch.as_deref(), Some("main"));
    assert_eq!(c.title, "Fix the login redirect");
    assert_eq!(c.path, path);
    assert_eq!(c.messages, None);
}

#[test]
fn claude_title_skips_slash_command_prompts() {
    let home = tempfile::tempdir().unwrap();
    write(
        home.path(),
        "projects/-tmp-p/abc.jsonl",
        &[
            user(
                "u1",
                None,
                "<command-name>/clear</command-name>\n<command-message>clear</command-message>",
            ),
            assistant("a1", "u1", "Cleared."),
            user("u2", Some("a1"), "Real question"),
            assistant("a2", "u2", "Answer."),
        ],
    );
    let found = list(Kind::Claude, home.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "Real question");
    assert_eq!(found[0].cwd, "/tmp/p");
}

#[test]
fn claude_title_drops_a_leading_gespan_context_block() {
    let home = tempfile::tempdir().unwrap();
    write(
        home.path(),
        "projects/-tmp-p/abc.jsonl",
        &[
            user(
                "u1",
                None,
                "<gespan_context>\nGespan adds this context.\n\n<soul>\nbe kind\n</soul>\n</gespan_context>\n\nShip the release notes\nand more",
            ),
            assistant("a1", "u1", "Done."),
        ],
    );
    let found = list(Kind::Claude, home.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "Ship the release notes");
}

#[test]
fn title_of_keeps_prompts_without_a_closed_gespan_context() {
    assert_eq!(title_of("  Plain prompt\nmore"), "Plain prompt");
    assert_eq!(
        title_of("<gespan_context>ctx</gespan_context>Same line"),
        "Same line"
    );
    assert_eq!(
        title_of("Quote <gespan_context>x</gespan_context> later"),
        "Quote <gespan_context>x</gespan_context> later",
        "only a leading block is Gespan's"
    );
    assert_eq!(
        title_of("<gespan_context>\nnever closed"),
        "<gespan_context>",
        "an unclosed block is left as written"
    );
    assert_eq!(title_of("<gespan_context>ctx</gespan_context>\n\n"), "");
}

#[test]
fn title_of_skips_each_leading_wrapper_block() {
    assert_eq!(
        title_of("<system-reminder>\nhook context\n</system-reminder>\nFix the build\nmore"),
        "Fix the build"
    );
    assert_eq!(
        title_of("<gespan_browser>\nUse your tabs.\n</gespan_browser>\n\nOpen the docs"),
        "Open the docs"
    );
    assert_eq!(
        title_of("<gespan_context>\nYou are Ada.\n</gespan_context>\n\nPlan the week"),
        "Plan the week"
    );
}

#[test]
fn title_of_skips_stacked_wrappers_in_any_order() {
    assert_eq!(
        title_of(
            "  <gespan_context>a</gespan_context>\n\n<gespan_browser>b</gespan_browser>\n\n \
             <system-reminder>c</system-reminder>\nShip it"
        ),
        "Ship it"
    );
    assert_eq!(
        title_of("<system-reminder>c</system-reminder><gespan_browser>b</gespan_browser>Go"),
        "Go"
    );
}

#[test]
fn title_of_a_wrapper_only_prompt_is_empty() {
    assert_eq!(title_of("<system-reminder>\nonly\n</system-reminder>"), "");
    assert_eq!(
        title_of("<gespan_browser>x</gespan_browser>\n<gespan_context>y</gespan_context>\n"),
        ""
    );
    assert_eq!(
        title_of("<gespan_browser>\nnever closed\nOpen the docs"),
        "<gespan_browser>",
        "an unclosed block is left as written"
    );
}

#[test]
fn claude_title_skips_a_system_reminder_block_before_the_prompt() {
    let home = tempfile::tempdir().unwrap();
    let first = serde_json::json!({
        "type": "user", "uuid": "u1", "parentUuid": null, "cwd": "/tmp/p",
        "message": {"role": "user", "content": [
            {"type": "text", "text": "<system-reminder>\nSessionStart hook context\n</system-reminder>"},
            {"type": "text", "text": "Fix the login redirect\nmore"},
        ]},
    })
    .to_string();
    write(
        home.path(),
        "projects/-tmp-p/abc.jsonl",
        &[first, assistant("a1", "u1", "On it.")],
    );
    let found = list(Kind::Claude, home.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "Fix the login redirect");
    assert_eq!(
        texts(&read_turns(&found[0]).unwrap()),
        [
            (TurnRole::User, "Fix the login redirect\nmore"),
            (TurnRole::Assistant, "On it."),
        ]
    );
}

#[test]
fn claude_title_moves_past_a_wrapper_only_first_prompt() {
    let home = tempfile::tempdir().unwrap();
    write(
        home.path(),
        "projects/-tmp-p/abc.jsonl",
        &[
            user(
                "u1",
                None,
                "<gespan_browser>\nUse your tabs.\n</gespan_browser>",
            ),
            assistant("a1", "u1", "Ready."),
            user("u2", Some("a1"), "Second prompt"),
        ],
    );
    let found = list(Kind::Claude, home.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "Second prompt");
}

#[test]
fn claude_skips_sidechain_folders_and_empty_files() {
    let home = tempfile::tempdir().unwrap();
    let good = [user("u1", None, "Hello"), assistant("a1", "u1", "Hi")];
    write(home.path(), "projects/x/abc/subagents/agent-1.jsonl", &good);
    write(
        home.path(),
        "projects/x/only-snapshots.jsonl",
        &[snapshot(), snapshot()],
    );
    fs::write(
        home.path().join("projects/x/bad.jsonl"),
        b"{not json\n\xff\xfe\n",
    )
    .unwrap();
    write(home.path(), "projects/x/good.jsonl", &good);
    fs::write(home.path().join("projects/x/notes.txt"), "hi").unwrap();

    let found = list(Kind::Claude, home.path());
    let ids: Vec<_> = found.iter().map(|c| c.provider_id.as_str()).collect();
    assert_eq!(ids, ["good"]);
}

#[test]
fn claude_head_read_is_bounded() {
    let home = tempfile::tempdir().unwrap();
    let filler = "x".repeat(900);
    let mut lines = vec![
        // The first prompt carries a cwd but no branch.
        serde_json::json!({
            "type": "user", "uuid": "u0", "cwd": "/tmp/p",
            "message": {"role": "user", "content": "First prompt"},
        })
        .to_string(),
    ];
    let mut parent = "u0".to_string();
    for i in 0..(3 * 1024 * 1024 / 1000) {
        let id = format!("a{i}");
        lines.push(assistant(&id, &parent, &filler));
        parent = id;
    }
    // Only a line far past the head limit names a branch.
    lines.push(
        serde_json::json!({
            "type": "assistant", "uuid": "last", "parentUuid": parent,
            "gitBranch": "late-branch",
            "message": {"role": "assistant", "id": "last",
                        "content": [{"type": "text", "text": "done"}]},
        })
        .to_string(),
    );
    let path = write(home.path(), "projects/big/s1.jsonl", &lines);
    assert!(fs::metadata(&path).unwrap().len() > 2 * HEAD_LIMIT);

    let found = list(Kind::Claude, home.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "First prompt");
    assert_eq!(found[0].cwd, "/tmp/p");
    assert_eq!(found[0].branch, None);

    // Reading the session itself is not bounded.
    let turns = read_turns(&found[0]).unwrap();
    assert_eq!(turns[0].text, "First prompt");
    assert_eq!(turns.last().unwrap().text, "done");
    assert!(turns.len() > 1000);
}

#[test]
fn claude_finds_a_prompt_past_the_quick_head() {
    let home = tempfile::tempdir().unwrap();
    // A file snapshot far larger than the first read, then the prompt.
    let snapshot = serde_json::json!({
        "type": "file-history-snapshot", "messageId": "m",
        "snapshot": {"trackedFileBackups": {"a": "y".repeat(200 * 1024)}},
    })
    .to_string();
    let prompt = serde_json::json!({
        "type": "user", "uuid": "u1", "parentUuid": null,
        "cwd": "/tmp/late", "gitBranch": "dev",
        "message": {"role": "user", "content": "Late prompt"},
    })
    .to_string();
    write(
        home.path(),
        "projects/-tmp-late/late.jsonl",
        &[snapshot, prompt],
    );
    let found = list(Kind::Claude, home.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "Late prompt");
    assert_eq!(found[0].cwd, "/tmp/late");
    assert_eq!(found[0].branch.as_deref(), Some("dev"));
}

#[test]
fn claude_count_messages_counts_user_and_assistant_turns() {
    let home = tempfile::tempdir().unwrap();
    write(
        home.path(),
        "projects/p/abc.jsonl",
        &[
            snapshot(),
            user("u1", None, "One"),
            assistant("a1", "u1", "Two"),
            user("u2", Some("a1"), "Three"),
            assistant("a2", "u2", "Four"),
        ],
    );
    let found = list(Kind::Claude, home.path());
    assert_eq!(count_messages(&found[0]), Some(4));
    let turns = read_turns(&found[0]).unwrap();
    assert_eq!(turns[0].role, TurnRole::User);
    assert_eq!(turns.len(), 4);
}

#[test]
fn claude_count_messages_is_none_for_a_missing_file() {
    let home = tempfile::tempdir().unwrap();
    write(
        home.path(),
        "projects/p/abc.jsonl",
        &[user("u1", None, "One"), assistant("a1", "u1", "Two")],
    );
    let c = list(Kind::Claude, home.path()).remove(0);
    fs::remove_file(&c.path).unwrap();
    assert_eq!(count_messages(&c), None);
    assert!(read_turns(&c).is_err());
}

#[test]
fn claude_list_is_newest_first() {
    let home = tempfile::tempdir().unwrap();
    let body = [user("u1", None, "Hello"), assistant("a1", "u1", "Hi")];
    let old = write(home.path(), "projects/a/old.jsonl", &body);
    let new = write(home.path(), "projects/b/new.jsonl", &body);
    set_mtime(&old, 1_000);
    set_mtime(&new, 2_000);
    let found = list(Kind::Claude, home.path());
    let ids: Vec<_> = found.iter().map(|c| c.provider_id.as_str()).collect();
    assert_eq!(ids, ["new", "old"]);
    assert_eq!(found[0].updated_at, 2_000_000);
}

#[test]
fn other_kinds_are_not_supported_yet() {
    let home = tempfile::tempdir().unwrap();
    assert!(list(Kind::Grok, home.path()).is_empty());
    let c = Candidate {
        kind: Kind::Grok,
        provider_id: "x".into(),
        cwd: String::new(),
        title: String::new(),
        branch: None,
        updated_at: 0,
        messages: None,
        path: PathBuf::new(),
    };
    assert!(read_turns(&c).is_err());
    assert_eq!(count_messages(&c), None);
}

const CODEX_ID: &str = "0198a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

fn rollout_name(id: &str, ext: &str) -> String {
    format!("rollout-2026-11-11T10-20-30-{id}.{ext}")
}

fn codex_meta(cwd: &str, branch: Option<&str>) -> String {
    let git = branch.map_or(serde_json::json!({}), |b| serde_json::json!({"branch": b}));
    serde_json::json!({
        "type": "session_meta",
        "payload": {"id": CODEX_ID, "cwd": cwd, "git": git},
    })
    .to_string()
}

fn codex_event(kind: &str, text: &str) -> String {
    serde_json::json!({
        "timestamp": "2026-11-11T10:20:31.000Z",
        "type": "event_msg",
        "payload": {"type": kind, "message": text},
    })
    .to_string()
}

fn codex_rollback(turns: u64) -> String {
    serde_json::json!({
        "type": "event_msg",
        "payload": {"type": "thread_rolled_back", "num_turns": turns},
    })
    .to_string()
}

fn texts(turns: &[crate::outside_turns::OutsideTurn]) -> Vec<(TurnRole, &str)> {
    turns.iter().map(|t| (t.role, t.text.as_str())).collect()
}

#[test]
fn codex_lists_meta_cwd_branch_title_and_id() {
    let home = tempfile::tempdir().unwrap();
    let rel = format!("sessions/2026/11/11/{}", rollout_name(CODEX_ID, "jsonl"));
    let path = write(
        home.path(),
        &rel,
        &[
            codex_meta("/tmp/p", Some("feature/x")),
            codex_event("user_message", "  Add the export button\nwith detail"),
            codex_event("agent_message", "Done."),
        ],
    );
    let found = list(Kind::Codex, home.path());
    assert_eq!(found.len(), 1);
    let c = &found[0];
    assert_eq!(c.kind, Kind::Codex);
    assert_eq!(c.provider_id, CODEX_ID);
    assert_eq!(c.cwd, "/tmp/p");
    assert_eq!(c.branch.as_deref(), Some("feature/x"));
    assert_eq!(c.title, "Add the export button");
    assert_eq!(c.path, path);
    assert_eq!(c.messages, None);
    assert_eq!(count_messages(c), Some(2));
}

#[test]
fn codex_lists_zst_rollouts_but_not_archived_ones() {
    let home = tempfile::tempdir().unwrap();
    let body = [
        codex_meta("/tmp/z", None),
        codex_event("user_message", "Compressed question"),
        codex_event("agent_message", "Compressed answer"),
    ]
    .join("\n")
        + "\n";
    let zst_id = "11111111-2222-3333-4444-555555555555";
    let dir = home.path().join("sessions/2026/10/01");
    fs::create_dir_all(&dir).unwrap();
    let zst = dir.join(rollout_name(zst_id, "jsonl.zst"));
    fs::write(&zst, zstd::encode_all(body.as_bytes(), 3).unwrap()).unwrap();
    // Codex refuses to resume an archived thread until it is unarchived, so
    // an archived rollout is not offered.
    write(
        home.path(),
        &format!("archived_sessions/{}", rollout_name(CODEX_ID, "jsonl")),
        &[
            codex_meta("/tmp/a", None),
            codex_event("user_message", "Archived question"),
        ],
    );

    let found = list(Kind::Codex, home.path());
    let ids: Vec<_> = found.iter().map(|c| c.provider_id.as_str()).collect();
    assert_eq!(ids, [zst_id]);
    let compressed = &found[0];
    assert_eq!(compressed.cwd, "/tmp/z");
    assert_eq!(compressed.branch, None);
    assert_eq!(compressed.title, "Compressed question");
    assert_eq!(
        texts(&read_turns(compressed).unwrap()),
        vec![
            (TurnRole::User, "Compressed question"),
            (TurnRole::Assistant, "Compressed answer")
        ]
    );
}

#[test]
fn codex_zst_head_is_bounded() {
    let home = tempfile::tempdir().unwrap();
    let mut body = codex_meta("/tmp/z", None) + "\n";
    body += &(codex_event("user_message", "First prompt") + "\n");
    // Far past the head limit, and compressible, so the .zst stays small.
    let filler = codex_event("agent_message", &"x".repeat(1000)) + "\n";
    body += &filler.repeat((HEAD_LIMIT as usize / filler.len()) * 3);
    body += &(codex_event("user_message", "Late prompt") + "\n");
    let dir = home.path().join("sessions/2026/11/11");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(rollout_name(CODEX_ID, "jsonl.zst")),
        zstd::encode_all(body.as_bytes(), 1).unwrap(),
    )
    .unwrap();
    let found = list(Kind::Codex, home.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "First prompt");
    // The full read still sees the late prompt.
    let turns = read_turns(&found[0]).unwrap();
    assert_eq!(turns.last().unwrap().text, "Late prompt");
}

#[test]
fn codex_skips_rollouts_with_no_user_message() {
    let home = tempfile::tempdir().unwrap();
    write(
        home.path(),
        &format!("sessions/2026/11/11/{}", rollout_name(CODEX_ID, "jsonl")),
        &[
            codex_meta("/tmp/p", None),
            codex_event("agent_message", "Unprompted"),
            codex_event("token_count", ""),
        ],
    );
    assert!(list(Kind::Codex, home.path()).is_empty());
}

#[test]
fn codex_read_honours_rollbacks() {
    let home = tempfile::tempdir().unwrap();
    let path = write(
        home.path(),
        &format!("sessions/2026/11/11/{}", rollout_name(CODEX_ID, "jsonl")),
        &[
            codex_meta("/tmp/p", None),
            codex_event("user_message", "First"),
            codex_event("agent_message", "First reply"),
            codex_event("user_message", "Second"),
            codex_event("agent_message", "Second reply"),
            codex_rollback(1),
        ],
    );
    let found = list(Kind::Codex, home.path());
    assert_eq!(found[0].path, path);
    assert_eq!(
        texts(&read_turns(&found[0]).unwrap()),
        vec![
            (TurnRole::User, "First"),
            (TurnRole::Assistant, "First reply")
        ]
    );
    assert_eq!(count_messages(&found[0]), Some(2));
}

#[test]
fn codex_bad_id_in_name_is_skipped() {
    let home = tempfile::tempdir().unwrap();
    let body = [
        codex_meta("/tmp/p", None),
        codex_event("user_message", "Hello"),
    ];
    for name in [
        "rollout-2026-11-11T10-20-30-bad id.jsonl",
        "rollout-2026-11-11T10-20-30-..jsonl",
        "rollout-2026-11-11T10-20-30-.jsonl",
        "notes-2026-11-11T10-20-30-abc.jsonl",
        "rollout.jsonl",
    ] {
        write(home.path(), &format!("sessions/2026/11/11/{name}"), &body);
    }
    assert!(list(Kind::Codex, home.path()).is_empty());
}

const KIRO_ID: &str = "sess_0f1e2d3c-4b5a-4968-8776-655443322110";

/// A Kiro V3 session folder, `sessions/<workspace hash>/<id>`, with the
/// fields a real `session.json` carries (values made up).
fn kiro_session(home: &Path, id: &str, workspaces: &[&str], lines: &[String]) -> PathBuf {
    let folder = home.join("sessions/0a1b2c3d4e5f6a7b").join(id);
    fs::create_dir_all(&folder).unwrap();
    let session = serde_json::json!({
        "_meta": {"kiro": {}},
        "agentMode": "auto",
        "createdAt": "2026-11-11T10:20:30.000Z",
        "dataModelVersion": 1,
        "id": id,
        "lastModifiedAt": "2026-11-11T10:21:00.000Z",
        "modelId": "auto",
        "rootPaths": workspaces,
        "schemaVersion": "1.0.0",
        "status": "idle",
        "title": "Kiro's own summary",
        "workspacePaths": workspaces,
    });
    fs::write(folder.join("session.json"), session.to_string()).unwrap();
    if !lines.is_empty() {
        fs::write(folder.join("messages.jsonl"), lines.join("\n") + "\n").unwrap();
    }
    folder
}

fn kiro_line(at: &str, payload: serde_json::Value) -> String {
    serde_json::json!({"id": "e", "payload": payload, "timestamp": at}).to_string()
}

fn kiro_user(at: &str, text: &str) -> String {
    kiro_line(
        at,
        serde_json::json!({
            "_meta": {"kiro": {"userMessageTag": "tag"}},
            "content": text, "documents": [], "images": [], "type": "user",
        }),
    )
}

/// An assistant entry; `operation` is `Say` for visible text and
/// `Reasoning` for thinking.
fn kiro_assistant(at: &str, operation: &str, text: &str) -> String {
    kiro_line(
        at,
        serde_json::json!({
            "_meta": {"kiro": {"agentMode": "auto"}},
            "content": text, "executionId": "x", "operationType": operation,
            "turnId": "t", "type": "assistant",
        }),
    )
}

fn kiro_event(kind: &str) -> String {
    kiro_line(
        "2026-11-11T10:20:30.000Z",
        serde_json::json!({"type": kind, "content": "not a turn", "turnId": "t"}),
    )
}

fn kiro_tool_call() -> String {
    kiro_line(
        "2026-11-11T10:20:31.000Z",
        serde_json::json!({
            "type": "tool_call", "toolCallId": "c", "toolName": "read",
            "title": "Reading file", "args": {"path": "a.rs"}, "status": "completed",
            "kind": "read", "turnId": "t",
        }),
    )
}

fn kiro_tool_result() -> String {
    kiro_line(
        "2026-11-11T10:20:32.000Z",
        serde_json::json!({
            "type": "tool_result", "toolCallId": "c", "success": true,
            "content": "fn main() {}", "turnId": "t",
        }),
    )
}

#[test]
fn kiro_lists_title_cwd_time_and_id() {
    let home = tempfile::tempdir().unwrap();
    let folder = kiro_session(
        home.path(),
        KIRO_ID,
        &["/tmp/p", "/tmp/q"],
        &[
            kiro_event("agent_note"),
            kiro_user("2026-11-11T10:20:30.000Z", "Fix the login redirect\nmore"),
            kiro_event("turn_start"),
            kiro_assistant("2026-11-11T10:20:31.000Z", "Say", "On it."),
        ],
    );
    set_mtime(&folder.join("messages.jsonl"), 1_800_000_000);

    let found = list(Kind::Kiro, home.path());
    assert_eq!(found.len(), 1);
    let c = &found[0];
    assert_eq!(c.kind, Kind::Kiro);
    assert_eq!(c.provider_id, KIRO_ID);
    assert_eq!(c.cwd, "/tmp/p");
    assert_eq!(c.title, "Fix the login redirect");
    assert_eq!(c.branch, None);
    assert_eq!(c.updated_at, 1_800_000_000_000);
    assert_eq!(c.messages, None);
    assert_eq!(c.path, folder);
}

#[test]
fn kiro_reads_user_and_assistant_text_only() {
    let home = tempfile::tempdir().unwrap();
    let mut lines = vec![
        kiro_event("agent_note"),
        kiro_user("2026-11-11T10:20:30.000Z", "Read a.rs"),
        kiro_event("turn_start"),
        kiro_assistant("2026-11-11T10:20:31.000Z", "Say", "Reading it:"),
        kiro_assistant("2026-11-11T10:20:31.500Z", "Reasoning", "..."),
        kiro_tool_call(),
        kiro_tool_result(),
        kiro_assistant("2026-11-11T10:20:33.000Z", "Say", "It is empty."),
        kiro_event("turn_end"),
        kiro_event("session_start"),
        kiro_user("2026-11-11T10:21:00.000Z", "Thanks"),
    ];
    // Kiro is still writing the next entry.
    lines.push(r#"{"id":"e","payload":{"type":"assistant","operationType":"Say","con"#.into());
    let folder = kiro_session(home.path(), KIRO_ID, &["/tmp/p"], &[]);
    fs::write(folder.join("messages.jsonl"), lines.join("\n")).unwrap();

    let c = list(Kind::Kiro, home.path()).remove(0);
    let turns = read_turns(&c).unwrap();
    assert_eq!(
        texts(&turns),
        [
            (TurnRole::User, "Read a.rs"),
            (TurnRole::Assistant, "Reading it:"),
            (TurnRole::Assistant, "It is empty."),
            (TurnRole::User, "Thanks"),
        ]
    );
    let sent: Vec<_> = turns.iter().map(|t| t.sent_at).collect();
    assert_eq!(
        sent,
        [
            Some(1_794_392_430_000),
            Some(1_794_392_431_000),
            Some(1_794_392_433_000),
            Some(1_794_392_460_000),
        ]
    );
    assert_eq!(count_messages(&c), Some(4));
}

#[test]
fn kiro_strips_gespan_wrappers_from_prompts() {
    let home = tempfile::tempdir().unwrap();
    kiro_session(
        home.path(),
        KIRO_ID,
        &["/tmp/p"],
        &[
            kiro_user(
                "2026-11-11T10:20:30.000Z",
                "<gespan_browser>\nUse your tabs.\n</gespan_browser>\n\nOpen the pricing page\nand read it",
            ),
            kiro_assistant("2026-11-11T10:20:31.000Z", "Say", "Opened."),
            kiro_user(
                "2026-11-11T10:21:00.000Z",
                "<gespan_browser>\nUse your tabs.\n</gespan_browser>",
            ),
        ],
    );
    let c = list(Kind::Kiro, home.path()).remove(0);
    assert_eq!(c.title, "Open the pricing page");
    assert_eq!(
        texts(&read_turns(&c).unwrap()),
        [
            (TurnRole::User, "Open the pricing page\nand read it"),
            (TurnRole::Assistant, "Opened."),
        ]
    );
}

#[test]
fn kiro_skips_sessions_with_no_user_message() {
    let home = tempfile::tempdir().unwrap();
    let good = [kiro_user("2026-11-11T10:20:30.000Z", "Hello")];
    // Opened but never prompted: no messages file at all.
    kiro_session(home.path(), "sess_never-prompted", &["/tmp/p"], &[]);
    kiro_session(
        home.path(),
        "sess_only-events",
        &["/tmp/p"],
        &[kiro_event("agent_note"), kiro_event("session_start")],
    );
    kiro_session(
        home.path(),
        "sess_blank-prompt",
        &["/tmp/p"],
        &[kiro_user("2026-11-11T10:20:30.000Z", "  \n")],
    );
    let broken = kiro_session(home.path(), "sess_broken-meta", &["/tmp/p"], &good);
    fs::write(broken.join("session.json"), b"{not json").unwrap();
    kiro_session(home.path(), "sess_good", &["/tmp/p"], &good);
    // Kiro's own bookkeeping next to the sessions.
    fs::create_dir_all(home.path().join("sessions/0a1b2c3d4e5f6a7b/.index")).unwrap();
    fs::write(
        home.path().join("sessions/0a1b2c3d4e5f6a7b/stray.json"),
        "{}",
    )
    .unwrap();

    let found = list(Kind::Kiro, home.path());
    let ids: Vec<_> = found.iter().map(|c| c.provider_id.as_str()).collect();
    assert_eq!(ids, ["sess_good"]);
}

#[test]
fn kiro_head_read_is_bounded() {
    let home = tempfile::tempdir().unwrap();
    let filler = kiro_line(
        "2026-11-11T10:20:30.000Z",
        serde_json::json!({"type": "agent_note", "content": "x".repeat(900)}),
    );
    let mut lines = vec![filler; (HEAD_LIMIT as usize / 900) + 10];
    lines.push(kiro_user("2026-11-11T10:20:30.000Z", "Too far in"));
    kiro_session(home.path(), KIRO_ID, &["/tmp/p"], &lines);

    assert!(list(Kind::Kiro, home.path()).is_empty());
}

const CURSOR_ID: &str = "0f1e2d3c-4b5a-4968-8776-655443322110";

/// The id Cursor gives a blob; any 64 hex digits do for a fixture.
fn cursor_blob_id(n: usize) -> String {
    format!("{n:064x}")
}

/// A root blob as Cursor writes it: a protobuf message whose field 1 repeats
/// the 32-byte ids of the conversation's messages, in order, and whose field
/// 21 holds the workspace path and branch. Field 26 is a time, as in the real
/// root.
fn cursor_root(ids: &[String], branch: Option<&str>) -> Vec<u8> {
    let mut root = Vec::new();
    for id in ids {
        root.extend([0x0a, 32]);
        root.extend(
            (0..64)
                .step_by(2)
                .map(|i| u8::from_str_radix(&id[i..i + 2], 16).unwrap()),
        );
    }
    if let Some(branch) = branch {
        let mut workspace = vec![0x0a, 6];
        workspace.extend(b"/tmp/p");
        workspace.extend([0x12, branch.len() as u8]);
        workspace.extend(branch.as_bytes());
        root.extend([0xaa, 0x01, workspace.len() as u8]);
        root.extend(workspace);
    }
    root.extend([0xd0, 0x01, 0xe8, 0x07]);
    root
}

/// A Cursor session folder, `<home>/<dir>/<id>/`, holding a `meta.json` and
/// a `store.db` in the shape Cursor writes (values made up): the `meta` row
/// `0` is hex-encoded JSON naming the latest root blob, and `blobs` holds the
/// JSON messages, the root, an ACP turn record in protobuf, and a message
/// left behind by an earlier root.
fn cursor_session(
    home: &Path,
    dir: &str,
    id: &str,
    messages: &[serde_json::Value],
    branch: Option<&str>,
) -> PathBuf {
    let folder = home.join(dir).join(id);
    fs::create_dir_all(&folder).unwrap();
    let meta =
        serde_json::json!({"schemaVersion": 1, "cwd": "/tmp/p", "title": "Cursor's own title"});
    fs::write(folder.join("meta.json"), meta.to_string()).unwrap();

    let db = rusqlite::Connection::open(folder.join("store.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);
         CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);",
    )
    .unwrap();
    let insert = |id: &str, data: &[u8]| {
        db.execute(
            "INSERT INTO blobs VALUES (?1, ?2)",
            rusqlite::params![id, data],
        )
        .unwrap();
    };
    let ids: Vec<String> = (1..=messages.len()).map(cursor_blob_id).collect();
    for (id, message) in ids.iter().zip(messages) {
        insert(id, &serde_json::to_vec(message).unwrap());
    }
    insert(
        &cursor_blob_id(900),
        &serde_json::to_vec(&cursor_assistant(serde_json::json!([
            {"type": "text", "text": "Abandoned draft"}
        ])))
        .unwrap(),
    );
    insert(
        &cursor_blob_id(901),
        &[0x1a, 0x06, 0x0a, 0x04, b'P', b'O', b'N', b'G'],
    );
    let root = cursor_blob_id(999);
    insert(&root, &cursor_root(&ids, branch));
    let metadata = serde_json::json!({
        "agentId": id, "latestRootBlobId": root, "name": "New Agent",
        "mode": "default", "createdAt": 1_794_362_533_999_u64,
    })
    .to_string();
    let hex: String = metadata.bytes().map(|b| format!("{b:02x}")).collect();
    db.execute("INSERT INTO meta VALUES ('0', ?1)", [hex])
        .unwrap();
    folder
}

fn cursor_system() -> serde_json::Value {
    serde_json::json!({"role": "system", "content": "You are an AI coding assistant.", "providerOptions": {}})
}

/// The context message Cursor sends before the first prompt.
fn cursor_context() -> serde_json::Value {
    serde_json::json!({
        "role": "user",
        "content": "<user_info>\nWorkspace Path: /tmp/p\n</user_info>\n<rules>\nbe brief\n</rules>",
        "providerOptions": {"cursor": {"requestContextCompleteness": {"rules": true}}},
    })
}

/// A prompt, wrapped the way Cursor stores it, with a mode reminder before it.
fn cursor_user(timestamp: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "role": "user",
        "content": [
            {"type": "text", "text": "\n<system_reminder>\nAsk mode is active.\n</system_reminder>"},
            {"type": "text", "text": format!(
                "<timestamp>{timestamp}</timestamp>\n<user_query>\n{text}\n</user_query>"
            )},
        ],
        "providerOptions": {"cursor": {"requestId": "r"}},
    })
}

fn cursor_assistant(parts: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "role": "assistant", "id": "msg_1", "content": parts,
        "providerOptions": {"cursor": {"modelProviderMessageId": "msg_1"}},
    })
}

fn cursor_tool_result() -> serde_json::Value {
    serde_json::json!({"role": "tool", "content": [
        {"type": "tool-result", "toolCallId": "c", "toolName": "Read", "result": "fn main() {}"}
    ]})
}

fn cursor_conversation() -> Vec<serde_json::Value> {
    vec![
        cursor_system(),
        cursor_context(),
        cursor_user(
            "Wednesday, Nov 11, 2026, 7:32 AM (UTC+5:30)",
            "Fix the login redirect\nmore",
        ),
        cursor_assistant(serde_json::json!([
            {"type": "reasoning", "text": "", "signature": "opaque"},
            {"type": "text", "text": "Let me look."},
            {"type": "tool-call", "toolCallId": "c", "toolName": "Read", "args": {"path": "a.rs"}},
        ])),
        cursor_tool_result(),
        cursor_assistant(serde_json::json!([
            {"type": "reasoning", "text": "Thinking it over."},
            {"type": "text", "text": "Fixed."},
        ])),
        cursor_user("Wednesday, Nov 11, 2026, 11:05 PM (UTC-4:00)", "Add a test"),
        cursor_assistant(serde_json::json!([{"type": "text", "text": "Done."}])),
    ]
}

fn unix_ms(rfc3339: &str) -> u64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .unwrap()
        .timestamp_millis() as u64
}

#[test]
fn cursor_lists_title_cwd_time_and_id() {
    let home = tempfile::tempdir().unwrap();
    let folder = cursor_session(
        home.path(),
        "acp-sessions",
        CURSOR_ID,
        &cursor_conversation(),
        Some("main"),
    );
    set_mtime(&folder.join("store.db"), 1_800_000_000);

    let found = list(Kind::Cursor, home.path());
    assert_eq!(found.len(), 1);
    let c = &found[0];
    assert_eq!(c.kind, Kind::Cursor);
    assert_eq!(c.provider_id, CURSOR_ID);
    assert_eq!(c.cwd, "/tmp/p");
    assert_eq!(c.title, "Fix the login redirect");
    assert_eq!(c.branch.as_deref(), Some("main"));
    assert_eq!(c.updated_at, 1_800_000_000_000);
    assert_eq!(c.messages, None);
    assert_eq!(c.path, folder);
}

#[test]
fn cursor_reads_user_and_assistant_text_only() {
    let home = tempfile::tempdir().unwrap();
    cursor_session(
        home.path(),
        "acp-sessions",
        CURSOR_ID,
        &cursor_conversation(),
        None,
    );
    let found = list(Kind::Cursor, home.path());
    assert_eq!(found[0].branch, None);

    let turns = read_turns(&found[0]).unwrap();
    assert_eq!(
        texts(&turns),
        vec![
            (TurnRole::User, "Fix the login redirect\nmore"),
            (TurnRole::Assistant, "Let me look."),
            (TurnRole::Assistant, "Fixed."),
            (TurnRole::User, "Add a test"),
            (TurnRole::Assistant, "Done."),
        ]
    );
    let sent: Vec<Option<u64>> = turns.iter().map(|t| t.sent_at).collect();
    assert_eq!(
        sent,
        vec![
            Some(unix_ms("2026-11-11T07:32:00+05:30")),
            None,
            None,
            Some(unix_ms("2026-11-11T23:05:00-04:00")),
            None,
        ]
    );
    assert_eq!(count_messages(&found[0]), Some(5));
}

#[test]
fn cursor_strips_gespan_wrappers_from_prompts() {
    let home = tempfile::tempdir().unwrap();
    cursor_session(
        home.path(),
        "acp-sessions",
        CURSOR_ID,
        &[
            cursor_system(),
            cursor_context(),
            cursor_user(
                "Wednesday, Nov 11, 2026, 7:32 AM (UTC+5:30)",
                "<gespan_browser>\nUse your tabs.\n</gespan_browser>\n\nOpen the docs",
            ),
            cursor_assistant(serde_json::json!([{"type": "text", "text": "Opened."}])),
        ],
        None,
    );
    let found = list(Kind::Cursor, home.path());
    assert_eq!(found[0].title, "Open the docs");
    assert_eq!(
        texts(&read_turns(&found[0]).unwrap()),
        vec![
            (TurnRole::User, "Open the docs"),
            (TurnRole::Assistant, "Opened."),
        ]
    );
}

#[test]
fn cursor_skips_chats_without_conversation() {
    let home = tempfile::tempdir().unwrap();
    cursor_session(
        home.path(),
        "acp-sessions",
        "good",
        &cursor_conversation(),
        None,
    );
    // Opened over ACP and never prompted: Cursor writes only `meta.json`.
    let empty = home.path().join("acp-sessions/never-prompted");
    fs::create_dir_all(&empty).unwrap();
    fs::write(
        empty.join("meta.json"),
        r#"{"schemaVersion":1,"cwd":"/tmp/p","title":"x"}"#,
    )
    .unwrap();
    // Context sent, but no prompt in it.
    cursor_session(
        home.path(),
        "acp-sessions",
        "context-only",
        &[cursor_system(), cursor_context()],
        None,
    );
    // A `cursor-agent` chat: ACP's session/load cannot find it, so it is not offered.
    cursor_session(
        home.path(),
        "chats/0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d",
        "terminal-chat",
        &cursor_conversation(),
        None,
    );
    fs::write(home.path().join("acp-sessions/stray"), "not a folder").unwrap();

    let ids: Vec<String> = list(Kind::Cursor, home.path())
        .into_iter()
        .map(|c| c.provider_id)
        .collect();
    assert_eq!(ids, vec!["good"]);
}

#[test]
fn cursor_corrupt_store_is_skipped_not_fatal() {
    let home = tempfile::tempdir().unwrap();
    cursor_session(
        home.path(),
        "acp-sessions",
        "good",
        &cursor_conversation(),
        None,
    );
    let garbage = home.path().join("acp-sessions/garbage");
    fs::create_dir_all(&garbage).unwrap();
    fs::write(garbage.join("meta.json"), r#"{"cwd":"/tmp/p"}"#).unwrap();
    fs::write(garbage.join("store.db"), b"not a database at all").unwrap();

    let bad_meta = cursor_session(
        home.path(),
        "acp-sessions",
        "bad-meta",
        &cursor_conversation(),
        None,
    );
    rusqlite::Connection::open(bad_meta.join("store.db"))
        .unwrap()
        .execute("UPDATE meta SET value = 'zz' WHERE key = '0'", [])
        .unwrap();

    let no_root = cursor_session(
        home.path(),
        "acp-sessions",
        "no-root",
        &cursor_conversation(),
        None,
    );
    rusqlite::Connection::open(no_root.join("store.db"))
        .unwrap()
        .execute("DELETE FROM blobs WHERE id = ?1", [cursor_blob_id(999)])
        .unwrap();

    // A message missing from the store, or not JSON, is left out of the rest.
    let holes = cursor_session(
        home.path(),
        "acp-sessions",
        "holes",
        &cursor_conversation(),
        None,
    );
    let db = rusqlite::Connection::open(holes.join("store.db")).unwrap();
    db.execute("DELETE FROM blobs WHERE id = ?1", [cursor_blob_id(4)])
        .unwrap();
    db.execute(
        "UPDATE blobs SET data = x'00ff' WHERE id = ?1",
        [cursor_blob_id(6)],
    )
    .unwrap();
    drop(db);

    let found = list(Kind::Cursor, home.path());
    let mut ids: Vec<&str> = found.iter().map(|c| c.provider_id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["good", "holes"]);
    let holes = found.iter().find(|c| c.provider_id == "holes").unwrap();
    assert_eq!(
        texts(&read_turns(holes).unwrap()),
        vec![
            (TurnRole::User, "Fix the login redirect\nmore"),
            (TurnRole::User, "Add a test"),
            (TurnRole::Assistant, "Done."),
        ]
    );

    let mut broken = found[0].clone();
    broken.path = garbage;
    assert!(read_turns(&broken).is_err());
    assert_eq!(count_messages(&broken), None);
}

mod placement {
    use super::*;

    fn real_root(p: &str) -> Option<String> {
        crate::git::linked_worktree_root(Path::new(p)).map(|r| r.to_string_lossy().into_owned())
    }

    fn no_root(_: &str) -> Option<String> {
        None
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn places_the_project_folder() {
        let dir = tempfile::tempdir().unwrap();
        let project = s(dir.path());
        let got = place(&project, std::slice::from_ref(&project), no_root);
        assert_eq!(got, Placement::Project { project });
    }

    #[test]
    fn places_a_path_spelled_differently() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        fs::create_dir(&project).unwrap();
        let projects = vec![s(&project)];
        let expected = Placement::Project {
            project: s(&project),
        };

        let slash = format!("{}/", s(&project));
        assert_eq!(
            place(&slash, &projects, no_root),
            expected,
            "trailing slash"
        );

        #[cfg(unix)]
        {
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&project, &link).unwrap();
            assert_eq!(place(&s(&link), &projects, no_root), expected, "symlink");
            // The project may be the one spelled through the link.
            let linked = vec![s(&link)];
            assert_eq!(
                place(&s(&project), &linked, no_root),
                Placement::Project { project: s(&link) },
                "project stored via symlink"
            );
        }
    }

    #[test]
    fn places_a_linked_worktree_under_its_project() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        fs::create_dir(&main).unwrap();
        let git = |args: &[&str]| crate::git::git(&main, args).unwrap();
        git(&["init", "-b", "main"]);
        fs::write(main.join("f"), "x\n").unwrap();
        git(&["add", "f"]);
        git(&[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-m",
            "init",
        ]);
        let tree = temp.path().join("tree");
        git(&["worktree", "add", "-b", "feat", &s(&tree)]);

        let projects = vec![s(&main)];
        assert_eq!(
            place(&s(&tree), &projects, real_root),
            Placement::Worktree {
                project: s(&main),
                worktree: s(&tree)
            }
        );
        // The project itself wins over the worktree rule.
        assert_eq!(
            place(&s(&main), &projects, real_root),
            Placement::Project { project: s(&main) }
        );
        // A worktree of a folder that isn't a project is skipped.
        assert_eq!(
            place(&s(&tree), &[s(temp.path())], real_root),
            Placement::Skip(Skip::NoProject)
        );
    }

    #[test]
    fn skips_a_plain_subfolder_of_a_project() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("src");
        fs::create_dir(&sub).unwrap();
        assert_eq!(
            place(&s(&sub), &[s(dir.path())], real_root),
            Placement::Skip(Skip::NoProject)
        );
    }

    #[test]
    fn skips_an_unknown_folder() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_eq!(
            place(&s(a.path()), &[s(b.path())], real_root),
            Placement::Skip(Skip::NoProject)
        );
        assert_eq!(
            place(&s(a.path()), &[], real_root),
            Placement::Skip(Skip::NoProject)
        );
    }

    #[test]
    fn skips_a_deleted_folder() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        // Even when it is spelled exactly like a project.
        assert_eq!(
            place(&s(&gone), &[s(&gone)], no_root),
            Placement::Skip(Skip::FolderMissing)
        );
        // A file is not a folder.
        let file = dir.path().join("file");
        fs::write(&file, "x").unwrap();
        assert_eq!(
            place(&s(&file), &[s(&file)], no_root),
            Placement::Skip(Skip::FolderMissing)
        );
    }

    #[test]
    fn a_placer_asks_git_once_per_folder_and_agrees_with_place() {
        let dir = tempfile::tempdir().unwrap();
        let project = s(&dir.path().join("app"));
        let tree = s(&dir.path().join("app-tree"));
        let loose = s(&dir.path().join("loose"));
        for folder in [&project, &tree, &loose] {
            fs::create_dir_all(folder).unwrap();
        }
        let projects = vec![project.clone()];
        let asked = std::cell::RefCell::new(Vec::new());
        let main = project.clone();
        let root = |cwd: &str| {
            asked.borrow_mut().push(cwd.to_owned());
            (cwd.ends_with("-tree")).then(|| main.clone())
        };
        let mut placer = Placer::new(&projects, &root);
        let folders = [
            &tree,
            &loose,
            &project,
            &tree,
            &loose,
            &tree,
            "/no/such/folder",
        ];
        let placed: Vec<Placement> = folders.iter().map(|cwd| placer.place(cwd)).collect();
        assert_eq!(
            *asked.borrow(),
            [tree.clone(), loose.clone()],
            "git is asked once per folder, never for the project itself"
        );
        let one_off = |cwd: &str| place(cwd, &projects, |c: &str| root(c));
        for (cwd, got) in folders.iter().zip(&placed) {
            assert_eq!(*got, one_off(cwd), "{cwd}");
        }
        assert_eq!(
            placed[0],
            Placement::Worktree {
                project: project.clone(),
                worktree: tree.clone()
            }
        );
        assert_eq!(placed[1], Placement::Skip(Skip::NoProject));
        assert_eq!(placed[2], Placement::Project { project });
        assert_eq!(placed[6], Placement::Skip(Skip::FolderMissing));
    }

    #[test]
    fn already_imported_matches_provider_and_id() {
        let sessions: std::collections::HashSet<(String, String)> =
            [("claude", "live"), ("claude", "old"), ("codex", "shared")]
                .into_iter()
                .map(|(provider, id)| (provider.to_owned(), id.to_owned()))
                .collect();
        assert!(already_imported(&sessions, Kind::Claude, "live"));
        assert!(already_imported(&sessions, Kind::Claude, "old"));
        assert!(!already_imported(&sessions, Kind::Claude, "new"));
        assert!(!already_imported(&sessions, Kind::Claude, ""));
        assert!(!already_imported(&sessions, Kind::Claude, "shared"));
        assert!(already_imported(&sessions, Kind::Codex, "shared"));
        assert!(!already_imported(&sessions, Kind::Codex, "live"));
        assert!(!already_imported(&Default::default(), Kind::Claude, "live"));
    }
}

mod importing {
    use super::*;
    use crate::outside_turns::{self, Checkpoint};
    use crate::store::Store;
    use crate::transcript::Interjection;
    use std::sync::Mutex;

    fn shared() -> Mutex<Store> {
        Mutex::new(Store::open_gespan(Path::new(":memory:")).unwrap())
    }

    fn backends() -> [(&'static str, Mutex<Store>); 1] {
        [("shared", shared())]
    }

    fn no_root(_: &str) -> Option<String> {
        None
    }

    /// A Claude session of two turns that ran in `cwd`.
    fn claude_file(home: &Path, id: &str, cwd: &str) -> PathBuf {
        let body = |uuid: &str, parent: Option<&str>, text: &str, at: &str| {
            let mut v: serde_json::Value = serde_json::from_str(&user(uuid, parent, text)).unwrap();
            v["cwd"] = cwd.into();
            v["timestamp"] = at.into();
            v.to_string()
        };
        let mut reply: serde_json::Value =
            serde_json::from_str(&assistant("a1", "u1", "On it.")).unwrap();
        reply["timestamp"] = "2026-11-11T10:00:05Z".into();
        write(
            home,
            &format!("projects/-x/{id}.jsonl"),
            &[
                snapshot(),
                body("u1", None, "Fix the login\nmore", "2026-11-11T10:00:00Z"),
                reply.to_string(),
                serde_json::json!({
                    "type": "assistant", "uuid": "t1", "parentUuid": "a1",
                    "message": {"role": "assistant", "id": "t1",
                                "content": [{"type": "tool_use", "id": "x", "name": "Bash",
                                             "input": {"command": "SECRET-TOOL-TEXT"}}]},
                })
                .to_string(),
            ],
        )
    }

    fn candidate_of(home: &Path, id: &str, cwd: &str) -> Candidate {
        let path = claude_file(home, id, cwd);
        let mut found = list(Kind::Claude, home);
        found.retain(|c| c.path == path);
        found.pop().unwrap()
    }

    fn project(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name);
        fs::create_dir_all(&path).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn home_of(home: &Path) -> impl Fn(Kind) -> Option<PathBuf> {
        let home = home.to_path_buf();
        move |_| Some(home.clone())
    }

    #[test]
    fn import_creates_a_resumable_session_with_rows() {
        for (name, store) in backends() {
            let home = tempfile::tempdir().unwrap();
            let work = tempfile::tempdir().unwrap();
            let p = project(&work, "app");
            let pick = candidate_of(home.path(), "abc", &p);
            let report = import(
                &store,
                &[pick],
                std::slice::from_ref(&p),
                no_root,
                home_of(home.path()),
            );
            assert!(report.skipped.is_empty(), "{name}: {:?}", report.skipped);
            assert_eq!(report.imported.len(), 1, "{name}");
            let done = &report.imported[0];
            assert_eq!(done.project, p, "{name}");
            let s = &done.session;
            assert_eq!(s.provider, "claude", "{name}");
            assert_eq!(s.provider_id.as_deref(), Some("abc"), "{name}");
            assert_eq!(s.cwd, p, "{name}");
            assert_eq!(s.title, "Fix the login", "{name}");

            let guard = store.lock().unwrap();
            let saved = guard
                .sessions()
                .unwrap()
                .into_iter()
                .find(|x| x.id == s.id)
                .unwrap();
            assert_eq!(saved.provider_id.as_deref(), Some("abc"), "{name}");
            assert!(saved.has_messages, "{name}");
            let rows = guard.load(s.id).unwrap().rows;
            let shape: Vec<(&str, &str)> = rows
                .iter()
                .map(|r| (r.role.as_str(), r.text.as_str()))
                .collect();
            let divider = serde_json::to_string(&Interjection {
                text: "Claude Code".into(),
                custom_type: IMPORTED_TYPE.into(),
                severity: None,
            })
            .unwrap();
            assert_eq!(
                shape,
                vec![
                    ("interjection", divider.as_str()),
                    ("user", "Fix the login\nmore"),
                    ("assistant", "On it."),
                ],
                "{name}"
            );
            assert!(rows.iter().all(|r| !r.text.contains("SECRET-TOOL-TEXT")));
            assert!(
                rows[1].sent_at.is_some_and(|t| t > 1_700_000_000_000),
                "{name}"
            );
            assert!(
                rows[2]
                    .sent_at
                    .is_some_and(|t| t >= rows[1].sent_at.unwrap()),
                "{name}"
            );
        }
    }

    #[test]
    fn import_files_a_worktree_session_under_its_project() {
        for (name, store) in backends() {
            let home = tempfile::tempdir().unwrap();
            let work = tempfile::tempdir().unwrap();
            let p = project(&work, "app");
            let tree = project(&work, "app-tree");
            let pick = candidate_of(home.path(), "wt1", &tree);
            let main = p.clone();
            let report = import(
                &store,
                &[pick],
                std::slice::from_ref(&p),
                move |_| Some(main.clone()),
                home_of(home.path()),
            );
            assert_eq!(report.imported.len(), 1, "{name}: {:?}", report.skipped);
            let s = &report.imported[0].session;
            assert_eq!(s.cwd, p, "{name}");
            assert_eq!(
                report.imported[0].worktree.as_deref(),
                Some(tree.as_str()),
                "{name}: the report names the worktree, so the sidebar can file it at once"
            );
            let saved = store.lock().unwrap().agent_settings(s.id).unwrap().unwrap();
            assert_eq!(saved.worktree_cwd.as_deref(), Some(tree.as_str()), "{name}");
        }
    }

    #[test]
    fn import_pins_the_default_account() {
        for (name, store) in backends() {
            let home = tempfile::tempdir().unwrap();
            let work = tempfile::tempdir().unwrap();
            let p = project(&work, "app");
            let pick = candidate_of(home.path(), "acc", &p);
            let report = import(
                &store,
                &[pick],
                std::slice::from_ref(&p),
                no_root,
                home_of(home.path()),
            );
            let id = report.imported[0].session.id;
            assert_eq!(report.imported[0].worktree, None, "{name}");
            let saved = store.lock().unwrap().agent_settings(id).unwrap().unwrap();
            assert_eq!(saved.account, crate::accounts::Profile::default(), "{name}");
            assert_eq!(saved.worktree_cwd, None, "{name}");
        }
    }

    #[test]
    fn import_twice_skips_the_second_as_already_imported() {
        for (name, store) in backends() {
            let home = tempfile::tempdir().unwrap();
            let work = tempfile::tempdir().unwrap();
            let p = project(&work, "app");
            let pick = candidate_of(home.path(), "dup", &p);
            // Twice in one call, then again in a later call.
            let first = import(
                &store,
                &[pick.clone(), pick.clone()],
                std::slice::from_ref(&p),
                no_root,
                home_of(home.path()),
            );
            assert_eq!(first.imported.len(), 1, "{name}");
            assert_eq!(first.skipped.len(), 1, "{name}");
            assert_eq!(first.skipped[0].1, Skip::AlreadyImported, "{name}");
            let second = import(
                &store,
                &[pick],
                std::slice::from_ref(&p),
                no_root,
                home_of(home.path()),
            );
            assert!(second.imported.is_empty(), "{name}");
            assert_eq!(second.skipped[0].1, Skip::AlreadyImported, "{name}");
            let held = store.lock().unwrap().sessions().unwrap();
            assert_eq!(held.len(), 1, "{name}");
        }
    }

    #[test]
    fn import_reports_skips_and_imports_the_rest() {
        for (name, store) in backends() {
            let home = tempfile::tempdir().unwrap();
            let work = tempfile::tempdir().unwrap();
            let p = project(&work, "app");
            let elsewhere = project(&work, "elsewhere");
            let good = candidate_of(home.path(), "good", &p);
            let stray = candidate_of(home.path(), "stray", &elsewhere);
            let gone = candidate_of(home.path(), "gone", &p);
            fs::remove_file(&gone.path).unwrap();
            let report = import(
                &store,
                &[gone.clone(), stray.clone(), good.clone()],
                std::slice::from_ref(&p),
                no_root,
                home_of(home.path()),
            );
            assert_eq!(report.imported.len(), 1, "{name}");
            assert_eq!(
                report.imported[0].session.provider_id.as_deref(),
                Some("good")
            );
            assert_eq!(report.skipped.len(), 2, "{name}");
            assert_eq!(report.skipped[0].0, gone, "{name}");
            assert!(matches!(report.skipped[0].1, Skip::Unreadable(_)), "{name}");
            assert_eq!(report.skipped[1].0, stray, "{name}");
            assert_eq!(report.skipped[1].1, Skip::NoProject, "{name}");
        }
    }

    #[test]
    fn import_reads_a_file_ending_mid_line() {
        for (name, store) in backends() {
            let home = tempfile::tempdir().unwrap();
            let work = tempfile::tempdir().unwrap();
            let p = project(&work, "app");
            let pick = candidate_of(home.path(), "open", &p);
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(&pick.path)
                .unwrap();
            use std::io::Write;
            write!(
                file,
                "{{\"type\":\"user\",\"uuid\":\"u9\",\"message\":{{\"role\":\"us"
            )
            .unwrap();
            let report = import(
                &store,
                &[pick],
                std::slice::from_ref(&p),
                no_root,
                home_of(home.path()),
            );
            assert_eq!(report.imported.len(), 1, "{name}: {:?}", report.skipped);
            let id = report.imported[0].session.id;
            let rows = store.lock().unwrap().load(id).unwrap().rows;
            let roles: Vec<&str> = rows.iter().map(|r| r.role.as_str()).collect();
            assert_eq!(roles, ["interjection", "user", "assistant"], "{name}");
        }
    }

    #[test]
    fn import_sets_the_terminal_sync_checkpoint() {
        for (name, store) in backends() {
            let home = tempfile::tempdir().unwrap();
            let work = tempfile::tempdir().unwrap();
            let p = project(&work, "app");
            let pick = candidate_of(home.path(), "sync", &p);
            let path = pick.path.clone();
            let report = import(
                &store,
                &[pick],
                std::slice::from_ref(&p),
                no_root,
                home_of(home.path()),
            );
            let id = report.imported[0].session.id;
            let raw = store
                .lock()
                .unwrap()
                .setting(&outside_turns::checkpoint_key(id))
                .unwrap()
                .unwrap_or_else(|| panic!("{name}: no checkpoint saved"));
            let checkpoint: Checkpoint = serde_json::from_str(&raw).unwrap();
            let none =
                outside_turns::sync(Kind::Claude, home.path(), "sync", Some(&checkpoint), None);
            assert!(none.turns.is_empty(), "{name}: {:?}", none.turns);

            let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
            use std::io::Write;
            writeln!(file, "{}", user("u2", Some("t1"), "Add a test")).unwrap();
            writeln!(file, "{}", assistant("a2", "u2", "Done.")).unwrap();
            let more =
                outside_turns::sync(Kind::Claude, home.path(), "sync", Some(&checkpoint), None);
            assert_eq!(
                texts(&more.turns),
                vec![
                    (TurnRole::User, "Add a test"),
                    (TurnRole::Assistant, "Done.")
                ],
                "{name}"
            );
        }
    }

    /// An Ask conversation holding `provider_id`, which no session list shows.
    fn ask_session(store: &Store, provider_id: &str) {
        let item = crate::inbox::Item {
            provider: "github".into(),
            kind: "pr".into(),
            number: 3,
            title: "Item 3".into(),
            url: "https://github.com/acme/widgets/pull/3".into(),
            repo: "acme/widgets".into(),
            ..Default::default()
        };
        let context = crate::inbox::ask::Context::new(&item, None);
        let (session, _) = store
            .open_inbox_ask(
                "claude",
                "/project",
                &context,
                &crate::agent::Settings::default(),
            )
            .unwrap();
        store.provider_id(session.id, provider_id).unwrap();
    }

    /// A session the sidebar hides: a hidden Pilot conversation.
    fn hidden_session(store: &Store, provider_id: &str) {
        let session = store.create("claude", "/project").unwrap();
        store.provider_id(session.id, provider_id).unwrap();
        let original = store.persisted_session_id(session.id).unwrap();
        store
            .conn()
            .execute(
                "UPDATE sessions SET sidebar_hidden=1 WHERE id=?1",
                [&original],
            )
            .unwrap();
    }

    #[test]
    fn sessions_gespan_hides_count_as_imported_and_are_never_imported_again() {
        for (name, store) in backends() {
            let home = tempfile::tempdir().unwrap();
            let work = tempfile::tempdir().unwrap();
            let p = project(&work, "app");
            let asked = candidate_of(home.path(), "asked", &p);
            let hidden = candidate_of(home.path(), "hidden", &p);
            {
                let guard = store.lock().unwrap();
                ask_session(&guard, "asked");
                hidden_session(&guard, "hidden");
                assert!(guard.sessions().unwrap().is_empty(), "{name}: both hidden");
                let held = guard.provider_session_ids().unwrap();
                assert!(already_imported(&held, Kind::Claude, "asked"), "{name}");
                assert!(already_imported(&held, Kind::Claude, "hidden"), "{name}");
                assert!(!already_imported(&held, Kind::Codex, "asked"), "{name}");
            }
            let report = import(
                &store,
                &[asked, hidden],
                std::slice::from_ref(&p),
                no_root,
                home_of(home.path()),
            );
            assert!(report.imported.is_empty(), "{name}: {:?}", report.imported);
            let why: Vec<&Skip> = report.skipped.iter().map(|(_, why)| why).collect();
            assert_eq!(
                why,
                [&Skip::AlreadyImported, &Skip::AlreadyImported],
                "{name}"
            );
            assert_eq!(
                store.lock().unwrap().provider_session_ids().unwrap().len(),
                2,
                "{name}: nothing new was created"
            );
        }
    }

    #[test]
    fn a_store_failure_is_reported_as_failed_not_unreadable() {
        for (name, store) in backends() {
            let home = tempfile::tempdir().unwrap();
            let work = tempfile::tempdir().unwrap();
            let p = project(&work, "app");
            let pick = candidate_of(home.path(), "full", &p);
            let table = "sessions";
            store
                .lock()
                .unwrap()
                .conn()
                .execute_batch(&format!(
                    "CREATE TRIGGER no_room BEFORE INSERT ON {table}
                     BEGIN SELECT RAISE(ABORT,'database or disk is full'); END"
                ))
                .unwrap();
            let report = import(
                &store,
                &[pick],
                std::slice::from_ref(&p),
                no_root,
                home_of(home.path()),
            );
            assert!(report.imported.is_empty(), "{name}");
            assert!(
                matches!(&report.skipped[0].1, Skip::Failed(why) if why.contains("disk is full")),
                "{name}: {:?}",
                report.skipped
            );
        }
    }

    #[test]
    fn a_session_whose_provider_id_cannot_be_saved_is_removed_and_failed() {
        for (name, store) in backends() {
            let home = tempfile::tempdir().unwrap();
            let work = tempfile::tempdir().unwrap();
            let p = project(&work, "app");
            let pick = candidate_of(home.path(), "orphan", &p);
            let (table, column) = ("sessions", "provider_session_id");
            store
                .lock()
                .unwrap()
                .conn()
                .execute_batch(&format!(
                    "CREATE TRIGGER no_id BEFORE UPDATE OF {column} ON {table}
                     WHEN NEW.{column} IS NOT NULL
                     BEGIN SELECT RAISE(ABORT,'locked'); END"
                ))
                .unwrap();
            let report = import(
                &store,
                &[pick],
                std::slice::from_ref(&p),
                no_root,
                home_of(home.path()),
            );
            assert!(report.imported.is_empty(), "{name}");
            assert!(
                matches!(&report.skipped[0].1, Skip::Failed(_)),
                "{name}: {:?}",
                report.skipped
            );
            assert!(
                store.lock().unwrap().sessions().unwrap().is_empty(),
                "{name}: the half-made session is removed again"
            );
        }
    }

    #[test]
    fn agent_names_are_the_product_names() {
        assert_eq!(agent_name(Kind::Claude), "Claude Code");
        assert_eq!(agent_name(Kind::Codex), "Codex");
        assert_eq!(agent_name(Kind::Cursor), "Cursor");
        assert_eq!(agent_name(Kind::Kiro), "Kiro");
    }

    #[test]
    fn imported_divider_label() {
        let note = Interjection {
            text: "Claude Code".into(),
            custom_type: "imported".into(),
            severity: None,
        };
        assert_eq!(note.label(), "Imported from Claude Code");
        assert_eq!(note.body(), "", "the agent's name is the label, not a body");
        let notice = Interjection {
            custom_type: "custom".into(),
            ..note
        };
        assert_eq!(notice.body(), "Claude Code");
    }
}

fn kinds_of(sources: &[Source]) -> Vec<Kind> {
    sources.iter().map(|s| s.kind).collect()
}

#[test]
fn sources_lists_only_agents_present_on_disk() {
    let user = tempfile::tempdir().unwrap();
    assert!(sources(user.path()).is_empty());

    // An agent's home without its session folder has nothing to import.
    fs::create_dir_all(user.path().join(".claude")).unwrap();
    fs::create_dir_all(user.path().join(".kiro/settings")).unwrap();
    assert!(sources(user.path()).is_empty());

    fs::create_dir_all(user.path().join(".claude/projects")).unwrap();
    fs::create_dir_all(user.path().join(".cursor/acp-sessions")).unwrap();
    let found = sources(user.path());
    assert_eq!(kinds_of(&found), vec![Kind::Claude, Kind::Cursor]);
    assert_eq!(found[0].home, user.path().join(".claude"));
    assert_eq!(found[1].home, user.path().join(".cursor"));
}

#[test]
fn sources_skips_codex_with_only_archived_sessions() {
    let user = tempfile::tempdir().unwrap();
    fs::create_dir_all(user.path().join(".codex/archived_sessions")).unwrap();
    assert!(
        sources(user.path()).is_empty(),
        "archived threads cannot resume"
    );
    fs::create_dir_all(user.path().join(".codex/sessions")).unwrap();
    let found = sources(user.path());
    assert_eq!(kinds_of(&found), vec![Kind::Codex]);
    assert_eq!(found[0].home, user.path().join(".codex"));
}

#[test]
fn sources_keeps_the_tab_order() {
    let user = tempfile::tempdir().unwrap();
    for dir in [
        ".kiro/sessions",
        ".cursor/acp-sessions",
        ".codex/sessions",
        ".claude/projects",
    ] {
        fs::create_dir_all(user.path().join(dir)).unwrap();
    }
    assert_eq!(
        kinds_of(&sources(user.path())),
        vec![Kind::Claude, Kind::Codex, Kind::Cursor, Kind::Kiro]
    );
}

#[test]
fn default_home_is_none_for_agents_without_sessions_to_import() {
    assert_eq!(default_home(Kind::Grok), None);
}

#[test]
fn present_sources_uses_each_agents_own_home() {
    // Homes moved by CLAUDE_CONFIG_DIR or CODEX_HOME sit anywhere, not under
    // the user's home, and still count.
    let root = tempfile::tempdir().unwrap();
    let claude = root.path().join("elsewhere/claude-config");
    let cursor = root.path().join("cursor");
    let kiro = root.path().join("kiro");
    fs::create_dir_all(claude.join("projects")).unwrap();
    fs::create_dir_all(&cursor).unwrap();
    fs::create_dir_all(kiro.join("sessions")).unwrap();
    let found = present_sources(|kind| match kind {
        Kind::Claude => Some(claude.clone()),
        Kind::Codex => None,
        Kind::Cursor => Some(cursor.clone()),
        Kind::Kiro => Some(kiro.clone()),
        _ => panic!("only importable agents are asked for"),
    });
    assert_eq!(
        found,
        vec![
            Source {
                kind: Kind::Claude,
                home: claude.clone()
            },
            Source {
                kind: Kind::Kiro,
                home: kiro.clone()
            },
        ]
    );
}
