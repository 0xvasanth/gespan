# Kiro CLI Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `kiro` (Kiro CLI) as a built-in ACP provider, wired everywhere Hermes is.

**Architecture:** A `providers/kiro/` folder (adapter, live session, protocol parsing, catalog) modelled on `providers/hermes/`, reusing `core/acp.ts` and the shared ACP helpers in `grokProtocol.ts`. Control surface (model selection) is chosen from the session result, not the CLI version. Rust gets a `kiro-cli` resolver.

**Tech Stack:** TypeScript, vitest, Tauri/Rust (`src-tauri/src/harness.rs`), React Native (`mobile/`).

**Spec:** `docs/superpowers/specs/2026-10-23-kiro-cli-provider-design.md`

## Global Constraints

- Provider id `kiro`; label `kiro`; title `Kiro CLI`; binary `kiro-cli` only (never the `kiro` IDE launcher).
- Launch `kiro-cli acp --agent-engine=v3`, falling back once to bare `kiro-cli acp`. `protocolVersion: 1`.
- `clientCapabilities`: `fs` read/write false, `terminal` false. `mcpServers: []`.
- Unknown agent→client requests get JSON-RPC `-32601`.
- Never synthesize a permission option the agent did not offer; cancelled permission answers `{outcome: "cancelled"}`.
- Auth help text ends: "Run `kiro-cli login` (check with `kiro-cli whoami`)."
- Install hint `curl -fsSL https://cli.kiro.dev/install | bash`, commented as unverified.
- Do not touch `core/jsonRpc.ts` (error codes are dropped there; classify by message).
- No Claude/Anthropic attribution in commits. Cubic review before any push to `main`.

## Review Focus

1. Kiro exits during `initialize` on the V3 flag (old CLI) → falls back to bare `acp`, one retry only (Task 4).
2. User cancels while a permission is pending → permission answered `cancelled`, next prompt not sent until old one settles (Task 4).
3. Resume via `session/load` replays history → none of it is re-emitted (Task 4).
4. `session/new` returns neither `models` nor `configOptions` → no crash, picker keeps `kiro:default` (Tasks 2, 3).
5. Signed-out CLI → error mentions `kiro-cli login`, not a raw RPC string (Task 2, 4).

---

### Task 1: Rust resolver for `kiro-cli`

**Files:**
- Modify: `src-tauri/src/harness.rs` (add `harness_resolve_kiro`, `resolve_kiro`, entries in `is_harness_argv_token`, `resolve_harness_binary_default`, `resolve_harness_binary_override`, `validate_harness_binary_version` marker)
- Modify: `src-tauri/src/lib.rs:502` (register command)

**Interfaces:**
- Produces: tauri command `harness_resolve_kiro() -> Result<CursorBinary, String>`; provider key `"kiro"` accepted by `harness_resolve_configured`.

