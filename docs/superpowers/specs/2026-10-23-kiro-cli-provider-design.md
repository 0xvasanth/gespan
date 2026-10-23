# Kiro CLI as an ACP provider

Recreated from ticket #809 ("[Feature]: Add Kiro CLI as an ACP provider") in
the parent repository owned by `hardbeat920`. That ticket has no PR, and this
repository has issues disabled, so the ticket and its design live here.

## Intent

Kiro users cannot use their Kiro sign-in, agents, steering, hooks, and MCP setup
from this app's sessions, worktrees, handoff, and orchestration UI. Kiro CLI
speaks the Agent Client Protocol over stdio (`kiro-cli acp`), so it is one more
consumer of the ACP plumbing already used by Hermes, Grok, Cursor, Devin and
Antigravity — not a new terminal-output protocol.

## Goals

Add `kiro` as a first-class built-in provider ("Kiro CLI") that:

1. Detects `kiro-cli` and shows an actionable install/sign-in message when it is
   missing or signed out.
2. Initializes ACP, then creates or loads a session in the selected
   project/worktree.
3. Streams assistant text, reasoning, tool calls, tool progress and turn
   completion into the transcript.
4. Supports cancel, permission requests (mapped to the app's access levels),
   session resume, and model selection when advertised.
5. Sends image/file prompt blocks only as far as the negotiated prompt
   capabilities allow.
6. Preserves the Kiro session id so a thread resumes its Kiro session.
7. Lists Kiro's models in the model picker via a catalog probe.
8. Is wired everywhere a provider appears: harness registry, availability,
   settings binary path, remote host provider list, Rust resolver, icons, mobile,
   docs.

## Non-goals

- Kiro-specific extension methods (`_kiro/*`, `_kiro.dev/*`) for the baseline
  chat lifecycle. Unknown agent-to-client requests get a JSON-RPC `-32601`
  error, which Kiro's docs require so the agent does not wait.
- A generic "custom ACP provider" facility (upstream #405/#570).
- Driving `kiro-cli login` from the app, or the ACP `authenticate` method: the
  docs do not specify its params or auth methods. Sign-in stays in the CLI.
- Kiro skills discovery, steering/hooks UI, usage/credits meter, mid-turn
  steering via `_session/steer`.
- Anything that needs a live account in CI.

## Anti-goals

- Hard-coding Kiro version numbers or method lists. Kiro's docs say support
  must come from `initialize` capabilities, not the version.
- Reading or writing `~/.kiro/sessions/cli/` directly.
- Reporting "works with real Kiro" without having run it against one.

## Constraints and facts from Kiro's docs (kiro.dev/docs/cli/acp, /cli/v3/acp-migration)

- Two server generations share `protocolVersion: 1`. V3 is launched with
  `kiro-cli acp --agent-engine=v3` and rejects `--agent/--model/--effort/
  --trust-*` on `acp`; legacy v2 is `kiro-cli acp [--agent X]`. Which engine a
  bare `kiro-cli acp` selects is not documented.
- V3 has no `session/set_model`; model and effort are session config options
  (`session/set_config_option {sessionId, configId, value}`). v2 has
  `session/set_model`.
- Result shapes for `session/new`/`load` are not documented. The parser must
  accept both the ACP `models`/`modes` state and `configOptions`, and tolerate
  absence of either.
- Updates are standard ACP `session/update` kinds plus `session_info_update`
  with `_meta.kiro.*`. `turn_end` is not a completion signal; the
  `session/prompt` response is.
- Register the `session/update` handler before `session/new`/`session/load`.
  `_meta.kiro.replay: true` marks replayed history on load.
- Cancel flow: answer any pending permission as cancelled, send
  `session/cancel`, wait for the original prompt response, only then send a new
  prompt.
- Permission options are `allow_once`/`allow_always`/`reject_once`/
  `reject_always`; never synthesize an option the agent did not offer. MCP tool
  annotations in `_meta` are untrusted and must never auto-approve.
- Use the full executable path (GUI apps lack the shell `PATH`).
- Credential order: browser session from `kiro-cli login`, then `KIRO_API_KEY`.

## Decisions (builder's calls, not the user's)

- **Built-in adapter modelled on Hermes**, reusing `core/acp.ts` and the shared
  `eventsFromAcpUpdate`/permission helpers from `grokProtocol.ts`. (Confirmed
  with the user.)
- **Launch strategy**: try `kiro-cli acp --agent-engine=v3`; if that process
  exits or `initialize` fails, fall back once to bare `kiro-cli acp`. Control
  surface is chosen from the session result, not the launch path.
- **Model selection**: use the advertised `model` config option via
  `session/set_config_option`; else `session/set_model` if the session returned
  a `models` state; else do nothing. The picker always has a `kiro:default`
  ("Configured model") entry.
- **Access levels**: handled client-side exactly like Hermes (auto-pick an
  advertised option by runtime mode; plan turns allow only read/search). Kiro's
  own agent/mode is left on its default. Exception to Hermes: cancelled
  permissions answer `{outcome: "cancelled"}`.
- **Cancel**: notify `session/cancel`, then wait up to 8s for the prompt to
  settle before forcing the pending request closed.
- **Auth errors**: JSON-RPC `-32000`/`auth_required`, and stderr lines like
  "not logged in", become a message ending in
  "Run `kiro-cli login` (check with `kiro-cli whoami`)".
- **Binary**: only `kiro-cli` is ever resolved (the `kiro` IDE launcher is not an
  ACP server). Install hint: `curl -fsSL https://cli.kiro.dev/install | bash`
  (from Kiro's published installer; the docs page itself lists no command, so
  this is flagged as unverified in code comments).
- **Icon**: a simple original ghost glyph, monochrome; the official brand asset
  is not bundled.

## Test plan

- Pure unit tests for the protocol module: session id / config-option / model
  state parsing in V3, v2 and empty shapes; auth error classification; startup
  error mapping; prompt block capability gating.
- A scripted fake `kiro-cli acp` (Node stdin/stdout JSON-RPC) driven through the
  real adapter in vitest, covering: V3 handshake, fallback to bare `acp`, text
  streaming + prompt completion, tool call flow, permission allow/deny/cancel,
  cancel-then-new-prompt ordering, `session/load` replay muting, auth failure.
- Rust unit tests: resolver candidates, override name/version validation,
  harness argv token.
- Registry/availability/child tests extended with `kiro`.
- Gate before commit: `npm run check`, relevant `cargo test`, then a local Cubic
  review.

## Open items needing a real Kiro

Result shapes of `session/new`, the unauthenticated error shape, `mcpServers`
element schema, and which engine bare `acp` picks are undocumented. They are
handled defensively and covered by tests against a scripted server, but only a
signed-in `kiro-cli` can confirm them.
