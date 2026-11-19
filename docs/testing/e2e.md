# End-to-end tests

Features are verified by tests that drive a real Gespan window, not by
launching or screen-recording the app. An end-to-end test builds a `Shell` in
a GPUI `TestAppContext`, starts a session from New chat, types into the
composer and presses Enter, and the turn runs through a real provider
**process**, the same `provider::run` path the app uses, over Claude Code's
stream-json.

The harness is `crates/gespan-gpui/src/e2e/mod.rs` (test builds only). Its
first tests are in `e2e/hello_tests.rs`.

```rust
#[gpui::test]
fn hi_reaches_the_agent_and_its_reply_shows(cx: &mut TestAppContext) {
    let e2e = E2e::new(cx, Mode::from_env());
    let id = e2e.start_session(Kind::Claude, cx);
    e2e.script_reply(id, "Hello from the fake agent");
    let reply = e2e.send(id, "hi", cx);
    assert!(e2e.prompts(id)[0].contains("hi"));
    // ...
}
```

## The harness

| Call | What it does |
| --- | --- |
| `E2e::new(cx, mode)` | A temporary git repository (one commit) opened as the window's only project. |
| `start_session(kind, cx) -> i64` | New chat in that project, with an agent folder of its own. |
| `script_reply(id, text)` | Queues the fake agent's text for the session's next turn. |
| `script_frames(id, json!({"text": .., "frames": [..]}))` | A turn with raw stream-json frames (tool calls, tasks) written before its text. |
| `send(id, text, cx) -> String` | Types `text`, presses Enter, runs until the turn ends (120 s fake, 300 s real). Returns the assistant's reply. |
| `prompts(id) -> Vec<String>` | Each prompt the agent process actually received, after Gespan wrapped it. |
| `received(id) -> Vec<Value>` | Every stdin frame: `initialize`, `mcp_set_servers`, prompts. |
| `launches(id) -> Vec<Value>` | One `{argv, cwd, env}` per agent process. `env` keeps `GESPAN_*` values and only the names of other variables. |
| `transcript(id, cx) -> Vec<Row>` | The rows the chat's transcript shows. |
| `session(id, cx)`, `store(cx)`, `window()`, `repo()` | The live `Conversation`, the store, the window, the project. |
| `script_frames(id, json!({"mcp": [{"tool": .., "arguments": {..}}]}))` | The turn calls Gespan's own tools (`sessions_start`, …) over the MCP endpoint Gespan mounted, for real, before its text. |
| `mcp_calls(id) -> Vec<Value>` | Each such call with its `result` or `error`. |
| `script_frames(id, json!({"text": .., "wait": true}))`, `release(id)` | A turn that keeps working until Gespan stops it (it then ends as failed) or the test releases it. |
| `script_next_child(json!([..]))` | Scripts the next session the app starts by itself, as `sessions_start` does. Every such session gets an agent folder of its own. |
| `until(cx, what, done)` | Runs the app until `done` holds, failing after the turn timeout. |
| `busy(id, cx)`, `all_sessions(cx)` | Whether a session is in a turn; every open session. |

## Fake mode (default)

`gespan-fake-agent` (`crates/gespan-fake-agent`, a Rust binary with only
`serde_json` and `ureq` for its MCP calls, so it runs the same on macOS, Windows and Linux) stands in for
`claude`. It answers each control request with `success`, and each prompt with
`system/init`, one `assistant` message and a `result`, then waits on the same
stdin for the next turn, like a warm Claude process. Its replies come from
`replies.json` in its folder, which `script_reply` writes; a turn past the end
of the script answers `fake reply <n>`.

The harness builds the agent once per test process with
`cargo build --manifest-path crates/gespan-fake-agent/Cargo.toml` into
`<target>/e2e-fake-agent`, a target folder of its own so the build never waits
on the one running the tests. To use a prebuilt binary instead, set
`GESPAN_E2E_FAKE_AGENT=/path/to/gespan-fake-agent`.

Each session gets its own copy of the binary in its own folder, and it reads
and writes only that folder, so tests run in parallel without sharing
anything.

```sh
cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-gpui e2e::
```

## Real mode (opt-in)

```sh
GESPAN_E2E_REAL=1 cargo test --manifest-path crates/gespan-gpui/Cargo.toml \
  -p gespan-gpui e2e::hello_tests::hi_reaches_the_agent_and_its_reply_shows
```

`Mode::from_env()` returns `Mode::Real` when `GESPAN_E2E_REAL=1`. The session
still runs the fake agent's binary, but with a `proxy` file naming the real
CLI, resolved as the app resolves it (`provider::resolve`: Settings path,
`GESPAN_CLAUDE_BINARY`, then the usual install folders). The agent starts the
real CLI with Gespan's argv, passes stdout and stderr straight through, and
records stdin on the way, so `prompts`, `received` and `launches` work the
same. Replies are the model's, so tests check them loosely: branch on
`e2e.mode()` and keep exact text for `Mode::Fake`. Tests written only for
`Mode::Fake` (with `E2e::new(cx, Mode::Fake)`) stay fake under the variable.

Real mode spends tokens on the signed-in account and needs network access;
CI does not run it.

A real CLI in a supervised chat asks before each of Gespan's tools. While a
turn runs, the harness clicks Allow once on approvals for `sessions_*` tools
only, as a user would, and `allowed()` lists them; any other approval still
blocks the turn. `live_tests.rs` and the last test in `remote_tests.rs` tell
the chat in plain words what to do, so in real mode the model picks the calls
itself:

```sh
GESPAN_E2E_REAL=1 cargo nextest run --manifest-path crates/gespan-gpui/Cargo.toml \
  -p gespan-gpui -E 'test(e2e::live_tests) | test(told_to_message_the_agent)'
```

## How a session's agent is chosen

`start_session` saves the agent's path in the session's store under
`e2e:agent-binary:<session id>`. In test builds `runtime::run_turn` asks
`e2e::agent_binary` first; when it names a binary, the turn uses it instead of
`provider::resolve`, and runs on a thread of its own. Nothing else changes:
sessions without the setting, and every non-test build, resolve the CLI as
before. The setting lives in the test's own in-memory store, so no process
environment variable (which parallel tests would race on) is involved.

The turn needs its own thread because GPUI's test executor runs background
tasks on the test thread: a turn that blocks on a child process there would
stop the foreground from draining its updates. `send` therefore alternates
`run_until_parked` with short real-time waits until the chat is idle.

## Writing a new end-to-end test

- Put it next to the feature or under `crates/gespan-gpui/src/e2e/`, and drive
  it the way a user would: `start_session`, `send`, the views' own actions.
- Assert on what crossed the process boundary (`prompts`, `received`,
  `launches`) and on what the user sees (`transcript`, the views' state).
- Script tool calls and other events with `script_frames`, in Claude's
  stream-json shapes (see `crates/gespan-core/src/provider/claude.rs`).
