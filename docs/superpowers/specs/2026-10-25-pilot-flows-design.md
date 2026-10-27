# Pilot Flows: independent, step-by-step task orchestration — Design

Status: draft for review (brainstormed with the user on 2026-10-25).
Calls marked **(builder call)** or **(my call)** were not decided by the user.

## Why

When a Pilot hands out work to several sessions at once, it cannot act on any
result until every session from that turn has stopped. A task that finishes in
2 minutes waits for one that takes 20, and every later stage waits again for the
slowest member.

This is deliberate in both apps today:

- **Tauri/TS.** `PilotSessionCompletionBatches`
  (`src/features/pilots/model/pilotSessionCompletion.ts:133`) groups sessions by
  `pilotId:turn`. `release()` (line 219) returns early until the launching turn
  is closed **and** every member has a result. The Pilot prompt
  (`src/features/pilots/model/pilotFiles.ts:348-349`) tells it to wait and give
  one consolidated report. Tests in `pilotCompletionBatches.test.ts` lock this in.
- **Native/GPUI.** `crates/gespan-gpui/src/pilots/host.rs:370-441` groups launches
  by `origin_turn` and only reports when `children.iter().all(..)` have stopped
  ("All sessions in this group have stopped"). The prompt in
  `crates/gespan-core/src/pilot.rs:321` says the same.

The work the user actually gives a Pilot isn't shaped that way. The Pilot splits
one goal into many tasks (for example 20). Each task gets a **different** request
serving the same goal, and the tasks are almost always independent. Each task
has its own sequence of steps, and those steps take very different amounts of
time from task to task. Waiting for the group throws all of that away.

## The idea

The Pilot is the orchestrator. For a goal it creates a **blueprint**: a Flow
that describes the steps a task goes through. For example:

