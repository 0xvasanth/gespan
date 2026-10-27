# Kiro CLI in Gespan Native — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kiro CLI works as a provider in the native app, as it already does in the Tauri app: detected, launched over ACP, streaming, permissions, models, resume and cancel.

**Architecture:** Kiro speaks the Agent Client Protocol (`kiro-cli acp`). Native already runs Hermes, Grok, Cursor, Devin and Antigravity through `crates/gespan-core/src/provider/acp.rs`, so the port is:
1. a new `Kind::Kiro`, registered wherever native lists providers;
2. a few Kiro-specific ACP behaviours, matching the Tauri adapter (`src/integrations/harness/providers/kiro/`);
3. the GPUI icon and the default model entry.

**Tech Stack:** Rust, `gespan-core`, `gespan-gpui`, cargo `native-dev` profile.

**Spec:** `docs/superpowers/specs/2026-10-23-kiro-cli-provider-design.md` (the Tauri design; its decided behaviours apply to native unchanged). Port facts were gathered from the Tauri adapter on 2026-10-25.

## Global Constraints

- Provider id `kiro`, label `Kiro CLI`, binary `kiro-cli` (never the `kiro` IDE launcher). The env override is `GESPAN_KIRO_BINARY`.
- `Kind::ALL` order: insert Kiro right after Hermes. `ALL` becomes 12 long.
- Missing-binary text, exactly: `Kiro CLI not found. Install it with \`curl -fsSL https://cli.kiro.dev/install | bash\` and run \`kiro-cli login\`, then retry.`
- Auth help, exactly: `Run \`kiro-cli login\` (check with \`kiro-cli whoami\`).`
- Launch: `kiro-cli acp --agent-engine=v3`. If the process exits before `initialize` completes, retry once with `kiro-cli acp`. A live process that times out fails without a retry.
- No `authenticate` stage. Kiro's own mode and agent stay at their defaults (no `session/set_mode`). No steering (`can_steer` stays false for Kiro).
- Tests: `cargo test --locked --manifest-path crates/gespan-gpui/Cargo.toml -p <crate> --profile native-dev -- --test-threads=1 <filter>`. Clippy `-D warnings`, `cargo fmt --check`, and `bun run native:test` at the end. Use only the name Gespan (`src/test/legacyNames.test.ts`).
- Git: stage only; commit only when the user says so; no Claude attribution.

## Review Focus

1. **`kiro-cli` missing**: a clear, actionable message (the exact missing-binary text), not a generic spawn error. *(Task 1 test.)*
2. **Signed out**: when Kiro's stderr reports an auth failure, the session error ends with the auth help, not a bare JSON-RPC error. *(Task 2 test.)*
3. **An old `kiro-cli` without v3**: the process exits on `--agent-engine=v3`, and the retry with bare `acp` succeeds. *(Task 2 test.)*
4. **Resuming a session Kiro no longer has**: `session/load` fails and a new session replaces it instead of erroring. *(Task 2 test.)*
5. **Cancel during a pending approval**: the approval is answered `cancelled`, not `reject`. *(Task 2 test.)*

---

### Task 1: Register Kiro as a native provider (core)

**Files:**
- `crates/gespan-core/src/provider/kind.rs`:
  - add `Kiro` to the enum and `ALL` (after Hermes, length 12);
  - `id` → `"kiro"`; `label` → `"Kiro CLI"`; `binary_name` → `"kiro-cli"`;
  - `login_hint` → `"kiro-cli login"`.
- `crates/gespan-core/src/remote/protocol.rs` (`supported_providers`, add `"kiro"` after `"hermes"`) and `crates/gespan-core/src/pilot/work.rs` (the harness list).
- `crates/gespan-core/src/provider.rs`: the discovery timeout match, `Kind::Kiro => Some(30)`. Find the missing-binary message path (the generic text is at `gespan-gpui/src/runtime.rs:~399`) and give Kiro the exact Global Constraints text.
- Decide Kiro membership, matching the Tauri lists: `provider/commands.rs`, `session_title.rs`, `store/side_threads.rs`, `gespan-gpui/src/conversation/btw.rs`, `skills.rs`, `desktop_preferences/session_defaults.rs`, `gespan-gpui/src/composer_completion.rs`. Copy the Tauri behaviour where it exists, else leave Kiro out. Record each decision in the report.
- Tests that change:
  - `store/shared/tests.rs` (~451-475) and `flow/definition.rs` (~984) use `"kiro"` as an unknown harness. Switch them to a still-unknown name, e.g. `"nonexistent"`.
  - `remote.rs:~842` and `gespan-gpui/src/runtime/remote/tests.rs:~83` assert the supported providers. Add kiro.

