# GPUI Adoption and Rebase Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring upstream's GPUI native app (`hardbeat920/monocode` branch `mc/migrate-monocode-to-gpui`) into Rebase. Fully rebrand it to Rebase with Pilots, so it shares `rebase.db` with Tauri Rebase. Retire the app's MonoCode `mc/` branch and folder prefix.

**Architecture:** Merge `upstream/gpui` (fetched as `refs/remotes/upstream/gpui`, head `d9e6e230`) into `feat/pilots-flows-board`. A dry run shows one conflict, in `scripts/test-remote-ssh.py`. After the merge, rename the two crates and every MonoCode identity and data contract in them. The Mono feature becomes Pilot in the GPUI code, matching what Tauri Rebase already did. Tauri Rebase stays the shipping app, and the GPUI app ships as the "Rebase Native" preview.

**Tech Stack:** Rust workspace + vendored GPUI (`vendor/gpui-pre*`), Tauri v2, React/TS, Bun, Vitest.

**Spec:** User decisions from 2026-10-23:
- Merge GPUI.
- Rename the GPUI crates and names too, not just identity.
- Rename the app branch/folder prefix.
- Leave the on-disk `monocode-worktrees/` folder and the `refs/monocode/*` checkpoint refs alone.

The GPUI evaluation findings are summarized in the Context section below.

## Global Constraints

- **Tooling:** Bun for UI tooling (`bun run <script>`, `bunx <tool>`), Vitest for tests, Cargo for Rust. The native scripts are `bun run native:build`, `bun run native:check` and whatever `docs/native/README.md` lists after the merge. Xcode 27.1 is installed with its license accepted.
- **Name mapping** (same rules as the Pilots rename):
  - `monocode→rebase`, `MonoCode`/`Monocode→Rebase`, `MONOCODE→REBASE`
  - `mono→pilot`, `Mono→Pilot`, `MONO→PILOT`, plus plurals
  - `mc/` and `mc-` (MonoCode short prefix) → `rebase/` and `rebase-`
- **Never rename:** `font-mono`, `--font-mono`, `monospace`, `ui-monospace`, `monokai`, `monochrome`, `mono(repo|tonic|lith|graph|poly|type|polize)`, font family names (SF Mono, JetBrains Mono, Menlo, …), the vendored GPUI tree `vendor/**` (third-party code), `CHANGELOG.md` history, `@hardbeat920` credits, and the `"mono"` mobile theme.
- **Target names:**
  - Crates: `crates/monocode-core` → `crates/rebase-core` (package `rebase-core`, lib `rebase_core`), and `crates/monocode-gpui` → `crates/rebase-gpui` (package `rebase-gpui`).
  - App: display name `Rebase Native`, bundle `Rebase Native.app`, `APP_ID = "com.rebase.native"`.
  - Native release tags `native-v*` stay, and the feed is GitHub Releases (see Task 5).
- **Data contracts shared with Tauri Rebase** (they must match exactly):
  - DB: `<app data>/com.rebase.desktop/rebase.db`, with tables `pilot_transcripts`, `pilot_turns`, `pilot_blocks`.
  - Webview storage dir: `com.rebase.desktop`.
  - localStorage keys `rebase:*`, including `rebase:pilot-roster`, `rebase:pilots-changed`, `rebase:open-pilot`.
  - Pilot files dir: `pilots/`.
  - Mobile JSON keys: `pilots`, `pilotId`, `pilotHabit`, with the `pilot:` prefix.
  - Checkpoint refs: `refs/rebase/checkpoints/…`.
- **Legacy recognition** is the only compatibility shim allowed. Branch-ownership checks accept both `rebase/` and `mc/` (and `rebase/orch-` / `mc/orch-`), so existing user worktrees keep working. New names always use `rebase/`.
- Commits carry no Co-Authored-By trailer and no AI attribution. No local Cubic review. Never stage the user's uncommitted `README.md` edit.

## Context (from the GPUI evaluation)

- **What upstream adds:**
  - `crates/monocode-core`: 140 `.rs` files, with no Tauri dependency.
  - `crates/monocode-gpui`: 250 `.rs` files.
  - `vendor/gpui-pre*`: patched GPUI 0.3.9.
  - `docs/native/*`, `.github/workflows/native.yml`, `.github/workflows/native-release.yml`, `scripts/package-native.mjs`, plus `native:*` scripts in `package.json`.
  - The crates are excluded from the root Cargo workspace.
