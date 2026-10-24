# Claude Heartbeat Frames Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Claude's 30-second "still running" heartbeat frames must never appear as steps or blocks. Long-running Agent and Bash calls should show as one running row that knows how long it has run. Sessions that already saved heartbeats should render cleanly. This applies to both Rebase Native and Tauri Rebase.

**Architecture:** Both parsers recognise a heartbeat before any routing. A heartbeat is a `tool_progress` frame with `heartbeat: true`, or one whose `tool_use_id` is `<parent_tool_use_id>-heartbeat-<n>`. The parser never turns it into a step or a block. It records the elapsed time on the parent tool, live only. At display time, both UIs hide steps saved earlier whose id ends in `-heartbeat-<n>`, and hide synthetic `background:` blocks whose steps are all heartbeats. Stored data is never rewritten.

**Tech Stack:** Rust (`crates/rebase-core`, `crates/rebase-gpui`), TypeScript/React (`src/`, `host/`), Vitest, cargo, Bun.

**Spec:** Capture and analysis from 2026-10-24, summarized under Evidence. User request: "All these issues need to be fixed in both native code and the Tauri build. Make sure we build in both places."

## Evidence

Claude Code 2.1.309, run with Rebase Native's exact flags. Captures are in `/private/tmp/claude-501/heartbeat-test/stdout-A.jsonl` (Agent) and `stdout-C.jsonl` (Bash).

- **Heartbeat frame shape.** While any tool call runs, Claude emits a top-level frame every ~30 s: `{"type":"tool_progress","tool_use_id":"<P>-heartbeat-<n>","tool_name":"<Agent|Bash|…>","parent_tool_use_id":"<P>","heartbeat":true,"elapsed_time_seconds":30*(n+1)}`. `<P>` is the running call itself.
- **No completion.** No result or completion ever references a heartbeat id.
- **Native.** `crates/rebase-core/src/provider/tasks.rs:1019-1039` (`claude_progress`) turns an Agent heartbeat into a permanently Running "Agent" step inside the parent's own run. A Bash heartbeat reaches the rule-4 fallback (`tasks.rs:795-800`) and becomes a fake "Background agent" block of "Bash" steps. That fallback arrived with PR #33; before it, these frames were silently dropped.
- **Tauri.** `claudeProtocol.ts:803-816` drops the `heartbeat` flag. `claude.ts:1537-1561` emits `agent.step`, and `claude.ts:1643-1669` (`routeChild`) opens a synthetic "Background agent" run.
- **UI.**
  - Native: an Agent-kind step with no run renders as a shimmering, non-expandable "Subagent" row (`transcript/tasks.rs:972-975`, `tool/tasks.rs:282-286`).
  - Tauri: `subagentBrief` yields "Subagent" (`transcriptActivity.ts:783-791`).
- **User data** (rebase.db session e170a449). Every `background:` block (183, 264, 285, 300) is 100% heartbeat steps: 2, 8, 20 and 5 of them. Sub-agent runs carry 4–12 phantom `-heartbeat-N` Agent steps (blocks 111, 173, 213).

## Global Constraints

- **Tooling.** Use Bun: `bun run <script>`, `bunx <tool>`. Tests run through Vitest and cargo.
- **Gates:**
  - `bun run check`
  - `env ZDOTDIR=$(mktemp -d) bun run native:check`
  - `bun run test:host` if `host/` is touched
  - Use `--locked` for every focused cargo run, so `crates/rebase-gpui/Cargo.lock` isn't rewritten.
- **Heartbeat definition, identical in both apps.**
  - A frame is a heartbeat if `type == "tool_progress"` and either `heartbeat == true`, or `tool_use_id` matches `^(.+)-heartbeat-\d+$` where the captured group equals `parent_tool_use_id`.
  - A saved step or call is a heartbeat if its id matches `-heartbeat-\d+$`.
