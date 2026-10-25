# Native Pilot Flows — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A native Pilot can own hidden Flows (blueprints), run one Flow run per task, have each run advance on its own, review every finished step in a hidden side run, take over failed tasks, adjust a single run's remaining steps, and report once when a whole goal is done.

**Architecture:**
- The Flow model gains an owner. Runs gain a goal/task tag and an edit history, and the runner gains a snapshot edit and a per-visit `VisitFinished` effect. All of this stays in `gespan-core`, with no DDL change: everything lives in the existing JSON columns.
- The GPUI flows host forwards `VisitFinished` and `Finished` for owned runs to the Pilots entity.
- The Pilots entity:
  - exposes `flows.*` app actions;
  - starts hidden review runs, built on the habit-run machinery but in their own slots;
  - buffers alerts (failures, review notes worth acting on, goal roll-ups) and delivers them to the main chat as **one** internal message whenever the chat is idle.

**Tech Stack:** Rust, GPUI (`crates/gespan-gpui`), `crates/gespan-core`, cargo `native-dev` profile.

**Spec:** `docs/superpowers/specs/2026-10-25-pilot-flows-design.md` (all sections except the Tauri items).
**Depends on:** `docs/superpowers/plans/2026-10-25-native-pilot-per-session-reports.md`, which must land first. Its `pilot_folder_id` and unconditional `group_launch` are used in Task 6.

## Global Constraints

- **Do not commit.** The user approves every commit. Tasks end by staging only. Use branch `feat/native-pilot-flows`, based on Plan A's branch; never `main`. Leave `README.md` untouched.
- Rust tests: `cargo test --locked --manifest-path crates/gespan-gpui/Cargo.toml -p <crate> --profile native-dev -- --test-threads=1 <filter>`; clippy `… --all-targets --profile native-dev -- -D warnings`; full suite `bun run native:test`.
- Flow tables' DDL is pinned (`crates/gespan-core/tests/shared_contracts.rs:317-381`) and must not change. New data goes in `definition_json` / `run_json` with `#[serde(default, skip_serializing_if = …)]`, so older rows still parse.
- Pilot Flows contain no `Approval` states.
- A Pilot reads, changes and runs only Flows whose `owner == Some(pilot.id)`. User Flows stay invisible to `flows.*`, and Pilot Flows stay invisible in the Flows view.
- On a successful step, nothing waits for the Pilot: the next `Launch` is applied before any notification.
- A running session is never edited or interrupted by a run edit. Edits apply only to states not yet entered.
- Side reviews are read-only: they may call only `flows.read`, `flows.run`, `flows.runs`, `sessions.read`, `memory.read`, `memory.search`.
- Memory has one writer: the main Pilot chat. Review runs cannot call `memory.add/replace/remove`.
- Naming: Gespan only; no earlier product names (enforced by `src/test/legacyNames.test.ts`).

## Review Focus

1. **App restart mid-goal.** `Runner::recover` fails running visits ("Interrupted by app restart"). That must produce a failure alert to the owning Pilot, not silence, and the goal must still roll up. *(Task 7 test.)*
2. **A run edit that changes or removes the current or an already-visited state** is rejected with a clear error, and leaves the snapshot unchanged. *(Task 2 test.)*
3. **A burst of failures while the Pilot is busy** becomes exactly one internal message when it goes idle, not one turn per failure. *(Task 7 test.)*
4. **A Pilot passing another Pilot's Flow id or run id, or a user Flow id,** to any `flows.*` action gets `"Flow not found"` / `"Run not found"`. It does not get a permission hint that leaks existence. *(Task 5 test.)*
5. **The user stops a step session from the sidebar.** The visit ends as cancelled or failed, and the Pilot gets a failure alert, the same as for any failure. *(Task 7 test.)*

---

### Task 1: Owned Flows and run tags in the model

**Files:**
- Modify: `crates/gespan-core/src/flow/definition.rs`:
  - `Flow` (154-168): add `owner`.
  - `validate_flow` (277-462): add the approval rule in the state loop at 301-328.
  - `FLOW_KEYS` (467-478).
  - `fully_populated()` (581).
