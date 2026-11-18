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
//!   the script answers `fake reply <n>`. `"error": true` ends that turn
//!   with a failed result. `"mcp": [{"tool": "...", "arguments": {...}}]`
//!   calls Gespan's own tools over the MCP endpoint Gespan mounted, before the
//!   reply, writing each call's tool_use and tool_result frames and a line to
//!   `mcp.jsonl` (`{"tool","arguments","result"}` or `"error"`). `"wait":
//!   true` holds the turn open until Gespan interrupts it (the turn ends as
//!   failed) or a `release` file appears next to the agent (it then goes on).
//! - `mcp.jsonl` (written): one line per tool call, as above.
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
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    sync::mpsc,
    time::Duration,
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
    // Stdin is read on a thread of its own, so a held turn still sees an
    // interrupt.
    let (lines, input) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut send =
        |value: Value| -> bool { writeln!(out, "{value}").and_then(|()| out.flush()).is_ok() };
    let mut agent = Agent {
        home: home.to_owned(),
        session,
        model,
        mcp: None,
        input,
    };
    while let Ok(line) = agent.input.recv() {
        append(&received, &line);
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let ok = match frame["type"].as_str() {
            Some("control_request") => agent.control(&frame, &mut send),
            Some("user") => {
                let turn = turns(&received);
                agent.reply(turn, &mut send)
            }
            _ => true,
        };
        if !ok {
            break;
        }
    }
    ExitCode::SUCCESS
}

/// Gespan's tool endpoint and the credential it mounted for this process.
struct Endpoint {
    url: String,
    authorization: String,
}

struct Agent {
    home: PathBuf,
    session: String,
    model: String,
    mcp: Option<Endpoint>,
    input: mpsc::Receiver<String>,
}

