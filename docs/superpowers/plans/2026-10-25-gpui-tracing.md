# gespan-gpui tracing migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every error-path `eprintln!` in `crates/gespan-gpui/src` becomes a structured `tracing` event, and each agent session turn runs inside a span carrying `session_id` and `provider`.

**Architecture:** #68 already installs the global subscriber (`gespan_core::diagnostics::init`): JSONL to the log folder, plus readable stderr lines on the `Dev` channel. This change only emits events into it. Events use fields (`session_id`, `provider`, `action`, `error`, …) instead of formatted strings. `runtime::run_turn` enters an `agent_session` span so everything a turn logs, including `gespan-core` events once #69 lands, carries the session id.

**Tech Stack:** Rust, `tracing 0.1`; tests use `tracing-subscriber` (dev-only) with `gespan_core::diagnostics::json_layer`, so they assert on the real log-line shape.

**Spec:** GitHub issue #70 (approved design). Background: #45. Working rules: #60.

## Global Constraints

- Scope: `crates/gespan-gpui/src/**` only. No changes to `gespan-core` (that's #69).
- Keep `println!` for the CLI subcommands (`--version`, `visual`, `app`, `visual-mcp`).
- Done when: no error-path `eprintln!` remains in `gespan-gpui`, and `bun run native:check` passes.
- Hotspots (`main.rs` mod list, `Cargo.toml`, `Cargo.lock`): small, additive edits only. Regenerate the lock with cargo, never by hand.
- Gespan names only (`src/test/legacyNames.test.ts`).

## Calls I made (not spelled out in #70)

1. **Pre-logger CLI messages stay on stderr.** `main.rs` messages printed before `diagnostics::init` runs (unknown option, fixtures without `--dev-data`, `visual-mcp` failure, and the init failure itself) have no subscriber to go to. They're CLI usage feedback, like the subcommands' `println!`.
2. **Fatal startup errors after init go through `fatal(message, code)`.** It logs at `error`, flushes, and exits. It echoes to stderr only when the readable stderr layer isn't already printing (logging failed, or `Release` channel), so a terminal user still sees why the process quit, without a duplicate line.
3. **`--check-data`'s report stays on stderr.** It's the output of a CLI mode, not an error.
4. **`GESPAN_NATIVE_TRACE` in `conversation.rs` becomes `debug!` events** (session settings action, model pick). Turn them on with `GESPAN_LOG=gespan_gpui::conversation=debug`. The two `gespan-core` uses belong to #69.
5. **Benchmark output in `#[test]`s stays on stderr** (`shell.rs` CoreText frames, `sidebar_list.rs` 1,000-session layout). It's test output, not an error path.
6. **The `agent_session` span is entered in `run_turn`**, around both the local and the remote path. It logs `agent session started`, and `agent turn failed` (warn, with `error`) when a turn ends in an error that isn't a user stop. Without that event the span would carry nothing on the failure path, which is the one we need.
7. **A source-scan test pins the remaining `eprintln!` calls** to the allow-list above, so a sibling branch can't quietly add a new one.

## Review Focus

1. A cancelled (user-stopped) turn must not log `agent turn failed`. Covered by a Task 1 test.
2. A turn rejected before history loads (preflight) still logs the failure, with `session_id`. Covered by a Task 1 test.
3. A release-data launch that can't open its database must still say why on a terminal. Covered by the Task 4 `echo` decision test.
4. A dev launch must not print a fatal error twice (stderr layer plus echo). Same Task 4 test.
5. New `eprintln!` calls from parallel branches. Covered by the Task 4 scan test.

---

### Task 1: Log capture for tests, the `agent_session` span and the turn-failure event

**Files:**
- Modify: `crates/gespan-gpui/Cargo.toml` (add `tracing = "0.1"`; dev: `tracing-subscriber = { version = "0.3", default-features = false, features = ["registry", "std"] }`), `Cargo.lock` via cargo
- Create: `crates/gespan-gpui/src/log_capture.rs` (`#[cfg(test)]`)
- Modify: `crates/gespan-gpui/src/main.rs` (one line: `#[cfg(test)] mod log_capture;`)
- Modify: `crates/gespan-gpui/src/runtime.rs` (`run_turn`, failure branch of `run_turn_using`)
- Test: `crates/gespan-gpui/src/runtime/persistence.rs` tests (they already drive `run_turn` without a provider)

**Interfaces:**
- Produces: `crate::log_capture::capture(body: impl FnOnce()) -> Vec<serde_json::Value>`. It runs `body` under a thread-local registry with `gespan_core::diagnostics::json_layer` and returns one JSON value per line.

- [ ] **Step 1: Write failing tests** in the persistence tests module:
  - `turn_logs_run_inside_an_agent_session_span`: rejected preflight (`worktree_removed: true`). Lines include `message == "agent session started"`, with `span == "agent_session"`, `session_id == id`, and `fields.provider` equal to the session's provider.
  - `rejected_turn_logs_the_failure_with_its_session`: same run. A `warn` line `agent turn failed` with `session_id == id` and `fields.error` containing `"Select a branch or worktree"`.
  - `stopped_turn_logs_no_failure`: the cancelled-delivery setup. No line has `message == "agent turn failed"`.
- [ ] **Step 2: Run** `cargo test --manifest-path crates/gespan-gpui/Cargo.toml --profile native-dev -p gespan-gpui runtime::persistence` and confirm FAIL (no such lines).
- [ ] **Step 3: Implement.** In `run_turn`, enter `info_span!("agent_session", session_id = summary.id, provider = %summary.provider, remote = <bool>)`, then emit `info!("agent session started")`. In `run_turn_using`, before building the final update: `if !cancelled && let Err(error) = &result { warn!(%error, "agent turn failed") }`.
- [ ] **Step 4: Run** the tests and confirm PASS.
- [ ] **Step 5: Commit.**

### Task 2: Error sites (review, board, editor analysis, syntax, keep-awake)

**Files:** `review.rs`, `board/turns.rs`, `file_editor.rs`, `editor_syntax.rs`, `shell/settings.rs`

- [ ] **Step 1: Failing test** `review::tests::failed_actions_log_their_action_and_error`. Calling `log_error("stage_hunk", "index locked")` under capture gives one `error` line with target `gespan_gpui::review`, message `review action failed`, `fields.action == "stage_hunk"`, `fields.error == "index locked"`.
- [ ] **Step 2: Run it** and confirm FAIL (`log_error` takes one argument).
- [ ] **Step 3: Implement.**
  - `log_error(action: &str, error: &str)`. `diff_event` carries the action name (`stage_file`, `discard_file`, `stage_hunk`, …) alongside `id`.
  - Board: `warn!(%error, "board unavailable")`; `warn!(%error, kind = ?kind, card = %id, "moving board cards failed")`.
  - Editor analysis: `warn!(%error, path = %path.display(), "editor analysis failed")`.
  - Grammar: `error!(%error, language = ?lang, "syntax grammar failed")`.
  - Keep-awake: `warn!(%error, mode = ?mode, "keep screen awake failed")`.
- [ ] **Step 4: Run it** and confirm PASS; `cargo clippy` stays clean.
- [ ] **Step 5: Commit.**

### Task 3: Conversation trace lines become `debug!` events

**Files:** `conversation.rs:1954`, `conversation.rs:2161`

- [ ] **Step 1:** Replace the env-gated `eprintln!` calls with `debug!(session_id = self.summary.id, action = %id, "session settings changed")` and `debug!(session_id, provider, model = ?…, "session model picked")`. These lines sit deep inside GPUI entity handlers and there's no seam a test can drive cheaply, so verify them by compile and clippy only. Task 4's scan test guards against regression.
- [ ] **Step 2: Commit.**

### Task 4: `main.rs` startup errors, `fatal`, and the scan guard

**Files:** `crates/gespan-gpui/src/main.rs` (body plus the `native_tests` module)

**Interfaces:**
- Produces: `fn echo_fatal(logging: bool, channel: Channel) -> bool` (true unless logging is on and the channel is `Dev`); `fn fatal(echo: bool, code: i32, message: &str) -> !`.

- [ ] **Step 1: Failing tests** in `native_tests`:
  - `fatal_errors_reach_a_terminal_exactly_once`: `echo_fatal(false, Dev)`, `echo_fatal(false, Release)` and `echo_fatal(true, Release)` are true; `echo_fatal(true, Dev)` is false.
  - `no_new_eprintln_in_gespan_gpui`: walk `src/`, count `eprintln!(` per file, and compare against the allow-list `main.rs: 6` (the five pre-logger and CLI lines plus the one in `fatal`), `shell.rs: 1`, `shell/sidebar_list.rs: 1`. Fail with the file name.
- [ ] **Step 2: Run them** and confirm FAIL.
- [ ] **Step 3: Implement.**
  - Dev-dir-points-at-desktop check, `Cannot open {db}`, and the `--check-data` DB error go through `fatal`.
  - `Cannot read workspace settings` becomes `warn!(error = %e, …)`.
  - `Seeded N rows` becomes `info!(rows, elapsed_ms, "seeded the transcript fixture")`.
- [ ] **Step 4: Run them** and confirm PASS.
- [ ] **Step 5: Commit**, together with this plan.