- Modify: `crates/gespan-core/src/flow/run.rs`: `FlowRun` gets `tag` and `edits`; add `RunTag` and `RunEdit`.
- Modify every `Flow { … }` and `FlowRun { … }` literal to add the new fields:
  - `flow/starter.rs:86,238,255`
  - `flow/template.rs:293`
  - the test literals in `flow/definition.rs`
  - `flow/runner/tests.rs:245`
  - `gespan-gpui/src/flows/host/tests.rs:96`
  - `gespan-gpui/src/shell/flow_view_tests.rs:60`
  - `Runner::start` (runner.rs:159)
- Modify `crates/gespan-core/src/flow/mod.rs` re-exports.

**Interfaces (produces):**
```rust
// definition.rs, on Flow:
#[serde(default, skip_serializing_if = "Option::is_none")] pub owner: Option<String>, // "pilot id"
// run.rs:
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct RunTag { pub goal: String, pub task: String }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct RunEdit { pub at: i64, pub note: String, pub states: Vec<String> } // changed/added state ids
// on FlowRun:
#[serde(default, skip_serializing_if = "Option::is_none")] pub tag: Option<RunTag>,
#[serde(default, skip_serializing_if = "Vec::is_empty")] pub edits: Vec<RunEdit>,
```
- Approval rule error, exactly: `State "{id}": approval states are not allowed in a Pilot Flow`.

- [ ] **Step 1: Write the failing tests:**
  - In `definition.rs` tests:
    - `pilot_flows_reject_approval_states`: an owned flow with an Approval state has that error. The same flow with `owner: None` doesn't.
    - `owner_round_trips_and_old_json_without_it_still_parses`: serde round-trip with `Some("p1")`; the existing starter JSON without `owner` parses to `None`.
  - In `run.rs` tests:
    - `run_tag_and_edits_round_trip_and_default_when_absent`.
  - Keep `the_known_key_lists_match_what_serde_writes` passing by setting `owner` in `fully_populated()`.
- [ ] **Step 2: Run** `… -p gespan-core … flow::`. Expected: FAIL (compile errors on missing fields).
- [ ] **Step 3: Implement** the fields, the rule, `FLOW_KEYS += "owner"`, and the literal updates. `Runner::start` sets `tag: None, edits: vec![]` until Task 2.
- [ ] **Step 4: Run** `… -p gespan-core … flow::` and `… -p gespan-core --test shared_contracts`. Expected: PASS.
- [ ] **Step 5: Stage** (no commit).

### Task 2: Runner: tagged start, run edits, `VisitFinished`

**Files:**
- Modify: `crates/gespan-core/src/flow/runner.rs`: `Effect` (48), `start` (150), `advance` (476), `finish` (455), `recover` (381); add `edit_run`.
- Modify: the callers of `start`: `gespan-gpui/src/flows/host.rs:95`, `shell/flow_runs.rs:123` (pass `None`).
- Test: `crates/gespan-core/src/flow/runner/tests.rs`

**Interfaces (produces):**
```rust
pub enum Effect { …, VisitFinished { run: FlowRun, visit: FlowVisit } }
pub fn start(&mut self, flow: &Flow, given: &HashMap<String, String>, tag: Option<RunTag>)
    -> Result<(FlowRun, Vec<Effect>), Vec<String>>;
pub fn edit_run(&mut self, run_id: &str, flow: Flow, note: &str) -> Result<Vec<Effect>, Vec<String>>;
```
- **`VisitFinished` is emitted:**
  - When an agent visit ends `Completed`, `Failed` or `Cancelled` through `on_agent_settled`, as the **last** effect of that call, after any `Launch` of the next state.
  - For each visit that `recover` fails.
  - **Not** for `Runner::cancel` (the caller already knows), and not for contract-retry follow-ups (the visit has not ended).
- **`edit_run` rules:**
  - The run must be active.
  - `flow.id` and `flow.owner` must equal the snapshot's.
  - `validate_flow(&flow)` must have no errors.
  - Every state id with any visit in `active.visits` (this includes the current one) must exist in `flow` and be `==` to the snapshot's state.
  - `initial` must be unchanged.
  - On success: replace `active.run.flow`, push `RunEdit { at: clock(), note, states }` where `states` lists the ids of new or changed states, and return `[Persist { run, visit: None }]`.
  - Errors, exactly: `"Run not found"`, `"A run edit cannot change the Flow's id, owner or initial state"`, `"State \"{id}\" has already run and cannot be changed"`, plus validation errors.

