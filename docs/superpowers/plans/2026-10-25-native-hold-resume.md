# Hold and Resume (Native) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One Hold action in Gespan Native stops every session at its next safe point without killing a running command, holds all new work and queued messages across quit/restart, and one Resume action continues every session where it stopped.

**Architecture:**
- **The safe point is decided where the raw provider stream is read.** That's the worker loop in `gespan-core/src/provider.rs`. A new `Decision::Hold` control asks the loop to stop at the next moment no foreground tool is running. It does that with a soft interrupt: no denials, no `stop_task`, and no process kill.
- **The app tracks the hold in one place.** A gpui global, `crate::hold::Hold`, owns the app-wide state and persists it through `gespan-core/src/hold.rs`.
- **Every turn start goes through one gate.** That gate is `Conversation::launch_prompt`, so composer sends, automations, habits, orchestration and Pilot `sessions.start`/`sessions.send` are all queued instead of run.
- **Resume reuses the existing paused-queue path** (`resume_queue` sends "Continue from where you left off."), staged and rate-limited by the Shell.

**Tech Stack:** Rust (gpui, serde, rusqlite store), cargo tests. The UI side is Native only.

**Spec:** `docs/superpowers/specs/2026-10-25-native-hold-resume-design.md` (GitHub issue #173)

## Global Constraints

- **Scope:** this plan covers Gespan Native only (`crates/gespan-gpui`, `crates/gespan-core`), with no Tauri/React changes. The desktop app follows in its own plan and PR once this one ships, with the same behaviour and placement.
- **Product name:** Gespan, in all copy, identifiers and comments. Never the old names.
- **Hold must never kill a running foreground command.** The only path that does is the explicit per-session **Stop now**.
- **Waiting is not killing.** Background tasks (`Event::Background`) don't block reaching On hold, and Hold doesn't stop them while the app is open.
- **Resume order:** plain sessions and Pilot workers first, then Pilot chats. At most **3** sessions resume at once.
- **Continuation text:** `"Continue from where you left off."` (the existing string in `conversation/follow_up.rs`).
- **Shortcut:** `cmd-shift-h` (action `workspace::ToggleHold`), with palette label `"Hold everything"` / `"Resume everything"`.
- **Hold is app-wide.** Its button lives in the bottom status bar (`footer_row`), never inside a session.
- **Every way of starting Hold asks first.** The button, ⌘⇧H, the palette, the app menu and the menu-bar item all show a confirmation dialog before anything happens.
- **Hold is one-way until complete.** While Holding there is no Resume and no undo. The status-bar control shows a continuous progress state ("Holding 2/5…"), and Resume appears only once the phase is On hold.
- **Store:** hold state lives in the `hold` setting key as JSON (`Store::setting` / `Store::save_setting`).
- **Commits:** no tool or AI attribution lines.
- **Tests:** `cargo test -p gespan-core` and `cargo test -p gespan-gpui` must stay green. Run them from the worktree with `CARGO_BUILD_JOBS=1` if memory is tight.

## Review Focus

1. **Hold pressed twice, ⌘⇧H while Holding, or the confirmation cancelled.** Cancel changes nothing. While Holding, the toggle does nothing: no Resume and no second confirmation. *Test in Task 4 (`cancelled_confirmation_changes_nothing`, `toggle_ignored_while_holding`).*
2. **A message typed into a session that is still settling (busy, Hold sent).** It must go into the queue with `steer = false` and must never be steered into the live turn. *Test in Task 3.*
3. **The app is quit while still Holding.** Gespan asks for confirmation. On relaunch, sessions that hadn't settled are recorded as `Reason::Restarted` and get the continuation on Resume. *Test in Task 1 (`after_restart_*`) and Task 4.*
4. **A Pilot worker held mid-turn.** The Pilot must not get "[Delegated work completed]" while the worker is held, or in the gap between Resume and the worker's turn starting. *Test in Task 5.*
5. **Resume with nothing held, or with a held session that has since been deleted or archived.** Resume clears the state and skips missing sessions without error. *Test in Task 1 (`resume_plan_skips_unknown`) and Task 4.*

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/gespan-core/src/hold.rs` (new) | `HoldState`, `Held`, `Reason`, `Phase`. Persistence on `Store`, restart normalisation, resume staging. Pure, with no gpui. |
| `crates/gespan-core/src/agent.rs` | `Decision::Hold`, `Decision::Release`. |
| `crates/gespan-core/src/provider.rs`, `provider/lifecycle.rs`, `provider/opencode.rs` | Safe-point detection, `Interrupt::{Stop, Hold}`, the `Event::HoldSettled(HoldPoint)` event, and the `"Held"` result. |
| `crates/gespan-gpui/src/hold.rs` (new) | The gpui `Global` with the live `HoldState` and its save/load. The `active(cx)` helper. |
| `crates/gespan-gpui/src/conversation.rs`, `conversation/follow_up.rs`, `runtime.rs` | Per-session hold and release. Launch gate. Hold-aware queueing. Handling the `"Held"` result. The held marker in the composer shelf. |
| `crates/gespan-gpui/src/shell/hold.rs` (new) | The coordinator: `hold_all`, `stop_now`, `resume_all` (staged), settle tracking, startup restore, the quit guard, and the banner and title-bar button render functions. |
| `crates/gespan-gpui/src/shell.rs`, `shell/layout.rs`, `shell/settings.rs`, `shell/automation_runs.rs`, `shell/orchestration.rs`, `quick_open.rs` | Wiring: action and key, banner slot, keep-awake, scheduler gates, palette command. |
| `crates/gespan-gpui/src/pilots/scheduler.rs`, `pilots/host.rs`, `pilots/status.rs` | Habit gate and timeout shift, completion suppression, and the menu-bar item. |

---

### Task 1: Core hold model (`gespan-core/src/hold.rs`)

**Files:**
- Create: `crates/gespan-core/src/hold.rs`
- Modify: `crates/gespan-core/src/lib.rs` (add `pub mod hold;`)
- Test: `crates/gespan-core/src/hold.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase { Holding, OnHold }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reason { Idle, Interrupted, AwaitingInput, Background, Restarted }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Held {
    pub session: i64,
    pub reason: Option<Reason>,      // None = still settling
    pub held_at: u64,                // unix ms when it settled (0 while settling)
    pub last_tool: Option<String>,   // title of the last completed tool
    pub background: Vec<String>,     // background task descriptions at settle time
    pub pilot_chat: bool,            // resumes in stage 2
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoldState { pub phase: Phase, pub since: u64, pub sessions: Vec<Held> }
impl HoldState {
    pub fn new(now: u64) -> Self;                         // Holding, no sessions
    pub fn track(&mut self, session: i64, pilot_chat: bool);
    pub fn settle(&mut self, session: i64, reason: Reason, now: u64, last_tool: Option<String>, background: Vec<String>);
    pub fn settled(&self) -> bool;                        // every Held has Some(reason)
    pub fn refresh_phase(&mut self);                      // Holding -> OnHold when settled()
    pub fn after_restart(self) -> Self;                   // see Step 1
    pub fn held_ms(&self, now: u64) -> u64;               // now - since
}
pub fn continuation(reason: Reason) -> Option<&'static str>;   // Interrupted|Restarted -> Some("Continue from where you left off."), else None
pub fn resume_plan(state: &HoldState, exists: impl Fn(i64) -> bool) -> Vec<Vec<i64>>; // [stage1, stage2], unknown sessions dropped
impl crate::store::Store {
    pub fn hold(&self) -> Result<Option<HoldState>, String>;    // setting "hold"
    pub fn save_hold(&self, state: &HoldState) -> Result<(), String>;
    pub fn clear_hold(&self) -> Result<(), String>;             // saves "" and hold() treats empty as None
}
```

- [ ] **Step 1: Write the failing tests.**
  - `settles_to_on_hold_only_when_every_session_settled`: track 2; settle 1, then `refresh_phase` gives `Holding`; settle the second, then `OnHold`.
  - `after_restart_turns_unsettled_and_live_waits_into_restarted`: sessions with `None`, `AwaitingInput` and `Background` become `Restarted`. `Interrupted` and `Idle` are unchanged. Phase becomes `OnHold`.
  - `continuation_only_for_interrupted_or_restarted`.
  - `resume_plan_puts_pilot_chats_last`: stage 1 = non-pilot ids in tracking order, stage 2 = pilot chats.
  - `resume_plan_skips_unknown`: ids for which `exists` is false are absent, and empty stages are dropped.
  - `store_round_trips_and_clears`: on `Store::open_in_memory()` (or the existing test constructor), `save_hold` then `hold()` gives an equal state, and `clear_hold` then `hold()` gives `None`.
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test -p gespan-core hold::`. Expected: compile errors or FAIL.
- [ ] **Step 3: Implement.** Use serde JSON in the `hold` setting. `track` ignores duplicates.
- [ ] **Step 4: Run again.** Run `cargo test -p gespan-core hold::`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Add the core Hold model and its persistence"`

### Task 2: Safe-point hold in the provider loop

**Files:**
- Modify: `crates/gespan-core/src/agent.rs:223` (`Decision`)
- Modify: `crates/gespan-core/src/provider.rs`. Change `Event` (line 79), `run_operation`'s loop around lines 1654-1662 (cancel and controls), the frame handling around lines 1731-1760, the end-of-run park/settle around lines 1874-1886, and the tests module.
- Modify: `crates/gespan-core/src/provider/lifecycle.rs:94-157` (`interrupt`)
- Modify: `crates/gespan-core/src/provider/opencode.rs` (same rule in its loop)

**Interfaces:**
- Consumes: none.
- Produces:
```rust
// agent.rs
pub enum Decision { /* existing */ Hold }
// provider.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldPoint { Interrupted, AwaitingInput, Background }
pub enum Event { /* existing */ HoldSettled(HoldPoint) }
// lifecycle.rs
pub(super) enum Interrupt { Stop, Hold }
pub(super) fn interrupt(&mut self, stdin: &mut impl Write, kind: Interrupt) -> Result<bool, String>;
pub(super) fn hold_point(&self) -> Option<HoldPoint>;  // None = must wait (foreground tool running)
```
- Result contract: a turn ended by Hold returns `Err("Held".into())`, and the process is `parked::put` (reusable), never `terminate`d.

**Rule (the algorithm the tests pin).** Once `Decision::Hold` arrives, the loop sets `hold_requested`. Hold can't be withdrawn. The loop then re-evaluates on every iteration while `hold_requested && !interrupted`:
1. **An approval or question is pending:** emit `HoldSettled(AwaitingInput)` once and keep answering nothing. The approval stays pending.
2. **A foreground tool is running:** do nothing yet. Foreground means `Event::Tool` with `ToolState::Running` and an id that is not in the current background set.
3. **The provider has yielded and only background tasks remain:** emit `HoldSettled(Background)` once. If any `Text`, `Reasoning` or foreground `Tool(Running)` event arrives afterwards, the agent woke up, so fall back to this rule again. "Yielded" means Claude `claude_result_seen`, or a non-empty `Event::Background`.
4. **Otherwise:** call `interrupt(stdin, Interrupt::Hold)`, set `interrupted`, and emit `HoldSettled(Interrupted)`. Wait for the terminal frame (Codex ack or `turn/completed`, Claude `result`, ACP/Pi stop reply) the same way Stop does, but **don't return early for Claude**. Return `Err("Held")`.

`Interrupt::Hold` differs from `Stop`: it never denies approvals or questions and never sends Claude `stop_task`. While `hold_requested`, the auto-deny block at provider.rs ~1745 must not fire, and question `expire_questions` is skipped.

- [ ] **Step 1: Write the failing tests** in `provider.rs` tests. Use the existing shell-script `fixture` pattern (see `cancellation_reaps_child_and_descendant_promptly`, line 2314), with `run_with_controls` (line 1325) sending `Control { id: String::new(), decision: Decision::Hold }`.
  - `hold_waits_for_running_tool_then_interrupts` (Claude and Codex fixtures). The fixture emits a Running tool frame, runs `sleep 1 && echo done > out`, emits the tool's completion, then would stream text forever. Send Hold while the tool runs. Assert:
    - the result is `Err("Held")`;
    - the `out` file exists, so the command finished;
    - the events contain `HoldSettled(Interrupted)` after the tool's completion event;
    - the fixture saw an interrupt frame (it logs stdin to a file);
    - no `stop_task` and no Deny `control_response` appear in that log.
  - `hold_interrupts_streaming_immediately`. The fixture streams text in a loop. Hold returns `Err("Held")` within 2 s.
  - `hold_leaves_pending_approval_pending`. The fixture requests approval and waits. Assert `HoldSettled(AwaitingInput)` is emitted, there's no Deny in the stdin log, and the run is still going after 500 ms. Then cancel to finish the test.
  - `hold_does_not_stop_background_tasks`. A Claude fixture yields a `result` with a background task registered. Assert `HoldSettled(Background)`, and no `stop_task` in the log.
  - `held_process_is_parked_not_killed`. After `Err("Held")`, the descendant `sleep 30 &` started by the fixture is still alive. Check with `kill(pid, 0)`.
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test -p gespan-core provider::tests::hold_ provider::tests::held_`. Expected: FAIL or compile error.
- [ ] **Step 3: Implement.** Implement `Decision::Hold`, `Interrupt`, `hold_point`, the loop rule, the opencode equivalent, and the `"Held"` result with `parked::put`. ACP and Pi reuse `acp_cancel` and `pi_cancel`, which are safe because rule 1 means no approvals are pending when they run.
- [ ] **Step 4: Run the new tests and the whole core suite.** Run `cargo test -p gespan-core`. Expected: PASS. The existing cancellation tests are unchanged.
- [ ] **Step 5: Verify Claude Code by hand.** Using the real CLI, check whether an interrupt control request (without `stop_task`) leaves a `run_in_background` Bash running. Record the outcome in a code comment on `Interrupt::Hold`. If the CLI kills it, rule 3 still holds those sessions without interrupting them, so only sessions that are streaming while a background task runs are affected. Note that in the PR.
- [ ] **Step 6: Commit.** `git commit -m "Stop a turn at its next safe point on Hold without killing commands"`

### Task 3: Per-session hold in the conversation

**Files:**
- Create: `crates/gespan-gpui/src/hold.rs` (with `mod hold;` in `main.rs`)
- Modify: `crates/gespan-gpui/src/conversation.rs`:
  - `stop` (around line 2629);
  - `launch_prompt` (line 969);
  - the event/update handling for `HoldSettled`;
  - a `held` field.
- Modify: `crates/gespan-gpui/src/conversation/follow_up.rs`:
  - `enqueue_follow_up` (line 95);
  - `pump_follow_ups` (line 270);
  - `resume_queue` (line 389);
  - the queue shelf copy.
- Modify: `crates/gespan-gpui/src/runtime.rs`:
  - forward `HoldSettled` in `Update`;
  - treat `"Held"` like a stop without the "Stopped" transcript row (line ~877, and `runtime/persistence.rs:154`).
- Test: `crates/gespan-gpui/src/conversation/` tests (follow the existing gpui `TestAppContext` conversation tests near `conversation.rs:3420`).

**Interfaces:**
- Consumes: `gespan_core::hold::{HoldState, Reason}` and `Decision::Hold`, `Event::HoldSettled`, `"Held"` from Task 2.
- Produces:
```rust
// gpui hold.rs
pub(crate) struct Hold(pub Option<gespan_core::hold::HoldState>);   // impl gpui::Global
pub(crate) fn active(cx: &gpui::App) -> bool;                       // Some(_) regardless of phase
// Conversation
pub(crate) fn hold(&mut self, cx: &mut Context<Self>);              // busy -> send Decision::Hold; idle -> emit HoldChanged(Reason::Idle)
pub(crate) fn stop_now(&mut self, cx: &mut Context<Self>);          // existing stop(); then settles as Interrupted
pub(crate) fn resume_from_hold(&mut self, reason: Reason, cx: &mut Context<Self>); // continuation (if any) then queued messages, via resume_queue
pub(crate) held: Option<Reason>;                                     // set when settled, cleared when its resumed turn begins
pub(crate) struct HoldChanged { pub session: i64, pub reason: Reason, pub last_tool: Option<String>, pub background: Vec<String> } // EventEmitter
```

**Behaviour:**
- **Launch gate.** While `hold::active(cx)`, `launch_prompt` enqueues the prompt instead of starting a turn. Its existing `stopping` branch already does this, so reuse it. The only exception is the call from `resume_from_hold`.
- **Queueing without steering.** While active, `enqueue_follow_up` forces `message.steer = false`, and `pump_follow_ups` returns early. The Pilot-chat early return at line 282 keeps working, because the gate is in `launch_prompt`.
- **Queue label.** The paused-queue shelf shows "Held: sends on Resume" instead of "Queue paused because you interrupted" while held.
- **Clearing the marker.** `held` is cleared in `begin_turn`.

- [ ] **Step 1: Write the failing tests.**
  - `submit_while_held_queues_without_steer_and_does_not_start`: set the `Hold` global, submit "x", and assert `!busy`, one queued message with `steer == false`, and that it was persisted (`persisted == revision`).
  - `held_busy_turn_sends_hold_decision_not_cancel`: with a fake controls channel, `hold()` sends `Decision::Hold` and the `cancel` flag stays false.
  - `held_result_marks_session_without_stopped_row`: deliver `HoldSettled(Interrupted)` and then a `"Held"` finish. Assert `held == Some(Reason::Interrupted)`, a `HoldChanged` was emitted, and there's no transcript row with the text "Stopped".
  - `resume_from_interrupted_sends_continuation_then_queue`: assert the first launched prompt is `"Continue from where you left off."`, and the queued message follows after `Done`.
  - `resume_from_idle_sends_only_queue`.
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test -p gespan-gpui conversation::`. Expected: FAIL.
- [ ] **Step 3: Implement** as specified.
- [ ] **Step 4: Run again.** Run `cargo test -p gespan-gpui conversation::`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Hold and resume a single session, queueing messages while held"`

### Task 4: Shell coordinator, startup restore, quit guard, keep-awake and scheduler gates

**Files:**
- Create: `crates/gespan-gpui/src/shell/hold.rs` (`mod hold;` in `shell.rs`)
- Modify: `crates/gespan-gpui/src/shell.rs`:
  - add `ToggleHold` to `actions!` (line 87);
  - bind `cmd-shift-h` in `bind_keys`;
  - restore state at startup, after the store is open;
  - subscribe to `HoldChanged`;
  - guard `request_quit` (line 1769).
- Modify: `crates/gespan-gpui/src/shell/settings.rs:326` (`sync_keep_awake`: pass `false` when the phase is `OnHold`)
- Modify: `crates/gespan-gpui/src/shell/automation_runs.rs:120` (`poll_automations`: when held, keep the per-session receipts loop but skip `recover`/`claim_due`)
- Modify: `crates/gespan-gpui/src/shell/orchestration.rs` (dispatch: return early when held, so workers are dispatched on Resume)
- Test: `crates/gespan-gpui/src/shell/hold.rs` tests (gpui `TestAppContext`, following the existing shell tests)

**Interfaces:**
- Consumes: Tasks 1 and 3.
- Produces:
```rust
impl Shell {
    pub(super) fn toggle_hold(&mut self, window: &mut Window, cx: &mut Context<Self>); // None -> confirm_hold; Holding -> no-op; OnHold -> resume_all
    pub(super) fn confirm_hold(&mut self, window: &mut Window, cx: &mut Context<Self>); // window.prompt like confirm_delete_project (shell/project_removal.rs:85); Ok(0) -> hold_all
    pub(super) fn hold_all(&mut self, cx: &mut Context<Self>);     // new HoldState, track every loaded session (busy or with queued messages; pilot_chat = settings.pilot.is_some()), call Conversation::hold, save
    pub(super) fn resume_all(&mut self, cx: &mut Context<Self>);   // only when phase == OnHold; resume_plan stages, max 3 concurrent; clear_hold when the last finishes starting
    pub(super) fn stop_now(&mut self, session: i64, cx: &mut Context<Self>);
    pub(super) fn hold_phase(&self, cx: &App) -> Option<gespan_core::hold::Phase>;
    fn restore_hold(&mut self, cx: &mut Context<Self>);           // store.hold() -> after_restart -> set global, mark sessions (load views lazily on resume)
}
```

**Confirmation copy.** Use `PromptLevel::Warning`.
- **Title:** "Hold everything?"
- **Detail:** "N running sessions will stop once their current command finishes, and nothing new will start. Messages you send will wait until you resume. You can't resume until every session has stopped."
- **Buttons:** "Hold", "Cancel".

The menu-bar item and the app-menu item first bring the main window forward and then show the same prompt.

**Staging.** A resumed session counts as running until it emits `SessionChanged` with `!busy` or its first turn begins and ends. Start the next one when fewer than 3 are running. Stage 2 starts after stage 1 has all started.

**Quit guard.** When the phase is `Holding` and some sessions are still settling, show the existing confirm modal: "Some sessions are still finishing a command. Quit anyway and stop them?" Do the same on menu-bar Quit.

- [ ] **Step 1: Write the failing tests.**
  - `hold_all_moves_to_on_hold_when_all_sessions_settle`: one idle session and one busy session, with `HoldChanged` simulated for both. The phase becomes `OnHold`, and the store has the state.
  - `cancelled_confirmation_changes_nothing`: answer the prompt with Cancel. Then `hold_phase` is `None`, no `Decision::Hold` was sent, and nothing was stored.
  - `toggle_ignored_while_holding`: with one session still settling, `toggle_hold` doesn't resume, doesn't prompt and doesn't change state.
  - `resume_respects_order_and_concurrency_limit`: 5 workers and 1 Pilot chat. The first wave is 3 workers, the Pilot starts last, and no more than 3 are busy at once.
  - `restore_after_restart_keeps_hold_and_marks_restarted`.
  - `automations_not_claimed_while_held`: a due automation is not launched while held, and is launched once after `resume_all`.
  - `keep_awake_released_on_hold`: the expected mode argument is `false` while `OnHold` with a busy Background session.
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test -p gespan-gpui shell::hold`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run again.** Run `cargo test -p gespan-gpui shell::hold`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Coordinate Hold across sessions and keep it across restarts"`

### Task 5: Pilots (habits, timeouts, completion reports)

**Files:**
- Modify: `crates/gespan-gpui/src/pilots/scheduler.rs:1187-1306`:
  - `check_habits`;
  - a new `pub(crate) fn resumed(&mut self, held_ms: u64, since: u64, cx)`.
- Modify: `crates/gespan-gpui/src/pilots/host.rs:~723` (`stopped` closure in the completion check)
- Modify: the Pilot session-view summary struct returned by `pilot_session_views` (add `held: bool`)
- Test: `crates/gespan-gpui/src/pilots/tests.rs`, plus `crates/gespan-core/src/pilot/completions.rs` tests if the predicate moves there

**Interfaces:**
- Consumes: `hold::active(cx)`; `Shell::resume_all` calls `Pilots::resumed(state.held_ms(now), state.since, cx)` after lifting the hold.
- Produces: `Pilots::resumed`, and `held: bool` on the session view summary.

**Behaviour:**
- **While held,** `check_habits`:
  - skips the timeout loop (no time-outs);
  - skips `check_completions`;
  - skips the claim block.
  - Pilot `sessions.start`/`sessions.send` need no change, because the Task 3 launch gate queues their prompts. Do confirm `pilot_external_send` goes through `launch_prompt`, and if it doesn't, route it there.
- **`resumed`:**
  - adds `held_ms` to every `running[*].started` and `waiting_since`;
  - for each enabled habit whose `next_run_at` lies in `[since, now]`, sets `next_run_at = now`, once (`habits::update`), so it runs a single catch-up instead of being skipped by `GRACE_MS`.
- **`stopped(session)`** additionally requires `!v.held`.

- [ ] **Step 1: Write the failing tests.**
  - `held_worker_does_not_report_completion`: the launch has `notify_on_complete`, and the worker view is `!busy`, `has_messages`, `held`. No report is sent.
  - `habit_not_claimed_and_not_timed_out_while_held`: a running habit whose `started` is older than `TIMEOUT_MS` isn't failed while held.
  - `resumed_shifts_habit_clock_by_held_time`.
  - `habit_due_during_hold_runs_once_on_resume`: two missed hourly slots lead to exactly one claim after `resumed`.
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test -p gespan-gpui pilots::tests::held_ pilots::tests::habit_ pilots::tests::resumed_`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run again.** Same command. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Keep Pilots quiet while held and catch up habits on resume"`

### Task 6: Placement (status-bar control, menu bar, palette)

**Files:**
- Modify: `crates/gespan-gpui/src/shell/hold.rs`:
  - `fn hold_control(&mut self, cx) -> AnyElement`, the status-bar control in all three states;
  - `fn hold_details(&mut self, cx) -> AnyElement`, the popover listing settling sessions.
- Modify: `crates/gespan-gpui/src/shell/layout.rs:763` (`footer_row`: add `.child(self.hold_control(cx))` as the last child, after `terminal_footer`, so it sits at the far right). The footer is hidden while a classic surface is open (`layout.rs:347`); Hold stays reachable through ⌘⇧H and the menus.
- Modify: `crates/gespan-gpui/src/quick_open.rs:30` (add `Command::Hold`, labelled "Hold everything", or "Resume everything" while held, with shortcut "⌘⇧H")
- Modify: `crates/gespan-gpui/src/pilots/status.rs:~95` (a "Hold everything" or "Resume everything" item with id `"hold"`, inserted before the last separator, with dispatch next to `"quit"`)
- Modify: the Native app menu (`file_editor.rs:72-105`): add a Gespan Native → "Hold everything ⌘⇧H" item
- Test: `crates/gespan-gpui/src/settings/layout_tests.rs`, or the shell layout tests, using the existing layout-test pattern

**The status-bar control's three states:**
- **Idle:** a "⏸ Hold all" button with the tooltip "Hold everything (⌘⇧H)". Clicking it opens the confirmation (Task 4).
- **Holding:** a non-clickable-to-undo pill with a continuously spinning loader, reading "Holding 2/5…" (settled out of tracked). Clicking it opens `hold_details`. That popover lists each session still settling with its running tool and elapsed time, for example "cargo test · Fix flaky reminders · 2m 10s", and has a **Stop now** link per row. There is no Resume in this state.
- **On hold:** "On hold · N sessions · M queued", with a "▶ Resume" button. Held sessions in the sidebar show a small pause glyph.

- [ ] **Step 1: Write the failing tests.**
  - `footer_shows_hold_control_at_far_right`.
  - `hold_control_idle_opens_confirmation`.
  - `hold_control_holding_shows_progress_and_no_resume`.
  - `hold_details_lists_settling_sessions_with_stop_now`.
  - `hold_control_on_hold_counts_sessions_and_queued_messages`.
  - `palette_lists_hold_command_and_toggles`.
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test -p gespan-gpui hold`. Expected: FAIL.
- [ ] **Step 3: Implement.** Use the existing icon set (check `assets.rs` for pause/play glyphs, and reuse the live-agents chip styling).
- [ ] **Step 4: Run again.** Run `cargo test -p gespan-gpui`. Expected: PASS, and the existing layout tests still pass.
- [ ] **Step 5: Commit.** `git commit -m "Add the Hold button, banner, menu-bar item and shortcut"`

### Task 7: End-to-end verification and PR

- [ ] **Step 1: Run the full test suites.** Run `cargo test -p gespan-core && cargo test -p gespan-gpui`. Expected: all pass.
- [ ] **Step 2: Build the dev app and launch it.** Run `bun run native:build`, or the dev run command from `package.json`.
- [ ] **Step 3: Check Hold on a long command.** In a Claude Code session, ask it to run `sleep 60 && echo done`. Press ⌘⇧H while it runs.
  - Expected: the confirmation appears; choose Hold.
  - The status bar shows "Holding 0/1…" with the spinner, and the details list "sleep 60 …" with no Resume offered.
  - After the command ends, the session shows Held, `done` is in the transcript, and the agent makes no further reply.
- [ ] **Step 4: Check the other states.** A streaming session holds within a second. A Codex session waiting on approval keeps its approval card. Typing in a held session queues the message with the label "Held: sends on Resume".
- [ ] **Step 5: Check restart.** Quit (⌘Q) while On hold, then relaunch. Expected: still On hold, with the same queued messages and nothing running.
- [ ] **Step 6: Check Resume.** Press Resume. Expected: sessions continue in order (workers, then the Delivery Pilot), queued messages are sent, and the Pilot gets no false completion.
- [ ] **Step 7: Open the PR.** Push `feat/session-handoff` and open a PR that closes #173 with a test summary. No attribution lines.

---

# Part 2: the desktop app (Tauri + React), same PR

The behaviour, copy, placement, confirmation and "no undo while Holding" rule are identical to Part 1. In the desktop app, turns run in TypeScript adapters (`src/integrations/harness/providers/<id>/`, interface `HarnessAdapter` in `src/integrations/harness/core/registry.ts:53`); Rust (`src-tauri/src/harness.rs`) only spawns and kills processes. The Hold logic therefore lives in TypeScript, with Vitest tests next to the code (`src/**/*.test.ts`), run with `bunx vitest run <file>`.

**Desktop constraints:**
- **Bun only:** `bun run`, `bunx vitest run`, `bunx tsc --noEmit`.
- **Persistence:** the hold state is saved under the localStorage key `gespan:hold` (JSON), next to the other `gespan:` keys, behind try/catch like the existing preference helpers.
- **Native commands:** any new Tauri command must be registered in `src-tauri/src/lib.rs` `generate_handler!`, because `src/test/invokeCommands.test.ts` checks this.
- **Restarts:** the auto-continue on launch (`canAutoContinue`, `src/features/sessions/model/inFlight.ts:116`, and the effect at `App.tsx:10616`) must not fire while held.

### Task 8: Desktop hold model and gate (`src/features/hold/model/`)

**Files:**
- Create: `src/features/hold/model/hold.ts` (+ `hold.test.ts`)
- Create: `src/features/hold/model/holdGate.ts` (+ `holdGate.test.ts`)

**Interfaces (Produces):**
```ts
export type HoldPhase = "holding" | "onHold";
export type HoldReason = "idle" | "interrupted" | "awaitingInput" | "background" | "restarted";
export type HeldSession = { sessionId: string; reason: HoldReason | null; heldAt: number; lastTool?: string; background: string[]; pilotChat: boolean };
export type HoldState = { phase: HoldPhase; since: number; sessions: HeldSession[] };
export function newHold(now: number): HoldState;
export function track(state: HoldState, sessionId: string, pilotChat: boolean): HoldState;
export function settle(state: HoldState, sessionId: string, reason: HoldReason, now: number, lastTool?: string, background?: string[]): HoldState; // also flips phase to onHold when all settled
export function afterRestart(state: HoldState): HoldState;
export function continuation(reason: HoldReason): string | null; // CONTINUE_PROMPT for interrupted|restarted
export function resumePlan(state: HoldState, exists: (id: string) => boolean): string[][];
export function loadHold(): HoldState | null; export function saveHold(state: HoldState | null): void; // localStorage "gespan:hold"
// holdGate.ts — same rule as Native provider/hold.rs
export type HoldAction = { kind: "interrupt" } | { kind: "settle"; point: "interrupted" | "awaitingInput" | "background" };
export class HoldGate { request(): void; observe(event: HarnessEvent): void; decide(awaitingInput: boolean): HoldAction | null; }
```
The gate observes these `HarnessEvent`s:
- `tool.started`: running; it counts as foreground unless its id is in the background set.
- `tool.updated` with `completed` or `failed`: not running.
- `background.updated`: sets the background set; a non-empty list means the agent has yielded.
- `message.delta` and `reasoning.delta`: the agent woke up.

- [ ] **Step 1: Write the failing tests.** Mirror Task 1 and Task 2's gate tests one for one: settled to onHold; dedupe; afterRestart; continuation; resumePlan with Pilot chats last and unknown ids skipped; storage round-trip and clear (`spyOnStorage` from `src/test/`); and the gate's six cases.
- [ ] **Step 2: Run them and confirm they fail.** Run `bunx vitest run src/features/hold`. Expected: FAIL (module not found).
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run again.** Same command. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Add the desktop Hold model and safe-point gate"`

### Task 9: Soft interrupt in every desktop adapter

**Files:**
- Modify: `src/integrations/harness/core/registry.ts` (add `holdTurn?(sessionId: string): Promise<boolean>` to `HarnessAdapter`, and the export `holdHarnessTurn(harness, sessionId)` next to `cancelHarnessTurn`, line ~382)
- Modify: the providers:
  - `claude/claude.ts`: send only `{subtype:"interrupt"}`; no deny, no `stop_task`;
  - `codex/codex.ts`: send only `turn/interrupt`, without `clearServerRequests`;
  - `cursor/cursor.ts`, plus the ACP siblings grok, kiro, hermes, devin, fx and antigravity: `session/cancel`;
  - `pi/piFamily.ts`: `abort`;
  - `opencode/opencode.ts`: interrupt or abort.
- Test: each adapter's existing `*.test.ts`.

`holdTurn` returns whether an interrupt was sent. It never calls `stopSession`, never kills the child, and never resolves approvals.

- [ ] **Step 1: Write the failing tests.** Per adapter, add `hold_turn_sends_only_the_interrupt`, using that adapter's existing fake-child test harness. The written frames contain the interrupt and contain no deny or decline, no `stop_task`, and no kill call.
- [ ] **Step 2: Run them and confirm they fail.** Run `bunx vitest run src/integrations/harness`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run again.** Same command. Expected: PASS, and the existing tests are still green.
- [ ] **Step 5: Commit.** `git commit -m "Interrupt a desktop turn softly for Hold"`

### Task 10: Hold coordinator hook and App wiring

**Files:**
- Create: `src/features/hold/model/useHold.ts` (+ `useHold.test.ts`). This is the coordinator:
  - `hold` state;
  - `requestHold()`, which opens the confirmation;
  - `confirmHold()`;
  - `observe(sessionId, event)`;
  - `turnSettled(sessionId, held)`;
  - `resume()`, staged with at most 3 at once;
  - `isHeld()`.
- Modify: `src/app/App.tsx`. Keep the wiring thin and call into the hook:
  - `applyTurnEvent` (around line 8247): call `hold.observe`. When `decide` returns interrupt, call `holdHarnessTurn` without bumping `turnGen`. When it returns settle, record the reason.
  - `submitSession` (line 7253): while held, queue the message (`queueStatus: "paused"`, never steer).
  - Automations interval (around line 9306): skip `claimDueAutomations` while held.
  - `usePilotHabits(host, !held)` (line 8810).
  - Orchestrator `pump()`: return early while held, through the host binding at line 10940.
  - The `handleAgentApp` `sessions.start` and `sessions.send` paths: queue instead of submitting.
  - Mobile `session.start` and `conversation.send`: queue.
  - Auto-continue on launch: suppressed while held.
  - `hasWorkingAgent` (`src/features/settings/model/keepAwake.ts:66`): returns false while on hold.
  - Pilot completion: a worker that is held doesn't report "cancelled", and it's watched again when it resumes (`pilotSessionCompletion.ts:134`).
- Test: `useHold.test.ts`, which drives the hook with fake sessions and a fake adapter:
  - `confirm_then_settle_reaches_on_hold`;
  - `cancelled_confirmation_changes_nothing`;
  - `toggle_ignored_while_holding`;
  - `queued_while_held_is_not_steered`;
  - `restart_restores_hold_and_suppresses_auto_continue`;
  - `resume_order_and_limit`;
  - `automations_and_habits_gated`;
  - `held_worker_does_not_report_completion`.

- [ ] **Step 1: Write the failing tests.** Steps 2–5 follow the same pattern as the other tasks. Commit with `git commit -m "Coordinate Hold in the desktop app"`.

### Task 11: Desktop placement (status bar, confirmation, menu, shortcut, palette)

**Files:**
- Create: `src/features/hold/ui/HoldControl.tsx` (+ test), which renders the three states with the copy from Task 6.
- Create: `src/features/hold/ui/HoldDetails.tsx`, a popover listing the settling sessions with **Stop now**.
- Create: `src/features/hold/ui/ConfirmHold.tsx`, built on `Modal` with the same copy as Task 4.
- Modify: `src/app/shell/UsageFooter.tsx:157` (the right-side `ml-auto` slot: render `HoldControl` last).
- Modify: `src/features/settings/model/appShortcuts.ts:13` (`hold`: `Mod+Shift+H`); `App.tsx` keydown (around line 13258) calls `requestHold` or `resume`.
- Modify: `src/features/files/ui/FilePicker.tsx:40` `ACTIONS` (`hold`, labelled "Hold everything" or "Resume everything"), plus `onRunAction` in App.
- Modify: `src-tauri/src/menu.rs` (a "Hold Everything" item with `CmdOrCtrl+Shift+H` that emits `hold_toggle`) and `src-tauri/src/pilot_chat/menu_bar.rs` `ACTIONS` (the menu-bar item); App listens for `hold_toggle`.
- Test: `HoldControl.test.ts` (happy-dom):
  - `idle_shows_hold_all`;
  - `holding_shows_progress_and_no_resume`;
  - `on_hold_shows_counts_and_resume`.
  
  `ConfirmHold.test.ts`: the copy and Cancel. `menu.rs` test: the item exists with its accelerator.

- [ ] **Steps 1–5:** follow the same pattern. Commit with `git commit -m "Add the desktop Hold control, confirmation, menu and shortcut"`.

### Task 12: Desktop verification

- [ ] **Step 1: Run the checks.** Run `bun run test`, `bunx tsc --noEmit`, and `cargo test -p gespan-desktop`, or the src-tauri crate's name. Expected: all pass.
- [ ] **Step 2: Repeat Task 7's manual checks.** Run `bun run tauri dev` and go through steps 3–6 of Task 7 in the desktop app.
