# gespan-core tracing migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every `eprintln!` in `gespan-core` becomes a structured `tracing` event, and git operations, worktree create/remove, app update, MCP start and remote SSH connect run inside spans whose failures are logged at `warn`.

**Architecture:** No new modules. Each call site uses `tracing` directly (`warn!`/`debug!` with fields, `info_span!(...).entered()` around the operation, `Result::inspect_err` to log the failure inside the span). A test in `crates/gespan-core/tests/` scans `src/**/*.rs` and fails on any `eprintln!`/`eprint!` outside a comment.

**Tech Stack:** Rust, `tracing` 0.1 (already a dependency since #68), the #68 JSONL layer for assertions.

**Spec:** GitHub issue #69 (approved design); background in #45; working rules in #60.

## Global Constraints

- Scope is `crates/gespan-core/src/**` only. `gespan-gpui` is #70.
- "CLI output that prints results stays as `println!`." `cli.rs` and other `println!` sites are not touched.
- "No error-path `eprintln!` remains in `gespan-core`; a grep check runs in a test or in CI."
- "Existing tests pass." Gate: `bun run native:check` and `bun run check`.
- Hotspots (`diagnostics/mod.rs`, `lib.rs`, Cargo files) get only small additive edits. This plan adds one test helper to `diagnostics/mod.rs` and nothing to Cargo files.
- Gespan names only; `src/test/legacyNames.test.ts` must pass.

## Decisions made here (my calls, not in the issue)

1. **Zero `eprintln!` in `gespan-core`, not only error paths.** The grep check is simplest and strictest as "none outside comments". The two `GESPAN_NATIVE_TRACE` developer traces in `provider.rs` become `tracing::debug!` events, and the `eprintln!` in an `#[ignore]` live test in `git.rs` becomes `println!` (test output).
2. **`GESPAN_NATIVE_TRACE` is dropped in `gespan-core`.** The provider traces are enabled with `GESPAN_LOG=gespan_core::provider=debug` instead (dev builds print them on stderr through #68's readable layer). The per-frame trace computes its fields only when `tracing::enabled!(Level::DEBUG)`. The gpui uses of the variable belong to #70.
3. **Spans are `info` level**, so their names and fields are attached to `warn` events under the default `info` filter. The #68 JSON layer writes nothing for a span on its own, so a span that sees no event costs no log line.
4. **Fields avoid personal data.** Span fields are ids, flags and enum-like names: `op` (the git subcommand only, never its arguments), `session_id` (lifted to the top-level `session_id` by the #68 layer), `step`, `version`, `server`, `reconnect`, `upgrade`. No SSH target (`user@host`), branch names or URLs. Error text is logged as `error`; it can contain paths, which stay on the user's machine and are scrubbed by #67/#72 at report time.
5. **Where failures are logged.** The low-level `git()` helper logs a non-zero exit at `debug` (many callers probe and expect failure) and a failure to start Git at `error`. The operations named in the issue log their `Err` at `warn` once, at the operation level. A refused worktree removal (main copy, locked, detached) is also logged at `warn`: it is rare and useful when someone reports "delete does nothing". A cancelled SSH connection is logged at `info`, not `warn`.
6. **MCP start** means the two per-session tool servers `gespan-core` starts: `visual_bridge::Server::start` (`server = "visual"`) and `pilot::bridge::Server::start` (`server = "pilot"`). `mcp.rs` only edits provider config files and is not a start.
7. **The SSH span is created inside the connection thread** in `Connections::start_ssh_job`, around both the preparation and `Job::complete`; the failure event is emitted in `Job::complete` so every SSH job outcome is logged in one place.
9. **Review follow-up: user Git actions and background SSH.** `scm.rs`'s stage/unstage/discard/commit/push/pull/sync run Git through their own helper, not `git::git`, so each runs in a `git_action { op }` span and logs its failure at `warn` once; the internal `git_checked` probe stays silent. The automatic tunnel reopen in `Tunnels::endpoint` runs in `ssh_connect { background = true }` and logs each failed attempt (the existing 10 s backoff rate-limits it).
10. **OpenSSH stderr is logged as-is.** It can name the host (`Could not resolve hostname …`). That is the same trade as Decision 4: the log stays local, and report-time scrubbing (#67/#72) owns redaction; adding the SSH target to the scrub list is listed as a follow-up.
8. **Grep check is an integration test** (`crates/gespan-core/tests/no_eprintln.rs`) so it runs in `native:check` and CI with no new workflow step.

## Review Focus

- A git probe that is expected to fail (e.g. `show-ref` while picking a free branch name) must not write a `warn` line → Task 2 test `a_failing_git_command_logs_only_at_debug`.
- A comment that mentions `eprintln!` (as `diagnostics/mod.rs` does) must not fail the grep check, but code after a comment-free prefix must → Task 1 test `offenders_ignore_comments_only`.
- A user cancelling an SSH connection must not look like a failure → Task 4 test `a_cancelled_ssh_job_logs_at_info`.
- Logged fields must not include the SSH target or branch names → Task 2/4 tests assert the exact `fields` objects.
- Starting a second visual turn for a busy session is refused and logged with that session's id → Task 4 test `a_refused_visual_server_start_is_logged`.

---

### Task 1: The grep check and the five `eprintln!` sites

**Files:**
- Create: `crates/gespan-core/tests/no_eprintln.rs`
- Modify: `crates/gespan-core/src/flow/store.rs:77`, `crates/gespan-core/src/desktop_preferences/webview.rs:152`, `crates/gespan-core/src/provider.rs:1431-1440,1710-1726`, `crates/gespan-core/src/git.rs:593`

**Interfaces:**
- Produces: `fn offenders(path: &str, source: &str) -> Vec<String>` (test-local; `"path:line"` for each line whose text before `//` contains `eprintln!` or `eprint!`).

- [ ] **Step 1: Write the failing tests**
  - `offenders_ignore_comments_only`: `"// as eprintln! did"` and `"    /// eprint!"` → none; `"let x = 1; eprintln!(\"a\");"` and `"eprint!(\"b\") // note"` → both reported with line numbers.
  - `gespan_core_has_no_eprintln`: walks `CARGO_MANIFEST_DIR/src` recursively over `*.rs`, asserts `offenders` is empty, listing them in the message.
  - Extend `listings_skip_a_row_whose_json_does_not_parse` (flow store): wrap `s.list()` in `crate::diagnostics::test_capture`; assert one `warn` line with `fields == {"table":"flows","id":"bad","error":<non-empty>}` and message `"skipped an unreadable row"`.
  - Extend `webview2_corrupt_tables_preserve_source_and_native_workspace`: wrap `load_webview2_from` in `test_capture`; assert a `warn` line with message `"could not read WebView2 preferences"` and non-empty `fields.path` and `fields.error`.
- [ ] **Step 2: Run** `cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-core --profile native-dev --test no_eprintln` and the two unit tests → FAIL (5 offenders; no warn lines).
- [ ] **Step 3: Implement** the replacements: `tracing::warn!(table, id = %id, %error, ...)`; `tracing::warn!(path = %path.display(), %error, ...)`; provider traces as `tracing::debug!` per Decision 2; `println!` in the ignored test.
- [ ] **Step 4: Run** the same tests → PASS.
- [ ] **Step 5: Commit.**

### Task 2: Git and worktree spans

**Files:**
- Modify: `crates/gespan-core/src/git.rs` (`git`, `create_worktree`, `create_project_worktree`), `crates/gespan-core/src/worktree_management.rs` (`remove`), `crates/gespan-core/src/diagnostics/mod.rs` (one test helper)

**Interfaces:**
- Produces: `#[cfg(test)] pub(crate) fn test_capture_with(spec: &str, body: impl FnOnce()) -> Vec<serde_json::Value>` in `diagnostics/mod.rs` (as `test_capture`, with a `GESPAN_LOG`-style filter).
- Spans: `git { op }`; `worktree_create { session_id }` (session worktrees) / `worktree_create { existing }` (named/project worktrees); `worktree_remove {}`.

- [ ] **Step 1: Failing tests** (in the existing `git.rs` / `worktree_management.rs` test modules, using their `repo()` fixtures):
  - `a_failing_git_command_logs_only_at_debug`: `test_capture` (info) around `git(&cwd, &["show-ref", "--verify", "refs/heads/nope"])` → `Err`, no lines. `test_capture_with("gespan_core::git=debug", …)` → one `debug` line, `span == "git"`, `fields.op == "show-ref"`, `fields.status` present, no `refs/heads/nope` anywhere in the line.
  - `a_failed_worktree_create_is_logged_with_its_session`: `create_worktree(&cwd, "no-such-base", 7)` → `Err`; one `warn` line, `span == "worktree_create"`, `session_id == 7`, `fields == {"error": <the Err text>}`.
  - `a_refused_worktree_removal_is_logged`: `remove(dir, dir)` (main copy) → one `warn` line, `span == "worktree_remove"`, `fields.error` contains `"main"`. A successful `remove` writes no `warn`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** the spans with `.entered()` and `inspect_err(|error| tracing::warn!(%error, "…"))`; in `git()`, `error!` when Git cannot start, `debug!(op, status = out.status.code(), …)` on non-zero exit.
- [ ] **Step 4: Run** → PASS, plus the full `git` and `worktree_management` test modules.
- [ ] **Step 5: Commit.**

### Task 3: App update spans

**Files:**
- Modify: `crates/gespan-core/src/app_update/mod.rs` (`check`, `Update::download`, `Update::install`), `crates/gespan-core/src/app_update/tests.rs`

**Interfaces:**
- Spans: `app_update { step = "check" }`, `app_update { step = "download", version }`, `app_update { step = "install", version }`.

- [ ] **Step 1: Failing tests:**
  - `a_failed_update_check_is_logged`: `Config::new("https://127.0.0.1:9/latest.json", KEY, "0.16.0")` → `check(..)` is `Err`; one `warn` line, `span == "app_update"`, `fields.step == "check"`, `fields.error` non-empty.
  - `a_tampered_install_is_logged_with_its_version`: fixture update, `Download` over a tampered file → `install` is `Err`; one `warn` line, `fields.step == "install"`, `fields.version == "0.16.0-preview.2"`.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS with the rest of `app_update`. **Step 5: Commit.**

### Task 4: MCP start and SSH connect spans

**Files:**
- Modify: `crates/gespan-core/src/visual_bridge.rs` (`Server::start`), `crates/gespan-core/src/pilot/bridge.rs` (`Server::start`), `crates/gespan-core/src/remote.rs` (`start_ssh_job`), `crates/gespan-core/src/remote/ssh.rs` (`Job::complete`)

**Interfaces:**
- Spans: `mcp_start { server = "visual", session_id }`, `mcp_start { server = "pilot" }`, `ssh_connect { reconnect, upgrade }`.

- [ ] **Step 1: Failing tests:**
  - `a_refused_visual_server_start_is_logged`: hold `Server::start(91)`, start 91 again → `Err`; one `warn` line, `span == "mcp_start"`, `session_id == 91`, `fields == {"server":"visual","error":"A visual session turn is already active"}`.
  - `a_failed_ssh_job_logs_at_warn`: `Job::new().complete(|| Err("Permission denied".into()))` inside `info_span!("ssh_connect", reconnect = false, upgrade = false)` → one `warn` line, `span == "ssh_connect"`, `fields == {"reconnect":false,"upgrade":false,"error":"Permission denied"}`; the job view has the error.
  - `a_cancelled_ssh_job_logs_at_info`: `job.cancel()` then `complete(..)` → one `info` line, no `warn`.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS with `visual_bridge`, `pilot::bridge`, `remote` modules. **Step 5: Commit.**

### Task 5: Verify

- [ ] `bun run native:check` → `test result: ok` for every suite, fmt and clippy clean.
- [ ] `bun run check` → all green (includes `legacyNames.test.ts`).