- **MonoCode-specific contracts the GPUI code reads today:**
  - `com.monocode.desktop/monocode.db` and the `mono_*` tables (`store/monos.rs`)
  - the WebKit dir `com.monocode.desktop` and `monocode:*` keys (4 keys)
  - mobile `monos` (`mobile/host.rs`)
  - `APP_ID = "dev.usemono.gpui"` (`app_update/{mod,install}.rs`)
  - the bundle name "Monocode Native" (`scripts/package-native.mjs`)
  - an R2 `native-preview/latest.json` feed (`native-release.yml`)
- **Size:** 2,572 feature `mono` references across 131 files, and 1,763 `monocode` references across the crates, scripts, docs and CI.
- **Running it:** `--check-data` reports the DB it would open, read-only. A launch with an empty temp `HOME` creates an isolated DB.

## Review Focus

1. **The GPUI app opens the wrong DB, or the right DB with the wrong tables.** It would silently show no sessions or Pilots. Task 2 adds a test pinning the resolved path and table names, and Task 6 runs `--check-data` against the real `rebase.db`.
2. **Over-replacement inside vendored GPUI or font names.** Breaks the build or the typography. `vendor/**` is excluded, and the guard covers font patterns (Task 3).
3. **`mc/` legacy recognition dropped.** The user's live `mc/*` worktrees would no longer be treated as app-owned. Task 4 tests both prefixes.
4. **The Tauri and GPUI apps disagree on a shared string** (DB path, keys, mobile JSON). A cross-check test compares constants from both sides (Task 2).
5. **The merge silently reverts our earlier work in shared files** (`package.json`, `Cargo.toml`, `scripts/test-remote-ssh.py`). Task 1 diffs those files against the pre-merge versions.

---

### Task 1: Merge upstream GPUI

**Files:** the merge commit. Resolve `scripts/test-remote-ssh.py` by keeping our `rebase-host` naming and adding upstream's `--native` support.

- [ ] **Step 1:** `git merge --no-ff upstream/gpui -m "Merge upstream GPUI migration branch (mc/migrate-monocode-to-gpui @ d9e6e230)"`. Expected: exactly one conflict, in `scripts/test-remote-ssh.py`.
- [ ] **Step 2:** Resolve the conflict so the result has both our naming and their native flag.
- [ ] **Step 3:** Verify nothing of ours was reverted. `git diff HEAD^1 HEAD -- package.json Cargo.toml src src-tauri mobile host scripts/release-channel.cjs .github/workflows/release.yml CLAUDE.md` must show only upstream's additions: the `native:*` scripts, the workspace `exclude`, and the native test flag.
- [ ] **Step 4:** Gates, all on the unrenamed code: `bun run check` (Tauri side), and `bun run native:check` plus `bun run native:build` as named in `docs/native/README.md`. This baseline proves the merge itself is sound before any renames.
- [ ] **Step 5:** Commit the merge. The message has no attribution.

### Task 2: GPUI identity and shared data contracts

**Files:** Modify the GPUI files that define:
- the data directory and DB name (grep `com.monocode.desktop`, `monocode.db`)
- the WebKit storage dir
- the localStorage key constants
- the `mono_*` table names in `store/monos.rs` and any SQL
- the Pilot files dir (`monos` → `pilots`)
- the mobile JSON contract in `mobile/host.rs`
- `APP_ID`
- the checkpoint ref prefix
- the dev data dir name ("Rebase Development")

Also create `crates/monocode-core/tests/shared_contracts.rs`, or a `#[cfg(test)]` module in the crate's store (the crate rename happens in Task 3).

**Interfaces:**
- Produces: core constants that Task 6 verifies, and that must equal Tauri's:
  - DB path `com.rebase.desktop/rebase.db`
  - tables `pilot_transcripts`, `pilot_turns`, `pilot_blocks`
  - storage dir `com.rebase.desktop`
  - keys `rebase:pilot-roster`, …
  - mobile keys `pilots`, `pilotId`, `pilotHabit`, and the `pilot:` prefix
  - `refs/rebase/checkpoints/`
  - `APP_ID = "com.rebase.native"`

