# First-run setup Implementation Plan

> **Update (2026-11-09):** the Look card's Layout row (Focused/Classic) was dropped along with
> the classic layout; Look now covers theme, accent and glass only.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show new users a card they can't close: Welcome, Coding agents, Look and Project. Only the project is required. The card lands them in a new thread in that project.

**Architecture:** The state is one new `Workspace.setup: Option<SetupStep>` field. `None` means done, and it's what every existing saved workspace deserializes to. `main.rs` sets it only on a true first run (no saved workspace and no sessions). A new `setup` module holds:
- pure rules, tested without GPUI;
- a `SetupCard` GPUI view that emits events.

The `Shell` owns the card. It renders the card above every other modal, routes its events to the existing setters, and finishes by adding and selecting the project.

**Tech Stack:** Rust, GPUI, serde, the `gespan-core` provider and accounts modules.

**Spec:** `docs/design/first-run-setup.md`. Canvas: https://claude.ai/artifact/LU1nZ6sAwGRHbQcePB6tn6. The card must match the canvas exactly; the spec's "Card spec" section has every value.

## Global Constraints

- Copy is sentence case, with no emoji and short plain words. Copy strings are taken verbatim from the canvas boards.
- Dark glass uses the app's theme helpers (`crate::theme::*`, `crate::modal::surface`). Light mirrors them; don't hard-code a second palette.
- The card can't be closed: no close button, and Esc, scrim clicks and shell shortcuts do nothing. Only ⌘O and ⌘Q pass through.
- Existing installs never see setup. A saved workspace without a `setup` key means done.
- Setup never blocks on agents. Only the Project card blocks.
- Tests run on macOS, Windows and Linux CI: no macOS-only paths in tests.
- Run Rust tests from `crates/gespan-gpui` (its own Cargo workspace): `cargo test <filter>`.

## Review Focus

- **Dock launch.** The launch folder is `/` or `$HOME`. No "Opened from" suggestion is shown, and the stray launch project is removed on finish. Owner: Task 1.
- **Tauri-migrated database.** It has sessions but no `native_settings.workspace` row, so setup must not show. Owner: Task 1 (`first_run`).
- **Quit on the Project card.** The relaunch reopens Project, not Welcome, and the earlier picks are kept. Owner: Task 4.
- **Folder picker cancelled.** The card stays on Project with the primary disabled, and no error toast appears. Owner: Task 4.
- **Rescan while a sign-in is running.** The list refreshes without starting a second sign-in. Owner: Task 3.

---

### Task 1: Setup state and rules

**Files:**
- Create: `crates/gespan-gpui/src/setup.rs` (module root, `pub(crate) mod rules;`, `mod card;`)
- Create: `crates/gespan-gpui/src/setup/rules.rs`
- Modify: `crates/gespan-gpui/src/workspace.rs`: add the field next to `new_layout_prompt_dismissed` (~line 180), plus `finish_setup`.
- Modify: `crates/gespan-gpui/src/main.rs`: `mod setup;`
- Test: `#[cfg(test)] mod tests` in `setup/rules.rs`, and in `workspace.rs`'s existing tests.

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)] pub(crate) enum SetupStep { Welcome, Agents, Look, Project }`
  - `Workspace.setup: Option<SetupStep>` (serde default `None`)
  - `rules::first_run(saved: Option<&str>, has_sessions: bool) -> bool`
  - `rules::with_pending_setup(saved: Option<String>) -> String`
  - `rules::not_found_line(missing: &[&str]) -> String`
  - `rules::found_count(installed: usize, total: usize) -> String`
  - `rules::skip_default(installed: &[Kind], signed_in: impl Fn(Kind) -> Option<bool>) -> Option<Kind>`
  - `rules::suggested_folder(launch: &str, home: &str) -> Option<String>`
  - `Workspace::finish_setup(&mut self, chosen: &str, launch: &str, launch_has_sessions: bool)`

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn first_run_only_without_saved_state_or_sessions() {
    assert!(first_run(None, false));
    assert!(!first_run(None, true));            // Tauri-migrated data: sessions, no workspace row
    assert!(!first_run(Some("{}"), false));     // any saved workspace is an existing install
}
#[test] fn pending_setup_is_written_into_saved_state() {
    let ws = Workspace::restore(Some(&with_pending_setup(None)), &[], "/w");
    assert_eq!(ws.setup, Some(SetupStep::Welcome));
    let ws = Workspace::restore(Some(r#"{"classic_layout":true}"#), &[], "/w");
    assert_eq!(ws.setup, None);                 // existing install: no key, done
}
#[test] fn not_found_line_names_what_is_missing() {
    assert_eq!(not_found_line(&[]), "Every supported agent is installed");
    assert_eq!(not_found_line(&["Kiro"]), "Not found: Kiro");
    assert_eq!(not_found_line(&["Grok", "Kiro"]), "Not found: Grok and Kiro");
    assert_eq!(not_found_line(&["Grok", "Omp", "Fx", "Hermes", "Devin", "Kiro"]),
               "Not found: Grok, Omp and 4 more");
    assert_eq!(found_count(6, 12), "6 of 12 installed");
}
#[test] fn skip_default_prefers_a_signed_in_agent() {
    let installed = [Kind::Codex, Kind::Claude];
    assert_eq!(skip_default(&installed, |k| Some(k == Kind::Claude)), Some(Kind::Claude));
    assert_eq!(skip_default(&installed, |_| None), Some(Kind::Codex));
    assert_eq!(skip_default(&[], |_| Some(true)), None);
}
#[test] fn dock_launch_folders_are_never_suggested() {
    assert_eq!(suggested_folder("/", "/Users/a"), None);
    assert_eq!(suggested_folder("/Users/a", "/Users/a"), None);
    assert_eq!(suggested_folder("/Users/a/langfuse", "/Users/a").as_deref(), Some("/Users/a/langfuse"));
}
#[test] fn finish_setup_selects_the_choice_and_drops_an_empty_launch_project() {
    let mut ws = Workspace::restore(Some(&with_pending_setup(None)), &[], "/Users/a");
    ws.finish_setup("/Users/a/langfuse", "/Users/a", false);
    assert_eq!(ws.setup, None);
    assert_eq!(ws.project, "/Users/a/langfuse");
    assert_eq!(ws.projects, vec!["/Users/a/langfuse".to_string()]);
}
#[test] fn finish_setup_keeps_a_launch_project_that_has_sessions() {
    let mut ws = Workspace::restore(Some(&with_pending_setup(None)), &[], "/Users/a/old");
    ws.finish_setup("/Users/a/new", "/Users/a/old", true);
    assert!(ws.projects.contains(&"/Users/a/old".to_string()));
}
```