- **No new persisted fields.** Elapsed time from heartbeats is live/in-memory only. The shared `rebase.db` format is unchanged, so both apps stay compatible.
- **Fixtures.** Add `tests/fixtures/claude/heartbeat-agent.jsonl` (from `stdout-A.jsonl`) and `tests/fixtures/claude/heartbeat-bash.jsonl` (from `stdout-C.jsonl`). Sanitize them exactly as the resume fixture was. The repo is PUBLIC.
  - Reduce `system/init` frames to `type`, `subtype`, `session_id`, `cwd`, `model`.
  - Empty the hook output, `commands_changed` lists, and `control_response` command, agent and account blocks.
  - Reduce rate-limit info to `status`.
  - Replace absolute paths with `/tmp/rebase-fixture`.
  - Verify with `grep -ciE "gmail|calendar|drive|rocketreach|wispr|rezi|n8n|mintlify|notion|linear|claude max|cc-socks|subscription|vasanth|itsvandn|@gmail|warp|superpowers"`. It must print 0 for both files.
- **Commits** carry no Co-Authored-By trailer and no AI attribution. Never stage the user's uncommitted `README.md`.
- **Branch:** `fix/claude-heartbeats`, from `origin/main` f39ba856.

## Design Decisions

- **Drop before routing.** Heartbeats are detected at the top of progress handling, before `claude_route`/`routeChild`. Effects:
  - The rule-4 fallback never sees them.
  - The fallback stays in place for real child frames.
- **Live elapsed only.**
  - On a heartbeat, set the parent tool's live elapsed seconds, if the parent is a known running tool in this turn: `elapsed_time_seconds`, or `30*(n+1)` when that field is missing.
  - Native: put it on the in-memory tool state. It is not serialized: use `#[serde(skip)]` or a separate live map.
  - Tauri: use the in-memory live state, or a transient event. The existing `tool.updated` must not persist it.
  - The UI shows "running · <elapsed>" on the parent row when it has a value. Format: `Ns` under 60 s, `Mm Ss` above that.
  - If the codebase already shows elapsed time on running tool rows from a start timestamp, reuse that and skip this field. State which you chose.
- **Legacy cleanup at display time.**
  - Hide agent steps whose id matches `-heartbeat-\d+$`.
  - Hide synthetic blocks (`callId` starting `background:`) whose steps are all heartbeat ids.
  - Recompute step counts and "Ran N commands" summaries without them.
  - Never mutate or re-save these blocks, and never hide a `background:` block that has any real step.
- **"Subagent" title.** Out of scope as a separate fix. With heartbeats gone, no empty Agent-kind steps remain from this cause.

## Review Focus

1. **A real child step that happens to contain "heartbeat" in its text.** Only the id or frame shape may classify a heartbeat, never the text (Task 1 and Task 2 tests).
2. **A heartbeat for a parent unknown in this turn**, e.g. after a restart. It is dropped silently, which is correct, and it must not create a block (tests on both sides).
3. **A legacy `background:` block mixing real and heartbeat steps.** It must stay, showing only the real steps (tests on both sides).
4. **A long Bash call in Tauri's remote host path.** It must also not create blocks (Task 2 host test, if the host parses Claude frames).
5. **Step-count and "Ran N commands" summaries** must exclude hidden heartbeats (UI tests).

---

### Task 1: Rebase Native

**Files:**
- `crates/rebase-core/src/provider/tasks.rs`: `claude_progress` and the shared heartbeat check.
- `crates/rebase-core/src/provider.rs`: only if the frame dispatch needs it.
- `crates/rebase-gpui/src/transcript/tasks.rs` and `crates/rebase-gpui/src/conversation/background_tasks.rs`, or wherever step lists, step counts and "Ran N commands" are built: legacy hiding and the elapsed display.
- Both new fixtures.
- Tests in `crates/rebase-core/src/provider/` (replay) and the GPUI view tests.
- `.github/workflows/native.yml` `pull_request.paths`: add the two fixture paths.

