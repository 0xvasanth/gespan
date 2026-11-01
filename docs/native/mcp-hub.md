# MCP hub

Gespan runs one local MCP endpoint for all its agents. Every capability it gives them (visuals, tickets and cards, app access, orchestration control) is a toolkit behind that endpoint, so a new capability needs no listener, token scheme or process of its own.

`crates/mcp-hub` is the generic part: the HTTP endpoint, tokens, toolkit registry and stdio relay. It depends on nothing of Gespan's and never names it; the host passes in the server name and any environment keys. `crates/gespan-core/src/tool_servers.rs` is the Gespan side: which toolkits exist, and how each provider is told about the endpoint.

## Adding a toolkit

1. Implement `mcp_hub::Toolkit`: a name, its tool definitions for a caller, and a call that does the work.
2. Register it in `tool_servers::hub`.
3. Add its name to the turn's `Mount` in `runtime.rs` when the session should have it.

No provider changes. A tool added to a live session reaches a running agent through `tools/list_changed`.

## How each agent reaches it

Checked against real releases: Claude Code 2.1.316, Codex 0.166.0, Kiro CLI 2.36.0, OpenCode 1.19.5, Pi 0.73.1.

| Agent | Delivery | Token goes through |
|---|---|---|
| Claude Code | `mcp_set_servers` control request | stdin |
| Codex | `config` on `thread/start` and `thread/resume` | stdin |
| ACP agents advertising `mcpCapabilities.http` (Kiro) | `mcpServers` entry of type `http` | stdin |
| Other ACP agents | `gespan-gpui mcp-relay` as a stdio server | the relay's own `env`, sent over stdin |
| OpenCode v1 | `POST /mcp` on the `serve` process that conversation owns | loopback HTTP |
| Pi, omp, and Grok or fx resuming a session | `gespan-gpui app` and `visual` CLIs | the agent's environment |

Pi ships without MCP, and Grok and fx don't refresh `mcpServers` on resume. On that last row every command the agent runs inherits the token, so it's a fallback, never a default.

OpenCode v2 connects to one managed service shared by every session, so a server added there would give each session's token to the others. Its turns run without Gespan's tools until it can scope a server to one session.

Codex reads its tool list when a thread starts, so a tool added mid-session reaches it on its next session.

## Servers

Settings → MCP holds Gespan's catalogue. Import copies servers out of each agent's config without changing those files. Each lands off, with its env values in the OS keychain, and runs only after the user approves it. The approval is a hash of the exact command, arguments and env, so an edit or a changed secret needs approving again.

Agents see `<server>__<tool>`, served from a cached list, so listing starts nothing. The first call starts the process.

| Class | One process per |
|---|---|
| `shared` | server and its command and secrets |
| `per-worktree` (the default) | worktree, started in that folder |
| `per-session` | agent session |

A process stops after five minutes with nothing in flight and no subscribers, or sooner if 20 are running. A shared server that asks an agent for sampling, elicitation or roots is refused, because the request can't be traced to one agent, and moves to `per-session`.

With "Agents load only these servers" on, Claude Code runs with `--strict-mcp-config` and Codex's own servers are switched off by name. ACP agents have no such switch: Cursor reads its own config, and Kiro's needs an agent file Gespan would have to write, so they keep loading their own servers alongside Gespan's.

Each server runs in its own process group with the same marker as agents. A record of their pids lets the next launch's sweep find a server that looks like any `node` process, and it still acts only on a dead parent in the marker. On Windows the kill-on-close job does this.

Remote servers are proxied the same way. Gespan is the OAuth client: one sign-in in Settings, tokens in the keychain, refreshed by Gespan, and sent only to that server. Agents talk to the hub and never receive them. A remote server is always `shared`, since one sign-in means one set of credentials.

## Security

- The endpoint listens on `127.0.0.1` only. A request with another `Host` or `Origin` gets 403 before its token is read, which is what stops DNS rebinding.
- Bad, expired and revoked tokens get 403, never 401: several agents answer a 401 by opening a browser for an OAuth sign-in.
- Each token is 256 random bits, kept only as a SHA-256 hash, and belongs to one provider process. A new process replaces it.
- Archiving or deleting a session revokes its token at once. Moving one changes its working folder, which starts a new process and so a new token. An unused token expires after an hour; a running turn keeps it alive.
- A token says who is calling. Every tool works from that grant, re-read on each request, and never from a session or project the agent names.
