# Hold and Resume: design

Source: GitHub issue #173.

## Problem

There's no clean way to stop work at the end of the day and pick it up later. When I leave the office I close the lid, and every session that was in the middle of a turn breaks: the agent's stream drops, the turn fails, and when I open the laptop at home I have to work out which sessions died, where each one was, and what to tell it.

The tools that exist today make this worse.

- **Stop and Quit both kill running commands.** Stop sends the provider's interrupt, and for Claude Code it also sends `stop_task` for every background task. It often ends with a `SIGKILL` of the whole process group (`process::terminate`, `crates/gespan-core/src/process.rs`). A `cargo build` or test run in the middle of executing dies with it. Quit (`process::shutdown_all`) sends `SIGKILL` to every provider group after one second.
- **Nothing recovers on launch.** Native doesn't write or read `in_flight_sessions`. After a restart, a session that was running shows whatever was last saved, with no sign that it was cut off.
- **Work keeps starting by itself.** Pilot habits (every 10 s), automations (every 30 s), orchestration workers and Pilot `sessions.start` can all start new turns while I'm trying to wrap up. There's no global switch.
- **Pilots get false completions.** An interrupted worker counts as "stopped" (`!busy && has_messages`, `pilots/host.rs`), so its Pilot receives "[Delegated work completed]" with partial output and acts on it.

## What I want

One **Hold** action that I can press on my way out. It takes effect right away and puts everything on hold without breaking anything. I can close the lid, quit, or restart. Later, one **Resume** action continues every session from where it stopped.

## Behaviour

### Pressing Hold

1. **No new work starts, from that moment.**
   - No new turns from the composer: messages I type are queued (see below).
   - No Pilot habit runs, no automation runs, no orchestration dispatches.
   - No Pilot `sessions.start` or `sessions.send`. These are queued against the Pilot and replayed on Resume.
2. **Each running session stops at its next safe point.** A safe point is when no foreground tool or command is running.
   - **The agent is thinking or streaming text:** interrupt immediately. Only the partial answer is lost, and the agent regenerates it on Resume.
   - **A tool or command is running:** let it finish. As soon as its result is recorded, interrupt the turn, before the agent's next step. Nothing is killed. This works the same for every provider (Claude Code, Codex, Pi/Omp, ACP agents, OpenCode) because it uses the tool start and end events they all emit, so it also covers Full Access and bypass sessions that never ask for approval.
   - **Waiting for an approval or a question:** leave it pending. While on hold, questions with `auto_resolve_at` don't auto-resolve.
   - **Interrupt for a hold has to be softer than Stop.** It must not deny approvals or questions, must not send Claude's `stop_task` for background tasks, and must not end in a process-group `SIGKILL`.
3. **Background tasks don't block the hold.** These are commands that keep running on their own, such as `bun run dev` or a watcher. Hold doesn't wait for them and doesn't stop them while Gespan is open. They end on Quit as they do today.
4. **Status while settling.**
   - The UI shows **Holding…** with what's left, for example "Waiting for `cargo test` in *Fix flaky reminders* (2m 10s)".
   - Each waiting session has a **Stop now** action. It's an explicit choice to kill that command.
   - When every session has reached its safe point, the state becomes **On hold**.
5. **The Mac can sleep.** When on hold, keep-awake is released. If I close the lid while a command is still running, that's fine. macOS suspends the process, the command finishes after wake, and the session then stops at its safe point.

### While on hold

- **Visible state.** The status-bar control shows "On hold · N sessions · M queued · ▶ Resume". The menu-bar item shows the same.
- **Queued messages.**
  - Typing in any session queues the message with `steer = false`, so it never goes into a live turn.
  - Messages are saved immediately through the existing follow-up queue (`follow-up:<session>`, attachments in `composer-messages/`).
  - Nothing sends until Resume.
- **Pilot chats.** They queue too. Pilot chats skip `pump_follow_ups` today, so they need the same hold-aware queueing.
- **Pilot reports.** No "Delegated work completed" report is sent for a worker that was held. Its launch stays pending.
- **Habit timers stop counting.** Run timeouts (`TIMEOUT_MS`, 1 h) and approval waits (`APPROVAL_WAIT_MS`, 12 h) exclude the time spent on hold, so holding doesn't time out or dismiss a habit run and its child sessions.

### Across quit and restart

- **What's persisted.** The hold state is saved in the store, along with a per-session record of what was held:
  - session id;
  - provider conversation id;
  - why it stopped (interrupted at a safe point, interrupted while streaming, waiting on approval, idle with queued messages);
  - the last completed tool;
  - the time it was held.
