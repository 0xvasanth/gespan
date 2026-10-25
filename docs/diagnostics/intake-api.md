# Diagnostics intake API

The HTTP API both Gespan apps send reports and metrics to. The service itself
lives in `services/intake/`. Apps call it only when [consent](consent.md)
allows it, or when the user clicks "Send".

## Conventions

- **Base URL**: set at build time from `GESPAN_REPORTS_URL`. Release builds
  point at the production intake; dev builds point at a staging intake or
  none. With no URL, the app uses the [fallback](#fallback-when-the-intake-cant-be-reached).
- **HTTPS only.**
- **Auth**: every request carries the header `X-Gespan-Ingest-Key`. The key
  is public and per build. It is baked into the app, so it is not a secret: it
  only lets the intake tell builds apart and revoke a leaked build's key.
  Abuse is held off by server-side size caps and rate limits, never by
  secrets in the app.
- **User agent**: `Gespan/<version> (<app>; <os>)`, for example
  `Gespan/0.1.0 (native; macos)`.
- Responses are JSON (`application/json; charset=utf-8`).
- The intake never logs request bodies, the contact email or the install ID.

## POST /v1/reports

Sends one diagnostics bundle.

### Report request

- `Content-Type: application/zip`
- Body: the bundle zip, exactly as shown in the preview (see
  [bundle-format.md](bundle-format.md)). At most 5 MB.
- `X-Gespan-Submission-Id`: the bundle's `submission_id`, so the intake can
  answer a retry without reading the zip.

The intake reads everything else (`app`, `version`, `kind`, `install_id`)
from `report.json` inside the zip.

### Report response

`201 Created` for a new report. `200 OK` for a retry of a `submission_id` it
has already accepted, with the same body as the first time.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `report_id` | string | yes | The intake's id for the report, for example `r_2026_10_7XK2M9`. The app shows it, and the user can quote it to ask for deletion (see [privacy](../privacy.md#how-to-have-it-deleted)). |
| `issue_url` | string or null | yes | The GitHub issue the report created or was added to. `null` for an automatic crash send that only went to the error tracker. |

```json
{
  "report_id": "r_2026_10_7XK2M9",
  "issue_url": "https://github.com/0xvasanth/gespan/issues/214"
}
```

## POST /v1/metrics

Sends one day's failure counters. Only sent with error metrics turned on, at
most once a day per day of data.

### Metrics request

`Content-Type: application/json`, at most 64 KB.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `app` | string | yes | `"native"` or `"tauri"`. |
| `version` | string | yes | App version that recorded the counts. |
| `build_channel` | string | yes | `"release"`, `"preview"` or `"dev"`. |
| `os` | string | yes | `"macos"`, `"linux"` or `"windows"`. |
| `arch` | string | yes | `"aarch64"` or `"x86_64"`. |
| `consent_version` | number | yes | The `consent_version` the user agreed to. |
| `day` | string | yes | The UTC day the counts are for, `YYYY-MM-DD`. |
| `send_id` | string | yes | A random UUID v4 made for this one send and reused only for its retries. |
| `counters` | array | yes | One `{ name, error_kind, count }` per counter, as in [`counters.json`](bundle-format.md#countersjson). |

There is deliberately no install ID or submission id: metrics are stored as
daily counts per version and are never tied to an install or a report.
`send_id` is fresh for every day's send, so it links nothing; the intake
keeps it for 24 hours only to drop retries.

```json
{
  "app": "native",
  "version": "0.1.0",
  "build_channel": "release",
  "os": "macos",
  "arch": "aarch64",
  "consent_version": 1,
  "day": "2026-10-24",
  "send_id": "9d4e7a12-6c3b-4f80-a5e1-2b8f0c7d3e96",
  "counters": [
    { "name": "worktree.create", "error_kind": "not_found", "count": 3 },
    { "name": "worktree.create", "error_kind": null, "count": 11 },
    { "name": "agent.start", "error_kind": "spawn_failed", "count": 1 }
  ]
}
```

### Metrics response

`202 Accepted` with an empty JSON object, `{}`. The intake adds the counts to
the day's totals for that version. A retry with a `send_id` it has already
accepted gets `202` again and is not counted twice. The app marks a day as
sent only after a `202`.

## Errors

Every error has the same body, and a fixed `code` the app can match on.

| Status | Code | When | What the app does |
|---|---|---|---|
| 400 | `invalid_request` | The zip or JSON is malformed, `report.json` is missing, or a required field is missing. | Show the error, offer "Copy report". Don't retry. |
| 401 | `invalid_ingest_key` | The ingest key is missing, unknown or revoked. | Use the fallback. Don't retry. |
| 413 | `too_large` | The body is over the size cap. | Rebuild the bundle without logs, then the fallback. |
| 415 | `unsupported_media_type` | The wrong `Content-Type`. | Don't retry (an app bug). |
| 429 | `rate_limited` | Too many requests from this install ID or IP. `Retry-After` says when to try again. | Queue it and retry after `Retry-After`. |
| 5xx | `server_error` | The intake failed. | Retry up to 3 times with backoff (1 s, 4 s, 16 s), then the fallback. |

Background sends (error metrics and automatic crash sends, which the user
didn't start) never use the fallback and never show an error. On `429` or
`5xx` they stay queued and are retried later; on any other error they are
dropped.

```json
{
  "error": {
    "code": "rate_limited",
    "message": "Too many reports from this install. Try again in 10 minutes."
  }
}
```

## What the intake does with a report

- Stores the bundle in a **private** R2 bucket at
  `reports/<yyyy>/<mm>/<report-id>.zip`, with a 90-day lifecycle rule.
- Forwards crash records (and, for `user` reports, the `error` lines in
  `logs.jsonl`) to the Sentry-compatible error tracker, tagged with the
  release version and the `report_id`, so a deletion request can find them.
- Creates or updates a GitHub issue in `0xvasanth/gespan` through a GitHub
  App (never a personal token):
  - title `[<app>] <user title>`;
  - labels `bug`, `from-app` and the app (`native` or `tauri`), plus `crash`
    for crash reports;
  - body: the user's text, a version/OS table, the top scrubbed error lines,
    a maintainers-only link to the private bundle, and the error-tracker link;
  - dedupe: the same crash fingerprint (panic location plus a hash of the
    message) on the same version comments on the existing issue instead of
    opening a new one, and raises its "affected installs" count.
- A `kind: "user"` report always becomes an issue, because a person asked for
  help. A `kind: "crash"` report goes to the error tracker only, and becomes
  an issue once its fingerprint passes the threshold set in the intake (for
  example, seen on 3 installs) or is new in a release.
- The contact email stays in the private bundle. It is never in the issue,
  the error tracker or the intake's logs.

## Fallback when the intake can't be reached

With no `GESPAN_REPORTS_URL`, a `401`, or retries used up, "Report an issue":

1. rebuilds the bundle with `report.json` written without `contact_email` and
   `install_id` (the report sheet hides the contact email field in this
   mode), shows that bundle in the preview, then saves the zip to the
   Downloads folder and reveals it;
2. opens
   `https://github.com/0xvasanth/gespan/issues/new?template=bug_report.yml&title=…&version=…&os=…&os-version=…&what-happened=…`
   with the bug report form's field ids as query parameters, asking the user
   to drag the zip in.

The report sheet must say clearly that this needs a GitHub account and that
the attached zip will be public.
