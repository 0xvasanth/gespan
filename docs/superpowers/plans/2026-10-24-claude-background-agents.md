# Claude Background Agents Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When Claude resumes a sub-agent with `SendMessage` and it keeps working in the background, both Gespan apps capture and show that work. The background card links to the work. Sub-agent reports render cleanly, not as Claude's raw hand-back text.

**Architecture:** Claude's live stream already carries everything (verified by capture, see Evidence). The losses all happen in our parsers, so the fix lives in the parser and display layers:

1. A per-session **agent-run registry** that outlives a single turn. It is seeded from persisted blocks when a process restarts or resumes. It maps the original Agent `tool_use_id` and the Claude `agentId` to the block that should receive steps right now.
2. A `SendMessage` whose result carries `resumedAgentId` becomes a **resumption agent run** (its own block). The resumed agent's steps and task events attach to it, and history stays append-only.
3. The background card row carries the block id. It shows live progress and opens that block.
4. Hand-back text is parsed at **display time** into report plus metadata. Stored data stays raw, so the shared `gespan.db` stays compatible between apps.

Wake is the reference for the model. It links a sub-agent run to its dispatching call through the agentId and `meta.json` `toolUseId`, so resumed runs stay attached to the original dispatch. It also classifies task notifications as injected system content, not user or assistant words.

**Tech Stack:** Rust (`crates/gespan-core`, `crates/gespan-gpui`), TypeScript/React (`src/`), Vitest, cargo test, Bun.

**Spec:** Root-cause analysis and capture test from 2026-10-24 (summarized under Evidence). User request: "fix all these issues … in a cleaner way … take Wake as a reference".

## Evidence (from the capture, Claude Code 2.1.309, Gespan Native's exact flags)

