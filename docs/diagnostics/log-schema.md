# Diagnostics log schema

Both Gespan apps, Gespan Native (GPUI) and Gespan (Tauri), write their local
logs in this one format. Each line of a log file is a single JSON object
(JSON Lines). The same lines, after scrubbing, become `logs.jsonl` in a
diagnostics bundle (see [bundle-format.md](bundle-format.md)).

Logs are always written locally. Writing them needs no consent, because they
never leave the machine unless the user sends a report (see
[consent.md](consent.md) and [../privacy.md](../privacy.md)).

## Files

| App | Platform | Folder |
|---|---|---|
| Native | macOS | `~/Library/Logs/dev.gespan.native/` |
| Native | Linux | `$XDG_STATE_HOME/gespan-native/logs/` (falls back to `~/.local/state/gespan-native/logs/`) |
| Native | Windows | `%LOCALAPPDATA%\dev.gespan.native\logs\` |
| Native (dev data) | macOS | `~/Library/Logs/dev.gespan.native.dev/` |
| Native (dev data) | Linux | `$XDG_STATE_HOME/gespan-native-dev/logs/` (falls back to `~/.local/state/gespan-native-dev/logs/`) |
| Native (dev data) | Windows | `%LOCALAPPDATA%\dev.gespan.native.dev\logs\` |
| Tauri | all | Tauri's `app_log_dir()`, for example `~/Library/Logs/dev.gespan.desktop/` on macOS |

- Dev logs never mix with release logs. Native running with dev data
  (`--dev-data`) uses the dev folders above. Tauri dev builds run as
  `dev.gespan.desktop.dev`, so `app_log_dir()` is already separate.
- Files rotate daily: `gespan-native.YYYY-MM-DD.jsonl` and
  `gespan-tauri.YYYY-MM-DD.jsonl`. The date is the UTC day.
- Retention: 7 days or 50 MB in total, whichever limit is hit first. Old
  files are pruned at startup, at each UTC midnight rollover, and whenever a
  write would take the total over 50 MB (oldest files first). If today's
  file alone fills the 50 MB, further lines that day are dropped.
- Crash records are separate files in `crashes/` next to the logs (see
  [bundle-format.md](bundle-format.md#crashesjson)).

## Levels

`error`, `warn`, `info`, `debug`, `trace`. Release builds log at `info` and
above. `GESPAN_LOG` takes an `EnvFilter` directive to raise the level for one
module, for example `GESPAN_LOG=gespan_core::git=debug`.

## Fields

| Field | Type | Required | Meaning |
|---|---|---|---|
| `ts` | string | yes | When the event happened: RFC 3339, UTC, millisecond precision (`2026-10-25T09:14:03.512Z`). |
| `level` | string | yes | One of the levels above, lowercase. |
| `target` | string | yes | Where the event came from: the Rust module path (`gespan_core::git`), or `webview::<module>` for lines from the Tauri frontend. |
| `message` | string | yes | Short, human-readable text. Never put user content here; use a deny-listed field instead so it is dropped. |
| `app` | string | yes | `"native"` or `"tauri"`. |
| `version` | string | yes | App version, for example `0.1.0` or `0.1.0-preview.2`. |
| `session_id` | string | no | The agent session the event belongs to, when there is one. Omitted otherwise, never `null`. |
| `span` | string | no | The enclosing operation span, outermost first, joined with `>`, for example `worktree.create>git.command`. Omitted when there is none. |
| `fields` | object | yes | Structured key/value data from the event. May be `{}`. Values are strings, numbers, booleans or `null`; nested objects are flattened to dotted keys (`http.status`). |
| `event_id` | string | yes | A random UUID v4, unique per line. The "Report" action on an error toast passes it to the report, so the report can point at that line. |

Rules for writers:

- Field names are `snake_case`. Unknown fields are allowed; readers must
  ignore fields they don't know.
- Every error path logs at `error` or `warn` with an `action` field (what was
  being attempted) and, where one exists, an `error_kind` field (a short fixed
  code such as `spawn_failed`, `timeout`, `auth`, `not_found` or `io`).
- A panic also emits an `error` line with `fields.kind = "panic"`.

## Deny-listed fields

Any field with one of these names is dropped entirely by the scrubber
(`crates/gespan-redact`) before a line is shown in a report preview or sent.
A name matches at the top level of the line, inside `fields`, and in any
dotted segment of a flattened key, so `request.body` and `git.output` are
dropped too.
Logging code should use these names for anything that could hold user
content, so it is removed automatically:

- `prompt`
- `content`
- `diff`
- `output`
- `body`
- `env`
- `audio`
- `transcript`

Scrubbing also replaces home paths, project paths, emails and tokens in every
remaining value. The scrubber's own fixtures are the source of truth for
those rules.

## Lines from the Tauri frontend

The frontend logger sends records through the `diagnostics_log` command as
`{ level, message, fields, ts }`. The Rust side turns each record into a log
line: it keeps `level`, `message`, `fields` and `ts`, sets `app` and
`version`, sets `target` to `webview::<module>` (from `fields.module`, or
`webview` when missing, and removes `module` from `fields`), and assigns a
fresh `event_id`.

## Examples

```jsonl
{"ts":"2026-10-25T09:14:03.512Z","level":"error","target":"gespan_core::worktree","message":"worktree create failed","app":"native","version":"0.1.0","session_id":"s_01JB7Q2W8K","span":"worktree.create>git.command","fields":{"action":"worktree.create","error_kind":"not_found","git.exit_code":128},"event_id":"0b7f1c2e-5a4d-4e8b-9f3a-2c6d1e0a9b47"}
{"ts":"2026-10-25T09:15:41.006Z","level":"warn","target":"webview::board","message":"board refresh timed out","app":"tauri","version":"0.1.0","fields":{"action":"board.refresh","error_kind":"timeout","elapsed_ms":15000},"event_id":"6e2a9d41-c3b7-4f05-8a1e-7d4b2f9c0e13"}
{"ts":"2026-10-25T09:16:00.250Z","level":"error","target":"gespan_lib::diagnostics","message":"panicked at src/board.rs:212:9","app":"tauri","version":"0.1.0","fields":{"kind":"panic","thread":"tokio-runtime-worker"},"event_id":"a41f8e30-2b9c-4d17-b6e5-90c3d8a7f214"}
```