- [ ] **Step 1: Write the failing tests** (the fake `Host` in `runner/tests.rs` records effects):
  - `a_finished_visit_is_reported_after_the_next_launch`: two-state flow; settle visit 1 completed. Assert the effects end `[…, Launch(state 2), VisitFinished{visit.state_id == "s1", status Completed}]`.
  - `a_failed_visit_and_a_failed_run_are_both_reported`: settle on the failed path. Assert both `VisitFinished` (status `Failed`) and `Finished { status: Failed }` are present.
  - `cancel_and_contract_retry_do_not_report_a_finished_visit`.
  - `recover_reports_each_interrupted_visit`.
  - `start_keeps_the_tag_on_the_run`.
  - `edit_run_changes_only_states_not_yet_entered`:
    - Change state 3's brief while in state 2 → `Ok`.
    - Later, entering state 3 renders the new brief (the `Launch` prompt contains it).
    - `run.edits.len() == 1` with `states == ["s3"]`.
  - `edit_run_refuses_visited_states_identity_changes_and_invalid_flows`: one case per error string above. The snapshot is unchanged after each.
- [ ] **Step 2: Run** `… -p gespan-core … flow::runner`. Expected: FAIL.
- [ ] **Step 3: Implement.** The host's `apply` (`gespan-gpui/src/flows/host.rs:310-343`) ignores `VisitFinished` for now (Task 4 wires it), so the GPUI build stays green.
- [ ] **Step 4: Run** `… -p gespan-core … flow::` and `… -p gespan-gpui … flows::`. Expected: PASS.
- [ ] **Step 5: Stage.**

### Task 3: Hide Pilot Flows from the Flows view; owned-store helpers

**Files:**
- Modify: `crates/gespan-core/src/flow/store.rs`: add helpers.
- Modify: `crates/gespan-gpui/src/flows/view.rs:231-283` (`refresh`): `flows.retain(|f| f.owner.is_none())`. Runs and active sets follow the filtered list.
- Test: `crates/gespan-core/src/flow/store.rs` tests, `crates/gespan-gpui/src/shell/flow_view_tests.rs`

**Interfaces (produces):**
```rust
impl FlowStore {
    pub fn owned(&self, owner: &str) -> Result<Vec<Flow>, String>;               // list() filtered
    pub fn owned_flow(&self, owner: &str, id: &str) -> Result<Option<Flow>, String>;
    pub fn owned_run(&self, owner: &str, run_id: &str) -> Result<Option<(FlowRun, Vec<FlowVisit>)>, String>; // owner from run.flow.owner
    pub fn owned_runs(&self, owner: &str, goal: Option<&str>, limit: usize) -> Result<Vec<FlowRun>, String>;
}
```
`owned_runs` scans the owner's flows' runs (`runs(flow_id, 50)`), filters by `tag.goal` when given, and sorts newest first. That is enough because Pilot Flows are few; no DDL change.

- [ ] **Step 1: Write the failing tests:**
  - `owned_helpers_return_only_that_pilots_flows_and_runs`: flows owned by `p1`, `p2` and the user. Each helper returns only `p1`'s. `owned_flow("p1", p2_flow_id)` is `None`.
  - `flows_view_lists_only_user_flows`, in `flow_view_tests.rs`.
- [ ] **Step 2: Run** `… -p gespan-core … flow::store` and `… -p gespan-gpui … flow_view`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same commands. Expected: PASS.
- [ ] **Step 5: Stage.**

### Task 4: Shell API for Pilot Flows and forwarding of step events

**Files:**
- Create: `crates/gespan-gpui/src/shell/pilot_flows.rs` (register it in `shell.rs`).
- Modify: `crates/gespan-gpui/src/flows/host.rs`:
  - `SessionDriver` (13): add `fn visit_finished(&mut self, run: FlowRun, visit: FlowVisit)` and `fn run_finished(&mut self, run_id: &str, status: RunStatus)`.
  - `apply` (310-343): `VisitFinished` → `driver.visit_finished`; `Finished` also calls `driver.run_finished` after its existing work.
  - Add `pub(crate) fn edit_run(&mut self, driver, run_id, flow, note) -> Result<(), Vec<String>>`.
  - Update the fake `Driver` in `flows/host/tests.rs` and the `Quiet` driver in `shell/flow_runs.rs:449`.