**Interfaces:**
- Produces `pub fn is_heartbeat_frame(frame: &serde_json::Value) -> bool` and `pub fn is_heartbeat_id(id: &str) -> bool` in rebase-core.
- Produces the live elapsed value on the running tool (not serialized).

- [ ] **Step 1: Sanitize and add both fixtures. Run the grep check.**
- [ ] **Step 2: Write failing tests:**
  - Replaying `heartbeat-agent.jsonl` through the real stdout entry point yields an Agent run whose steps are exactly the sub-agent's real tool calls. There are no `-heartbeat-` steps, and the run has no step with `tool_kind` Agent.
  - Replaying `heartbeat-bash.jsonl` creates no `background:` block. The Bash block exists, completes, and recorded live elapsed ≥ 30.
  - A synthetic frame with `heartbeat:true` and an unknown parent creates nothing.
  - A real child tool_use whose input text contains "heartbeat" is still a step.
  - The existing resume replay tests still pass unchanged.
  - **Legacy view:**
    - A saved run containing `-heartbeat-N` steps renders without them, and its step count excludes them.
    - A saved `background:` block of only heartbeat steps isn't rendered.
    - A mixed one renders with only its real steps.
    - "Ran N commands" counts only real commands.
  - **Elapsed:** a running Bash row with live elapsed 150 shows "2m 30s".
- [ ] **Step 3: Run** `cargo test --locked --manifest-path crates/rebase-gpui/Cargo.toml -p rebase-core heartbeat` and the GPUI tests (native profile flags). Expect **FAIL**.
- [ ] **Step 4: Implement.**
- [ ] **Step 5: Verify** `env ZDOTDIR=$(mktemp -d) bun run native:check`, `bun run native:build` and `bun run check`. All must PASS.
- [ ] **Step 6: Commit:** `Ignore Claude heartbeat frames in Rebase Native and hide heartbeats saved by earlier builds`

### Task 2: Tauri Rebase

**Files:**
- `src/integrations/harness/providers/claude/claudeProtocol.ts`: keep `heartbeat` in `parseToolProgress`.
- `src/integrations/harness/providers/claude/claude.ts`: `handleToolProgress`, drop before `routeChild`, live elapsed.
- `src/features/sessions/model/transcriptActivity.ts` and `src/features/sessions/ui/AgentTranscript.tsx`: legacy hiding, counts, elapsed display.
- `host/`: only if the host parses Claude frames separately.
- Tests: `src/integrations/harness/providers/claude/claudeHeartbeat.test.ts`, plus UI tests in the existing style.

**Interfaces:**
- Produces TS `isHeartbeatProgress(progress)` and `isHeartbeatId(id: string): boolean`, with the same definition as Rust.

- [ ] **Step 1: Write failing tests** mirroring every Task 1 bullet. Use the same two fixtures through the real adapter entry point, the same legacy-view cases, and the same elapsed format.
- [ ] **Step 2: Run** `bunx vitest run src/integrations/harness/providers/claude/claudeHeartbeat.test.ts` and the UI test files. Expect **FAIL**.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify** `bun run check`, `bun run test:host`, and `env ZDOTDIR=$(mktemp -d) bun run native:check`. All must PASS.
- [ ] **Step 5: Commit:** `Ignore Claude heartbeat frames in Tauri Rebase and hide heartbeats saved by earlier builds`

### Task 3: Build, install, verify (controller)

- [ ] Run the full gates. Build both apps: `bun run tauri build --bundles app` and `bun run native:build`.
- [ ] Before quitting either running app, check for in-flight agent turns as a separate step. If one is running, ask the user first.
- [ ] Back up `rebase.db`, then install both apps.
- [ ] Confirm the old session e170a449 renders without "Background agent" heartbeat blocks or phantom "Subagent" rows. Do this with a read-only data check of what each UI would hide.
- [ ] Push and open a PR after the user confirms.
