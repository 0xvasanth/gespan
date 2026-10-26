# Hold follow-ups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (the human partner chose inline execution for Hold). Steps use checkbox (`- [ ]`) syntax.

**Goal:** Close the three known gaps in Hold (#173 / PR #175), and replace the confirmation dialog with **hold-to-confirm**, in both Gespan Native and the desktop app.

**Architecture:**
- **Hold-to-confirm** is a self-contained control: a 1.2 s linear left-to-right fill, a faster drain on early release, and confirmation at the end.
  - Native builds it in GPUI (`crates/gespan-gpui/src/hold_confirm.rs`). Its state machine is pure and tested, rendered with an inverted, clipped label.
  - The desktop builds it as a React component (`src/features/hold/ui/HoldToConfirm.tsx`) with the same state machine (`holdToConfirm.ts`).
  - **Holding ⌘⇧H** drives the same fill. Key-down starts it and key-up (or releasing ⌘/⇧) drains it.
  - The **palette and menu-bar items** bring the window forward and show the button's "Hold to confirm" hint.
- **Per-session marker:** a "held" state in the existing session marks (sidebar, tabs) and a thin notice above the composer.
- **Background work:**
  - Native pins the parked processes of held sessions in `provider/parked.rs`, so they don't expire or get evicted while on hold.
  - The desktop doesn't idle-park held sessions.

**Spec:** issue #173 plus the user's follow-up. The behaviour reference is https://www.stealth.pm/ui/hold-to-confirm. Key values: duration 1200 ms, linear fill; drain `max(180 ms, progress × 500 ms)` ease-out; a tap under 250 ms shows "Hold to confirm" for 1.8 s; done shows a tick; label width stays fixed; Space/Enter hold; click-only input arms with "Press again to confirm" for 4 s; reduced motion keeps the fill and drops scale and label travel.

## Global Constraints

- **Bun** for the UI side; **cargo** for Rust. No attribution lines in commits or the PR.
- **Product name:** Gespan. No dialog remains on any path into Hold. Resume needs no confirmation, as today.
- **Durations:** `HOLD_MS = 1200`, `TAP_MS = 250`, `HINT_MS = 1800`, `ARMED_MS = 4000`. The drain is `max(180, progress × 500)` ms.
- **Copy:**
  - resting: "Hold all"
  - hint: "Hold to confirm"
  - armed: "Press again to confirm"
  - done: "Holding…"
  - per-session notice: "Held · continues on Resume"

## Review Focus

1. **⌘⇧H key repeat:** auto-repeat key-downs must not restart or speed up the fill. Releasing ⇧ or ⌘ before 1.2 s cancels.
2. **Window blur or pointer leave mid-hold:** drains and doesn't confirm.
3. **A tap or a click with no pointer behind it** (keyboard Enter tap, VoiceOver): never confirms on the first press.
4. **The pinned parked process of a held session:** must still be killed on Quit and on session delete. It is unpinned on Resume, with its idle timer restarted, not already expired.
5. **The held marker:** must clear the moment the session's resumed turn starts. It must also show for sessions restored after a restart, which have no live view state.

---

### Task 1: Pure hold-to-confirm state machine (both apps)

**Files:**
- Create: `crates/gespan-gpui/src/hold_confirm.rs`, holding `Press` (a pure model plus tests).
- Create: `src/features/hold/model/holdToConfirm.ts`, with `holdToConfirm.test.ts`.

**Interfaces** (the same shape in Rust and TypeScript):
- `Press::default()`
- `start(now)`: from Idle or a partial drain, it resumes from the current progress. Repeats while Filling are ignored.
- `release(now) -> Released::{Tap, Drain}`
- `click(now) -> bool`: click-only input. The first click arms; a second click within `ARMED_MS` confirms and returns true.
- `progress(now) -> f32`
- `tick(now) -> Option<Event::{Confirmed}>`
- `label(now) -> Label::{Rest, Hint, Armed, Done}`

**Tests:**
- Fills linearly and confirms exactly at 1200 ms.
- Repeat starts are ignored.
- An early release drains in `max(180, p × 500)` ms, and pressing again resumes from the remaining progress.
- A tap under 250 ms shows Hint for 1800 ms.
- `click` arms, then confirms within 4 s; after 4 s it disarms.
- Confirms only once.

### Task 2: Native hold-to-confirm control, held ⌘⇧H, and the palette and menu entry

**Files:**
- `crates/gespan-gpui/src/shell/hold.rs`:
  - `hold_control` (the Idle state) becomes the pressable control;
  - remove `confirm_hold` and the `window.prompt`;
  - add `fn arm_hold_hint(window, cx)`.
- `crates/gespan-gpui/src/shell.rs`:
  - The `ToggleHold` handler: Idle starts a key press, OnHold resumes, and Holding does nothing.
  - Add an `on_key_up` and `on_modifiers_changed` listener on the root to release.
- `pilots/mod.rs` (menu bar) and `shell/quick_open.rs` (palette): call `arm_hold_hint` instead of `toggle_hold` while idle.

**Rendering:**
- The fill is drawn with `pulse::looping` while Filling or Draining.
- The fill is an absolutely positioned overlay holding the inverted label, with width `progress × 100%` and `overflow_hidden`, so the text changes colour under the edge.
- A fixed label width keeps the button from resizing when the text changes.

**Tests (gpui):**
- Press and hold the button for 1.2 s → `Phase::Holding`, with no prompt shown.
- Release at 0.5 s → no hold.
- `ToggleHold` dispatched with a key-up after 0.3 s → no hold. Held for 1.2 s → holding.
- Releasing modifiers cancels.
- A palette command → no hold; the hint label shows.
- Remove the old confirmation tests; `cancelled_confirmation…` becomes `early_release_changes_nothing`.

### Task 3: Desktop HoldToConfirm, held ⌘⇧H, and the palette and menu entry

**Files:**
- Create: `src/features/hold/ui/HoldToConfirm.tsx`, using the Task 1 model, `requestAnimationFrame`, and CSS `clip-path: inset(0 X% 0 0)` on the inverted label copy.
- `HoldControl.tsx`: the Idle state renders `HoldToConfirm`.
- Delete `ConfirmHold.tsx` and the `useHold` confirm promise. `toggle()` becomes `hold()` / `resume()`.
- `App.tsx`: add a keyup listener to the capture handler for ⌘⇧H. The `hold_toggle` menu event and the palette `hold` action call `armHint()`.

**Tests (happy-dom, fake timers):**
- Pointer hold for 1200 ms → onConfirm.
- An early pointerup → no confirm, and it drains.
- A tap shows the hint.
- Space held → confirm; a Space tap → no confirm.
- A click with `detail === 0` arms, and a second click confirms.
- Reduced motion keeps the fill.
- ⌘⇧H held through the App key handler (a unit test on the extracted `shortcutHold` helper) → hold; key-up early → none.

### Task 4: Per-session held marker (both apps)

**Native:**
- `session_mark.rs`: add `State::Held` (a `queue-pause` icon, aria "Held").
- `shell.rs` `refresh_session_mark`: Held when `self.session_held(id, live, cx)`, ranked before Working and Done.
- `shell/hold.rs`: refresh marks on every hold state change and resume.
- `conversation.rs` composer: a thin notice "Held · continues on Resume" when `held.is_some() || hold_requested` and a hold is active.

**Desktop:**
- Add `heldSessionIds: Set<string>` to the Sidebar props and `SessionCard` (a pause icon ranked before busy).
- Add a SessionPane notice above the composer.
- App computes the set from `holdRef.current.isSessionHeld`.

**Tests:**
- Native: the mark shows Held for a tracked session, including one restored after a restart with no view; it clears when the resumed turn begins; the notice renders.
- Desktop: a SessionCard with `held` renders the pause icon and the aria label; the notice renders only when held.

### Task 5: The queue shelf's own Resume while held (Native)

**Files:** `conversation/follow_up.rs:499`. Hide the Resume button while `crate::hold::active(cx)`, mirroring the desktop.

**Test:** the shelf renders no "queue-resume" button while held, and does render it when not held.

### Task 6: Keep background work alive for held sessions

**Native core** (`provider/parked.rs`, `provider.rs`):
- Add a static `HOLD_PINNED: Mutex<HashSet<i64>>`.
- Add `pub fn pin_idle_agents(ids)` and `pub fn unpin_idle_agents()`.
- Pinned owners are skipped by the WATCH 180 s expiry, `enforce_idle_limit`, `take`'s expiry check and `settle_cancelled`'s bound. Unpin resets `at = now`.
- When `LIMIT == 0`, a held interrupted session is still parked while pinned. `put` accepts it if the owner is pinned.
- Quit (`shutdown_all`), `forget_session` and `discard_session` still kill.

**Native shell:** `hold_all` pins the tracked ids; `resume_all` unpins.

**Desktop:**
- `registry.ts`: add `holdIdlePark(keep: (id) => boolean)`. While `keep(id)`, `scheduleIdlePark` doesn't arm and `enforceHarnessIdleLimit` skips the session.
- App passes `isSessionHeld`. On Resume, re-arm idle parking for those sessions.

**Tests:**
- Core: with `configure(1)`, a pinned parked process outlives an 180 s expiry; to test this, an injectable clock or a test-only short TTL is needed; reuse the `put` path with `at` back-dated.
- Core: with `LIMIT 0`, a pinned session is parked, not killed.
- Core: unpin restores normal expiry; `forget_session` still kills a pinned process.
- Desktop: `scheduleIdlePark` doesn't stop a held session; after Resume it parks again after 3 min.

### Task 7: Verify and open the PR

- [ ] Run `cargo test` (core + gpui, `--test-threads=1`), clippy `-D warnings`, fmt, `bun run test` and `tsc`, and src-tauri fmt, clippy and test.
- [ ] Final whole-branch review by a fresh reviewer, then the fix pass.
- [ ] Open the PR against `main` (#175 has merged).