- Modify: `crates/gespan-gpui/src/shell/flow_runs.rs`: `Driver` (40-70) forwards both new calls to `Shell::pilot_flow_event` (in the new file).
- Test: `crates/gespan-gpui/src/flows/host/tests.rs`, plus GPUI tests in `shell/pilot_flows.rs`.

**Interfaces (produces):**
```rust
// shell/pilot_flows.rs, impl Shell:
pub(crate) fn pilot_flows_store(&self) -> Result<StoreHandle, String>; // "Flows are not ready" if not opened
pub(crate) fn start_pilot_flow(&mut self, pilot_id: &str, flow_id: &str, inputs: HashMap<String,String>,
    tag: RunTag, cx: &mut Context<Self>) -> Result<FlowRun, Vec<String>>;   // "Flow not found" unless owned
pub(crate) fn edit_pilot_run(&mut self, pilot_id: &str, run_id: &str, flow: Flow, note: &str,
    cx: &mut Context<Self>) -> Result<(), Vec<String>>;
pub(crate) fn cancel_pilot_run(&mut self, pilot_id: &str, run_id: &str, cx: &mut Context<Self>) -> Result<(), String>;
pub(crate) fn resume_pilot_run(&mut self, pilot_id: &str, run_id: &str, state_id: &str, cx: &mut Context<Self>) -> Result<(), String>;
pub(crate) enum PilotFlowEvent { Step { run: FlowRun, visit: FlowVisit }, Finished { run_id: String, status: RunStatus } }
fn pilot_flow_event(&mut self, event: PilotFlowEvent, cx: &mut Context<Self>);
// forwards to `self.pilot_manager().update(cx, |m, cx| m.flow_event(pilot_id, event, cx))`, only when the
// run's `flow.owner` is Some; Finished looks the run up through `host.run_of` before it is dropped.
```
- Ownership: every method loads through Task 3's `owned_*`. On a mismatch it returns `"Flow not found"` / `"Run not found"`.

- [ ] **Step 1: Write the failing tests:**
  - In `flows/host/tests.rs`:
    - `visit_finished_reaches_the_driver_after_the_next_launch`.
    - `run_finished_reaches_the_driver`.
  - In `shell/pilot_flows.rs`:
    - `a_pilot_starts_only_its_own_flows`: `p1` starting `p2`'s flow or a user flow → `Err(["Flow not found"])`; its own → `Ok(run)` with `run.tag == Some(tag)`.
    - `owned_step_events_reach_the_pilots_entity_and_user_flow_events_do_not`: assert through a counter or record on `Pilots` used by Task 7 (expose `#[cfg(test)] flow_events: Vec<…>`).
- [ ] **Step 2: Run** `… -p gespan-gpui … flows::host shell::pilot_flows`. Expected: FAIL.
- [ ] **Step 3: Implement.** `Pilots::flow_event(&mut self, pilot_id: &str, event: PilotFlowEvent, cx)` starts as a stub that only records the event under `cfg(test)`. Task 7 fills it in.
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Stage.**

### Task 5: `flows.*` app actions

**Files:**
- Modify: `crates/gespan-core/src/pilot/bridge.rs:17-40` (`HELP`), `crates/gespan-core/src/pilot/api.rs:4-21,36-53` (key lists and object exemptions).
- Modify: `crates/gespan-gpui/src/pilots/host.rs`, `app_call` (136-319): the new match arms.
- Test: `crates/gespan-gpui/src/pilots/tests.rs` (use the `call` helper at 353).

**Actions** (input → result). All are scoped to the calling Pilot:

