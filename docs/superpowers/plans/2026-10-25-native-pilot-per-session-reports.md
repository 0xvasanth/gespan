# Native Pilot: Per-Session Completion Reports and Mandatory Pilot Folders — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In the native app, a Pilot hears about each delegated session as soon as that session stops, never waiting for its siblings, and every session a Pilot starts is always visible and filed in that Pilot's folder.

**Architecture:** The readiness rule moves out of `Pilots::check_completions` into a pure function in `gespan-core` (`pilot::completions`), which groups *ready* launches by the chat that launched them instead of waiting for whole `origin_turn` groups. The GPUI side keeps its triggers (Pilot chat idle, 10 s tick) and delivery paths (main outbox or habit mailbox). Folder filing loses its per-Pilot opt-in.

**Tech Stack:** Rust, GPUI (`crates/gespan-gpui`), `crates/gespan-core`, cargo with the `native-dev` profile.

**Spec:** `docs/superpowers/specs/2026-10-25-pilot-flows-design.md`: sections "Plain sessions, without a Flow (both apps)", "Step sessions in the sidebar", outline items 10–11 (native half only). Tauri changes are out of scope for this plan.

## Global Constraints

- **Do not commit.** The user approves every commit. Tasks end by staging (`git add`) only. Work on a branch `feat/native-pilot-per-session-reports` created from `main`; never on `main`.
- Leave the user's unrelated working-tree change (`README.md`) untouched and unstaged.
- Rust tests: `cargo test --locked --manifest-path crates/gespan-gpui/Cargo.toml -p <gespan-core|gespan-gpui> --profile native-dev -- --test-threads=1 <filter>`.
- Lint gate before handing back: `cargo clippy --manifest-path crates/gespan-gpui/Cargo.toml --locked -p gespan-core -p gespan-gpui --all-targets --profile native-dev -- -D warnings`.
- Full native suite must pass at the end: `bun run native:test`.
- Existing guarantees stay: no duplicate reports, drafts and rejected launches never report, a session the Pilot stops, archives or deletes doesn't report (`dismissed`), the chat is never steered mid-turn (internal messages wait while busy).
- Naming: the product is Gespan; use no earlier product names in code, copy or messages (enforced by `src/test/legacyNames.test.ts`).
- Match the surrounding code style (comment density, naming); tests must exercise real behaviour, no filler.

## Review Focus

