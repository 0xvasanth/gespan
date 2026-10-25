# Flows for Gespan Native — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user write a deterministic orchestrator once and run it again and again. Gespan Native calls this a **Flow**.

**Architecture:**
- A Flow is a state machine. Agent states each run a supported harness in a fresh session. A state receives only the inputs that are linked to it, and returns typed JSON outputs that choose the next state.
- The Flow model, validation, bindings, output contract, storage and runner live in `crates/gespan-core` as plain Rust, with no GPUI.
- The runner is a step-driven state machine behind a `FlowHost` trait.
- `crates/gespan-gpui` implements that host on the session-launch path automations already use. It adds a Flows app view for listing, running and watching Flows.
- Phase 1 ships a usable Flows with starter Flows and a JSON definition editor. Phases 2 and 3 add a visual editor, automation triggers, Board integration and Tauri parity.

**Tech Stack:** Rust (`gespan-core`, GPUI `gespan-gpui`), SQLite through the shared `gespan.db`, cargo tests with the `native-dev` profile, Bun for any UI tooling.

**Spec:**
- The state-machine design the user approved on 2026-10-23 (`docs/superpowers/plans/2026-10-23-flows.md`, Design Decisions). Decisions recorded there: JSON output contract, shared worktree per run, sequential with loops in v1, Kiro separate.
- The 2026-10-24 research summarized in Evidence below.

## Evidence (research, 2026-10-24, `origin/main` 71f80daa)

**Prior work**
- Plan `2026-10-23-flows.md` targeted Tauri/TS. Its design still holds, but every task has to move to Rust and GPUI.
- Task 1 exists only as a TS stash: `a17401ea` "flows task1 partial", 720 lines, untested because it was stopped mid-run. Read it with `git show a17401ea^3:src/features/flows/model/<file>`. It contains:
  - `flowDefinition.ts`: types, constants, `outputsOf`, `validateFlow` with dominator warnings.
  - `flowTemplate.ts`: `renderTemplate`, `listBindings`, `resolveInputs`.
  - `starterFlows.ts`
  - `flowDefinition.test.ts` (23 cases) and `flowTemplate.test.ts` (15 cases).
- No other Flows code exists on any branch.