- [ ] **Step 2: Run them and see them fail.** Run `cargo test setup::rules finish_setup`. Expected: they don't compile, because `rules` doesn't exist.

- [ ] **Step 3: Implement.**
  - `with_pending_setup` parses `saved` (or `{}`), sets `"setup": "Welcome"` and re-serializes.
  - `skip_default` walks `installed` in `Kind::ALL` order: the first one with `Some(true)`, else the first installed.
  - `finish_setup` sets `setup = None`, calls `add_project(chosen)` and sets `project = chosen`. It removes `launch` from `projects` when `launch != chosen && !launch_has_sessions`.

- [ ] **Step 4: Run the tests again.** Run `cargo test setup::rules finish_setup`. Expected: they PASS.

- [ ] **Step 5: Commit**: "Add first-run setup state and rules".

### Task 2: Start setup on a true first run

**Files:**
- Modify: `crates/gespan-gpui/src/main.rs:468-520`. Compute it after `saved` and `cwd`, before `db.sessions()` and the "create initial session" push.
- Test: `crates/gespan-gpui/src/shell.rs`, the existing `#[cfg(test)]` shell tests module.

**Interfaces:**
- Consumes: `rules::first_run` and `rules::with_pending_setup` (Task 1).
- Produces: on a first run, the saved string handed to `Shell::new_with_metadata` carries `"setup":"Welcome"`, and `sessions` stays empty. No "initial session" is created in the launch folder.

- [ ] **Step 1: Write the failing test** `shell_starts_with_no_sessions_while_setup_is_pending`. Build `Shell::new` with an empty session list and `Some(with_pending_setup(None))`. Assert that it renders without panicking, that `shell.workspace.setup == Some(SetupStep::Welcome)` and that `session_summaries` is empty.
- [ ] **Step 2: Run it and see it fail.** Run `cargo test shell_starts_with_no_sessions_while_setup_is_pending`.
- [ ] **Step 3: Implement in `main.rs`.**
  - Compute `let first_run = setup::rules::first_run(saved.as_deref(), !db.sessions().map(|s| s.is_empty()).unwrap_or(true));`.
  - If it's true, set `saved = Some(with_pending_setup(saved))`.
  - Guard the `sessions.is_empty()` "create initial session" push with `&& !first_run`.
  - If the shell can't tolerate zero sessions, fix that in the shell rather than creating a stray session.
- [ ] **Step 4: Run the test again**, plus `cargo test restore_`. Expected: PASS, with no regressions.
- [ ] **Step 5: Commit**: "Start first-run setup only on a fresh install".

### Task 3: The setup card

**Files:**
- Create: `crates/gespan-gpui/src/setup/card.rs`, the `SetupCard` view, frame, header, footer and step switch.
- Create: `crates/gespan-gpui/src/setup/steps.rs`, the render functions for the four cards.
- Test: `#[cfg(test)] mod tests` in `setup/card.rs` (gpui `TestAppContext`, following `settings/layout_tests.rs`).

