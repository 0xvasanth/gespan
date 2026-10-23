# Monos → Pilots Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the "Mono" feature to "Pilot" everywhere: code, files, Rust commands, SQLite tables, storage keys, CSS, UI copy and mobile, with no behavior change.

**Architecture:** A mechanical rename done with `git mv` plus scripted, case-preserving substitution (`mono→pilot`, `Mono→Pilot`, `MONO→PILOT`, `monos→pilots`). It is driven by an explicit exclusion list and enforced afterward by a regression test that fails if a feature-level `mono` reference comes back, for example from an upstream merge.

**Tech Stack:** Tauri v2 / Rust (rusqlite), React 19 + TypeScript, Vitest, Expo mobile.

**Spec:** Decisions from the 2026-10-23 conversation (feature name "Pilots", default names `Pilot<Mascot>`). The Design Decisions section below records them; there is no separate spec doc.

## Global Constraints

- Case mapping: `mono→pilot`, `Mono→Pilot`, `MONO→PILOT`, `monos→pilots`, `Monos→Pilots`, `MONOS→PILOTS`.
- **Never rename:** `font-mono`, `--font-mono`, `monospace`, `ui-monospace`, `monokai`, `monochrome` (including `android-icon-monochrome.png`), and the `"mono"` theme in `mobile/src/ui/theme.ts` and its call sites.
- The product name stays `Rebase`. Don't touch any `rebase:` storage prefix other than the segment after it.
- Default names: `defaultPilotName("crab") === "PilotCrab"`, `defaultPilotName("mushroom") === "PilotShroom"`.
- Slash command: `/pilot` replaces `/mono`. `/operator` and `/rebase` stay. `/mono` is no longer recognized.
- Commits carry no Claude or co-author attribution (user's global CLAUDE.md).
- Run a local Cubic review before pushing or opening a PR.

## Design Decisions (my calls, flag if wrong)

- **No data migration shims.** The bundle identifier already changed to `com.rebase.desktop`, so no shipped Rebase install has `mono_*` tables, a `monos/` app-data directory, or `rebase:mono-*` keys. Tables, directory names and keys are renamed outright. In a dev build that already exists, Pilot history starts empty.
- **Bump `MONO_PROMPT_VERSION` 18 → `PILOT_PROMPT_VERSION = 19`.** The prompt text changes ("a Pilot in Rebase"), and the version constant is how the app knows to re-seed it.
- **Upstream merge cost:** after this rename, merges from upstream MonoCode will conflict throughout this feature. That's accepted, and the guard test catches stragglers.

## Review Focus

1. Tauri command name mismatch: a JS `invoke("pilot_load")` paired with a Rust `mono_load` only fails at runtime, never in tsc. Task 2 adds a test that every `invoke("…")` literal under `src/` names a registered command.
2. Over-replacement into `monospace` / `font-mono` would break the typography app-wide. The guard test asserts these survive.
3. Window labels (`pilot-chat-` prefix) must match between `window.rs`, `macos.rs` and the floating-window HTML entry. Otherwise the floating chat never opens. Covered by the manual check in Task 4.
4. The `mono-chat.html` entry renamed to `pilot-chat.html` must also be renamed in `vite.config.ts` inputs and the Rust window URL. Covered by the manual check in Task 4.
5. `/mono` typed out of habit should now be sent as plain text, not an operator command. Task 3 tests this.

---

### Task 1: Rust backend rename

**Files:**
- Rename: `src-tauri/src/mono.rs` → `pilot.rs`, `mono_transcript.rs` → `pilot_transcript.rs`, `mono_chat.rs` → `pilot_chat.rs`, `mono_chat/` → `pilot_chat/`, `codex_mono_store.rs` → `codex_pilot_store.rs`
- Modify: `src-tauri/src/lib.rs` (mod decls, `generate_handler!` list, `pilot_chat::init`, `pilot_chat::window_closed`), `src-tauri/src/session_store.rs:535,1013,1369-1370,1495-1496,2110+`, `src-tauri/src/window.rs:21`, `src-tauri/src/macos.rs`
- Test: inline `#[cfg(test)]` in `pilot_transcript.rs`

**Interfaces:**
- Produces commands: `pilot_load`, `pilot_read`, `pilot_save`, `pilot_session_get`, `pilot_session_page`, `pilot_session_upsert`, `pilot_session_find`, `codex_pilot_store_prepare`, `codex_pilot_store_copy`, `codex_pilot_store_restore_agent_state`, `pilot_chat_sync`, `pilot_chat_publish`, `pilot_chat_state`, `pilot_chat_ready`, `pilot_chat_action`, `pilot_chat_take`, `pilot_chat_accept`, `pilot_chat_reply`, `pilot_chat_keep_alive`, `pilot_chat_switch`, `pilot_menu_bar_set_visible`.
- Produces: tables `pilot_transcripts`, `pilot_turns`, `pilot_blocks`; index `pilot_blocks_user`; constants `PILOTS_DIR = "pilots"`, `PILOT_CHAT_PREFIX = "pilot-chat-"`; tray id `"pilot-menu-bar"`; events `"pilot_chat_changed"`, `"pilot_chat_request"`; Rust types `PilotPage`, `PilotTranscript`, `PilotRecord` (serde field `pilot_transcript`).
- Unchanged: `LEGACY_DIR = "agents"`.

- [ ] **Step 1: Write the failing test** in `pilot_transcript.rs` tests (create the file by `git mv` first so the test lives there):

```rust
#[test]
fn creates_pilot_tables_and_no_mono_tables() {
    let store = SessionStore::open_in_memory().unwrap();
    let conn = store.lock_conn().unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','index')").unwrap()
        .query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    for t in ["pilot_transcripts", "pilot_turns", "pilot_blocks", "pilot_blocks_user"] {
        assert!(names.iter().any(|n| n == t), "missing {t}");
    }
    assert!(!names.iter().any(|n| n.starts_with("mono_")), "stale mono table: {names:?}");
}
```

- [ ] **Step 2: Run it:** `cd src-tauri && cargo test pilot_transcript::` → FAIL (`pilot_transcripts` missing / module not found).
- [ ] **Step 3: Rename.** `git mv` the five paths, then apply the case mapping to `src-tauri/src/**/*.rs` with the exclusion list (e.g. a `perl -pi` with negative lookahead for `space|kai|chrome`). Rename test `search_keeps_older_mono_messages_available_after_conversion` → `…_pilot_…`.
- [ ] **Step 4: Verify:** `npm run check:rust` → fmt, clippy and all tests pass. `grep -rniE 'mono(?!space|kai|chrome)' -P src-tauri/src` → no output.
- [ ] **Step 5: Commit:** `git commit -m "Rename Mono backend modules, commands and tables to Pilot"`. The frontend still calls `mono_*` until Task 2, so don't run the app between Tasks 1 and 2.

### Task 2: Frontend rename (files, identifiers, keys, CSS)

**Files:**
- Rename: `src/features/monos/` → `src/features/pilots/`, plus every `mono*`/`Mono*`/`*Mono*` file inside it (e.g. `model/mono.ts` → `model/pilot.ts`, `ui/FloatingMonoChat.tsx` → `ui/FloatingPilotChat.tsx`, `model/floatingMono.ts` → `model/floatingPilot.ts`, `hooks/useMonoTranscript.ts` → `hooks/usePilotTranscript.ts`)
- Rename outside the feature: `src/app/shell/MonoRailSection.tsx(+test)` → `PilotRailSection`, `src/app/hooks/useFloatingMono.ts`, `useMonoHabits.ts` (+tests), `src/features/sessions/model/monoWorkStatus.ts(+test)`, `src/features/sessions/data/monoSessionStore.test.ts`, `src/features/sessions/ui/MonoWorkTicker.tsx(+test)`, `src/debugMonoScroll.ts`, `mono-chat.html` → `pilot-chat.html`
- Modify: every importer (App.tsx, Sidebar.tsx, TitleBar.tsx, SettingsView.tsx, AgentTranscript.tsx, `useIdleSessionDetach.ts`, `integrations/harness/core/apply.test.ts`, `tests/browser/transcript.tsx`), `src/features/settings/model/settings.ts` (section id `"pilots"`, `loadPilotsEnabled`/`savePilotsEnabled`/`subscribePilotsEnabled`), `src/styles/index.css` (`.mono-*` → `.pilot-*`), `vite.config.ts` (HTML input), every `data-mono*` attribute
- Create: `src/test/invokeCommands.test.ts`

**Interfaces:**
- Consumes: command names from Task 1.
- Produces: `Pilot`, `PilotProject`, `PilotLook`, `PilotStatus`, `PilotState`, `PILOT_COLORS`, `PILOT_STATUS_LABEL`, `listPilots`, `findPilot`, `pilotForSession`, `isPilotSession`, `createPilot`, `updatePilot`, `removePilot`, `subscribePilots`, `pilotsSnapshot`, `defaultPilotName`, `PilotSessionCompletionResult`, `PilotSessionCompletionBatches`, and so on (a 1:1 mapping of the existing exports). Storage keys: `rebase:pilot-roster`, `rebase:pilots-changed`, `rebase:pilot-handed`, `rebase:open-pilot`, `rebase:pilot-files-changed`, `rebase:pilot-habits-active`, `rebase:pilot-habits-check-now`. Legacy keys `rebase:monos` / `rebase:mono-profiles` read by `migrateLegacyPilotStorage` become `rebase:pilots` / `rebase:pilot-profiles` (no shipped data, see Design Decisions).

- [ ] **Step 1: Write the failing test** `src/test/invokeCommands.test.ts`: read `src-tauri/src/lib.rs`, extract the identifiers inside `generate_handler![…]` (last path segment), glob `src/**/*.{ts,tsx}` excluding tests, extract every `invoke<…>?("name"` literal, and assert:

```ts
expect(unknown).toEqual([]);           // every invoked name is registered
expect(registered.filter(n => /mono/.test(n))).toEqual([]);
```

- [ ] **Step 2: Run:** `npx vitest run src/test/invokeCommands.test.ts` → FAIL, listing `mono_load`, … as unknown.
- [ ] **Step 3: Rename.** Use `git mv` for files and dirs, then apply the case mapping across `src/`, `tests/`, `vite.config.ts`, `index.html` and `pilot-chat.html` with the exclusion list. Update `defaultPilotName` so its template prefix is `Pilot`.
- [ ] **Step 4: Update test expectations** that hardcode names: `pilot.test.ts:132-133` (`"PilotCrab"`, `"PilotShroom"`), `ProjectBackgroundDialog.test.ts:66`, `AgentTranscript.inlineWork.test.ts:294,398`.
- [ ] **Step 5: Verify:** `npm run check:web` → all Vitest suites and `tsc --noEmit` pass, including `invokeCommands.test.ts`.
- [ ] **Step 6: Commit:** `git commit -m "Rename Monos feature to Pilots in the frontend"`

### Task 3: User-facing copy, prompt and slash command

**Files:**
- Modify: `src/features/pilots/model/pilotFiles.ts` (`PILOT_PROMPT_VERSION = 19`, `"a Pilot in Rebase"`), `src/features/sessions/model/operatorCommand.ts`, `src/features/settings/ui/SettingsView.tsx` (labels "Show pilots", "Pin pilots to the icon rail", "Your pilots"), `src/features/pilots/ui/PilotIntro.tsx` and any remaining user-visible string
- Test: `src/features/sessions/model/operatorCommand.test.ts`, `src/features/pilots/model/pilotFiles.test.ts`

- [ ] **Step 1: Write failing tests.**

```ts
// operatorCommand.test.ts
it.each(["/pilot fix it", "/operator fix it", "/rebase fix it"])("treats %s as operator", (t) => {
  expect(parseOperatorCommand(t)).not.toBeNull();   // use the module's existing parser export
});
it("no longer treats /mono as an operator command", () => {
  expect(parseOperatorCommand("/mono fix it")).toBeNull();
});
// pilotFiles.test.ts
it("introduces the agent as a Pilot", () => {
  expect(pilotContext(/* existing fixture */)).toContain("a Pilot in Rebase");
  expect(pilotContext(/* fixture */)).not.toMatch(/\bMono\b/);
});
```

- [ ] **Step 2: Run:** `npx vitest run src/features/sessions/model/operatorCommand.test.ts src/features/pilots/model/pilotFiles.test.ts` → FAIL.
- [ ] **Step 3: Implement.** Set both regexes to `/^\s*\/(?:operator|pilot|rebase)(?=\s|$)\s*/i` and `LEGACY_COMMAND = /^\s*\/(?:pilot|rebase)(?=\s|$)\s*/i`. Update the copy and bump the version.
- [ ] **Step 4: Verify:** the same command → PASS.
- [ ] **Step 5: Commit:** `git commit -m "Update Pilot copy, prompt version and /pilot command"`

### Task 4: Mobile, guard test and end-to-end verification

**Files:**
- Rename: `mobile/src/app/mono/[monoId].tsx` → `mobile/src/app/pilot/[pilotId].tsx` (`PilotScreen`, `PilotTranscript`)
- Modify: `mobile/src/features/connection/{WorkspacePicker.tsx,selection.ts}` (`PILOTS_ENABLED`, `pilotIds`, `PilotMascot`), `mobile/src/app/index.tsx`, `mobile/src/app/settings/*` ("Projects and pilots"), `mobile/tests/*`. Do **not** touch `mobile/src/ui/theme.ts`'s `"mono"` theme.
- Create: `src/test/noMonoReferences.test.ts`

- [ ] **Step 1: Write the guard test.** Walk `src/`, `src-tauri/src/`, `mobile/src/`, `mobile/tests/`, `tests/`, the root `*.html` and `vite.config.ts`. Strip allowed tokens with `/font-mono|monospace|monokai|monochrome/gi`, skip `mobile/src/ui/theme.ts` and any line matching `theme.*["']mono["']`, then:

```ts
expect(offenders).toEqual([]);   // offenders: `${file}:${line}` where /mono/i still matches
expect(read("src/styles/index.css")).toMatch(/font-mono|monospace/); // typography survived
```

- [ ] **Step 2: Run:** `npx vitest run src/test/noMonoReferences.test.ts` → FAIL listing the mobile files.
- [ ] **Step 3: Rename the mobile files** with the same mapping. Then `cd mobile && npx tsc --noEmit && npm test` → pass.
- [ ] **Step 4: Verify everything:** `npm run check` → pass; guard test → PASS.
- [ ] **Step 5: Manual run:** `npm run tauri dev`. Create a Pilot and confirm it's named `Pilot<Mascot>`. Open the floating Pilot chat and the menu-bar item, send a message, restart, and confirm the transcript reloads. `sqlite3 <app data>/…db ".tables"` should show `pilot_*`. App data should contain `pilots/<id>/SOUL.md`.
- [ ] **Step 6: Commit:** `git commit -m "Rename mobile Monos to Pilots and guard against stray mono references"`
- [ ] **Step 7:** Run the local Cubic review on the branch and address every finding before pushing.