- [ ] **Step 1: Write failing tests:**
  - `shared_db_path_is_rebase`: the resolved DB path ends with `com.rebase.desktop/rebase.db`.
  - `pilot_tables_match_tauri`: an in-memory store created by core's migrations has `pilot_transcripts`, `pilot_turns` and `pilot_blocks`, and no `mono_*` tables.
  - `storage_keys_are_rebase`: every localStorage key constant starts with `rebase:`, and the Pilot roster key equals `"rebase:pilot-roster"`.
  - `mobile_contract_uses_pilots`: a serialized mobile catalog has a `pilots` key and `pilot:` ids.
  - `checkpoint_refs_are_rebase`.
  - **Cross-app check:** a Vitest test `src/test/sharedContracts.test.ts`. It reads the Tauri constants (`src-tauri` DB name, table names, the `src/features/pilots` key constants) and the GPUI constants (grep the Rust source for the literal values), then asserts they're equal.
- [ ] **Step 2: Run** `cargo test --manifest-path crates/monocode-core/Cargo.toml shared` and `bunx vitest run src/test/sharedContracts.test.ts`. Expected: FAIL.
- [ ] **Step 3: Implement.** Change the constants and SQL to the shared contracts. Upstream has no legacy migration from `com.monocode.desktop`, so don't add one.
- [ ] **Step 4: Verify** with the same commands plus `bun run native:check`. Expected: PASS.
- [ ] **Step 5: Commit:** `git commit -m "Point the GPUI app at Rebase data and Pilot contracts"`

### Task 3: Rename the GPUI crates and names to Rebase and Pilots

**Files:**
- `git mv crates/monocode-core crates/rebase-core` and `git mv crates/monocode-gpui crates/rebase-gpui`.
- Update the package and lib names, and every `use monocode_core::`.
- Update the root `Cargo.toml` `exclude` entries, the `package.json` `native:*` scripts, `scripts/package-native.mjs` (bundle `Rebase Native.app`, display name `Rebase Native`), `docs/native/*`, and `.github/workflows/native*.yml` paths.
- Rename the Mono feature to Pilot inside the crates: modules `mono*` → `pilot*`, the `monos/` dirs → `pilots/`, types `Mono*` → `Pilot*`, and UI strings ("Monos" → "Pilots", `Mono<Mascot>` defaults → `Pilot<Mascot>`). Rename the remaining MonoCode → Rebase strings.
- Extend the guard tests `src/test/noMonoReferences.test.ts` and `src/test/upstreamLinks.test.ts` to scan `crates/`, `docs/native/` and the `native*.yml` workflows, with `vendor/**` and `crates/*/target` excluded.

- [ ] **Step 1: Extend the guards first.** Run `bunx vitest run src/test/noMonoReferences.test.ts src/test/upstreamLinks.test.ts`. Expected: FAIL, listing the GPUI offenders.
- [ ] **Step 2: Rename** with `git mv` and case-preserving substitution, applying the never-rename list. Afterwards, check that nothing was over-replaced: `git grep -nP 'pilot(space|kai|chrome|repo|tonic|lith|graph|poly|type|polize)|font-pilot|(SF|JetBrains|Liberation|Nerd Font)\s?Pilot' -- crates docs/native` must print nothing.
- [ ] **Step 3: Verify:**
  - the guards pass
  - `bun run native:check` passes (all crate tests)
  - `bun run native:build` produces `Rebase Native.app`
  - `bun run check` passes (Tauri)
- [ ] **Step 4: Commit:** `git commit -m "Rename the GPUI crates and app to Rebase with Pilots"`. One commit is fine because it's a mechanical rename; a separate preceding commit for the guard extension is also fine.

### Task 4: Rebase branch and folder prefix (`mc/` → `rebase/`)

**Files:**
- Modify `src/features/source-control/model/worktrees.ts:79-92` (new names `rebase/<token>`, `rebase/orch-<token>`; the sanitizer emits `rebase/<clean>`) and `src-tauri/src/worktrees.rs:417,744,758`.
- Modify `host/git-worktrees.ts` and the matching GPUI core worktree code (grep `"mc/"`, `mc/orch-`, `mc-` in `crates/rebase-core`).
- Update the tests in `src-tauri/src/worktrees.rs`, `host/git-worktrees.test.ts`, `src/features/source-control/ui/Worktrees.test.ts`, `src/features/sessions/ui/Composer.test.ts`, `src/features/files/ui/FileTree.test.ts` and `src/features/orchestration/model/orchestration.test.ts`.