**Board**
- Tauri Board is merged (#32). It has `LINK_KINDS = ["session","pilot","flow-run"]` (`src-tauri/src/board.rs:16`).
- Native Board is in flight on `mc/sql-workflow-state-tracking` (worktree `mc-c4738962`, not merged). Its core module `crates/gespan-core/src/board/` has the same link kinds (`mod.rs:37`) and the hook `board::on_linked_work(service, kind, id, LinkedWorkEvent)` (`auto_move.rs:52`).

**Session launch to reuse**
- `Shell::launch_automation` (`crates/gespan-gpui/src/shell/automation_runs.rs:286-530`) is the launch path to copy:
  1. `store.create(harness, cwd)` (`:422`)
  2. `save_agent_settings`
  3. worktree selection: `settings.new_worktree` + `worktree_base: "HEAD"`, or `settings.worktree_cwd` to reuse (`:385-388`, fields in `crates/gespan-core/src/agent.rs:136-138`)
  4. `self.add(summary)`, then `Conversation::quick_submit(prompt, settings)` (`conversation.rs:779`)
- Completion is reported through `Shell::automation_session_changed` (`automation_runs.rs:534`, called from `shell.rs:1239`), plus `external_submission_pending()` and `external_outcome()` (`composer_features.rs:~451-470`).
- `external_submission_pending` already waits out the Claude background-wake window.
- Final text comes from `gespan_core::response::prose` (`response.rs:56`). The outcome path truncates it to 12,000 chars, so the contract must use the untruncated last assistant message instead.

**Worktrees**
- `git::create_worktree` (`git.rs:187`) makes a `gespan/native-…` branch on the first turn (`runtime.rs:223`). The resulting path is reported in `Update.worktree_cwd` (`runtime.rs:966`).
- Later sessions reuse it through `settings.worktree_cwd`.

**Storage**
- Core modules own tables through `ensure_tables(conn)`, using DDL identical to Tauri's. Examples: `automations.rs:207`, `board/mod.rs:106`, native-only `board/extras.rs:17`.
- Cross-app contract tests: `crates/gespan-core/tests/shared_contracts.rs`, `src/test/sharedContracts.test.ts`.

**Harnesses**
- Native `Kind` (`crates/gespan-core/src/provider/kind.rs:3-14`): Codex, Claude, Cursor, Grok, OpenCode, Antigravity, Pi, Omp, Fx, Hermes, Devin.
- There's no Kiro in Native.

**App views**
- `enum AppView` is at `crates/gespan-gpui/src/workspace.rs:10`, with render and labels in `shell/app_views.rs:14-16`.
- Native Board commit `3b34648a` shows the full set of places a new view touches: `workspace.rs`, `app_views.rs`, `project_rail.rs`, `sidebar_list.rs`, `title_tabs.rs`, a settings toggle, and `classic_surface_tests.rs`.

**Orchestration**
- `crates/gespan-core/src/orchestration.rs` is planned by a lead agent, so it isn't deterministic. Flows don't use it.

**Claude**
- Sub-agent and background-agent capture and heartbeat dropping already work (`provider/agent_registry.rs`, `provider/tasks.rs`).
- The runner only needs the settled status and the final assistant text.

## Global Constraints

- **Target app.** Gespan Native, macOS first. Tauri parity is Phase 3. No change may break the Tauri app's use of the shared `gespan.db`.
- **Tooling.**
  - Rust: `cargo … --locked --manifest-path crates/gespan-gpui/Cargo.toml -p <pkg> --profile native-dev`.
  - Gates: `env ZDOTDIR=$(mktemp -d) bun run native:check` and `bun run check`.
  - Never run cargo without `--locked`, which rewrites `Cargo.lock`.
- **Limits (unchanged from the approved design).**
  - States: 1–30.
  - `maxVisits`: 1–10, default 3.
  - Run cap: 50 total visits, failing with `"Run exceeded 50 steps"`.
  - Output contract: exactly 1 retry.
  - Binding values truncated at 8,000 chars plus `"\n…[truncated]"`.
  - Names match `^[a-z][a-z0-9_]{0,39}$`.
  - Terminals are `"$succeeded"` and `"$failed"`.
- **Copy (exact strings).**
  - Contract footer: `---\nWhen you are done, end your reply with one fenced ```json block containing exactly these fields:\n- <name> (<type>): <description>\nDo not add other fields.`
  - Retry prompt: `Your JSON block was invalid: <errors joined by "; ">. Reply with only the corrected ```json block.`
  - Errors:
    - `Output contract not met: <errors>`
    - `State <id> exceeded <n> visits`
    - `No transition matched from <id>`
    - `Interrupted by app restart`
    - `Binding states.<id>.outputs.<field> has no value yet`
- **Harnesses.** Agent states may use any `gespan_core::provider::Kind`. Kiro is out of scope until Native supports it.
- **Commits.** No Co-Authored-By trailer and no AI attribution. No local Cubic review. Never stage the user's uncommitted `README.md`.
- **Branch.** `feat/native-flows`, from `origin/main` 71f80daa.

## Design Decisions

The approved 2026-10-23 decisions carry over:
- bindings `{{inputs.x}}` / `{{states.s.outputs.f}}`, latest visit, a trailing `?` marks a binding optional
- the last ```json block wins
- structured guards, first match wins
- approval states output `{approved, note}`
- a definition snapshot per run
- visit caps
- recovery marks visits as `Interrupted by app restart`
- "Resume from state" appends a visit

These are new for Native and are **my calls, flagged for review**:
1. **Core owns everything except the host.** `gespan_core::flow` has `definition`, `template`, `contract`, `store` and `runner`. GPUI only implements `FlowHost` and the views. A future Tauri port reuses the same rules through its own TS twin (Phase 3).
2. **The runner is step-driven, not async.** Every runner method returns a list of `Effect`s (`Launch`, `FollowUp`, `Cancel`, `NeedsApproval`, `Finished`), and the host reports results back:
   - `on_agent_settled(run_id, visit_id, AgentOutcome)`
   - `on_worktree(run_id, path)`
   - `resolve_approval`
   - `cancel`

   This fits GPUI's callback-driven session events and makes the runner fully unit-testable with no clock or executor.
3. **Visible background sessions.** Each agent visit is a normal session, launched like a scheduled automation: titled `"<Flow name> · <state title>"`, not revealed, and placed in a session folder per run when the project supports folders. A visit links to its session by `session_id`. Sessions get **no** new column, so the shared DB stays unchanged. A truly hidden session is a later option.
4. **Worktree handoff.** The first agent visit launches with `new_worktree = true` and `worktree_base = "HEAD"`. The host reads the created path from that session's settled update and calls `on_worktree`. Every later visit launches with `worktree_cwd = <path>`.
5. **Final text for the contract.** Use the **untruncated** text of the last assistant message group after the last `user` row. That needs a new `gespan_core::response::last_assistant_text(rows)` beside `prose`, not the 12,000-char `external_outcome` text. A follow-up retry reads its own turn the same way.
6. **Storage.**
   - Tables:
     - `flows(id PK, definition_json, updated_at)`
     - `flow_runs(id PK, flow_id FK→flows ON DELETE CASCADE, status, created_at, run_json)`, with indexes `(flow_id, created_at DESC)` and `(status)`
     - `flow_visits(id PK, run_id FK→flow_runs ON DELETE CASCADE, seq, state_id, status, visit_json, UNIQUE(run_id, seq))`, with index `(run_id, seq)`
   - All created by `gespan_core::flow::store::ensure_tables`.
   - `crates/gespan-core/tests/shared_contracts.rs` pins the DDL text so the Phase 3 Tauri twin can match it exactly.
   - The Tauri app never creates or reads these tables until Phase 3, so it is unaffected.
   - Deleting a Flow with a `running` or `waiting-approval` run is refused with `"Flow has an active run"`.
7. **Definitions in Phase 1.**
   - "New Flow" offers the built-in starters ("Fix an issue", plus one non-code starter, "Research and summarise"), saved with a chosen project.
   - A **JSON editor** (validated live by `validate_flow`, with errors and warnings listed) edits any saved Flow.
   - The visual editor is Phase 2.
8. **Concurrency.** Any number of runs can run at once, each in its own worktree. One state at a time per run.
9. **Cancellation.** `cancel(run)` asks the host to stop the visit's session through the conversation's existing stop path. The visit and the run both become `cancelled`.

## Review Focus

1. **A loop that never converges.** The `maxVisits` cap and the 50-step cap both end the run with readable errors (Task 4 tests).
2. **An agent forgets the JSON block, or puts JSON inside prose.** The last block counts, there's one retry in the same session, then a clear failure (Tasks 2 and 4).
3. **The app quits mid-state.** Startup recovery fails that visit, and "Resume from state" relaunches it in the same worktree (Tasks 4 and 5, plus the manual check in Task 7).
4. **The first visit's worktree path never arrives** (its turn failed before creating one). The run must fail the visit with the agent's error, not hang waiting for `on_worktree` (Task 4).
5. **A Flow is edited or deleted mid-run.** The run keeps its snapshot, and deletion is refused while a run is active (Tasks 3 and 4).

---

## Phase 1 — Usable Flows in Gespan Native

### Task 1: Flow definition, validation, bindings and starters (core)

**Files:**
- Create: `crates/gespan-core/src/flow/mod.rs`, `flow/definition.rs`, `flow/template.rs`, `flow/starter.rs`, with tests inline or in `flow/tests/`.
- Modify: `crates/gespan-core/src/lib.rs`, to add `pub mod flow;`.
- Port source: stash `a17401ea^3`, specifically `src/features/flows/model/flowDefinition.ts`, `flowTemplate.ts`, `starterFlows.ts` and both test files.

**Interfaces (serde camelCase, so the JSON matches the TS shape):**

```rust
pub enum OutputType { String, Number, Boolean, StringList }            // JSON "string" | "number" | "boolean" | "string[]"
pub enum OutputValue { String(String), Number(f64), Bool(bool), List(Vec<String>) }
pub struct FlowInput { name, label, required: bool, default: Option<String> }
pub struct OutputField { name, r#type: OutputType, description }
pub enum Guard { Status { is: VisitResult }, Output { field, op: GuardOp, value: Option<serde_json::Value> } }  // tag "kind"
pub struct Transition { to: String, when: Option<Guard> }
pub enum FlowState { Agent(AgentState), Approval(ApprovalState) }      // tag "kind": "agent" | "approval"
pub struct AgentState { id, title, harness: String /* Kind id */, model, brief, outputs: Vec<OutputField>, transitions, max_visits: u8 }
pub struct ApprovalState { id, title, message, transitions, max_visits: u8 }
pub struct Flow { id, name, description, cwd, inputs, initial, states, created_at: i64, updated_at: i64 }
pub const SUCCEEDED: &str = "$succeeded"; pub const FAILED: &str = "$failed";
pub fn approval_outputs() -> Vec<OutputField>;                          // approved:boolean, note:string
pub fn outputs_of(state: &FlowState) -> Vec<OutputField>;
pub fn validate_flow(flow: &Flow) -> Validation { errors: Vec<String>, warnings: Vec<String> }
// template.rs
pub struct BindingContext<'a> { inputs: &'a HashMap<String,String>, outputs: &'a HashMap<String, HashMap<String, OutputValue>> }
pub fn render_template(text: &str, ctx: &BindingContext) -> Result<String, Vec<String>>;
pub fn list_bindings(text: &str) -> Vec<(String, bool)>;                // (ref, optional)
pub fn resolve_inputs(flow: &Flow, given: &HashMap<String,String>) -> Result<HashMap<String,String>, Vec<String>>;
// starter.rs
pub fn starter_flows() -> Vec<Flow>;                                    // id/cwd/timestamps empty; "Fix an issue", "Research and summarise"
```

- [ ] **Step 1: Port every stash test to Rust, with identical inputs and assertions.** That's the 23 `flowDefinition` cases and the 15 `flowTemplate` cases (names listed in the stash files). Add these:
  - `harness_must_be_a_native_kind`: an unknown harness id produces an error containing `"unknown harness"`.
  - `research_starter_validates_without_code_steps`
  - `flow_json_round_trips_with_ts_shape`: a fixture JSON in the TS shape deserializes and re-serializes identically.
- [ ] **Step 2: Run** `cargo test --locked --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-core --profile native-dev flow::`. Expect FAIL.
- [ ] **Step 3: Implement.** Port the algorithms faithfully:
  - Reachability: BFS from `initial`.
  - "Cannot reach `$succeeded`": reverse reachability.
  - Must-precede dominators: iterative dominator sets.
  - Single-pass binding substitution using `\{\{\s*(inputs\.[a-z][a-z0-9_]*|states\.[a-z][a-z0-9_]*\.outputs\.[a-z][a-z0-9_]*)(\?)?\s*\}\}`.
- [ ] **Step 4: Verify.** The same command passes, and so does clippy (`-D warnings`).
- [ ] **Step 5: Commit** `Add Flow definitions, validation, bindings and starters to gespan-core`.

### Task 2: Output contract and transition selection (core)

**Files:**
- Create: `crates/gespan-core/src/flow/contract.rs`, with tests.
- Modify: `crates/gespan-core/src/response.rs`, to add `last_assistant_text`.

**Interfaces:**

```rust
pub fn contract_footer(outputs: &[OutputField]) -> String;   // exact copy; "" for no outputs
pub fn retry_prompt(errors: &[String]) -> String;            // exact copy
pub fn parse_outputs(reply: &str, outputs: &[OutputField]) -> Result<HashMap<String, OutputValue>, Vec<String>>;
pub enum VisitResult { Completed, Failed }
pub fn select_transition<'a>(state: &'a FlowState, result: VisitResult, values: &HashMap<String, OutputValue>) -> Option<&'a Transition>;
// response.rs
pub fn last_assistant_text(rows: &[Row]) -> String;          // untruncated last assistant message group after the last user row
```

- [ ] **Step 1: Write the failing tests.**
  - `contract_footer` matches the exact copy, and is `""` when there are no outputs.
  - `parse_outputs`:
    - The last ```json block wins.
    - ```js blocks and unlabeled fences are ignored.
    - No block gives `"No ```json block found"`.
    - Invalid JSON gives an error starting `"Invalid JSON"`.
    - It reports `"missing field passed"`, `"unexpected field x"`, `"field passed must be boolean"`, a `string[]` holding a non-string, and a non-object value.
    - Zero declared outputs with no block gives `Ok({})`.
  - `select_transition`:
    - First match wins, and a transition with no `when` always matches.
    - `eq`/`neq` compare with JSON equality. `truthy`/`falsy` treat an empty list as falsy. `contains` covers both strings and lists.
    - A `Failed` result only considers `status: failed` guards.
    - No match returns `None`.
  - `last_assistant_text`:
    - It returns the last assistant group after the last user row, in full. A 30,000-char reply comes back untruncated.
    - It ignores tool rows and earlier turns.
- [ ] **Step 2: Run** `… -p gespan-core flow::contract response::`. Expect FAIL.
- [ ] **Step 3: Implement.** Find fences with `(?s)```json\s*\n(.*?)````. Build `last_assistant_text` on the same row walk `prose` uses.
- [ ] **Step 4: Verify.** The tests pass and clippy is clean.
- [ ] **Step 5: Commit** `Add the Flow output contract and transition selection`.

### Task 3: Flow storage (core)

**Files:**
- Create: `crates/gespan-core/src/flow/store.rs`.
- Modify: `crates/gespan-core/tests/shared_contracts.rs`, to pin the DDL. Also modify whatever opens `gespan.db` for Native features, so it calls `flow::store::ensure_tables` the way automations and board do.

**Interfaces:**

```rust
pub fn ensure_tables(conn: &rusqlite::Connection) -> rusqlite::Result<()>;
pub struct FlowStore { /* own connection like automations::Service */ }
impl FlowStore {
  pub fn open(db: &Path) -> Result<Self, String>;
  pub fn list(&self) -> Result<Vec<Flow>, String>;
  pub fn upsert(&self, flow: &Flow) -> Result<Flow, String>;            // sets created_at on insert, updated_at always
  pub fn delete(&self, id: &str) -> Result<(), String>;                 // Err("Flow has an active run") if any run running|waiting-approval
  pub fn runs(&self, flow_id: &str, limit: usize) -> Result<Vec<FlowRun>, String>;   // newest first, limit 50
  pub fn active_runs(&self) -> Result<Vec<FlowRun>, String>;
  pub fn run(&self, id: &str) -> Result<Option<FlowRun>, String>;
  pub fn save_run(&self, run: &FlowRun) -> Result<(), String>;
  pub fn visits(&self, run_id: &str) -> Result<Vec<FlowVisit>, String>;  // by seq
  pub fn save_visit(&self, visit: &FlowVisit) -> Result<(), String>;    // upsert by id; UNIQUE(run_id, seq)
}
```

`FlowRun` and `FlowVisit` are the types defined in Task 4. Define them in `flow/run.rs` here, so storage and the runner share them.

```rust
pub enum RunStatus { Running, WaitingApproval, Succeeded, Failed, Cancelled }
pub enum VisitStatus { Running, WaitingApproval, Completed, Failed, Cancelled }
pub struct FlowRun { id, flow_id, flow: Flow /*snapshot*/, status, inputs: HashMap<String,String>, worktree_cwd: Option<String>, current_state: Option<String>, error: Option<String>, created_at, finished_at: Option<i64> }
pub struct FlowVisit { id, run_id, seq: u32, state_id, status, prompt: Option<String>, session_id: Option<String>, outputs: Option<HashMap<String,OutputValue>>, error: Option<String>, retried: bool, note: Option<String>, started_at, finished_at: Option<i64> }
```

- [ ] **Step 1: Write the failing tests.** They use an in-memory DB with `PRAGMA foreign_keys=ON`.
  - `upsert_then_list_round_trips_definition`
  - `upsert_preserves_created_at_and_bumps_updated_at`
  - `delete_flow_cascades_runs_and_visits`
  - `delete_refused_with_active_run`, for both `running` and `waiting-approval`
  - `runs_listed_newest_first_and_capped_at_50`
  - `active_runs_lists_only_running_and_waiting`
  - `visit_seq_unique_per_run`
  - `visits_listed_in_seq_order`
  - `run_save_rejects_unknown_flow`
  - `ensure_tables_is_idempotent_alongside_other_native_tables`
  - In `shared_contracts.rs`: `flow_ddl_is_pinned`, which asserts the exact `CREATE TABLE` text for all three tables.
- [ ] **Step 2: Run** `… -p gespan-core flow::store` and `--test shared_contracts`. Expect FAIL.
- [ ] **Step 3: Implement.** Mirror `automations::Service`: WAL, a 5 s `busy_timeout`, and JSON blobs with mirrored indexed columns.
- [ ] **Step 4: Verify.** The tests pass and clippy is clean.
- [ ] **Step 5: Commit** `Store Flows, runs and visits in gespan.db`.

### Task 4: The state-machine runner (core)

**Files:**
- Create: `crates/gespan-core/src/flow/runner.rs`, with tests.

**Interfaces:**

```rust
pub struct LaunchRequest { pub run_id: String, pub visit_id: String, pub harness: String, pub model: String, pub cwd: String, pub worktree_cwd: Option<String>, pub title: String, pub prompt: String }
pub struct AgentOutcome { pub status: AgentStatus /* Completed | Failed | Cancelled */, pub text: String /* last_assistant_text */, pub error: Option<String> }
pub enum Effect { Launch(LaunchRequest), FollowUp { session_id: String, prompt: String }, Cancel { session_id: String }, Persist { run: FlowRun, visit: Option<FlowVisit> }, NeedsApproval { run_id: String, visit_id: String }, Finished { run_id: String, status: RunStatus } }
pub const MAX_RUN_STEPS: u32 = 50;
pub struct Runner { /* in-memory active runs, ids/clock injected */ }
impl Runner {
  pub fn new(clock: impl Fn() -> i64 + 'static, ids: impl FnMut() -> String + 'static) -> Self;
  pub fn start(&mut self, flow: &Flow, given: &HashMap<String,String>) -> Result<(FlowRun, Vec<Effect>), Vec<String>>;
  pub fn on_session(&mut self, run_id: &str, visit_id: &str, session_id: &str) -> Vec<Effect>;   // host reports the launched session
  pub fn on_worktree(&mut self, run_id: &str, path: &str) -> Vec<Effect>;
  pub fn on_agent_settled(&mut self, run_id: &str, visit_id: &str, outcome: AgentOutcome) -> Vec<Effect>;
  pub fn resolve_approval(&mut self, run_id: &str, approved: bool, note: &str) -> Vec<Effect>;
  pub fn cancel(&mut self, run_id: &str) -> Vec<Effect>;
  pub fn resume(&mut self, run: FlowRun, visits: Vec<FlowVisit>, state_id: &str) -> Vec<Effect>;
  pub fn recover(&mut self, active: Vec<(FlowRun, Vec<FlowVisit>)>) -> Vec<Effect>;  // running visits → Failed "Interrupted by app restart"; waiting-approval untouched
}
```

**Loop semantics:**
- `start` validates the Flow. Any error refuses to start. It then resolves inputs, snapshots the Flow, and emits `Persist` followed by the first visit's effects.
- **Agent visit:**
  1. Check the visit caps and the step cap.
  2. Render the brief and append `contract_footer`.
  3. `Persist` the visit as running.
  4. `Launch`. Pass `worktree_cwd` only once the run has one. The first visit launches with none and waits for `on_worktree` before any later visit launches.
- **On settle:**
  1. If the agent failed, take only a `status: failed` transition; otherwise the run fails.
  2. Run `parse_outputs`. On error, emit `FollowUp` with `retry_prompt` once, then fail with `Output contract not met: …`.
  3. `select_transition`. With no match, fail with `No transition matched from <id>`.
  4. Move to the next state or to a terminal.
- **Approval visit:** emit `Persist` with waiting-approval and `NeedsApproval`.
- **Every status change** emits `Persist`.

- [ ] **Step 1: Write failing tests with a fake host that records effects.** These are the old plan's Task 4 cases, retargeted at effects:
  - happy path, with seq 1–4 and the final `succeeded`
  - a loop where the second implement visit's prompt contains the first test's `failures`, and the optional binding renders empty the first time
  - shared worktree: only the first launch has `worktree_cwd: None`, and later launches use the `on_worktree` path
  - isolation: every agent visit is a new `Launch`, and `FollowUp` is only used for the retry
  - contract retry succeeds with `retried: true`; a double failure gives `Output contract not met`
  - a status guard to `$failed`
  - the `maxVisits` cap
  - the 50-step cap
  - no transition matched
  - a missing required binding produces no `Launch`
  - approval reject, with the note rendered into the next brief
  - cancel emits `Cancel{session_id}` and marks the visit and run cancelled
  - `recover`
  - `resume` appends seq last+1 and reuses the worktree
  - snapshot isolation
  - Also: `first_visit_failing_before_a_worktree_fails_the_run_without_hanging` (Review Focus 4)
- [ ] **Step 2: Run** `… -p gespan-core flow::runner`. Expect FAIL.
- [ ] **Step 3: Implement.** Keep all state transitions inside `Runner`. The host never decides flow logic.
- [ ] **Step 4: Verify.** The tests pass, and the full `-p gespan-core` suite and clippy are clean.
- [ ] **Step 5: Commit** `Add the Flow state-machine runner`.

### Task 5: GPUI host — sessions, worktree handoff, completion, cancel, recovery

**Files:**
- Create: `crates/gespan-gpui/src/flows/mod.rs` and `flows/host.rs`.
- Modify:
  - `crates/gespan-gpui/src/shell/automation_runs.rs`: extract the shared launch path into `Shell::launch_background_session(spec) -> SessionId`. `launch_automation` calls it with no behaviour change.
  - The session-changed hook (`shell.rs:~1239`): also notify Flows.
  - Shell init: open a `FlowStore`, create a `Runner`, and run `recover` on startup.

**Interfaces:**
- `BackgroundSessionSpec { harness, model, cwd, worktree: WorktreeChoice { New, Reuse(path), Current }, title, folder: Option<String>, prompt }`
- `Shell::flow_effects(&mut self, effects: Vec<Effect>, cx)` applies the runner's effects:
  - **Launch:** `launch_background_session`, then `runner.on_session`.
  - **FollowUp:** `quick_submit` to that session.
  - **Cancel:** the conversation's stop.
  - **Persist:** `FlowStore`.
  - **NeedsApproval / Finished:** notify the Flows view.
- When a visit's session settles, using the same pending/settled check automations use (`external_submission_pending() == false`), the host:
  1. Reads `last_assistant_text` from that conversation's transcript rows.
  2. For the run's first visit, reads the worktree path from the session's settings or update, then calls `runner.on_worktree`.
  3. Calls `runner.on_agent_settled`.

- [ ] **Step 1: Write failing tests.** Use the existing GPUI test style; `automation_runs` has tests to copy.
  - `launch_automation_unchanged_after_extracting_the_shared_launch`: the existing automation tests still pass.
  - Use a fake provider or session as the automation tests do. A two-state Flow then:
    - launches the first visit with a new worktree
    - feeds the worktree path into the second visit
    - persists visits
    - finishes `succeeded`
  - Cancel stops the session and marks the run cancelled.
  - Startup recovery marks a persisted running visit `Interrupted by app restart`.
  - A settled turn whose text exceeds 12,000 chars still parses its trailing JSON block, which proves the untruncated path.
- [ ] **Step 2: Run** `… -p gespan-gpui flows:: automation`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify** with `env ZDOTDIR=$(mktemp -d) bun run native:check`.
- [ ] **Step 5: Commit** `Run Flows on background sessions in Gespan Native`.

### Task 6: Flows app view — list, new from starter, JSON editor, run dialog, run view

**Files:**
- Create: `crates/gespan-gpui/src/flows/view.rs`, `flows/editor.rs` (JSON), `flows/run_view.rs`.
- Modify: everything the native Board commit `3b34648a` touched for a new `AppView`:
  - `workspace.rs` (`AppView::Flows`)
  - `shell/app_views.rs`
  - `project_rail.rs`
  - `sidebar_list.rs`
  - `title_tabs.rs`
  - a settings toggle, if views are toggleable
  - `classic_surface_tests.rs`

**UI behaviour:**
- **List:** saved Flows plus "New Flow", which offers the starters and picks a project. Each Flow shows its last run status, and has Run, Edit and Delete actions. Delete is disabled while a run is active, with a tooltip.
- **JSON editor:** a monospace text area holding the Flow definition. `validate_flow` runs on every change, and errors and warnings are listed under the editor. Save is disabled while there are errors. Changes are saved with `FlowStore::upsert`.
- **Run dialog:** one field per input, prefilled with defaults and marked when required. Run is disabled until `resolve_inputs` succeeds.
- **Run view:**
  - Header: status, Flow name, inputs, and the worktree path with a reveal action.
  - Visit timeline in seq order: state title, status, duration and a "retried" badge. Expanding a visit shows the prompt, the outputs, the error, and "Open session", which focuses that session tab.
  - Approval visits show the message, a note field and Approve / Reject.
  - Cancel while running. "Resume from…" on failed or cancelled runs, with a state picker defaulting to the failed state.

- [ ] **Step 1: Write failing view tests** in the GPUI test style used by `classic_surface_tests.rs` and the Board views:
  - The Flows view opens from the rail.
  - "New Flow" from "Fix an issue" saves a valid Flow.
  - A JSON edit that introduces an unknown target shows `"unknown target"` and disables Save.
  - The run dialog keeps Run disabled until a required input is filled.
  - A run view with a waiting-approval visit dispatches `resolve_approval(false, "redo")` on Reject.
  - Resume on a failed run defaults to the failed state.
- [ ] **Step 2: Run** `… -p gespan-gpui flows::`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify** with `env ZDOTDIR=$(mktemp -d) bun run native:check` and `bun run check`.
- [ ] **Step 5: Commit** `Add the Flows view to Gespan Native`.

### Task 7: End-to-end check (controller)

- [ ] Full gates.
- [ ] `bun run native:dev`, which runs as Gespan Native Dev with dev data.
- [ ] In a scratch git repo:
  1. Create "Fix an issue" from the starter and run it.
  2. Watch triage, then implement, then test, each in its own background session sharing one `gespan/native-…` worktree.
  3. Check that implement's prompt contains triage's `plan`.
  4. Reject at review and watch it loop back to implement. Approve and see `succeeded`.
- [ ] Edge cases:
  - Quit mid-state, relaunch, and see "Interrupted by app restart". Resume from that state.
  - Cancel a running run.
  - Delete is refused while a run is active.
- [ ] Ask the user before pushing or opening the PR.

## Phase 2 — Authoring, triggers, Board

Each task here gets its own detailed plan once Phase 1 has merged.

- **Task 8: Visual Flow editor (GPUI).**
  - State cards (agent and approval), a harness/model picker drawn from `Kind`, an outputs table, and transitions with a guard builder (Always / Status / Output field + op + value).
  - An "Insert binding" menu listing inputs and earlier states' outputs.
  - Live `validate_flow`.
  - The JSON editor stays available as an advanced tab.
  - Size: about 1,000–1,500 lines.
- **Task 9: Automations can start a Flow.**
  - Add optional `flowId` and `flowInputs` to the automation definition, which means changing the JSON shape shared with Tauri. Tauri has to accept, though not act on, the extra fields; check `src-tauri/src/automations.rs` serde (unknown fields).
  - `launch_automation` branches before session creation and calls `runner.start`.
  - The run maps to automation statuses: succeeded or waiting-approval → succeeded, failed → failed, cancelled → cancelled.
- **Task 10: Board integration.** Starts after native Board (`mc/sql-workflow-state-tracking`) merges.
  - The runner host calls `board::on_linked_work(.., "flow-run", run_id, Started | Completed | Failed)` on run status transitions only.
  - Add a "Run a Flow" action on a card, which links the card to the new run.

## Phase 3 — Parity and reach

Each task gets its own plan when it's reached.

- **Task 11: Tauri parity.**
  - Port the stash TS model, then add the contract, runner, storage, React views and Tauri DDL.
  - The DDL must be identical to Native's. Pin it on both sides through `src/test/sharedContracts.test.ts`.
  - Tauri and Native read and write the same runs.
- **Task 12: Kiro in Native.** Add a provider `Kind`, then Flows can use it.
- **Task 13: Remote-host Flow runs.** Launch visits on an SSH host's sessions.

## Inventory

**Done**
- The state-machine design (approved 2026-10-23).
- TS Task 1 in stash `a17401ea`: types, validation, bindings and starter, with 38 test cases. It's the porting source for Task 1.
- The Board `"flow-run"` link kind in Tauri and native core, and `board::on_linked_work`.
- Claude sub-agent, background-agent and heartbeat capture.

**Reused as is**
- The `launch_automation` session/worktree/submit/settle path (Task 5 extracts it as a shared helper).
- `git::create_worktree` and `worktree_cwd` reuse.
- `response::prose` (Task 2 adds `last_assistant_text` beside it).
- The `automations::Service` storage pattern.
- `shared_contracts.rs`.
- The `AppView` registration pattern (native Board `3b34648a`).

**To build (Phase 1)**
- Core model, contract, storage and runner (Tasks 1–4).
- GPUI host (Task 5).
- Flows view with the JSON editor (Task 6).
- E2E check (Task 7).

**Later**
- Visual editor (8), automation trigger (9), Board integration (10).
- Tauri parity (11), Kiro in Native (12), remote hosts (13).