| Action | Input | Result |
|---|---|---|
| `flows.list` | `{}` | `{flows:[{id,name,description,states:[id…],updatedAt}]}` |
| `flows.read` | `{flowId}` | the Flow JSON |
| `flows.save` | `{flow}` | `{flow}` saved. `owner` is forced to the caller; an empty `id` gets a new id. An `id` that exists but isn't the caller's → `"Flow not found"`. Validation errors are returned verbatim. |
| `flows.delete` | `{flowId}` | `{deleted:true}`. Refused while active, with the existing `"Flow has an active run"`. |
| `flows.start` | `{flowId, goal, task, inputs?:{}}` | `{runId}`. `goal` and `task` are required strings, ≤ 300 chars. |
| `flows.runs` | `{goal?}` | `{runs:[{runId,flowId,goal,task,status,currentState,error}]}` |
| `flows.run` | `{runId}` | `{run, visits:[{seq,stateId,status,sessionId,outputs,error}]}` |
| `flows.editRun` | `{runId, flow, note}` | `{edited:true}` or errors |
| `flows.cancel` | `{runId}` | `{cancelled:true}` |
| `flows.resume` | `{runId, stateId}` | `{resumed:true}`. Only for failed/cancelled runs (`Runner::resume`). |

- Object or array keys exempt from the string check: `flow`, `inputs`.
- `HELP` gets one line per family, in the existing style. For example: `flows.start {flowId,goal,task,inputs?}`.
- Mutating actions go through the existing idempotent receipt path (`finish_call`).
- The read-only gate for review runs is added in Task 7, which creates them.

- [ ] **Step 1: Write the failing tests:**
  - `flows_actions_round_trip_for_the_owning_pilot`: save → list → read → start (draft-free flow whose `Launch` is captured) → runs (filtered by goal) → run → editRun → cancel.
  - `flows_actions_never_reveal_other_pilots_or_user_flows`: a second Pilot and a user flow; every action with a foreign id returns `"Flow not found"` / `"Run not found"`; `flows.list` excludes them.
  - `flows_save_rejects_approval_states_and_forces_the_owner`.
  - `flows_start_requires_goal_and_task`.
  - `api_validate_rejects_unknown_flows_keys`, in `pilot/api.rs` tests.
- [ ] **Step 2: Run** `… -p gespan-gpui … pilots::tests::flows_` and `… -p gespan-core … pilot::api`. Expected: FAIL.
- [ ] **Step 3: Implement.** Handlers call Task 4's Shell methods through `self.owner.update(cx, …)`. Reads use `pilot_flows_store()` off-thread through `call_work`.
- [ ] **Step 4: Run** the same commands. Expected: PASS.
- [ ] **Step 5: Stage.**

### Task 6: Step sessions of Pilot runs are titled per task and filed in the Pilot's folder

**Files:**
- Modify: `crates/gespan-gpui/src/shell/flow_runs.rs:330-391` (`launch_flow_visit`).
- Modify: `crates/gespan-core/src/flow/runner.rs`, the `enter_state` title, so tagged runs get `"{task} · {state title}"`. Untagged runs keep today's `"<Flow name> · <state title>"`.
- Test: `shell/flow_runs.rs` tests, `runner/tests.rs`

**Interfaces:**
- Consumes: Plan A's `pilot_folder_id` and `Pilots::group_launch(pilot_id, session, cx)`.
- Step sessions are **not** added to `pilot.launches` (Flow events report them; adding them would duplicate reports).

- [ ] **Step 1: Write the failing tests:**
  - `tagged_runs_title_step_sessions_by_task`, in runner tests.
  - `a_pilot_runs_step_sessions_are_visible_and_in_the_pilots_folder`: in a GPUI shell test, the launched visit session is listed in the sidebar, and `groups.folder_of(session).id == pilot_folder_id(pilot)`. Its `pilot.launches` is unchanged.
  - `user_flow_step_sessions_are_not_filed`.
- [ ] **Step 2: Run** `… flow::runner::tests::tagged_runs` and `… shell::flow_runs`. Expected: FAIL.
- [ ] **Step 3: Implement.** After `this.add(summary, cx)`, when `run.flow.owner` is `Some(p)`, call `pilot_manager().update(cx, |m, cx| m.group_launch(&p, id, cx))`. The owner comes from `host.run_of(run_id)`.
- [ ] **Step 4: Run** the same commands. Expected: PASS.
- [ ] **Step 5: Stage.**

### Task 7: The Pilot's response to step events: reviews, alerts, goal roll-up