- [ ] **Step 1: Write the failing tests:**
  - `kiro_is_a_known_acp_provider_named_kiro_cli`: parse, id, label, binary `kiro-cli`, `is_acp`, position after Hermes, `ALL.len() == 12`.
  - `kiro_missing_binary_message_is_actionable`: the exact text.
  - `remote_supported_providers_include_kiro`.
- [ ] **Step 2: Run** `… -p gespan-core … provider::kind` (and the others). Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** core and gpui suites for the touched modules. Expected: PASS.
- [ ] **Step 5: Stage.**

### Task 2: Kiro's ACP behaviour (core)

**Files:**
- `crates/gespan-core/src/provider/acp.rs`
- `crates/gespan-core/src/provider.rs` (stderr handling, if that's where `acp_failure` details come from)
- Tests inline in `acp.rs` (pattern: `protocol(kind)`) and process-level fake-binary tests in `provider.rs` (pattern near `cancellation_reaps_child_and_descendant_promptly`).

**Behaviour:**
- **Launch args (`configure`):** `Kind::Kiro => ["acp", "--agent-engine=v3"]`.
  - If the child exits before the `initialize` response, relaunch once with `["acp"]` and continue.
  - A timeout, or an error reply, does not retry.
- **Timeouts:** like Hermes. Initialize and configure: 20 s. Session new/load: 45 s. Prompt: 30 min. Catalog probe: 30 s overall, 20 s per request.
- **Load failure replaces the session:** add Kiro to the `replace` match in `acp_failure`.
- **Cancel:** add Kiro to the matches that answer pending approvals `cancelled` (as Devin and Antigravity do). Keep native's existing cancel flow (send `session/cancel`, mark done) and do not add Tauri's 8 s settle wait (controller ruling: consistent with every other native ACP provider).
- **Models:**
  - Add Kiro to the `default` skip filter, so `kiro:default` sends no model change.
  - Use `session/set_config_option` when `configOptions` has a `model` entry (id or category), else `session/set_model`.
- **Auth hint:** port the Tauri `kiroStderrAuthError` / `AUTH_STDERR` matching (read `src/integrations/harness/providers/kiro/kiroProtocol.ts` for the exact regex and its debug/info/warn line filter).
  - When Kiro's stderr shows an auth failure, or startup fails with an auth detail, the error shown is `"Kiro CLI: <detail> <auth help>"`.
  - A startup timeout reads `"Kiro CLI did not start. <auth help>"`.
- **Prompt blocks:** images only if `promptCapabilities.image`; otherwise a text block `Attached image (read from disk): "<path>"`. Check that native's ACP prompt building already does this for every kind; if so, nothing to add.

- [ ] **Step 1: Write the failing tests:**
  - `kiro_launches_v3_and_falls_back_to_plain_acp_when_v3_exits` (fake binary that exits on `--agent-engine=v3`).
  - `kiro_does_not_retry_a_live_process_that_times_out`.
  - `kiro_replaces_a_missing_session_on_load`.
  - `kiro_cancel_answers_pending_approvals_cancelled`.
  - `kiro_default_model_sends_no_model_change` and `kiro_selects_models_through_config_options_then_set_model`.
  - `kiro_signed_out_errors_end_with_the_login_help` (stderr auth line; also a debug-tagged auth line is ignored).
  - The existing `every_acp_provider_runs_its_native_session_lifecycle` now covers Kiro too.
- [ ] **Step 2: Run** `… -p gespan-core … provider::`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run.** Expected: PASS.
- [ ] **Step 5: Stage.**

### Task 3: GPUI surface and docs

**Files:**
- Create `crates/gespan-gpui/assets/kiro.svg`, copied from `src/assets/providers/kiro.svg` (monochrome).
- `crates/gespan-gpui/src/theme.rs` `provider_icon`: add a `"kiro"` arm, monochrome like Hermes (same scale approach).
- Model picker: add a `kiro:default` "Configured model" entry, as Tauri does (`src/.../models.ts:~198,227`). Find where native builds per-kind default entries (catalog.rs, pickers.rs) and follow the Hermes/Devin pattern if one exists.
- `gespan-gpui/src/runtime/visual_discovery.rs` (~186): add Kiro to the MCP-hint test list if Kiro takes the generic ACP path.
- `docs/native/parity.md`: "All twelve providers", plus a Kiro row ("ACP, no live check").

- [ ] **Step 1: Write the failing tests:**
  - `kiro_has_its_own_monochrome_icon` (`provider_icon("kiro")` isn't the codex fallback).
  - `kiro_model_picker_offers_the_configured_model` (`kiro:default`, label "Configured model", first).
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run gpui `settings::`, `pickers`, and the touched tests; then the gates:** clippy, fmt check, `bun run native:test`, `bunx vitest run src/test/legacyNames.test.ts`. Expected: all green.
- [ ] **Step 5: Stage.**
