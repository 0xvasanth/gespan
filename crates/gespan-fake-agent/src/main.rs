//! A stand-in for the `claude` CLI in Gespan's end-to-end tests.
//!
//! Gespan launches it exactly as it launches Claude Code (same argv, same
//! stream-json on stdin and stdout). Everything it needs lives in the folder
//! that holds the executable, so parallel tests never share state:
//!
//! - `launches.jsonl` (written): one line per start, `{"argv","cwd","env"}`.
//!   `env` keeps the values of `GESPAN_*` variables and only the names of the
//!   rest, so no credential lands in a test folder.
//! - `received.jsonl` (written): every stdin frame, verbatim, in order: the
//!   `initialize` and `mcp_set_servers` control requests and each prompt.
//! - `replies.json` (read): a JSON array, one entry per turn. An entry is a
//!   string (the assistant's text) or `{"text": "...", "frames": [...]}`,
//!   whose raw frames are written before the reply. A turn past the end of
//!   the script answers `fake reply <n>`.
//! - `proxy` (read): when present, its first line is the real CLI. The agent
//!   then starts it with the same argv and only records stdin on the way
//!   through, so a real-model run still shows what Gespan sent.
//!
//! Without `proxy`, each `user` frame gets `system/init`, one `assistant`
//! message and a `result`, then the agent waits for the next turn on the same
//! stdin, like a warm Claude process. Control requests get a `success`
//! answer. It exits when stdin closes.
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::{BufRead, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
};

fn main() -> ExitCode {
    let home = match std::env::current_exe() {
        Ok(exe) => exe.parent().map(Path::to_path_buf).unwrap_or_default(),
        Err(error) => {
            eprintln!("gespan-fake-agent: no executable path: {error}");
            return ExitCode::FAILURE;
        }
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    record_launch(&home, &args);
    match std::fs::read_to_string(home.join("proxy")) {
        Ok(real) => proxy(&home, real.lines().next().unwrap_or("").trim(), &args),
        Err(_) => fake(&home, &args),
    }
}

fn append(path: &Path, line: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

fn record_launch(home: &Path, args: &[String]) {
    let env: serde_json::Map<String, Value> = std::env::vars_os()
        .map(|(key, value)| {
            let key = key.to_string_lossy().into_owned();
            let value = if key.starts_with("GESPAN_") {
                Value::String(value.to_string_lossy().into_owned())
            } else {
                Value::Null
            };
            (key, value)
        })
        .collect();
    let cwd = std::env::current_dir()
        .map(|dir| dir.to_string_lossy().into_owned())
        .unwrap_or_default();
    append(
        &home.join("launches.jsonl"),
        &json!({"argv": args, "cwd": cwd, "env": env}).to_string(),
    );
}

fn proxy(home: &Path, real: &str, args: &[String]) -> ExitCode {
    let mut child = match Command::new(PathBuf::from(real))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!("gespan-fake-agent: cannot start {real}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut input = child.stdin.take();
    let received = home.join("received.jsonl");
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        append(&received, &line);
        let sent = input
            .as_mut()
            .map(|stdin| writeln!(stdin, "{line}").and_then(|()| stdin.flush()));
        if !matches!(sent, Some(Ok(()))) {
            break;
        }
    }
    drop(input);
    match child.wait() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

/// The value after `flag` in argv.
fn flag<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|at| args.get(at + 1))
        .map(String::as_str)
}

fn fake(home: &Path, args: &[String]) -> ExitCode {
    let session = flag(args, "--session-id")
        .or_else(|| flag(args, "--resume"))
        .unwrap_or("fake-session")
        .to_owned();
    let model = flag(args, "--model").unwrap_or("fake-model").to_owned();
    let received = home.join("received.jsonl");
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut send =
        |value: Value| -> bool { writeln!(out, "{value}").and_then(|()| out.flush()).is_ok() };
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        append(&received, &line);
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let ok = match frame["type"].as_str() {
            Some("control_request") => send(json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": frame["request_id"],
                    "response": {},
                },
            })),
            Some("user") => {
                let turn = turns(&received);
                reply(home, turn, &session, &model, &mut send)
            }
            _ => true,
        };
        if !ok {
            break;
        }
    }
    ExitCode::SUCCESS
}

/// How many prompts the folder's log holds, this one included.
fn turns(received: &Path) -> usize {
    std::fs::read_to_string(received)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|frame| frame["type"] == "user")
        .count()
}

fn reply(
    home: &Path,
    turn: usize,
    session: &str,
    model: &str,
    send: &mut impl FnMut(Value) -> bool,
) -> bool {
    let script: Vec<Value> = std::fs::read_to_string(home.join("replies.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let entry = script.get(turn.saturating_sub(1));
    let text = match entry {
        Some(Value::String(text)) => text.clone(),
        Some(entry) => entry["text"].as_str().unwrap_or("").to_owned(),
        None => format!("fake reply {turn}"),
    };
    let frames = entry
        .and_then(|entry| entry["frames"].as_array())
        .cloned()
        .unwrap_or_default();
    let usage = json!({"input_tokens": 1, "output_tokens": 1});
    let mut ok = send(json!({
        "type": "system", "subtype": "init", "session_id": session,
        "model": model, "cwd": std::env::current_dir().ok(), "tools": [],
    }));
    for frame in frames {
        ok = ok && send(frame);
    }
    if !text.is_empty() {
        ok = ok
            && send(json!({
                "type": "assistant", "session_id": session, "parent_tool_use_id": null,
                "message": {
                    "id": format!("fake-message-{turn}"), "role": "assistant", "model": model,
                    "content": [{"type": "text", "text": text}], "usage": usage,
                },
            }));
    }
    ok && send(json!({
        "type": "result", "subtype": "success", "is_error": false, "num_turns": 1,
        "result": text, "session_id": session, "usage": usage, "total_cost_usd": 0.0,
    }))
}
