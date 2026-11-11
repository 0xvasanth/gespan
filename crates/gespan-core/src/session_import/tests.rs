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
fn codex_lists_zst_and_archived_rollouts() {
    let home = tempfile::tempdir().unwrap();
    let body = [
        codex_meta("/tmp/z", None),
        codex_event("user_message", "Compressed question"),
        codex_event("agent_message", "Compressed answer"),
    ]
    .join("\n")
        + "\n";
    let zst_id = "11111111-2222-3333-4444-555555555555";
    let dir = home.path().join("archived_sessions/2026/10/01");
    fs::create_dir_all(&dir).unwrap();
    let zst = dir.join(rollout_name(zst_id, "jsonl.zst"));
    fs::write(&zst, zstd::encode_all(body.as_bytes(), 3).unwrap()).unwrap();
    write(
        home.path(),
        &format!("archived_sessions/{}", rollout_name(CODEX_ID, "jsonl")),
        &[
            codex_meta("/tmp/a", None),
            codex_event("user_message", "Archived question"),
        ],
    );

    let mut found = list(Kind::Codex, home.path());
    found.sort_by(|a, b| a.provider_id.cmp(&b.provider_id));
    let ids: Vec<_> = found.iter().map(|c| c.provider_id.as_str()).collect();
    assert_eq!(ids, [CODEX_ID, zst_id]);
    let compressed = &found[1];
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