**Interfaces:**
- Consumes: `SetupStep`, `not_found_line`, `found_count` and `suggested_folder` (Task 1); `installed_providers::snapshot`; `Kind::ALL` and `Kind::label`; `gespan_core::accounts::login::signed_in`; `crate::modal::{scrim, surface, enter}`.
- Produces:
  - `pub(crate) struct SetupCard`, with `SetupCard::new(step: SetupStep, prefs: Preferences, classic: bool, launch: String, home: String) -> Self`.
  - `pub(crate) enum SetupEvent { Step(SetupStep), Preferences(Box<Preferences>), Layout { classic: bool }, DefaultAgent(Kind), SignIn(Kind), Rescan, ChooseFolder, ChooseRemoteFolder, Finish(String) }`.
  - `impl EventEmitter<SetupEvent> for SetupCard`.
  - `SetupCard::set_folder(&mut self, path: String, cx)` and `SetupCard::refresh_agents(&mut self, cx)`.

The card layout follows the spec's "Card spec" exactly: 720 wide, radius 24, padding 28/40/28, the header with "Step N of 3" and three 28×4 bars, and the footer pills. Copy per step comes from the canvas:
- **Welcome:** "Welcome to Gespan.", the description, agent chips, "Skip setup", "Get started".
- **Agents:** "Your coding agents.", "Gespan found these on this Mac. Pick the one new threads start with.", Rescan.
  - Only installed rows, in a list with max height 286 that scrolls.
  - Status words: "Signed in", "Not signed in" with a "Sign in" button, "Signs in on first use".
  - Under the list, `not_found_line` and `found_count`.
  - With zero installed: "No coding agent found." plus "Install one, then press Rescan. You can also finish setup now and add one later."
- **Look:** "Make it yours.", "Changes show behind this card as you pick.", Theme (System/Dark/Light), Layout (Focused/Classic with the canvas sketches), Accent color (the five presets from `settings/pages/appearance.rs`), and Glass ("Let the desktop show through the window.").
- **Project:** "Pick a project to start in.", the lead, "Opened from" (only when `suggested_folder` is `Some`), "Choose a folder…" with ⌘O, and "Folder on a machine…". The primary is "Open {folder name}", disabled until a folder is set. When an earlier card was skipped, show "Using defaults: …". No Skip.

Steps:
- [ ] **Step 1: Write the failing tests.**
  - `skip_setup_jumps_to_project`: click "Skip setup" on Welcome; the last event is `Step(Project)`.
  - `project_cannot_finish_without_a_folder`: on Project, click the disabled primary; no `Finish` is emitted.
  - `project_finishes_with_the_suggested_folder`: with `launch = "/tmp/x/langfuse"`, click the suggestion and then the primary; it emits `Finish("/tmp/x/langfuse")`.
  - `project_has_no_skip`: there's no element with debug selector `setup-skip` on Project.
  - `agents_lists_only_installed`: with an installed snapshot of `[Claude, Codex]`, there are two rows and the line reads "Not found: Cursor, Grok and 8 more".
  - `rescan_during_sign_in_starts_no_second_sign_in`: with a pending `SignIn(Codex)`, clicking Rescan emits `Rescan` and no second `SignIn`.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test setup::card`.
- [ ] **Step 3: Implement `SetupCard` and the steps.**
  - Give every control a `debug_selector` named `setup-<thing>` so the tests and recordings can find it.
  - Look picks update a local `Preferences` copy and emit `Preferences` at once, so the shell restyles live.
  - Sign-in status is read off the UI thread with `cx.background_executor().spawn`.
- [ ] **Step 4: Run the tests again.** Run `cargo test setup::card`. Expected: PASS.
- [ ] **Step 5: Commit**: "Add the first-run setup card".

### Task 4: Wire the card into the shell, unclosable

**Files:**
- Modify: `crates/gespan-gpui/src/shell.rs`: add a field `setup: Option<(Entity<SetupCard>, Subscription, FocusHandle)>` beside the other modal state. Open it in `build` when `workspace.setup.is_some()`, and render it from the modal stack (~line 9892) last, so it paints above every other modal.
- Create: `crates/gespan-gpui/src/shell/setup.rs`, with `impl Shell { fn open_setup, fn setup_overlay, fn on_setup_event }`.
- Test: `crates/gespan-gpui/src/shell/setup_tests.rs`.

**Interfaces:**
- Consumes: `SetupCard` and `SetupEvent` (Task 3), `Workspace::finish_setup` (Task 1), `Shell::set_preferences`, `toggle_layout`, `Preferences::use_by_default`, `Accounts::begin_footer_auth` (via `accounts::get(cx)`), `installed_providers::probe`, `open_project`'s picker (`cx.prompt_for_paths`) and the remote project modal, and `create_session(kind, true, cx)`.
- Produces: `Shell::open_setup(&mut self, step: SetupStep, window, cx)`, used by Task 5.

Event handling:
- `Step(s)`: set `workspace.setup = Some(s)` and save. If the user is leaving Agents (by Skip
  or Skip setup) and `preferences.default_provider` is `None`, apply `skip_default` through
  `use_by_default`.
- `Preferences(p)`: `set_preferences(*p, window, cx)`.
- `Layout { classic }`: `toggle_layout` when it differs.
- `DefaultAgent(k)`: `use_by_default(k)`.
- `SignIn(k)`: `begin_footer_auth(k, &selected(k, project).id)`.
- `Rescan`: `installed_providers::probe(cx)`, then `refresh_agents`.
- `ChooseFolder`: open the picker. On a pick, call `set_folder`; on cancel, do nothing.
- `ChooseRemoteFolder`: open the remote project modal above the card, so its priority is higher.
- `Finish(path)`:
  - `remember_project_location`;
  - `workspace.finish_setup(&path, &self.cwd, launch_has_sessions)`;
  - `select_project`;
  - `create_session(self.default_kind(cx), true, cx)`;
  - drop the card, save, and show the "You're set up" toast through the existing notice stack, with the copy from the canvas.

The overlay copies `notice_modals`' What's new block, except that:
- the key handler swallows every key except ⌘O (sent to `ChooseFolder`) and quit;
- the scrim mouse-down handler does nothing;
- there's no close button.

Steps:
- [ ] **Step 1: Write the failing tests.**
  - `escape_and_scrim_clicks_leave_setup_open`.
  - `shell_shortcuts_are_blocked_while_setup_is_open`: ⌘T creates no session.
  - `finishing_setup_selects_the_project_and_opens_a_thread`.
  - `quitting_on_project_resumes_on_project`: the saved workspace JSON after `Step(Project)` restores with `setup == Some(Project)`.
  - `cancelled_folder_picker_keeps_project_disabled`.
  - `look_picks_apply_to_the_shell_live`: `Preferences` with `theme = "light"` changes `workspace.preferences.theme`.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test shell::setup_tests`.