**Files:**
- Create: `crates/gespan-core/src/pilot/flow_notes.rs`: pure note parsing, alert text, and roll-up text.
- Create: `crates/gespan-gpui/src/pilots/flow_reviews.rs`: review runs, the alert buffer, and `flow_event`.
- Modify: `crates/gespan-core/src/pilot.rs`: `Pilot` gets `#[serde(default)] flow_notes: Vec<FlowNote>`, `#[serde(default)] flow_alerts: Vec<String>`, `#[serde(default)] goals: Vec<Goal>`.
- Modify: `crates/gespan-gpui/src/pilots/scheduler.rs:1302-1342`: extract `start_habit`'s session creation into `fn spawn_hidden_run(&mut self, pilot_id: &str, title: String, prompt: String, cx) -> Task<Result<(i64, Entity<Conversation>), String>>`, used by both habits and reviews. Habit behaviour is unchanged.
- Modify: `crates/gespan-gpui/src/pilots/mod.rs`:
  - `chat_changed` (972-1012) and `tick` (`scheduler.rs:1183`) also call `flush_flow_alerts` and `pump_reviews`;
  - add the `reviews: HashMap<i64, RunningReview>` field and `review_queue: VecDeque<(String, FlowRun, FlowVisit)>`.
- Test: `pilot/flow_notes.rs` tests, `pilots/tests.rs`

**Interfaces (produces):**
```rust
// gespan-core pilot/flow_notes.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "kebab-case")]
pub enum NoteKind { NoChange, SuggestStepChange, NeedsAttention, BlueprintIdea }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct FlowNote { pub run_id: String, pub goal: String, pub task: String, pub state_id: String,
    pub kind: NoteKind, pub text: String, pub at: i64 }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct Goal { pub name: String, pub runs: Vec<String>, pub reported: bool }
pub const MAX_REVIEWS: usize = 3;          // concurrent review runs per Pilot
pub const MAX_NOTES: usize = 500;          // oldest notes of reported goals dropped first
pub fn review_prompt(run: &FlowRun, visit: &FlowVisit, step_text: &str) -> String;
pub fn parse_note(reply: &str) -> (NoteKind, String); // first line `NO_CHANGE|SUGGEST_STEP_CHANGE|NEEDS_ATTENTION|BLUEPRINT_IDEA`; anything else → (NeedsAttention, whole reply)
pub fn failure_alert(run: &FlowRun, visit: &FlowVisit) -> String;
pub fn note_alert(note: &FlowNote) -> String;
pub fn goal_rollup(goal: &Goal, runs: &[FlowRun], notes: &[FlowNote]) -> String;
pub fn bundle(alerts: &[String]) -> String; // one → "[Flow update]\n\n{alert}", n → "[Flow update]\n{n} updates from your Flows.\n\n{joined by \"\\n\\n---\\n\\n\"}"
```

**Behaviour:**
- **`flows.start`** records the run id under `goals` (creating the goal by name).
- **`flow_event(Step)` with a Completed visit:**
  - Push onto `review_queue`.
  - `pump_reviews` starts up to `MAX_REVIEWS` hidden runs through `spawn_hidden_run`:
    - title: `"{pilot} · review · {task}"`;
    - the session goes in `roster.hidden_sessions`;
    - prompt: `review_prompt`.
  - When a review's chat goes idle:
    1. `parse_note(prose)`;
    2. push a `FlowNote`;
    3. unless `NoChange`, push `note_alert` onto `flow_alerts`;
    4. remove the run;
    5. pump again.
  - A review is time-boxed by habit `TIMEOUT_MS`. On timeout, record `NeedsAttention` with `"Review timed out"`.
- **`flow_event(Step)` with a Failed or Cancelled visit:** push `failure_alert` (task, goal, state, error, session id, and `"Use flows.run {runId} to inspect, then flows.resume, flows.editRun or flows.start; ask the user only if you can't recover."`).
- **`flow_event(Finished)`:**
  - Mark the run ended in its goal.
  - When every run of the goal has ended and `!reported`: push `goal_rollup` (per-task final status, plus every note that is not `NoChange`, plus a count of `NoChange` notes), and set `reported = true`.