**Interfaces:**
- Produces `isAppBranch(name)` / `is_app_branch(name)`: true for `rebase/…` and legacy `mc/…`. Orchestration detection is true for `rebase/orch-…` and legacy `mc/orch-…`. Generated names always use `rebase/`.

- [ ] **Step 1: Write failing tests:**
  - New worktree branch names start with `rebase/`, and their folder names start with `rebase-`.
  - Orchestration worker branches start with `rebase/orch-`.
  - Renaming or cleanup still works on an existing `mc/12345678` branch (the legacy case).
  - Orchestration detection accepts `mc/orch-x` and `rebase/orch-x`, and rejects `feature/x`.
  - In the GPUI core, the same three behaviours.
- [ ] **Step 2: Run** the focused tests on each side (`cargo test worktrees`, the GPUI core tests, and `bunx vitest run` on the TS files). Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify** `bun run check`, `bun run native:check` and `bun run test:host`. Expected: PASS.
- [ ] **Step 5: Commit:** `git commit -m "Name new worktree branches rebase/ and keep recognizing legacy mc/ branches"`

### Task 5: Native release feed and icons

**Files:**
- `.github/workflows/native-release.yml`: publish the native `latest.json` as an asset on the `native-v*` GitHub release. The default endpoint is `https://github.com/0xvasanth/rebase/releases/download/<tag>/latest.json`, resolved through a `releases/latest`-equivalent for the native channel. Mirror Task L's approach: R2 optional and the pubkey required. Use the tested helpers in `scripts/release-channel.cjs` where they apply.
- The native updater endpoint constant in the GPUI core (`app_update`).
- Native app icons: point `scripts/package-native.mjs` at our purple icons (`src-tauri/icons/icon.icns` or a copy under `crates/rebase-gpui/assets`), and recolor any GPUI in-app logo assets using the same luminance mapping as the Tauri icons (dark → `#4C1D95` through light → `#EDE9FE`).

**Context:** GitHub's `releases/latest` serves only the newest non-prerelease release, and Tauri stable tags `v*` share it. The native feed therefore needs either its own stable URL or an endpoint the workflow keeps updated. Pick the simplest correct option and document it in `docs/native/README.md`: for example, a fixed release tag `native-latest` whose `latest.json` asset the workflow overwrites, so the URL is always `…/releases/download/native-latest/latest.json`.

- [ ] **Step 1: Write failing tests:**
  - In `scripts/release-channel.test.cjs`: the native feed URL resolves to the chosen GitHub URL.
  - A missing pubkey still errors.
  - A partial-R2 beta config still fails fast (if the native workflow has a beta channel).
- [ ] **Step 2: Run** `node --test scripts/release-channel.test.cjs`. Expected: FAIL.
- [ ] **Step 3: Implement** the workflow change, the constant change and the icon wiring. Validate the YAML.
- [ ] **Step 4: Verify:**
  - the tests pass
  - `bun run native:build` passes
  - the bundle's icon is purple: render it with `NSWorkspace` and check that it has purple pixels and no neutral-white grid
  - `bun run check` passes
- [ ] **Step 5: Commit:** `git commit -m "Ship Rebase Native from GitHub Releases with purple icons"`

### Task 6: End-to-end verification and install

Controller-run. No implementer.

- [ ] Run the full gates: `bun run check`, `bun run native:check` and `bun run test:host`.
- [ ] Build both apps: `bun run tauri build --bundles app` and `bun run native:build`.
- [ ] Run `Rebase Native --check-data` read-only. It must report `com.rebase.desktop/rebase.db` with the existing session count, the Pilot tables and no migration.
- [ ] Install `Rebase.app` and `Rebase Native.app` into `/Applications`, moving the previous copies to the Trash. Launch both, check the icons, and check that the same sessions appear in each.
- [ ] Re-raise the PR to `main` on `0xvasanth/rebase`, after the user confirms. Pushing and opening the PR are visible actions.
