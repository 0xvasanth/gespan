# Diagnostics bundle format

A diagnostics bundle is what "Report an issue" sends, or saves for the user to
attach when the intake can't be reached. Both apps build the same bundle. This
is `bundle_version: 1`.

Everything in a bundle is scrubbed on the user's machine by
`crates/gespan-redact` **before** it is shown in the report preview. The
preview shows exactly the bytes that will be sent.

## Layout

A zip named `gespan-diagnostics-<app>-<version>-<timestamp>.zip`, for example
`gespan-diagnostics-native-0.1.0-20261025T091403Z.zip`. `<app>` is `native` or
`tauri`, and `<timestamp>` is UTC in the basic format `YYYYMMDDTHHMMSSZ`.

| Entry | Always present | Contents |
|---|---|---|
| `report.json` | yes | The report itself: what the user wrote, the app and system, and the consent state. |
| `logs.jsonl` | no | The last 2,000 log lines or 24 hours, whichever is smaller, in the [log schema](log-schema.md). Left out when the user turns off "Include recent logs" or unticks "Attach diagnostics". |
| `crashes/*.json` | no | Unsent crash records, one file each, keeping their on-disk names. |
| `counters.json` | no | The last 7 days of failure counters. |
| `environment.json` | no | Installed provider CLIs and counts of projects and sessions. |

- The whole zip is at most 5 MB. If it would be larger, the oldest log lines
  are dropped first, then the oldest crash records. `report.json` is never
  dropped.
- Entries are stored at the zip root (no wrapping folder), UTF-8, with `\n`
  line endings.
- With "Attach diagnostics" unticked, the bundle holds only `report.json`.

## Contents by kind

`kind` in `report.json` says who started the send. A send from the crash
notice ("Send" or "Always send") is a `crash` send, the same as an automatic
one.

| Kind | `report.json` | `logs.jsonl` | `crashes/*.json` | `counters.json` | `environment.json` |
|---|---|---|---|---|---|
| `user` | Always | If "Attach diagnostics" is ticked and logs are included | If "Attach diagnostics" is ticked, unless crash reports are set to "Never send" | If "Attach diagnostics" is ticked | If "Attach diagnostics" is ticked |
| `crash` | Always | Never | Always: the crash records being sent | Never | Never |

## report.json