- **Quitting while on hold** is safe. Every session is already at a safe point, so `shutdown_all` kills no command in the middle of running. Quitting while still **Holding…** asks before stopping whatever is still running.
- **On launch:**
  - Gespan opens still on hold.
  - Held sessions show a "Held, continue on Resume" marker.
  - Nothing starts by itself.
  - Approvals that were pending can't survive a restart, because the agent process is gone. Those sessions resume with "continue" and re-ask.

### Resume

1. Lift the hold. Schedulers and the composer work again.
2. Resume sessions in stages, with a concurrency limit (default 3), in this order:
   1. Plain sessions and Pilot workers. Each gets "Continue from where you left off." if it was interrupted, then its queued messages in order (the existing `resume_queue` path).
   2. Pilot chats last, so their reports reflect real finished work and not the hold. Then replay their queued `sessions.start` and `sessions.send` calls.
3. **Habits that came due while held** run once each (catch-up), not once per missed slot. Automations that came due are claimed once.
4. Resume works whether the app stayed open or was restarted, because the provider's own resume is used: `--resume`, `thread/resume`, `session/load` and so on.

### Placement and confirmation

- **Status bar.** Hold is app-wide, so its control sits at the far right of the bottom status bar, never inside a session.
- **Confirmation.** Every way of starting it asks first: the button, **⌘⇧H**, the command palette, the Gespan Native app menu and the menu-bar item all show "Hold everything?" with Hold and Cancel.
- **No undo while holding.** Once confirmed, Hold can't be undone until every session has stopped. The control shows a continuous progress state ("Holding 2/5…"), and clicking it lists what is still finishing, each with **Stop now**. **Resume** appears only once everything is on hold.
- **On hold.** The control reads "On hold · N sessions · M queued · ▶ Resume".

## Rollout

1. **Gespan Native first.** That's this plan: `docs/superpowers/plans/2026-10-25-native-hold-resume.md` on `feat/session-handoff`.
2. **Then the desktop app (Tauri + React)** in a second PR. It has the same behaviour and placement, but its own agent runners and UI.

## Out of scope

- Keeping an approval request alive across a restart. The agent asks again after Resume.
- Pausing remote or mobile sessions on other machines.
- Freezing processes with `SIGSTOP`. Long pauses would break the agents' API streams. Stopping at a safe point is the approach.

## Where

- `crates/gespan-core/src/hold.rs` (new): the hold state machine, per-session hold records, persistence and resume ordering.
- `crates/gespan-core/src/provider.rs`, `provider/lifecycle.rs`, `provider/parked.rs`, `acp.rs`, `pi.rs`, `opencode.rs`: a soft interrupt that keeps approvals, background tasks and the process.
- `crates/gespan-gpui/src/conversation.rs`, `conversation/follow_up.rs`: safe-point detection, hold-aware queueing (`steer = false`), held markers.
- `crates/gespan-gpui/src/shell.rs`, `shell/layout.rs`, `shell/automation_runs.rs`, `shell/orchestration.rs`, `shell/settings.rs` (keep-awake): status-bar control, confirmation, actions, scheduler gates.
- `crates/gespan-gpui/src/pilots/scheduler.rs`, `pilots/host.rs`, `crates/gespan-core/src/pilot/habits.rs`, `pilot/completions.rs`: habit gating, timeout pause, report suppression, queued Pilot calls.
- `crates/gespan-gpui/src/pilots/status.rs`: menu-bar item.

## Done when

- Pressing Hold with a session running a long command (for example `sleep 60 && echo done`) lets the command finish, then stops that session with no further model request. The command's output is in the transcript.
- Pressing Hold with a session that is streaming stops it within a second. Pressing Hold with a session that is waiting on approval leaves the approval pending.
- While on hold:
  - no habit, automation, orchestration worker or Pilot `sessions.start` starts a turn;
  - typed messages are queued and not sent, including in Pilot chats;
  - keep-awake is released.
- Quitting while on hold and relaunching leaves Gespan on hold, with the same held sessions and queued messages and nothing running.
- Resume continues every held session (workers before Pilots, at most 3 at once) and sends queued messages in order. A Pilot gets a completion report only for work that actually finished after Resume.
- Habit runs that were held aren't timed out by the hold. Habits that came due while held run once on Resume.
- Tests cover:
  - the hold state machine (Holding → On hold → Resume, plus restart while in each state);
  - safe-point detection (tool running vs streaming vs approval);
  - the soft interrupt not denying approvals or stopping background tasks;
  - queueing while held (`steer = false`, persisted, not sent);
  - scheduler gates;
  - completion-report suppression;
  - timeout accounting that excludes held time;
  - resume ordering and the concurrency limit.