1. **A launch re-armed while its report is being built** (`sessions.send` with `notifyOnComplete` after the session stopped but before the background report finished) must not be marked completed by the stale report. *(Task 2 test.)*
2. **A session that stops while the launching turn is still running** is held until that chat is idle, then delivered. It is not lost, and not delivered mid-turn. *(Task 1 test.)*
3. **A removed session** (deleted outside the Pilot, so there's no view) still reports as "Session was removed" once, the same as today. *(Task 1 test: missing view counts as stopped.)*
4. **Habit runs**: a habit's delegated children report to the habit's mailbox one by one, and the habit run still stays alive until every child with `notify_on_complete` has reported. *(Task 2 test.)*
5. **Renaming a Pilot** must not split its sessions across two folders. *(Task 3 test.)*

---

### Task 1: Pure readiness rule in core

**Files:**
- Create: `crates/gespan-core/src/pilot/completions.rs`
- Modify: `crates/gespan-core/src/pilot.rs` (add `pub mod completions;` next to the other `pilot` submodules)

**Interfaces:**
- Consumes: `pilot::Launch` (`pilot.rs:150-164`): `session_id: i64`, `origin_turn: u64`, `notify_on_complete`, `completed`, `dismissed`.
- Produces:
  ```rust
  /// Launches whose sessions have stopped, bundled by the chat that launched them.
  pub struct Ready { pub source: i64, pub launches: Vec<(i64, u64)> } // (session_id, origin_turn)
  pub fn source_of(origin_turn: u64) -> i64; // (origin_turn >> 32) as i64
  pub fn ready(
      launches: &[Launch],
      stopped: impl Fn(i64) -> bool,      // session id → stopped (missing view counts as stopped)
      source_busy: impl Fn(i64) -> bool,  // launching chat session id → busy
      in_flight: impl Fn(i64) -> bool,    // session id already being reported
  ) -> Vec<Ready>;
  pub fn report_message(entries: &[String]) -> String;
  ```
  - A launch qualifies if `notify_on_complete && !completed && !dismissed && stopped(id) && !in_flight(id)`, and `!source_busy(source_of(origin_turn))`.
  - Results are grouped by source, in the launches' order. A source with no qualifying launch is omitted.
  - **A qualifying launch is never held back because a sibling (same `origin_turn`) is still running.**
- `report_message` copy, exactly:
  - one entry: `"[Delegated work completed]\nA session you delegated has stopped. Inspect its result and continue that task now; other delegated sessions may still be running and will report separately.\n\n{entry}"`
  - n > 1 entries: `"[Delegated work completed]\n{n} sessions you delegated have stopped. Inspect each result and continue each task now; other delegated sessions may still be running and will report separately.\n\n{entries joined by \"\\n\\n\"}"`

- [ ] **Step 1: Write the failing tests** in `completions.rs` (`#[cfg(test)] mod tests`), with a small `launch(session, source, turn)` helper building a `Launch` with `notify_on_complete: true`:
  - `a_stopped_session_reports_while_its_sibling_still_runs`: two launches with the same `origin_turn`, only the first stopped → `ready(..)` returns one `Ready { source, launches: [(first, turn)] }`.
  - `reports_wait_for_the_launching_chat_to_be_idle`: stopped launch, `source_busy` true → empty. Same input with `source_busy` false → one entry.
  - `stopped_sessions_from_one_chat_bundle_and_other_chats_stay_separate`: two stopped launches from source A (different turns) and one from source B → two `Ready`, A with both launches in order, B with one.
  - `completed_dismissed_unmonitored_and_in_flight_launches_never_report`: one launch each with `completed`, `dismissed`, `notify_on_complete: false`, and `in_flight` true, all stopped → empty.
  - `report_message_names_one_or_many_sessions`: assert both exact strings above for `["Session 1: A\nok"]` and for two entries (prefix `"[Delegated work completed]\n2 sessions you delegated have stopped."`).
- [ ] **Step 2: Run** `… -p gespan-core --profile native-dev -- --test-threads=1 pilot::completions`. Expected: FAIL (module/functions missing).
- [ ] **Step 3: Implement** `source_of`, `ready`, `report_message` with the signatures above.
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Stage** `git add crates/gespan-core/src/pilot/completions.rs crates/gespan-core/src/pilot.rs` (no commit).

### Task 2: Per-session delivery in `Pilots::check_completions`

**Files:**
- Modify: `crates/gespan-gpui/src/pilots/host.rs:364-447` (`check_completions`)
- Modify: `crates/gespan-gpui/src/pilots/mod.rs:129,355` (`completion_work` type)
- Modify: `crates/gespan-core/src/pilot.rs:321` (prompt sentence)
- Test: `crates/gespan-gpui/src/pilots/tests.rs`

**Interfaces:**
- Consumes: `completions::{ready, Ready, report_message, source_of}` from Task 1.
- Produces: `completion_work: HashSet<(String, i64)>` keyed by `(pilot_id, session_id)`, replacing `(pilot_id, origin_turn)`.

- [ ] **Step 1: Write the failing GPUI tests** in `pilots/tests.rs` using `setup(cx)`. Start children with `call(&s, "r1", "sessions.start", json!({... "draft": true}), cx)`, then set `notify_on_complete = true` on their `roster` launches and drive their state through `owner.pilot_session_entity(child)` (`busy`, `summary.has_messages`), then call `m.check_completions(cx)` and `run_until_parked`:
  - `a_finished_child_reports_while_its_sibling_keeps_running`:
    - Setup: two children from one turn; child A stopped with messages, child B busy.
    - Assert: the Pilot's `outbox` gains exactly one internal message, which starts with `"[Delegated work completed]\nA session you delegated has stopped."` and contains `"Session {A}:"` but not `"Session {B}:"`. A's launch is `completed`; B's is not.
    - Then: stop B and run `check_completions` again. Assert: a second message mentioning only B.
  - `a_rearmed_launch_is_not_completed_by_a_stale_report`: A stopped; call `check_completions`; before `run_until_parked`, overwrite A's launch `origin_turn` to a new value with `completed=false` (as `sessions.send` re-arming does). Park. Assert: A's launch is still `completed == false`.
  - `habit_children_report_to_the_habit_mailbox_one_session_at_a_time`: insert a `RunningHabit` (or start one through the existing habit test helpers if present) whose session is the children's source. Two children; stop one → that run's `mailbox` holds one message for it, and `habit_changed` keeps the run alive while the other child is unreported.
- [ ] **Step 2: Run** `… -p gespan-gpui --profile native-dev -- --test-threads=1 pilots::tests::a_finished_child pilots::tests::a_rearmed pilots::tests::habit_children`. Expected: FAIL (today's code waits for the group).
- [ ] **Step 3: Rewrite `check_completions`:**
  - Build `stopped` from `pilot_session_views` (missing view or `!busy && has_messages`), `source_busy` from the existing chats/running busy check, and `in_flight` from `completion_work`. Call `completions::ready`.
  - For each `Ready`, insert every `(pilot_id, session_id)` into `completion_work`. Build the per-session entries off-thread exactly as today (`"Session {id}: {title}\n{limit(prose, 12000)}"`, or `"Session was removed"`).
  - On return, remove those keys. Mark `completed = true` **only on launches whose `session_id` and `origin_turn` both match the reported pair**. Deliver `report_message(&entries)` through the existing branch: habit `mailbox` if `running` has the source, else `enqueue(.., internal = true)` + `ensure_chat`.
- [ ] **Step 4: Replace** the prompt sentence at `pilot.rs:321`, `"You will receive one completion report after every session in that turn's group stops; then inspect and integrate the results."`, with exactly: `"Each delegated session reports back as soon as it stops; reports that arrive while you are busy come together in one message. Act on each result as it arrives, continuing that task without waiting for unrelated sessions, and give the user an integrated update when related work is done."`
- [ ] **Step 5: Run** the Step 2 command, then `… -p gespan-core … pilot::` and `… -p gespan-gpui … pilots::`. Expected: PASS, no regressions.
- [ ] **Step 6: Stage** the touched files (no commit).

### Task 3: Every Pilot-started session is visible and filed in the Pilot's folder

**Files:**
- Modify: `crates/gespan-gpui/src/pilots/host.rs:321-362` (`group_launch`), `:250-256` (`sessions.send` arm)
- Modify: `crates/gespan-gpui/src/pilots/mod.rs:239-259` (`sidebar_preferences`), `:406-420` (`hidden_session`)
- Modify: `crates/gespan-gpui/src/pilots/panels.rs:1681-1728` (remove the two switches), `crates/gespan-gpui/src/settings/pages/pilots.rs:52-71` (remove the show switch), `crates/gespan-gpui/src/settings/mod.rs:101` if `PilotSetting.folders` becomes unused
- Test: `crates/gespan-gpui/src/pilots/tests.rs`; update `crates/gespan-gpui/src/settings/layout_tests.rs:175-179` and any test asserting the removed switches

**Interfaces:**
- Consumes: `session_groups::Groups::{folder, create_folder, rename_folder, set_color, add_to_folder, folder_of}`.
- Produces: `fn pilot_folder_id(pilot_id: &str) -> String`, returning `format!("pilot-{pilot_id}")`, in `pilots/host.rs`.
- `Pilot.show_started_sessions_in_sidebar` / `use_sidebar_folders` stay in the core struct for serde compatibility, but nothing reads them anymore (keep `#[serde(default)]`).

- [ ] **Step 1: Write the failing tests:**
  - `every_started_session_is_visible_and_in_the_pilots_folder`: with both legacy flags `false` (the default), start a child (draft). Assert:
    - `!m.hidden_session(child)`;
    - `owner.pilot_groups(&project).folder_of(child)` has id `pilot-{id}` and name `pilot.label()`.
  - `renaming_the_pilot_keeps_one_folder`: start a child; rename the Pilot; start a second child. Assert both are in folder `pilot-{id}`, the folder name is the new label, and there is exactly one folder containing either.
  - `a_follow_up_files_a_loose_session_but_leaves_a_foldered_one_alone`:
    - Case 1: `sessions.send` with `notifyOnComplete:true` to a session in no folder → it is moved to `pilot-{id}`.
    - Case 2: a session already in a user folder → it stays in that folder.
- [ ] **Step 2: Run** `… -p gespan-gpui … pilots::tests::every_started pilots::tests::renaming_the_pilot pilots::tests::a_follow_up_files`. Expected: FAIL.
- [ ] **Step 3: Implement:**
  - `group_launch` drops the `show_started_sessions_in_sidebar && use_sidebar_folders` gate.
  - It finds the folder by `pilot_folder_id`, falling back once to the legacy match by name (`f.name == pilot.label()`). If found that way, it migrates by creating `pilot-{id}` with the same sessions and color and dissolving the old one. Otherwise `create_folder(&[session], pilot_folder_id(id))`.
  - It always renames the folder to the current label.
  - The `sessions.send` notify arm calls `group_launch` only when `groups.folder_of(session).is_none()`.
  - `hidden_session` no longer hides Pilot launches. Habit-run sessions in `roster.hidden_sessions` stay hidden.
  - `sidebar_preferences` re-files every launch unconditionally.
  - Remove the "Show sessions in sidebar" / "Group in a folder" switches and their handlers, and the settings-page switch.
- [ ] **Step 4: Run** the Step 2 command, plus `… -p gespan-gpui … settings::` and `… pilots::`. Expected: PASS (layout tests updated to the removed switches).
- [ ] **Step 5: Run the gates:** clippy command and `bun run native:test`. Expected: clean, all pass.
- [ ] **Step 6: Stage** the touched files (no commit). Report to the user for commit approval.