| Field | Type | Required | Meaning |
|---|---|---|---|
| `bundle_version` | number | yes | Always `1` for this format. |
| `submission_id` | string | yes | A random UUID v4 made when the bundle is built. The intake uses it to ignore retries of the same send. |
| `kind` | string | yes | `"user"` for a report a person wrote, `"crash"` for an automatic crash send. |
| `created_at` | string | yes | RFC 3339 UTC. |
| `app` | string | yes | `"native"` or `"tauri"`. |
| `version` | string | yes | App version. |
| `build_channel` | string | yes | `"release"`, `"preview"` or `"dev"`. |
| `os` | string | yes | `"macos"`, `"linux"` or `"windows"`. |
| `os_version` | string | yes | For example `"15.1"`. |
| `arch` | string | yes | `"aarch64"` or `"x86_64"`. |
| `locale` | string | yes | BCP 47 tag, for example `"en-GB"`. |
| `title` | string | yes | The user's title. For a crash send, generated from the panic message. |
| `description` | string | yes | "What happened?". Empty for a crash send. |
| `steps` | string | no | "Steps to reproduce". Omitted when empty. |
| `contact_email` | string | no | Optional, entered by the user. Kept in the private bundle only and never put in the public issue. |
| `install_id` | string | no | The anonymous install ID, if the user has opted in to anything (see [consent.md](consent.md#anonymous-install-id)). Omitted otherwise. |
| `event_ids` | array of strings | yes | `event_id`s of the log lines this report is about (from an error toast's "Report" action). May be empty. |
| `consent` | object | yes | The consent record at the time of sending, as defined in [consent.md](consent.md#consent-record). |
| `consent.consent_version` | number | yes | Which version of the consent wording the user agreed to. |
| `files` | array of strings | yes | The other entries in this zip, so the intake can tell an empty file from a missing one. |

```json
{
  "bundle_version": 1,
  "submission_id": "3f9b2c71-8d4e-4a6f-b1c5-0e7d9a2f6b38",
  "kind": "user",
  "created_at": "2026-10-25T09:14:03.000Z",
  "app": "native",
  "version": "0.1.0",
  "build_channel": "release",
  "os": "macos",
  "os_version": "15.1",
  "arch": "aarch64",
  "locale": "en-GB",
  "title": "New worktree fails in a fresh folder",
  "description": "Creating a worktree shows an error and nothing happens.",
  "steps": "1. Add a folder that is not a git repository\n2. New worktree",
  "install_id": "c2d8e4a1-7b3f-4e9c-a6d0-5f1b8e2c9a74",
  "event_ids": ["0b7f1c2e-5a4d-4e8b-9f3a-2c6d1e0a9b47"],
  "consent": {
    "consent_version": 1,
    "crash_reports": "ask",
    "metrics": true,
    "include_logs": true,
    "decided_at": "2026-10-20T08:00:00.000Z"
  },
  "files": ["logs.jsonl", "counters.json", "environment.json"]
}
```

## crashes/*.json

Written by the panic hook with a synchronous, fsync'd write, named
`crashes/<timestamp>-<random>.json`: the same timestamp format as the zip
name, then 8 random hex digits, so two panics in the same second keep both
records. A record leaves the local `crashes/` folder once it has been sent,
once the user chooses "Don't send", or after 7 days, whichever comes first.
The 7-day limit applies whatever the consent choice, so with "Never send"
records don't pile up.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `ts` | string | yes | RFC 3339 UTC. |
| `app` | string | yes | `"native"` or `"tauri"`. |
| `version` | string | yes | App version. |
| `os` | string | yes | As in `report.json`. |
| `os_version` | string | yes | As in `report.json`. |
| `arch` | string | yes | As in `report.json`. |
| `thread` | string | yes | The thread name, or `"<unnamed>"`. |
| `message` | string | yes | The panic message. |
| `location` | object | yes | `{ file, line, column }` of the panic. |
| `location.file` | string | yes | Source path relative to the crate, for example `src/board.rs`. |
| `backtrace` | string | yes | The captured backtrace, or `""` when capture was disabled. |
| `minidump` | string | no | Reserved for a minidump file name inside `crashes/`, for native crashes in a later phase. Omitted today. |

```json
{
  "ts": "2026-10-25T09:16:00.250Z",
  "app": "tauri",
  "version": "0.1.0",
  "os": "macos",
  "os_version": "15.1",
  "arch": "aarch64",
  "thread": "tokio-runtime-worker",
  "message": "called `Option::unwrap()` on a `None` value",
  "location": { "file": "src/board.rs", "line": 212, "column": 9 },
  "backtrace": "   0: gespan_lib::board::refresh\n   1: tokio::runtime::task::harness::poll"
}
```

## counters.json

The last 7 days of failure counters from the app's own diagnostics database.
These are counts, never error text.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `generated_at` | string | yes | RFC 3339 UTC. |
| `counters` | array | yes | One row per `(day, name, error_kind, app_version)`. |

Each row is `{ day, name, error_kind, count, app_version }`:

- `day`: the UTC day, `YYYY-MM-DD`.
- `name`: the operation, for example `worktree.create`, `agent.start`,
  `git.command`, `mcp.start`.
- `error_kind`: a fixed short code (`spawn_failed`, `timeout`, `auth`,
  `not_found`, `io` and so on), or `null` for a success count.
- `count`: a whole number.
- `app_version`: the version that recorded the count.

```json
{
  "generated_at": "2026-10-25T09:14:03.000Z",
  "counters": [
    { "day": "2026-10-25", "name": "worktree.create", "error_kind": "not_found", "count": 3, "app_version": "0.1.0" },
    { "day": "2026-10-25", "name": "worktree.create", "error_kind": null, "count": 11, "app_version": "0.1.0" },
    { "day": "2026-10-24", "name": "agent.start", "error_kind": "spawn_failed", "count": 1, "app_version": "0.1.0" }
  ]
}
```

## environment.json

Only what helps reproduce a problem. **No** paths, project names or
repository URLs.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `providers` | array | yes | One `{ id, version }` per provider CLI found on the machine. `version` is `null` when it couldn't be read. |
| `projects_count` | number | yes | How many projects are added. |
| `sessions_count` | number | yes | How many agent sessions exist. |

```json
{
  "providers": [
    { "id": "claude-code", "version": "2.1.4" },
    { "id": "codex", "version": "0.48.0" },
    { "id": "cursor", "version": null }
  ],
  "projects_count": 4,
  "sessions_count": 57
}
```

## Sent reports

Each app keeps `sent-reports.json` next to its logs: a list of
`{ report_id, kind, sent_at }` for every send the intake accepted, including
automatic crash sends, kept for 90 days to match the private storage. It is
how a user finds the `report_id` of a send they never saw, to ask for its
deletion. It holds nothing else.

## Versioning

- Adding an optional field, or a new optional entry in the zip, keeps
  `bundle_version: 1`. Readers ignore fields and entries they don't know.
- Removing or renaming a field, changing its type, or making a field required
  needs `bundle_version: 2`, and the intake must accept both until the old
  app versions are gone.
- Sending something new (for example screenshots) is a change to what is
  collected. It also bumps `consent_version` (see [consent.md](consent.md)),
  so users are asked again.