- Turn 1 (foreground Agent `A`): child frames carry `parent_tool_use_id = A`.
- Turn 2 (`SendMessage` `S` resumes agent `X`): the `S` tool_result is `{"success":true,…,"resumedAgentId":"X"}`. The resumed agent's tool frames arrive on stdout **after** turn 2's `result`, with `parent_tool_use_id = A` (turn 1's Agent id) and no sidechain flag.
- `system` events `task_started` (with `is_backgrounded:true`, `task_type:local_agent`), `task_progress` ×N and `task_notification` (with `summary`, `output_file`) carry `task_id = X` and `tool_use_id = S`. `task_updated {status:completed}` carries only `task_id = X`.
- Completion arrives as `system/task_notification`, followed by a Claude-initiated turn whose `result.origin.kind = "task-notification"`.
- Agent `tool_result` text begins `[Subagent hand-back] … The report follows:` and ends with a trailer: `agentId: X (use SendMessage with to: 'X', …) subagent_tokens: N tool_uses: N duration_ms: N`.
- Gespan Native drops these at:
  - `crates/gespan-core/src/provider/tasks.rs:51-53` and `:462`: there's no parent in this turn's `tools`, so the frame is swallowed.
  - `crates/gespan-core/src/tool/tasks.rs:486`: `SendMessage` isn't an agent tool, so task events attach to nothing.
  - `parked.rs:292`: `tools` is cleared between turns.
- The card (`crates/gespan-gpui/src/conversation/background_tasks.rs:155-230`) holds only a description and a start time.
- Reports render verbatim (`tool/tasks.rs:545-549`, Tauri `transcriptActivity.ts:826`).
- The run title is overwritten by progress text (`provider/tasks.rs:534-538`).

## Global Constraints

- Tooling is Bun: `bun run <script>` and `bunx <tool>`. Tests run through Vitest and cargo.
- Gates:
  - `bun run check`
  - `env ZDOTDIR=$(mktemp -d) bun run native:check`
  - `bun run test:host` if `host/` is touched
- **Shared DB compatibility.** Both apps read and write `sessions.blocks_json`. Every new block field is **optional and additive**:
  - `agentRun.agentId?: string`
  - `agentRun.resumeOf?: string` (the original Agent tool_use_id)
  - `agentRun.activity?: string`

  Both apps must load blocks written by the other with or without these fields. No existing field changes meaning. Stored tool output stays raw.
- **Fixture.** The capture becomes the shared test fixture `tests/fixtures/claude/resumed-background-agent.jsonl`, the 238 stdout frames. Machine-specific absolute paths are replaced with `/tmp/gespan-fixture`. Both parsers' tests replay it.
- Commits carry no Co-Authored-By trailer and no AI attribution. No local Cubic review. Never stage the user's uncommitted `README.md` edit.
- Branch: `fix/claude-background-agents`, created from `main`. #29 was merged as 0fb387c7, so this branch sits directly on `main`.

## Design Decisions

- **Resumed steps go on the `SendMessage` block, not the old Agent block.** History stays append-only, and the work appears where it happened in the timeline. That block renders as an agent run titled "Resumed · <original agent name>".
- **Routing a child frame** with `parent_tool_use_id = P`, in order:
  1. If `P` is a live agent tool in this turn, attach as today.
  2. Otherwise, if the registry knows `P` as an Agent dispatch whose agentId `X` has an active resumption block, attach to that block.
  3. Otherwise, if the registry knows `P`, attach to a synthetic continuation agent-run block, created once per `(P, turn)`, titled "Continued · <name>".
  4. Otherwise, keep the frame as a top-level step labelled "Background agent". **Never drop it silently.**
- **Task events** (`task_started` / `task_progress` / `task_notification` / `task_updated`) find their run by `tool_use_id` first. If the `tool_use_id` is a `SendMessage` whose `resumedAgentId = X`, they go to that block. Otherwise they fall back to `task_id = X` through the registry.
  - Progress text sets `agentRun.activity`, never the run's `name`. This fixes the "Reading ." title.
  - `task_notification.summary` sets the run's summary and status.
- **Registry lifetime.** The registry lives on the session's provider state, not the per-turn `tools` map. On every process start or `--resume`, rebuild it from persisted blocks: Agent blocks with `nativeIds` / `agentRun.agentId`, and resumption blocks with `resumeOf`. The agentId comes from the Agent tool_result trailer (`agentId: X`), from the `SendMessage` result's `resumedAgentId`, or from `task_started.task_id`.
- **Hand-back parsing** is a pure function in each language with identical cases:
  - It strips the `[Subagent hand-back] … The report follows:` preamble and the per-line harness indentation.
  - It extracts the trailer into `{ agentId, subagentTokens, toolUses, durationMs }`.
  - Text without the framing passes through unchanged.
  - The UI shows the report as markdown with a muted metadata line: "N tool uses · N tokens · Ns".
- **Card.** `BackgroundTask` gains `toolId` (the block's tool id), `latest` (the last progress description) and `steps` (a count). A row shows the description, latest activity, step count and elapsed time. Clicking a row scrolls to the block and expands it. A row can only be interactive when its `toolId` exists.
- **Self-started turn.** A turn whose `result.origin.kind == "task-notification"` gets a small leading marker, "Background agent finished · <summary>", so Claude's follow-up isn't mistaken for a reply to the user. This mirrors Wake treating notifications as injected content.
- **Out of scope:** reading Claude's on-disk side files (not needed, since stdout is complete), and any Kiro work.

## Review Focus

1. **A continuation frame that arrives after the app restarted mid-run.** The registry must be rebuilt from persisted blocks before stdout is read, otherwise frames drop again. Task 2 tests a fresh registry seeded from blocks.
2. **Two resumes of the same agent in one session.** Steps must go to the latest resumption block, not the first one. Task 2 tests this.
3. **An unknown parent.** It must never be dropped silently, which is the root cause we're fixing. Task 2 asserts the fallback step exists.
4. **Old blocks without the new fields, and blocks written by the other app.** They must load and render. Task 1 adds tests on both sides, and Task 5's contract check covers it.
5. **A user report that legitimately contains a line starting with `agentId:`.** The trailer parser must only match the final trailer shape, anchored to the end of the text. Task 1 tests this.

---

### Task 1: Fixture, and the hand-back parser in Rust and TS

**Files:**
- Create: `tests/fixtures/claude/resumed-background-agent.jsonl`, from `/private/tmp/claude-501/capture-test/stdout.jsonl` with absolute machine paths replaced by `/tmp/gespan-fixture`.
- Create: `crates/gespan-core/src/tool/handback.rs` (plus `mod` declaration and tests), `src/features/sessions/model/subagentHandback.ts`, `src/features/sessions/model/subagentHandback.test.ts`.
- Modify: none yet. The UI wiring is in Tasks 4 and 5.

**Interfaces:**

```rust
pub struct Handback { pub report: String, pub agent_id: Option<String>, pub subagent_tokens: Option<u64>, pub tool_uses: Option<u64>, pub duration_ms: Option<u64> }
pub fn parse_handback(text: &str) -> Handback
```

```ts
export type Handback = { report: string; agentId?: string; subagentTokens?: number; toolUses?: number; durationMs?: number };
export function parseHandback(text: string): Handback;
```

- [ ] **Step 1: Write failing tests**, with identical cases in both languages:
  - **The fixture's frame-73 Agent tool_result text.** `report` starts with the agent's first report line, contains no `[Subagent hand-back]` and no indentation prefix, and ends before `agentId:`. `agentId`, `subagentTokens`, `toolUses` and `durationMs` equal the fixture values.
  - **Plain text with no framing** comes back unchanged, with every field undefined.
  - **A report body containing a line `agentId: fake` mid-text plus the real trailer** extracts only the trailer's id, and keeps the mid-text line in `report`.
  - **A trailer-only variant without the preamble** still extracts the fields.
  - **Empty string** returns an empty report.
- [ ] **Step 2: Run** `cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-core handback` (with the native-script profile flags) and `bunx vitest run src/features/sessions/model/subagentHandback.test.ts`. Expect **FAIL**.
- [ ] **Step 3: Implement.** Anchor the trailer regex to the end of the text: `agentId:\s*(\S+)[\s\S]*?subagent_tokens:\s*(\d+)\s*tool_uses:\s*(\d+)\s*duration_ms:\s*(\d+)\s*$`. Make the preamble strip match the exact `[Subagent hand-back]` lead paragraph up to and including `The report follows:`, then remove the common leading indentation from the rest.
- [ ] **Step 4: Run** the same tests. Expect **PASS**.
- [ ] **Step 5: Commit:** `Add the Claude resume capture fixture and a hand-back report parser`

### Task 2: Gespan Native core, an agent-run registry across turns, and resume routing

**Files:**
- Modify: `crates/gespan-core/src/provider/tasks.rs` (`agent_step` :51-53, `claude_activity` :376-462, task events :504-560, `track_claude_task` :575-614, the `description` patch :534-538), `crates/gespan-core/src/tool/tasks.rs` (`is_agent_tool` :486, `agent_mut`, `AgentRun` fields), `crates/gespan-core/src/provider.rs` (the SendMessage tool_result handling, and seeding on process start and `--resume`), `crates/gespan-core/src/provider/parked.rs:292`.
- Test: a new `crates/gespan-core/src/provider/tasks/resume_tests.rs` (or the existing test module) that replays the fixture through the real parser entry point that `provider.rs` uses for stdout lines.

**Interfaces:**
- **Produces** `AgentRegistry` on the session provider state:
  - `fn note_dispatch(tool_id, agent_id: Option<&str>, name)`
  - `fn note_agent_id(tool_id, agent_id)`
  - `fn note_resume(send_tool_id, agent_id)`
  - `fn target_for_parent(parent) -> Option<ToolId>`
  - `fn target_for_task(tool_use_id: Option<&str>, task_id: Option<&str>) -> Option<ToolId>`
  - `fn seed_from_blocks(blocks: &[Block])`
- **Produces** optional `AgentRun` fields `agent_id`, `resume_of`, `activity`, serde camelCase with `skip_serializing_if = "Option::is_none"`. `is_agent_tool` (or an equivalent `agent_mut` path) returns true for a `SendMessage` tool once it's known to resume an agent.
- **Produces** `BackgroundTask { tool_id: Option<String>, latest: Option<String>, steps: u32, ..existing }`.

- [ ] **Step 1: Write failing tests**, replaying the fixture frames in order:
  - **Turn 1's foreground Agent** still gets its child steps (regression guard). The step count equals the fixture's child tool calls for turn 1.
  - **The SendMessage block** becomes an agent run with `resumeOf = turn-1 Agent id` and `agentId = X`. It receives all 5 resumed tool calls as steps (`echo resumed-1`, `sleep`, `echo resumed-2`, `sleep`, `echo resumed-3`), plus the final text.
  - **The resumption run's status and summary:** status is `completed` and `summary` equals `task_notification.summary` after frame 194. Its `name` is unchanged by `task_progress`, and `activity` holds the last progress description.
  - **The background card:** `BackgroundTask` for task `X` has `tool_id = SendMessage id`, `latest` set, and `steps = 5` while running. It's removed after `task_notification`.
  - **Restart case:** build a fresh parser state, `seed_from_blocks` from blocks serialized after turn 1, then replay only the turn-2 frames. The same assertions hold.
  - **Two resumes:** duplicate the resume segment with a second SendMessage id `S2`. The second batch of steps lands on `S2`'s block, not `S`'s.
  - **Unknown parent:** a child frame with `parent_tool_use_id = "toolu_unknown"` produces a visible fallback step labelled "Background agent", not a silent drop.
  - **Blocks JSON:**
    - Old blocks without the new fields still deserialize.
    - New fields round-trip.
    - A block without `agentRun.activity` renders its title from `name`.
- [ ] **Step 2: Run** `cargo test … -p gespan-core resume` (native profile flags). Expect **FAIL**.
- [ ] **Step 3: Implement** the routing order and registry exactly as in Design Decisions:
  - Never return `true` from `claude_activity` for a frame that wasn't attached.
  - The registry must not be cleared at `parked.rs:292`. Only the per-turn `tools` map is cleared.
  - Seed the registry wherever the provider (re)starts a Claude process for an existing session.
- [ ] **Step 4: Run** the focused tests, then `env ZDOTDIR=$(mktemp -d) bun run native:check`. Expect **PASS**.
- [ ] **Step 5: Commit:** `Keep Claude agent runs across turns and route resumed background work to its run`

### Task 3: Gespan Native UI, resumed runs, clean reports, linked card rows, and the finished marker

**Files:**
- Modify:
  - `crates/gespan-gpui/src/transcript/tasks.rs`. Run title: "Resumed · <name>" for `resume_of`, "Continued · <name>" for continuation, and `activity` as a subtitle while running. The report goes through `parse_handback` (`:370`, `:518-529`), with a metadata line "N tool uses · N tokens · Ns".
  - `crates/gespan-gpui/src/conversation/background_tasks.rs`. Rows show description, `latest`, `steps` and elapsed time. A click scrolls to the block with `tool_id` and expands it. Rows without a `tool_id` stay non-interactive.
  - The conversation/turn renderer. When a turn's result has `origin.kind == "task-notification"`, render a leading muted marker "Background agent finished · <summary>".
- Test: GPUI view tests in the existing style for these modules (e.g. the `transcript/tasks.rs` tests and the `background_tasks.rs` tests).

- [ ] **Step 1: Write failing tests:**
  - **Report:** a run whose report has the hand-back framing renders without `[Subagent hand-back]` or `agentId:` text, and the metadata line reads "5 tool uses" (or the fixture's count).
  - **Resume title:** a resumption run renders the title prefix "Resumed ·".
  - **Activity:** while running, the subtitle shows `activity`.
  - **Card:** a background row with `tool_id` is clickable, and clicking it requests a scroll and expand for that block id. Assert through the existing view-test harness or the action it dispatches. A row without `tool_id` isn't clickable. A row shows `latest` and "5 steps".
  - **Finished marker:** a turn with `origin.kind = "task-notification"` renders the marker text with the summary.
- [ ] **Step 2: Run** the focused GPUI tests. Expect **FAIL**.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify** `env ZDOTDIR=$(mktemp -d) bun run native:check` passes and `bun run native:build` succeeds.
- [ ] **Step 5: Commit:** `Show resumed background agents, clean reports and linked background rows in Gespan Native`

### Task 4: Tauri parity, parser

**Files:**
- Modify: `src/integrations/harness/providers/claude/claude.ts` (`noteSubagentTool` / `noteSubagentNarration` ~:763, :818-823, and the task event handling), `src/integrations/harness/providers/claude/claudeProtocol.ts` (the `origin` handling ~:620-628), plus the session-level state that survives turns.
- Test: `src/integrations/harness/providers/claude/claudeResume.test.ts`, which replays the same fixture through the Tauri Claude adapter's stream handling.

**Interfaces:**
- **Produces** the same optional block fields as Task 2 (`agentRun.agentId`, `resumeOf`, `activity`), with identical JSON names so both apps read each other's blocks.
- **Produces** a TS `AgentRegistry` with the same routing order, seeded from the session's persisted blocks when the Claude process (re)starts.

- [ ] **Step 1: Write failing tests** mirroring Task 2's bullets: foreground regression, the resume block with 5 steps, status and summary, name not overwritten, the restart-seeded case, two resumes, the unknown-parent fallback, and old-block compatibility.
- [ ] **Step 2: Run** `bunx vitest run src/integrations/harness/providers/claude/claudeResume.test.ts`. Expect **FAIL**.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the tests, then `bun run check`. Expect **PASS**.
- [ ] **Step 5: Commit:** `Keep Claude agent runs across turns in Tauri Gespan and route resumed background work`

### Task 5: Tauri parity, UI, and the cross-app block contract

**Files:**
- Modify:
  - `src/features/sessions/model/transcriptActivity.ts`: `subagentReport` (:826) uses `parseHandback`. "Running in background" (:1076) gains the latest activity.
  - `src/features/sessions/ui/AgentTranscript.tsx`: the report render (:4054) shows the metadata line, the "Resumed ·" / "Continued ·" titles, the activity subtitle, and the "Background agent finished · <summary>" marker.
  - `src/test/sharedContracts.test.ts` and `crates/gespan-core/tests/shared_contracts.rs`: assert both sides use the same JSON field names (`agentId`, `resumeOf`, `activity`) for `agentRun`.
- Test: `src/features/sessions/ui/AgentTranscript.*.test.ts` (existing style), and `transcriptActivity` tests.

- [ ] **Step 1: Write failing tests:**
  - **Report:** it renders without the framing, and the metadata line is present.
  - **Titles and subtitle:** the resumption title, the continuation title, and the activity subtitle all render.
  - **Marker:** the finished marker renders for a task-notification turn.
  - **Contract test:** fails if either side renames the fields.
- [ ] **Step 2: Run** the focused vitest files. Expect **FAIL**.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify** `bun run check` and `env ZDOTDIR=$(mktemp -d) bun run native:check` both pass.
- [ ] **Step 5: Commit:** `Render resumed agents and clean sub-agent reports in Tauri Gespan`

### Task 6: End-to-end verification (controller)

- [ ] Run the full gates.
- [ ] Build both apps and install them into `/Applications`. Back up `gespan.db` first.
- [ ] **Live check.** In each app, run a session that dispatches an agent, then resumes it with `SendMessage` in the background, using the capture scenario prompts. Confirm all of these:
  - The resumed steps appear under a "Resumed ·" run.
  - The background row shows the latest activity and step count, and clicking it opens the run.
  - The report has no hand-back framing.
  - The follow-up turn shows the "Background agent finished" marker.
  - The original session `mc/sql-workflow-state-tracking` still loads.
- [ ] Push `fix/claude-background-agents` and open a PR against `main`, after the user confirms.