- **Review-run gate** (`pilots/host.rs` `app_call`, next to the habit gate at 152): if `call.session` is in `reviews`, any action outside the read-only list in Global Constraints → `"Reviews are read-only; report your suggestion instead"`.
- **`flush_flow_alerts`:** when the main chat is not busy and `flow_alerts` is non-empty, `enqueue(pilot, bundle(&alerts), vec![], internal = true)`, clear the buffer, `ensure_chat`, `save`. The buffer is persisted in the roster, so a restart keeps it.
- **`review_prompt` copy:**
  - It states the goal, task, step title, the step's outputs and reply (≤ 12 000 chars), and the remaining state ids.
  - It ends with exactly: `"Review only this step. You cannot change anything. Reply with one first line: NO_CHANGE, SUGGEST_STEP_CHANGE, NEEDS_ATTENTION or BLUEPRINT_IDEA, then at most a short paragraph explaining it."`

- [ ] **Step 1: Write the failing core tests** in `flow_notes.rs`:
  - `parse_note_reads_the_kind_line_and_defaults_to_needs_attention`;
  - `bundle_formats_one_and_many_alerts`;
  - `goal_rollup_lists_each_task_and_only_notes_worth_reading`;
  - `review_prompt_truncates_long_step_text_and_ends_with_the_contract`.
- [ ] **Step 2: Write the failing GPUI tests** in `pilots/tests.rs`. Drive `m.flow_event(...)` directly with constructed `FlowRun`/`FlowVisit`, and fake review replies by writing assistant rows into the review session and setting `busy = false`:
  - `a_completed_step_starts_a_hidden_review_and_never_touches_the_main_chat_when_nothing_changes`.
  - `review_suggestions_and_failures_while_busy_arrive_as_one_message_when_idle`: main chat busy; one failure, two failures, one `SUGGEST_STEP_CHANGE` note. No outbox messages yet. Go idle → exactly one internal outbox message starting `"[Flow update]\n4 updates from your Flows."`.
  - `no_more_than_three_reviews_run_at_once_and_the_queue_drains`.
  - `a_goal_rolls_up_once_when_its_last_run_ends`: three runs; finishing two → no roll-up; finishing the third → one roll-up; a duplicate `Finished` → no second roll-up.
  - `interrupted_and_user_stopped_steps_alert_the_pilot` (Review Focus 1 and 5).
  - `reviews_cannot_mutate`: a call from a review session to `flows.editRun` and to `memory.add` → `"Reviews are read-only; report your suggestion instead"`.
- [ ] **Step 3: Run** `… -p gespan-core … pilot::flow_notes` and `… -p gespan-gpui … pilots::tests::` (the new names). Expected: FAIL.
- [ ] **Step 4: Implement** per the Behaviour block.
- [ ] **Step 5: Run** the same commands plus the existing `pilots::` habit tests. Expected: PASS; habits unchanged.
- [ ] **Step 6: Stage.**

### Task 8: Teach the Pilot

**Files:**
- Modify: `crates/gespan-core/src/pilot.rs:321` (`context`).
- Test: `crates/gespan-core/src/pilot/tests.rs`

- [ ] **Step 1: Write the failing test** `context_explains_pilot_flows_supervision_and_memory`. It asserts the context contains the exact paragraph below and still contains Plan A's completion sentence.
- [ ] **Step 2: Run** `… -p gespan-core … pilot::tests::context_explains`. Expected: FAIL.
- [ ] **Step 3: Add** this paragraph after the habits paragraph, exactly:

  `"For a goal with several independent tasks that each need multiple focused steps, write a Flow blueprint with flows.save (one role per agent state, no approval states) and start one run per task with flows.start, giving the goal and task. Runs move on their own; never drive or poll them. You receive [Flow update] messages for failed steps, review suggestions and finished goals: recover failed tasks yourself with flows.run, flows.resume, flows.editRun or flows.start, and adjust a single task's remaining steps with flows.editRun. Ask the user only when you need clarification, when an action conflicts with your memory or their earlier input, or when a decision would affect or block them. Propose blueprint changes to the user and apply them with flows.save only after they agree. Record every user instruction about how work should run, and every blueprint change with its reason, in memory."`
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Run the gates:** clippy and `bun run native:test`. Expected: clean, all pass.
- [ ] **Step 6: Stage.** Report to the user for commit approval.
