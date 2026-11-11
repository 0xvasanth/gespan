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