> study the code → implement → review → test → open a PR (or reach the task's end state)

Each task runs as its **own run** of a blueprint. The Flow engine moves each run
forward on its own. The Pilot doesn't drive each step. It **supervises**: it
watches every step finish, steps in when something goes wrong, and keeps
improving its blueprints over time from what it sees and what the user tells it.

Flows become the blueprint layer that lets Pilots work at different scales.

## Goals

1. **Every task moves on its own.** When a task's step finishes, its next step
   starts right away, whatever the other tasks are doing. No step of any task
   waits on another task.
2. **One role per step, one session per step.** Each step usually runs in a
   fresh session focused on one job (study, implement, review, test). A reviewer
   should not be the session that wrote the code. Steps of one task share that
   task's worktree, so later steps see earlier changes.
3. **Pilot-owned Flows.** A Pilot can create, run and change its own Flows.
4. **The Flow moves itself; the Pilot supervises.** (See Behaviour.)
5. **Blueprints mature.** Through use and user feedback, a Pilot's blueprints get
   cleaner and more reliable, and the Pilot records what it learned in memory so
   nothing is lost between conversations.
6. **Per-task tracking.** For every task it is possible to know which step it is
   on, what each step produced, which session ran it, and where it stopped.

## Non-goals (this ticket)

- **Sub-flows.** A step starting child Flows, with or without waiting for them.
  Deferred, to be designed separately.
- **A procedures (SOP) view.** See Future ideas. Not built now.
- **Showing Pilot Flows to the user.** Hidden completely for now: no list entry,
  no setting, no read-only view.
- **Pilot Flows in the Tauri app.** A separate follow-up ticket, picked up once
  Pilot Flows are mature. (Per-session reports and Pilot folders *are* in
  scope for Tauri; see Constraints.)
- **New Flow state kinds** such as parallel branches or join/"wait for all" states.
  Independence comes from one run per task, not from branching inside a run.

## Anti-goals (would be a failure even if it works)

- Any task's step waits on another task's step.
- The Flow pauses after a successful step until the Pilot replies.
- The Pilot's main chat with the user floods with per-step reports, or gets
  blocked by them.
- A Pilot changes a running session mid-turn, or changes a blueprint shared by
  other runs when it meant to change only one task.
- Two Pilots end up sharing or editing the same Flow.
- What the user told the Pilot about how work should run is forgotten next time.
- Pilot Flows show up in the user's Flows list.

## Behaviour (decided with the user)

### Two kinds of Flow

- **User Flows:** what exists today. Created and edited by people in the Flows
  view. Unchanged.
- **Pilot Flows:** created by a Pilot and tagged with that Pilot as owner.
  - Exactly **one owner**. No two Pilots ever share a Pilot Flow. A Pilot wanting
    something similar makes its own.
  - **Hidden** from the Flows view and any user-facing list.
  - Changed only by the owning Pilot. A user who wants a change asks that Pilot.

### Running tasks

- The Pilot starts one run per task. It can use one blueprint for many tasks or
  different blueprints for different tasks. Each run gets its own inputs (the
  task's request).
- Each agent step launches its own session (already how the engine works).

### Step sessions in the sidebar

- Every step session stays a normal session, visible in the sidebar, so the
  user can watch or step into any task. 20 tasks × 5 steps means 100 sessions;
  that is fine.
- **Grouping is mandatory, not optional.** Every session a Pilot triggers,
  directly or through a Pilot Flow, is always filed into a folder that says who
  triggered it. There is no way to turn this off for Pilot-triggered sessions.
- Folder shape **(my call, builder may refine)**: one folder per Pilot (the
  native app already files Pilot launches into the Pilot's folder,
  `crates/gespan-gpui/src/pilots/host.rs` `group_launch`). If folders can nest,
  add a subfolder per goal or task; if not, the session title carries the task
  name and step (`"<task> · <step>"`). The Tauri app needs the same automatic
  filing; today its folders are only placed by hand.

### Asking the user, not approval steps

- Pilot Flows run **autonomously**. They contain no `Approval` states; validation
  rejects one in a Pilot Flow. (The Flows view, the only place approvals can be
  given today, never shows Pilot Flows.)
- The Pilot goes back to the user, in its chat, only when it genuinely needs to:
  it needs clarification, what it is about to do conflicts with its memory or
  earlier user input, or the decision would affect or block the user. Otherwise
  it carries on by itself.

### When a step finishes

- **Success:** the Flow chooses the next step by its own transition rules and
  launches it **immediately**. The Pilot is woken *in parallel* to review that
  step. Nothing waits for the review.
- **Failure:** that task's run stops (as today: no matching transition, or the
  failed path). Only that task. The Pilot is woken and takes over that task: it
  retries, adjusts that task's plan, or asks the user. Other tasks keep running.
- **Why wake the Pilot on success:** to observe and learn. Most of the time it
  changes nothing. Sometimes it spots something worth doing, such as
  "the review step keeps finding missing tests; put that in the implement brief."
  It then adjusts that task's remaining steps, or notes the improvement for the
  blueprint and raises it with the user.
- **Changes apply to steps not yet started (my call).** If the Pilot's review
  lands after the Flow has moved on, the change takes effect from the next
  not-yet-started step. A running session is never interrupted or rewritten.
  So a finding from step N's review often reaches step N+2 or later; that is
  accepted (user: "Flow doesn't stop for pilot responses"). There is **no**
  "Pilot must review before continuing" step kind.
- **Failures go to the main Pilot (my call).** Side reviews only observe and
  suggest; they never retry, resume or edit runs. A failed task is handed to
  the main Pilot, which retries or adjusts it on its own and only asks the user
  when it can't recover. Several failures arriving together come as one
  message, so a burst doesn't flood the chat.

### Changing Flows

- **One task only:** the Pilot changes that run's own copy of the blueprint
  (each run already snapshots its Flow at start). Other runs are untouched.
- **The blueprint itself:** the Pilot proposes the change and confirms it with
  the user before applying it (user said "it can confirm … with the user").
  Applying it affects runs started afterwards. Whether running tasks pick it up
  is a builder call; default: no.

### Keeping the main chat clear: side reviews

- Each step review happens in a **separate, focused Pilot sub-thread**, not in
  the Pilot's main chat. A sub-thread sees only that task's step result and the
  context it needs, so many can run side by side without mixing context or
  blocking the user's conversation.
- Each sub-thread ends with a short note: *no change*, *adjusted next step*,
  *task needs attention*, or *blueprint idea*.
- The main Pilot collects these notes (my guess, offered to the user and not
  objected to) and uses them for the final consolidated report on the goal and
  for the blueprint changes it proposes.
- **Memory has one writer (my guess, as above).** Sub-threads suggest memory
  updates; the main Pilot writes them. This avoids conflicting entries.

### Memory

- Every input the user gives the Pilot about how work should run is recorded in
  the Pilot's memory, along with every blueprint change and why it was made.
- Use the existing Pilot memory (dated entries and topic notes,
  `src/features/pilots/model/pilotMemory.ts`, `crates/gespan-core/src/pilot/memory.rs`)
  instead of inventing a new store. A topic per blueprint is a sensible default
  **(builder call)**.

### Plain sessions, without a Flow (both apps)

The "wait for the whole group" behaviour goes away everywhere, not only for
Flows. In **both** the Tauri and native apps:

- Each session the Pilot starts or follows up with `notifyOnComplete` reports
  back **as soon as it stops** (finished, failed or cancelled), on its own.
- If several reports arrive while the Pilot is busy (in a turn with the user,
  say), they are delivered together as one message on its next free turn. That
  only bundles what has already arrived; nothing is held back waiting for other
  sessions.
- The Pilot acts on each result as it comes (starts follow-up work, fixes a
  failure) and, when it has a set of related results, gives the user an
  integrated update. The prompt rules say this instead of "you'll get one
  report after every session in the group stops".
- Existing guarantees stay: no duplicate reports, rejected launches never
  report, a session the Pilot stops or archives itself doesn't report, and the
  chat is never steered mid-turn.

### When the goal is done

When every task of a goal has reached an end state (finished or failed and not
recovered), the main Pilot gives the user **one** consolidated update, built from
the sub-thread notes. This is the only point where results are gathered, and it
never holds back any task's progress.

## What exists today (facts, checked 2026-10-25 on `main` d6d62658)

**Flow engine:** `crates/gespan-core/src/flow/`, hosted only by the native app
(`crates/gespan-gpui/src/flows/`, `shell/flow_runs.rs`).

- A Flow is a state machine of `Agent` and `Approval` states with guarded
  transitions on typed outputs (`definition.rs`). A run is in exactly one state
  at a time. There are no parallel branches, no sub-flows, and no join state.
- `Runner` is step-driven and returns `Effect`s (`Launch`, `FollowUp`, `Cancel`,
  `Persist`, `NeedsApproval`, `Finished`) for the host to apply (`runner.rs`).
- **Many runs of one Flow can be active at once.** Runs are keyed by run id; an
  active run blocks only *deleting* the Flow (`flow_runs.rs:224-232`,
  `flows/view.rs:500-545`).
- **Each run snapshots its Flow** at start (`FlowRun.flow`, `run.rs`). There is no
  operation to edit a running run's snapshot.
- **One worktree per run**, created by the first agent visit and reused by later
  visits.
- `Flow` has no owner/tag field. Every Flow is listed in the Flows view.

**Pilots:** exist in both apps.

- Tauri/TS: completion batching above; Pilot CLI actions include `sessions.*`,
  `memory.*`, `habits.*`, `artifacts.*`, `notes.*`. No `flows.*` actions.
- Native: `crates/gespan-core/src/pilot*.rs`, `crates/gespan-gpui/src/pilots/`. Has
  memory, soul, habits (heartbeat), and the grouped completion report above. No
  `flows.*` actions.

## What has to be built (outline, not a plan)

1. **Ownership on Flows.** An owner tag (`pilot:<id>` or a dedicated field) on
   Flow definitions, stored in the shared DB. Filtered out of the Flows view.
   Only the owning Pilot can read, change or run its Flows. Shared-DB contract
   tests must cover the new column/field.
2. **Pilot actions for Flows.** CLI actions for the Pilot to create, validate,
   update, start (one run per task, with inputs), inspect and cancel its Flows
   and runs. Each must reject Flows the calling Pilot doesn't own.
3. **Edit a run's own copy.** A runner operation that changes the snapshot of an
   active run for states not yet entered, validated like any Flow, and recorded
   in the run's history.
4. **Step-finished notifications.** On every visit that ends (completed, failed,
   cancelled), notify the owning Pilot with run, task, state,
   outputs, session id and status. On success this never delays the next `Launch`.
5. **Side reviews.** A way for the Pilot to handle each notification in a
   focused sub-thread, outside the main chat, that returns a short structured
   note to the main Pilot. Design is a **builder call**. Reusing the habit/run
   mechanism ("hidden run") is a likely starting point.
6. **Failure handoff.** When a run fails, the Pilot gets the task with enough
   context to retry, adjust and resume (or restart) that task, or ask the user.
7. **Goal-level roll-up.** Track which runs belong to one goal so the Pilot knows
   when all of them have ended and can give the single consolidated update.
8. **Per-task tracking data.** Run and visit records already carry state, outputs,
   session and timestamps. Add the task/goal link and the review notes so a
   task's full history can be reconstructed.
9. **Prompt and rule updates.** Teach the Pilot to use Flows for multi-step,
   multi-task work, to supervise rather than drive, to confirm blueprint changes
   with the user, to ask the user only in the cases above, and to record
   learnings in memory.
10. **Per-session completion reports, both apps.** Replace group release with
    per-session release:
    - Tauri: `PilotSessionCompletionBatches` (`pilotSessionCompletion.ts`) and
      its wiring in `src/app/App.tsx`; rewrite `pilotCompletionBatches.test.ts`
      to prove a finished session is delivered while its siblings still run.
    - Native: the group check in `crates/gespan-gpui/src/pilots/host.rs`
      (`children.iter().all(..)`), with tests.
    - Both prompts: `pilotFiles.ts:348-349` and `crates/gespan-core/src/pilot.rs:321`.
11. **Mandatory Pilot folders.** Automatic, non-optional filing of every
    Pilot-triggered session (plain or Flow step) in both apps.
12. **No approvals in Pilot Flows.** A validation rule rejecting `Approval`
    states when a Flow has a Pilot owner.

## Constraints

- **Where to build (my call, to be revisited in the follow-up ticket).** Engine
  and model changes go in `crates/gespan-core` (plain Rust, no GPUI), where Flows
  already live. The native app is the first host because it is the only one that
  runs Flows today and it already has Pilots. Tauri support is the follow-up
  ticket's decision.
- **Tauri in this ticket (user decided the first part).** The Tauri app gets
  per-session completion reports (item 10) and mandatory Pilot folders
  (item 11). It gets no `flows.*` actions and no Flow prompt rules **(my
  call)**; those go only into the native Pilot prompt. If a Tauri Pilot meets a
  Pilot Flow run started natively, it leaves it alone.
- Shared SQLite DB: schema changes must stay readable by both apps (see
  `crates/gespan-core/tests/shared_contracts.rs`, `src/test/sharedContracts.test.ts`).
- Agents in Flows run unattended; keep the existing access-mode default (`auto`).
- `MAX_RUN_STEPS` (50) and per-state `max_visits` still bound every run.
- Many tasks mean many sessions and many reviews (20 tasks × 5 steps = 100). The
  user accepts that cost early on; it shrinks as blueprints mature.
- Project rules: Bun for UI tooling, Vitest for TS tests, cargo for Rust;
  meaningful tests for every behaviour above.

## Left to the builder

- Data shapes: owner field vs tag, goal/task link, review-note format.
- How a side review is hosted and how its notes reach the main Pilot.
- Whether a blueprint change reaches already-running tasks (default: no).
- A limit on how many sessions one Pilot runs at once, if usage limits demand it.
- Exact CLI action names and payloads.

## Future ideas (captured, not in scope)

- **Procedures (SOP) view.** A tab showing how Flows are built and connected:
  boxes and links, not live. It would help users see how work is organised and
  make the product easier to understand. Not needed now; revisit later.
- **Sub-flows.** A step that starts child Flows to split one problem further.
- **Pilot Flows in the Tauri app**, as a separate ticket.