- [ ] **Step 3: Implement**, as described above.
- [ ] **Step 4: Run the new tests, then all `shell::` tests.** Run `cargo test shell::`. Expected: PASS.
- [ ] **Step 5: Commit**: "Show first-run setup over the shell until a project is picked".

### Task 5: Run setup again

**Files:**
- Modify: `crates/gespan-gpui/src/settings/mod.rs` (`SettingsEvent`, ~line 121): add `RunSetup`.
- Modify: `crates/gespan-gpui/src/settings/pages/general.rs:260`: add a `secondary_button("run-setup", "Run setup again", …)` beside "What's new".
- Modify: `crates/gespan-gpui/src/shell/settings.rs:180`: on `RunSetup`, close settings, then `open_setup(SetupStep::Welcome, …)`.
- Test: in `shell/setup_tests.rs`.

- [ ] **Step 1: Write the failing test** `run_setup_again_opens_welcome_with_current_values_and_keeps_projects`. With two projects and `theme = "dark"`, emitting `RunSetup` opens the card on Welcome with the card's prefs theme `"dark"`. Finishing on one of the existing projects leaves both projects in place.
- [ ] **Step 2: Run it and see it fail.** Run `cargo test run_setup_again`.
- [ ] **Step 3: Implement** the button, the event and the handler.
- [ ] **Step 4: Run the test again.** Run `cargo test run_setup_again`. Expected: PASS.
- [ ] **Step 5: Commit**: "Add Run setup again to Settings".

### Task 6: Check it end to end and record it

- [ ] **Step 1: Run the full suite.** Run `cargo test` in `crates/gespan-gpui`, then `cargo test -p gespan-core`. Expected: all pass.
- [ ] **Step 2: Fresh-install run.** Start from an empty data dir: `GESPAN_NATIVE_DATA_DIR=$(mktemp -d) cargo run --profile native-dev`, launched from `$HOME`. Walk Welcome → Agents → Look → Project → a thread opens in the project.
  - Screenshot each card and compare it with the canvas boards. The sizes, colours and copy must match.
- [ ] **Step 3: Resume run.** Quit on Look, relaunch with the same data dir, and check that it opens on Look with the earlier picks kept. Quit on Project, relaunch, and check that it opens on Project.
- [ ] **Step 4: Existing-install run.** Launch with a data dir that has sessions and no workspace row (the `gespan-first-launch` fixture). Setup must not show.
- [ ] **Step 5: Record the PR video**, using the stubbed Claude binary per the repo's recording notes. Then open the PR with the issue, how it's solved and a Mermaid chart.
