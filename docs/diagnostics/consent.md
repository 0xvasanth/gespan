# Diagnostics consent

What each Gespan app asks before anything leaves the machine, what it stores
about the answer, and the exact wording it shows. Gespan Native and Gespan
(Tauri) use the same choices, defaults, record and wording.

**Default: nothing is sent.** Local logs, crash files and counters are always
written, because they never leave the machine. Sending any of them needs one
of the choices below, or a click on "Send" for a single report.

The current `consent_version` is **1**.

## Choices

Shown in Settings → **Privacy & diagnostics**. The three choices are
independent.

| Setting | Options | Default |
|---|---|---|
| **Crash reports** | Ask each time / Always send / Never send | Ask each time |
| **Error metrics** (daily failure counts, no content) | Off / On | Off |
| **Include recent logs in reports I send** | On / Off | On |

- **Crash reports**: what happens with a crash record on the next launch.
  "Ask each time" shows the crash notice and sends nothing without a click.
  "Always send" sends crash records to the error tracker without asking.
  "Never send" keeps them on the machine and shows no notice.
- **Error metrics**: when on, one aggregate of the day's failure counters is
  sent once a day (see [intake-api.md](intake-api.md#post-v1metrics)).
- **Include recent logs**: only applies when the user sends a report. It sets
  the starting state of the logs part of the report preview; the user can
  still remove it for one report.

The same section has **Report an issue**, **Open logs folder**, **Copy
diagnostics** and a link to [the privacy notice](../privacy.md).

## Consent record

Stored locally: in the desktop preferences for Gespan Native, and in a JSON
file in the app config directory for Gespan (Tauri), so the panic path can
read it without the webview. It is also copied into each report's
`report.json` (see [bundle-format.md](bundle-format.md#reportjson)).

| Field | Type | Required | Meaning |
|---|---|---|---|
| `consent_version` | number | yes | The `consent_version` the user answered. |
| `crash_reports` | string | yes | `"ask"`, `"always"` or `"never"`. |
| `metrics` | boolean | yes | Error metrics on or off. |
| `include_logs` | boolean | yes | Include recent logs in reports. |
| `decided_at` | string or null | yes | When the user last answered the prompt or changed a choice, RFC 3339 UTC. `null` until then. |

```json
{
  "consent_version": 1,
  "crash_reports": "always",
  "metrics": true,
  "include_logs": true,
  "decided_at": "2026-10-25T09:00:00.000Z"
}
```

## Defaults

The record an app uses before the user has answered anything, and after an
unreadable or missing record file. It sends nothing on its own.

```json
{
  "consent_version": 1,
  "crash_reports": "ask",
  "metrics": false,
  "include_logs": true,
  "decided_at": null
}
```

## First-run prompt

Shown once after install, or after the update that introduces this feature,
and again whenever the stored `consent_version` is lower than the current
one. It is a small, dismissible sheet, never a modal that blocks work.

> **Help improve Gespan?**
>
> Crash reports and failure counts help us fix bugs. They never include your
> code, prompts or file contents.
>
> [What we collect](../privacy.md)
>
> [Send crash reports automatically] [Ask me each time] [No thanks]

Each answer writes the record below, with the current `consent_version` and
`decided_at` set to now. `include_logs` is not asked here and stays `true`.

| Choice | `crash_reports` | `metrics` | `include_logs` |
|---|---|---|---|
| Send crash reports automatically | `"always"` | `true` | `true` |
| Ask me each time | `"ask"` | `false` | `true` |
| No thanks | `"never"` | `false` | `true` |
| Dismissed (closed without a choice) | `"ask"` | `false` | `true` |

"Send crash reports automatically" turns on error metrics too, because the
prompt names failure counts. Every other answer leaves them off.

## Crash notice

Shown on the next launch when there are unsent crash records and
`crash_reports` is `"ask"`. It doesn't block work.

> Gespan quit unexpectedly last time. Send a report?
>
> [Send] [Don't send] [Always send]

- **Send** sends the crash records this once.
- **Don't send** deletes them.
- **Always send** sends them and sets `crash_reports` to `"always"`.

With `"always"` the records are sent without a notice. With `"never"` there
is no notice and nothing is sent.

## Changing or revoking consent

- Changes in Settings take effect immediately and update `decided_at`.
- Turning a choice **off** (crash reports to "Never send", or error metrics
  to Off) deletes any queued, unsent reports or metrics of that kind and stops
  sending at once.
- When nothing is opted in any more (crash reports not "Always send" and
  error metrics off), the anonymous install ID is deleted too.

## Anonymous install ID

A random UUID v4, created only when the user first opts in to something
(crash reports "Always send" or error metrics on). It groups reports from the
same install so a fingerprint can count "affected installs". It is never
derived from hardware, the user name or an email, and it is deleted when the
user opts out of everything.

## When `consent_version` changes

Increase `consent_version` whenever Gespan starts collecting or sending
something new, or sends it somewhere new. When the stored version is lower
than the current one, the app keeps sending only what the old record allowed
(never more) and shows the first-run prompt again. Never quietly widen what
is sent.

| Version | Change |
|---|---|
| 1 | First version: crash reports, error metrics, user-sent reports with optional logs. |
