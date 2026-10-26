# Pilot Chat Dictation (Native) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Pilot chat composer in Gespan Native gets the same dictation as a
session composer: the mic, the caption row, "Composer: Dictate" with
hold-to-talk, Escape-to-cancel and the model download prompt.

**Architecture:** The Pilot chat is a `Conversation` in Pilot mode and already
owns `self.dictation`. We move the session composer's dictation key handling
(shortcut, key-up, modifier release, Escape capture) into one
`Conversation::dictation_keys` helper in `conversation/dictation.rs`'s existing
`impl Conversation` block, use it on both `composer-box` and `pilot-composer`,
attach it to the Pilot chat's root (`pilot-chat`), and add `dictation::bar` and
`dictation::mic_button` to the Pilot composer. When
a Pilot chat is opened, `dictation::switched_to` cancels dictation running in
any other composer, as switching sessions already does.

**Tech Stack:** Rust, GPUI (`crates/gespan-gpui`), the existing speech hub in
`conversation/dictation.rs` and its `with_sink` test transport.

**Spec:** GitHub issue #185 (approved design, part of epic #50; follow-up to #117).

## Global Constraints

- Native only (`crates/gespan-gpui`). No Tauri changes; Pilots don't exist there.
- Reuse `conversation/dictation.rs` as it is; don't fork it. The only edit there is the additive `dictation_keys` helper (Task 1).
- Dictated text goes into the Pilot composer's field; sending stays a separate step.
- Same states as the session mic: idle, listening, downloading, error. Same Settings → Dictation choices (they're global `Preferences`; nothing per-Pilot).
- Shortcut is the existing `input::Dictate` binding (`{cmd|ctrl}-shift-m`, context `"Composer"`, remappable as `DICTATE_COMMAND`); hold threshold `HOLD_TO_TALK` (400 ms).
- Escape cancels dictation before the field or a menu sees it, also while the composer is read-only.
- Gespan names only (`src/test/legacyNames.test.ts`). No tool or AI attribution in commits or PR.
- `crates/gespan-gpui/src/conversation.rs` and `pilots/tests.rs` are large shared files: keep edits small.

## My calls (no question changed what gets built)

- **Shared key handler, not a copy.** The ~35 lines of key handling on `composer-box` move into `dictation_keys` and both composers call it. This touches the session composer, but only by moving code; the existing session tests in `conversation/dictation/tests.rs` guard it.
- **Mic placement:** in `pilot_composer_tools`, right after `pilot-emoji` and before the model pill, i.e. with the other left-hand tools under the box. It uses `dictation::mic_button` unchanged, so its debug selector stays `composer-mic` and its look is the session mic's `tool_pill` (26 px square), not the Pilot `tool` style. Restyling would mean forking `mic_button`.
- **Where the keys attach in Pilot mode:** on the `pilot-chat` root returned by `attachment_drop_target`, not on `pilot-composer`. The Pilot's model list and composer menus render as children of that root, so Escape is captured there even when a menu holds focus. The shortcut itself only fires from the field (`"Composer"` key context), so the wider element changes nothing else. If `attachment_drop_target` doesn't return `Stateful<Div>`, make `dictation_keys` generic over `InteractiveElement + StatefulInteractiveElement` instead.
- **Caption row:** `dictation::bar` goes inside `pilot-composer`, after the attachment chips and above the field row, the same spot as in the session composer.
- **Vocabulary context:** `render_pilot` calls `self.dictation_context(cx)` like the session render, so the Pilot's working directory and model name feed the custom words.
- **Opening a Pilot cancels dictation elsewhere.** `Pilots::open` calls `dictation::switched_to` with the opened chat's `Dictation`. Opening a session already does this via `Shell::activate_session`, which also cancels a Pilot's dictation. Effect: one dictation at a time, matching session-to-session switching.
- **Floating Pilot windows** render the same `Conversation` entity (`pilots/floating.rs`), so they get the mic automatically. Nothing to add. The overview has no composer.
- **Blur ending a hold** already works for Pilot chats: the `on_blur` hook at the top of `Conversation::render` runs before the `render_pilot` branch.
- **Tests live in a new `src/pilots/dictation_tests.rs`** (registered next to `mod tests;` in `pilots/mod.rs`) rather than growing the 5k-line `pilots/tests.rs`. `Setup` and `setup()` in `pilots/tests.rs` become `pub(super)` so the new file can reuse them. The speech harness is rebuilt there in a few lines from the existing `pub(crate)` `dictation::with_sink` and `dictation::tests::make_ready`.

## Review Focus

1. Escape while the Pilot's model list or a composer menu is open and dictation is running: dictation cancels and the menu stays open. (Task 2, `pilot_escape_cancels_dictation_before_menus`)
2. Dictating in a session, then opening a Pilot: the session's late final text must not land in either field. (Task 3)
3. Dictating in Pilot A, then opening Pilot B: A cancels, and B's field stays empty. (Task 3)
4. The final text arrives after the Pilot composer turned read-only (agent busy): it still lands in the Pilot field, as in sessions. (Task 2, final assertion in `pilot_mic_dictates_into_the_pilot_field`)
5. Dictation must not send: after the final text, the Pilot has no new pending or delivered message. (Task 2)

---

### Task 1: Share the composer's dictation key handling

**Files:**
- Modify: `crates/gespan-gpui/src/conversation/dictation.rs` (the `impl super::Conversation` block holding `dictation_context`, ~line 833)
- Modify: `crates/gespan-gpui/src/conversation.rs:2892-2923` (the `composer-box` chain)

**Interfaces:**
- Produces: `pub(super) fn dictation_keys(&self, el: gpui::Stateful<gpui::Div>, cx: &mut Context<super::Conversation>) -> gpui::Stateful<gpui::Div>`. It attaches, unchanged and in the same order: `.on_action(input::Dictate → shortcut_down)`, `.on_key_up(→ shortcut_up)`, `.on_modifiers_changed(release of secondary+shift while chord_held → shortcut_up)`, `.capture_action(shell::DismissMenu → cancel + stop_propagation when cancellable)`, `.capture_key_down("escape" → cancel + stop_propagation when cancellable)`. Move the existing comments with the code.

- [ ] **Step 1: Run the session dictation tests as the baseline**

Run: `cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-gpui --profile native-dev conversation::dictation`
Expected: all pass (record the `test result:` line).

- [ ] **Step 2: Add `dictation_keys` and replace the inline chain in `conversation.rs` with `let keys = ...; self.dictation_keys(div().id("composer-box")..., cx)`** (or wrap the builder so the call sits where the chain was; keep `capture_action(input::Paste)` on `composer-box` where it is).

- [ ] **Step 3: Rerun Step 1's command**

Expected: same tests, all pass, including `cmd_shift_m_tap_toggles_and_hold_stops` and `stop_and_cancel_controls_work_while_disabled`.

- [ ] **Step 4: Commit**

```bash
git add crates/gespan-gpui/src/conversation/dictation.rs crates/gespan-gpui/src/conversation.rs
git commit -m "Share the composer's dictation key handling"
```

### Task 2: Dictation in the Pilot composer

**Files:**
- Modify: `crates/gespan-gpui/src/conversation/pilot.rs` (`render_pilot`, `pilot_composer_box`, `pilot_composer_tools`)
- Modify: `crates/gespan-gpui/src/pilots/tests.rs` (`struct Setup`, its fields and `fn setup` → `pub(super)`)
- Modify: `crates/gespan-gpui/src/pilots/mod.rs:23-24` (add `#[cfg(test)] mod dictation_tests;`)
- Create: `crates/gespan-gpui/src/pilots/dictation_tests.rs`

**Interfaces:**
- Consumes: `Conversation::dictation_keys` (Task 1); `dictation::{bar, mic_button, with_sink, Speech, Dictation, DictationState, HOLD_TO_TALK}`, `dictation::tests::make_ready`, `speech_protocol` `Request`/`Event` as used in `conversation/dictation/tests.rs`.
- Produces (test helpers in `dictation_tests.rs`, reused by Task 3): `struct Speech { _dir, sent, hub }` with `fn new(ready: bool, cx) -> Self`, `fn emit(&self, Event, cx)`, `fn take(&self) -> Vec<Request>`; `fn pilot_chat(s: &Setup, cx) -> (VisualTestContext, Entity<Conversation>)` which dismisses Pilot settings (as `chat_composer_keeps_attach_under_the_box_and_opens_the_model_list` does), focuses the chat input and returns `s.manager.read(cx).chat()`. Install the speech hub **before** `setup()` so the chat's `Dictation` joins it.

- [ ] **Step 1: Write the failing tests in `dictation_tests.rs`**

```rust
#[gpui::test]
fn pilot_composer_shows_the_mic_with_its_tools(cx) {
    // mic = debug_bounds("composer-mic"), field = "pilot-composer",
    // attach = "pilot-attach", model = "pilot-model"
    assert!(mic.top() >= field.bottom());
    assert!(mic.left() > attach.right());
    assert!(model.left() > mic.right());
    assert_eq!(mic.size, size(px(26.), px(26.)));
}

#[gpui::test]
fn pilot_mic_dictates_into_the_pilot_field(cx) {
    // ready model; click "composer-mic"; emit Listening
    assert_eq!(state, DictationState::Listening);
    assert!(field.contains(&debug_bounds("dictation-bar").center())); // caption row inside the Pilot box
    // emit Event::Level (so `heard` is set, as the session `listen` helper does), then Event::Partial { text: "hel" } → "dictation-partial" visible
    // click "composer-mic" again → take() == [Request::Stop]
    // set the chat input read-only, then emit Final "hello pilot"
    assert_eq!(chat input text, "hello pilot");
    assert!(chat.pilot_rows(cx).is_empty() && chat.pilot_delivered().is_empty()); // nothing sent
}

#[gpui::test]
fn pilot_shortcut_tap_toggles_and_hold_stops(cx) {
    // mirror cmd_shift_m_tap_toggles_and_hold_stops against the Pilot chat:
    // tap → Starting, Listening; tap → Finishing + [Stop], final "tapped";
    // hold past HOLD_TO_TALK with repeats, release modifiers → [Stop], final "held";
    assert_eq!(text, "tapped held");
}

#[gpui::test]
fn pilot_escape_cancels_dictation_before_menus(cx) {
    // open the model list (chat.toggle_model_menu), start via the shortcut, emit Listening
    // simulate_keystrokes("escape")
    assert_eq!(state, DictationState::Idle);
    assert_eq!(take(), vec![Request::Cancel]);
    assert_eq!(chat.pilot_menus_open(), (true, false)); // the list stayed open
    // second Escape, with nothing to cancel, now reaches the menu
}

#[gpui::test]
fn pilot_without_a_model_shows_the_download_prompt(cx) {
    // Speech::new(false); click "composer-mic"
    assert_eq!(state, DictationState::NeedsModel);
    assert!(debug_bounds("dictation-download").is_some());
    assert!(take().is_empty()); // nothing started
    // Escape closes the prompt
    assert_eq!(state, DictationState::Idle);
    assert!(debug_bounds("dictation-download").is_none());
}
```

Use the session file's `SHORTCUT` constant pattern (`cmd-shift-m` on macOS, `ctrl-shift-m` elsewhere) and its `click` helper shape.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-gpui --profile native-dev pilots::dictation_tests`
Expected: FAIL, `composer-mic is not on screen` / state stays `Idle`.

- [ ] **Step 3: Wire dictation into the Pilot composer**

In `render_pilot`: call `self.dictation_context(cx)` before building. Wrap the `self.attachment_drop_target("pilot-chat", cx)` root in `self.dictation_keys(..., cx)`. In `pilot_composer_box`: add `.children(dictation::bar(&self.dictation, &self.input, cx))` after `.children(self.attachment_chips(cx))`. In `pilot_composer_tools`: add `.child(dictation::mic_button(&self.dictation, &self.input, cx))` after the `pilot-emoji` child.

- [ ] **Step 4: Run Step 2's command, then the Pilot and session suites**

Run: `cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-gpui --profile native-dev pilots::` and `... conversation::dictation`
Expected: all pass; `chat_composer_keeps_attach_under_the_box_and_opens_the_model_list` still passes.

- [ ] **Step 5: Commit**

```bash
git add crates/gespan-gpui/src/conversation/pilot.rs crates/gespan-gpui/src/pilots/
git commit -m "Add dictation to the Pilot chat composer"
```

### Task 3: One dictation at a time across Pilot and session composers

**Files:**
- Modify: `crates/gespan-gpui/src/pilots/mod.rs` (`Pilots::open`, after `self.ensure_chat(id, cx)`)
- Test: `crates/gespan-gpui/src/pilots/dictation_tests.rs`

**Interfaces:**
- Consumes: `crate::conversation::dictation::switched_to(&Entity<Dictation>, &mut App)`; Task 2's `Speech` and `pilot_chat` helpers.

- [ ] **Step 1: Write the failing tests**

```rust
#[gpui::test]
fn opening_a_pilot_cancels_session_dictation(cx) {
    // create a second session in s.store ("claude", &s.project), Shell::open it,
    // start its dictation (toggle), emit Listening, take()
    // Pilots::open(&s.id)
    assert_eq!(session state, DictationState::Idle);
    assert_eq!(take(), vec![Request::Cancel]);
    // emit Event::Final { text: "stray", capped: false } + Event::Cancelled
    assert_eq!(session input text, "");
    assert_eq!(pilot input text, "");
}

#[gpui::test]
fn opening_a_session_cancels_pilot_dictation(cx) {
    // dictate in the Pilot chat (Listening), then Shell::open the other session
    assert_eq!(pilot state, DictationState::Idle);
    // late Final "stray" lands in neither field
}

#[gpui::test]
fn switching_pilots_cancels_the_first_pilots_dictation(cx) {
    // create a second Pilot (manager.create), reopen the first, dictate there,
    // then Pilots::open(second)
    assert_eq!(first state, DictationState::Idle);
    // late Final lands in neither Pilot's field
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-gpui --profile native-dev pilots::dictation_tests`
Expected: `opening_a_pilot_cancels_session_dictation` and `switching_pilots_...` FAIL (state stays `Listening`); `opening_a_session_cancels_pilot_dictation` may already pass through `activate_session`, which is fine: it pins that behaviour.

- [ ] **Step 3: In `Pilots::open`, after `ensure_chat`, call `switched_to` with `self.chats[id].read(cx).dictation.clone()`.**

- [ ] **Step 4: Rerun Step 2's command**

Expected: all `pilots::dictation_tests` pass.

- [ ] **Step 5: Commit**

```bash
git add crates/gespan-gpui/src/pilots/
git commit -m "Cancel other dictation when a Pilot chat opens"
```

### Task 4: Full verification

- [ ] **Step 1:** `bun install` if `node_modules` is missing, then `bun run native:check` (foreground, alone). Expected: fmt clean, clippy `-D warnings` clean, `test result: ok` lines for `gespan-core` and `gespan-gpui`. Rerun known load-flaky tests before treating them as failures.
- [ ] **Step 2:** `bunx vitest run src/test/legacyNames.test.ts`. Expected: passes.
- [ ] **Step 3:** Manual check in `bun run native:dev` if a mic is available: open a Pilot, click the mic, speak, see the caption row and the text in the Pilot field; press Escape mid-take; tap and hold the shortcut. Note in the PR if it couldn't be run.

## Follow-ups (for the PR, not this branch)

- The mic uses the session `tool_pill` look, not the Pilot tools' style (17 px icons, `content(0.55)`). A style option on `mic_button` would align it.
- Opening a Pilot in a floating window doesn't go through `Pilots::open`, so it doesn't cancel dictation in the main window; the hub still allows only one take, so text can't leak.
