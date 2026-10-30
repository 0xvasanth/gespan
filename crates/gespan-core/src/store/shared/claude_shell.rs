//! Older desktop builds saved some Claude Bash calls as a bare "Shell" row.
//! Claude's own transcript still holds the complete input by tool-use id, so
//! the command comes back when the conversation is opened.
use crate::{tool::ToolCall, transcript::Row};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

const BARE_COMMAND: &str = "Shell";

fn bare_call(row: &Row) -> Option<ToolCall> {
    if row.role != "tool" {
        return None;
    }
    let call = serde_json::from_str::<ToolCall>(&row.text).ok()?;
    let input = serde_json::from_str::<Value>(&call.input).ok()?;
    (call.name == "Bash" && input["command"].as_str()?.trim() == BARE_COMMAND).then_some(call)
}

/// The tool-use ids of rows that lost their command.
pub(super) fn bare_ids(rows: &[Row]) -> HashSet<String> {
    rows.iter()
        .filter_map(bare_call)
        .map(|call| call.id)
        .filter(|id| !id.is_empty())
        .collect()
}

/// Claude names a transcript after its session id. Anything else could walk
/// out of the projects folder.
fn transcript(home: &Path, session: &str) -> Option<PathBuf> {
    let valid = !session.is_empty()
        && session
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if !valid {
        return None;
    }
    let name = format!("{session}.jsonl");
    std::fs::read_dir(home.join("projects"))
        .ok()?
        .flatten()
        .map(|project| project.path().join(&name))
        .find(|path| path.is_file())
}

fn commands(path: &Path, wanted: &HashSet<String>) -> HashMap<String, String> {
    let Ok(file) = File::open(path) else {
        return HashMap::new();
    };
    let mut found = HashMap::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if record["type"] != "assistant" {
            continue;
        }
        let Some(content) = record["message"]["content"].as_array() else {
            continue;
        };
        for block in content {
            let (Some(id), Some(command)) =
                (block["id"].as_str(), block["input"]["command"].as_str())
            else {
                continue;
            };
            if block["name"] == "Bash" && wanted.contains(id) && !command.trim().is_empty() {
                found.insert(id.to_owned(), command.to_owned());
            }
        }
    }
    found
}

fn apply(rows: &mut [Row], commands: &HashMap<String, String>) {
    for row in rows {
        let Some(mut call) = bare_call(row) else {
            continue;
        };
        let Some(command) = commands.get(&call.id) else {
            continue;
        };
        call.input = json!({ "command": command }).to_string();
        if let Some(display) = call.display.as_mut().and_then(Value::as_object_mut) {
            display.insert("title".into(), json!(command));
        }
        if let Ok(text) = serde_json::to_string(&call) {
            row.text = text;
        }
    }
}

/// A missing or unreadable Claude transcript leaves the rows as saved.
pub(super) fn restore(rows: &mut [Row], home: &Path, session: &str) {
    let wanted = bare_ids(rows);
    if wanted.is_empty() {
        return;
    }
    if let Some(path) = transcript(home, session) {
        apply(rows, &commands(&path, &wanted));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash(id: &str, command: &str) -> Row {
        let call = ToolCall {
            id: id.into(),
            name: "Bash".into(),
            input: json!({ "command": command }).to_string(),
            output: String::new(),
            state: crate::tool::ToolState::Completed,
            panel: None,
            display: Some(json!({ "title": command })),
            live_elapsed_secs: None,
        };
        Row {
            role: "tool".into(),
            text: serde_json::to_string(&call).unwrap(),
            first: true,
            sent_at: None,
        }
    }
    fn command_of(row: &Row) -> String {
        let call: ToolCall = serde_json::from_str(&row.text).unwrap();
        call.presentation().command.unwrap_or_default()
    }
    fn home(session: &str, lines: &[Value]) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join("projects/-work-app");
        std::fs::create_dir_all(&project).unwrap();
        let body: Vec<String> = lines.iter().map(Value::to_string).collect();
        std::fs::write(
            project.join(format!("{session}.jsonl")),
            body.join("\n") + "\nnot json\n",
        )
        .unwrap();
        home
    }
    fn assistant(blocks: Value) -> Value {
        json!({"type":"assistant","message":{"content":blocks}})
    }

    #[test]
    fn a_bare_shell_row_gets_its_command_back_by_tool_id() {
        let home = home(
            "abc-1",
            &[assistant(json!([
                {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}},
                {"type":"tool_use","id":"t2","name":"Bash","input":{"command":"git status"}},
            ]))],
        );
        let mut rows = [bash("t1", "Shell"), bash("t3", "Shell"), bash("t2", "ls")];
        restore(&mut rows, home.path(), "abc-1");
        assert_eq!(command_of(&rows[0]), "cargo test");
        assert_eq!(command_of(&rows[1]), "Shell", "no transcript entry");
        assert_eq!(
            command_of(&rows[2]),
            "ls",
            "a real command is never replaced"
        );
    }

    #[test]
    fn only_bash_calls_from_assistant_records_count() {
        let home = home(
            "abc-1",
            &[
                json!({"type":"user","message":{"content":[
                    {"id":"t1","name":"Bash","input":{"command":"from a user record"}}
                ]}}),
                assistant(json!([
                    {"id":"t1","name":"Read","input":{"command":"wrong tool"}},
                    {"id":"t1","name":"Bash","input":{"command":"  "}},
                ])),
            ],
        );
        let mut rows = [bash("t1", "Shell")];
        restore(&mut rows, home.path(), "abc-1");
        assert_eq!(command_of(&rows[0]), "Shell");
    }

    #[test]
    fn a_session_id_cannot_leave_the_projects_folder() {
        let home = home("abc-1", &[]);
        for session in ["", "../abc-1", "abc/1", "abc.1"] {
            assert_eq!(transcript(home.path(), session), None, "{session}");
        }
        assert!(transcript(home.path(), "abc-1").is_some());
        assert_eq!(transcript(home.path(), "other"), None);
    }

    #[test]
    fn a_missing_home_leaves_the_rows_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut rows = [bash("t1", "Shell")];
        let before = rows[0].text.clone();
        restore(&mut rows, &dir.path().join("gone"), "abc-1");
        assert_eq!(rows[0].text, before);
    }
}