impl Agent {
    /// Answers a control request, keeping the MCP endpoint it mounts.
    fn control(&mut self, frame: &Value, send: &mut impl FnMut(Value) -> bool) -> bool {
        let request = &frame["request"];
        if request["subtype"] == "mcp_set_servers" {
            if let Some(server) = request["servers"]
                .as_object()
                .and_then(|s| s.values().next())
            {
                self.mcp = Some(Endpoint {
                    url: server["url"].as_str().unwrap_or_default().to_owned(),
                    authorization: server["headers"]["Authorization"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                });
            }
        }
        send(json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": frame["request_id"],
                "response": {},
            },
        }))
    }

    fn reply(&mut self, turn: usize, send: &mut impl FnMut(Value) -> bool) -> bool {
        let home = self.home.clone();
        let (session, model) = (self.session.clone(), self.model.clone());
        let script: Vec<Value> = std::fs::read_to_string(home.join("replies.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let entry = script.get(turn.saturating_sub(1)).cloned();
        let text = match &entry {
            Some(Value::String(text)) => text.clone(),
            Some(entry) => entry["text"].as_str().unwrap_or("").to_owned(),
            None => format!("fake reply {turn}"),
        };
        let field = |key: &str| entry.as_ref().map(|entry| entry[key].clone());
        let frames = field("frames")
            .and_then(|frames| frames.as_array().cloned())
            .unwrap_or_default();
        let usage = json!({"input_tokens": 1, "output_tokens": 1});
        let mut ok = send(json!({
            "type": "system", "subtype": "init", "session_id": session,
            "model": model, "cwd": std::env::current_dir().ok(), "tools": [],
        }));
        for frame in frames {
            ok = ok && send(frame);
        }
        let calls = field("mcp")
            .and_then(|calls| calls.as_array().cloned())
            .unwrap_or_default();
        for (n, call) in calls.iter().enumerate() {
            ok = ok && self.call_tool(turn, n, call, send);
        }
        if field("wait") == Some(Value::Bool(true)) && !self.hold(send) {
            return ok
                && send(json!({
                    "type": "result", "subtype": "error_during_execution",
                    "is_error": true, "num_turns": 1, "result": "",
                    "session_id": session, "usage": usage, "total_cost_usd": 0.0,
                }));
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
        // `{"error": true}` ends the turn as failed, as an API error would.
        let failed = field("error") == Some(Value::Bool(true));
        ok && send(json!({
            "type": "result",
            "subtype": if failed { "error_during_execution" } else { "success" },
            "is_error": failed, "num_turns": 1,
            "result": text, "session_id": session, "usage": usage, "total_cost_usd": 0.0,
        }))
    }

    /// Holds a turn open. `true` when released, `false` when interrupted or
    /// when stdin closed.
    fn hold(&mut self, send: &mut impl FnMut(Value) -> bool) -> bool {
        let release = self.home.join("release");
        let received = self.home.join("received.jsonl");
        loop {
            if std::fs::remove_file(&release).is_ok() {
                return true;
            }
            match self.input.recv_timeout(Duration::from_millis(20)) {
                Ok(line) => {
                    append(&received, &line);
                    let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if frame["type"] == "control_request" {
                        let interrupted = frame["request"]["subtype"] == "interrupt";
                        self.control(&frame, send);
                        if interrupted {
                            return false;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return false,
            }
        }
    }

    /// One scripted call to Gespan's tools, shown as Claude shows an MCP call.
    fn call_tool(
        &self,
        turn: usize,
        n: usize,
        call: &Value,
        send: &mut impl FnMut(Value) -> bool,
    ) -> bool {
        let tool = call["tool"].as_str().unwrap_or_default();
        let arguments = call.get("arguments").cloned().unwrap_or(json!({}));
        let id = format!("toolu_mcp_{turn}_{n}");
        let mut ok = send(json!({
            "type": "assistant", "session_id": self.session, "parent_tool_use_id": null,
            "message": {
                "id": format!("fake-mcp-{turn}-{n}"), "role": "assistant", "model": self.model,
                "content": [{
                    "type": "tool_use", "id": id, "name": format!("mcp__gespan__{tool}"),
                    "input": arguments,
                }],
                "usage": {"input_tokens": 1, "output_tokens": 1},
            },
        }));
        let result = match &self.mcp {
            Some(endpoint) => mcp_call(endpoint, tool, &arguments),
            None => Err("no MCP server was mounted".to_owned()),
        };
        let line = match &result {
            Ok(value) => json!({"tool": tool, "arguments": arguments, "result": value}),
            Err(error) => json!({"tool": tool, "arguments": arguments, "error": error}),
        };
        append(&self.home.join("mcp.jsonl"), &line.to_string());
        let (content, is_error) = match result {
            Ok(value) => (value.to_string(), false),
            Err(error) => (error, true),
        };
        ok = ok
            && send(json!({
                "type": "user", "parent_tool_use_id": null,
                "message": {"role": "user", "content": [{
                    "type": "tool_result", "tool_use_id": id, "content": content, "is_error": is_error,
                }]},
            }));
        ok
    }
}

/// `initialize`, `notifications/initialized` and one `tools/call` over MCP's
/// streamable HTTP. The tool's JSON answer, or its error text.
fn mcp_call(endpoint: &Endpoint, tool: &str, arguments: &Value) -> Result<Value, String> {
    let (session, _) = post(
        endpoint,
        None,
        &json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},
            "clientInfo":{"name":"gespan-fake-agent","version":"0"},
        }}),
    )?;
    post(
        endpoint,
        session.as_deref(),
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )?;
    let (_, reply) = post(
        endpoint,
        session.as_deref(),
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":arguments}}),
    )?;
    let reply = reply.ok_or("the endpoint did not answer the call")?;
    if let Some(error) = reply["error"]["message"].as_str() {
        return Err(error.to_owned());
    }
    let text = reply["result"]["content"][0]["text"].as_str().unwrap_or("");
    if reply["result"]["isError"] == true {
        return Err(text.to_owned());
    }
    Ok(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned())))
}

/// One POST. Returns the MCP session it named and the JSON-RPC answer with
/// the request's id, read from a JSON body or an event stream.
fn post(
    endpoint: &Endpoint,
    session: Option<&str>,
    body: &Value,
) -> Result<(Option<String>, Option<Value>), String> {
    let mut request = ureq::post(&endpoint.url)
        .timeout(Duration::from_secs(120))
        .set("Authorization", &endpoint.authorization)
        .set("Accept", "application/json, text/event-stream")
        .set("Content-Type", "application/json");
    if let Some(session) = session {
        request = request.set("Mcp-Session-Id", session);
    }
    let response = request
        .send_string(&body.to_string())
        .map_err(|error| error.to_string())?;
    let named = response.header("mcp-session-id").map(str::to_owned);
    let Some(id) = body.get("id").cloned() else {
        return Ok((named, None));
    };
    let stream = response
        .header("content-type")
        .is_some_and(|kind| kind.starts_with("text/event-stream"));
    let mut reader = std::io::BufReader::new(response.into_reader());
    if !stream {
        let mut text = String::new();
        reader
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        return Ok((named, serde_json::from_str(&text).ok()));
    }
    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        if let Ok(message) = serde_json::from_str::<Value>(data.trim()) {
            if message["id"] == id {
                return Ok((named, Some(message)));
            }
        }
    }
    Ok((named, None))
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