- [ ] **Step 1: Failing tests** in the existing `#[cfg(test)]` module: `looks_like_harness_argv("/Users/n/.local/bin/kiro-cli acp")` is true; override with `kiro-cli` filename accepted and `kiro` filename rejected; version marker requires `kiro` in `--version` output (mirror the existing hermes/devin tests).
- [ ] **Step 2:** `cargo test -p <crate> kiro` → fails (unknown provider).
- [ ] **Step 3: Implement** `resolve_kiro()` with candidates `~/.local/bin/kiro-cli`, `~/.kiro/bin/kiro-cli`, `~/.npm-global/bin`, `/opt/homebrew/bin` (macOS), `/usr/local/bin`, `/usr/bin`, `LOCALAPPDATA/kiro-cli/kiro-cli` (windows), then `which_via_login_shell("kiro-cli")`, via `first_binary`. Error text: "Kiro CLI not found. Install it with `curl -fsSL https://cli.kiro.dev/install | bash` and run `kiro-cli login`, then retry."
- [ ] **Step 4:** `cargo test` (harness tests) → PASS; `cargo fmt --check`; `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `Resolve the Kiro CLI binary in the desktop host`.

### Task 2: Protocol module (pure)

**Files:**
- Create: `src/integrations/harness/providers/kiro/kiroProtocol.ts`
- Test: `src/integrations/harness/providers/kiro/kiroProtocol.test.ts`

**Interfaces (all exported; `asRecord` re-exported from hermesProtocol is NOT reused — define locally):**
- `KIRO_AUTH_HELP: string`, `KIRO_LAUNCH_ARGS: readonly [readonly string[], readonly string[]]` = `[["acp","--agent-engine=v3"],["acp"]]`
- `kiroStderrAuthError(line: string): string | null`
- `kiroStartupError(error: unknown): Error`
- `kiroSessionId(result: unknown): string | undefined`
- `kiroInitCapabilities(init: unknown): { loadSession: boolean; image: boolean; embeddedContext: boolean }`
- `kiroPromptBlocks(text: string, attachments: Attachment[], caps: { image: boolean }): PromptContentBlock[]` (drop image blocks when `!caps.image`, keeping a text path note instead)
- `type KiroModelControl = { kind: "config"; configId: string; current?: string } | { kind: "setModel"; current?: string } | { kind: "none" }`
- `kiroModelControl(session: unknown): KiroModelControl`
- `modelsFromKiroSession(session: unknown): AgentModel[]` (ids `kiro:<nativeId>`; reads `configOptions` entry with id/configId `model` (options `{value,name}`), else `models.availableModels`; current first; deduped; `[]` if neither)
- `kiroIsReplay(params: unknown): boolean` (`update._meta.kiro.replay === true`)
- `kiroContextUsage(params: unknown): number | undefined` (`session_info_update` with `_meta.kiro.kind === "context_usage"` → `usagePercentage`)

- [ ] **Step 1: Failing tests** (names + key assertions): `session id accepts sessionId and session_id`; `model control prefers a model config option over models state` (config → `{kind:"config",configId:"model",current}`); `model control falls back to models state` → `setModel`; `model control is none for an empty result`; `models list puts the current model first and dedupes`; `empty session yields no models`; `capabilities read loadSession and promptCapabilities.image, defaulting false`; `image blocks are replaced by a path note when image unsupported`; `stderr auth lines map to help`: "Not logged in" → message containing "kiro-cli login", ordinary `[info]` line → null; `startup error with auth wording ends with KIRO_AUTH_HELP`, timeout error says Kiro CLI did not start; `replay flag detected`; `context usage extracted only for context_usage kind`.
- [ ] **Step 2:** `npx vitest run src/integrations/harness/providers/kiro/kiroProtocol.test.ts` → FAIL (module missing). (Requires `HarnessId` to include `"kiro"` — do Task 3 step 1 first, or the file won't typecheck; vitest still runs.)
- [ ] **Step 3: Implement** each function above; parsing is tolerant of camelCase/snake_case like `hermesProtocol.ts`.
- [ ] **Step 4:** same command → PASS.

### Task 3: Provider identity wiring + catalog

**Files (modify):** `src/features/sessions/model/session.ts` (HarnessId, HARNESSES, HARNESS_LABEL, HARNESS_TITLE), `models.ts` (default model entry `kiro:default` "Configured model", `DEFAULT_MODEL_ID`, `HARNESS_ORDER`), `integrations/harness/core/child.ts` (`resolveKiroBinary`, command map `kiro: "harness_resolve_kiro"`), `core/availability.ts` (CLI entry + probe branch), `core/availabilityState.ts`, `core/register.ts`, `harness/index.ts` (exports), `features/connections/model/protocol.ts` (`REMOTE_PROVIDERS`), `features/settings/model/settings.ts:448` (search keywords), `features/sessions/ui/HarnessIcon.tsx` + new `src/assets/providers/kiro.svg`, `mobile/src/features/workspace/types.ts`, `mobile/src/features/sessions/SessionFilterMenu.tsx`, `mobile/src/features/sessions/providerSvgs.ts`, `docs/providers.md`, `README.md`, `docs/remote-access.md`.
**Create:** `providers/kiro/kiroCatalog.ts` (`refreshKiroCatalog(): Promise<void>`, `discoverKiroModels(workingDirectory?: string): Promise<AgentModel[]>` — same probe shape as `hermesCatalog.ts`, using the first launch args that initialize) with test `kiroCatalog.test.ts` (fake child: models returned → `setHarnessModels("kiro", …)`; empty session → catalog untouched).
**Tests to extend:** `core/registry.test.ts` and `core/child.test.ts` (add `kiro` rows), `src/shared/ui/icons.test.ts` only if icon sizing differs.

- [ ] **Step 1:** Add `"kiro"` to the unions/records above; run `npx tsc --noEmit` and follow errors until only the missing adapter remains.
- [ ] **Step 2:** Write `kiroCatalog.test.ts` (fails), implement catalog, pass.
- [ ] **Step 3:** Extend registry/child tests; run `npx vitest run src/integrations/harness/core`.

### Task 4: Live adapter

**Files:** Create `providers/kiro/kiro.ts` (live session, copy Hermes structure minus background/subagent logic), `providers/kiro/kiroAdapter.ts` (`kiroAdapter: HarnessAdapter`, `ensureKiroRegistered()`), test `providers/kiro/kiroLive.test.ts` (mock `../../core/child` exactly like `cursorLive.test.ts`, scripted replies).

**Interfaces:**
- Produces: `sendKiroTurn, steerKiroTurn, cancelKiroTurn, respondKiroApproval, stopKiroSession, forgetKiroSession, bindKiroSession` with the same signatures as the Hermes equivalents. `steerKiroTurn` throws `TurnNotReadyError` unless a turn is live; mid-turn steer sends a queued `session/prompt` like Hermes. `adapter.canSteer = false`.
- Consumes: Task 2 functions; `eventsFromAcpUpdate`, `permissionOptionId`, `permissionRequestFromAcp`, `pickAutoOption` from `grokProtocol.ts`.

- [ ] **Step 1: Failing tests** in `kiroLive.test.ts`: 
  1. `V3 handshake sends initialize, session/new with mcpServers [] and emits providerBound + started`.
  2. `streams message deltas and completes when session/prompt resolves` (assert `message.delta` text, `message.completed`).
  3. `tool_call and tool_call_update become tool events`.
  4. `supervised permission request emits approval.requested and replies with the chosen offered optionId`; `full-access auto-selects allow_always without prompting`; `plan intent denies a write request but allows read`.
  5. `cancel answers a pending permission as cancelled, sends session/cancel, and holds the next session/prompt until the first prompt resolves`.
  6. `falls back to bare acp when the V3 launch exits before initialize` (assert `spawnChild` args `["acp","--agent-engine=v3"]` then `["acp"]`, and only two attempts).
  7. `session/load replays are muted` (updates with `_meta.kiro.replay` or arriving before load resolves emit nothing).
  8. `model change uses session/set_config_option when a model option is advertised`, and `session/set_model` for a `models` state, nothing for neither.
  9. `signed-out error mentions kiro-cli login`.
  10. `unknown agent request is answered with -32601`.
- [ ] **Step 2:** `npx vitest run src/integrations/harness/providers/kiro` → FAIL.
- [ ] **Step 3: Implement** `kiro.ts`. Cancel: `respond` cancelled to pending approvals, `notify("session/cancel")`, then await the in-flight prompt up to 8s before `rejectPending`. Launch fallback: loop over `KIRO_LAUNCH_ARGS`, treating a child exit or `initialize` failure before a session exists as "try next".
- [ ] **Step 4:** tests PASS; `npx tsc --noEmit` clean.

### Task 5: Verify, review, merge

- [ ] **Step 1:** `npm run check` (vitest, tsc, fmt, clippy, cargo test) green; paste real output in the final report.
- [ ] **Step 2:** Run the app-level smoke if the environment allows (`npm run tauri dev` is not available headlessly — state this if skipped). Without `kiro-cli` installed, state that live Kiro behaviour is unverified.
- [ ] **Step 3:** Local Cubic review; address every finding (fix, or document why invalid).
- [ ] **Step 4:** Commit (no attribution trailers per user rules), then merge to `main` only after Steps 1–3 are clean.
