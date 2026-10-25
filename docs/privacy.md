# Privacy: diagnostics and issue reports

This page covers what Gespan and Gespan Native record when something goes
wrong, what can leave your machine, where it goes, how long it is kept and
how to have it deleted. It covers errors and failures only: Gespan does no
product analytics (no page views, feature usage, funnels or retention
tracking).

**Nothing is sent by default.** Gespan sends diagnostics only when you allow
it in Settings → Privacy & diagnostics, or when you click "Send" on a report.
There are no network calls for diagnostics before you choose.

The current `consent_version` is **1**. If Gespan ever starts collecting
something new, this number goes up and the app asks you again. It never
quietly widens what it sends. The exact choices and wording are in
[diagnostics/consent.md](diagnostics/consent.md).

## How you are asked

After you install Gespan, or after the update that adds this, a small sheet
asks "Help improve Gespan?", with three answers:

- **Send crash reports automatically**: crash reports are sent without
  asking.
- **Ask me each time** (also what closing the sheet means): after a crash,
  Gespan asks before sending anything.
- **No thanks**: crash reports are never sent.

None of the answers turns on error metrics. Those are only turned on in
Settings → Privacy & diagnostics. You can change any choice there at any
time.

## What is collected

### Kept on your machine (always)

These never leave your machine unless you send a report:

- **Logs**: error, warning and info lines from the app, in the format in
  [diagnostics/log-schema.md](diagnostics/log-schema.md).
- **Crash records**: the panic message, where in Gespan's source it
  happened, the thread name, a backtrace, and the app and OS versions.
- **Failure counters**: counts of failures per day by kind, such as
  "worktree creation failed 3 times, `not_found`". The kind is a fixed code,
  never error text.
- **Your consent choices.**
- **An anonymous install ID**, only once you opt in to something. It is a
  random number, not derived from your hardware, user name or email.

### Sent only with your consent

- **A report you send** (Help → Report an issue, or "Report" on an error).
  Every report holds what you type (title, what happened, optional steps,
  optional contact email), your app and OS versions, architecture and
  locale, your consent choices, your anonymous install ID if you have one,
  and the IDs of the log lines the report is about. If "Attach diagnostics"
  is ticked, it also holds the last 2,000 log lines or 24 hours of them,
  unsent crash records (not if crash reports are set to "Never send"), the
  last 7 days of failure counters, and which provider CLIs are installed
  with their versions and how many projects and sessions you have. You see a preview of
  exactly what will be sent and can remove anything first. The format is in
  [diagnostics/bundle-format.md](diagnostics/bundle-format.md).
- **Crash reports**, if you chose "Always send" or clicked "Send" on the
  crash notice: the crash records above, with the same app, OS, locale,
  consent and install ID details as a report. No logs, counters or
  environment details.
- **Error metrics**, if you turned them on: once a day, that day's failure
  counts with the app version, OS and architecture. They carry no install ID
  and are stored only as daily totals per version, so they aren't linked to
  you or to a report. (Like any web request, the intake sees your IP address
  while it receives them. It uses it for rate limiting and doesn't store it
  with the data.) The format is in
  [diagnostics/intake-api.md](diagnostics/intake-api.md#post-v1metrics).

## What is never sent

Gespan is built not to record these in its diagnostics, and scrubs what it
does record:

- Your code, file contents, diffs, prompts, transcripts, voice recordings,
  terminal output or provider responses.
- API keys, tokens or passwords, including anything Gespan reads from the
  keychain or your accounts.
- Your full home-directory path, project names or repository URLs.

Error messages are free text, and scrubbing works by known patterns, so it
can miss something unusual. That is why you see the exact contents of a
report before you send it. What **you type** into a report (title and
description) is not changed, and it is published in a public GitHub issue as
you wrote it.

Before anything is shown in a preview or sent, Gespan scrubs it **on your
machine**: your home folder becomes `~`, project folders become
`<project-1>` and so on (the file name is kept), email addresses become
`<email>`, and anything that looks like a key or token becomes
`<redacted>`. Log fields with these names are dropped entirely: `prompt`,
`content`, `diff`, `output`, `body`, `env`, `audio`, `transcript`.

## Where it goes

- **Your machine**: the logs folder for each app (Settings → Privacy &
  diagnostics → Open logs folder). On macOS that is
  `~/Library/Logs/dev.gespan.native/` for Gespan Native and
  `~/Library/Logs/dev.gespan.desktop/` for Gespan.
- **Gespan's report intake**, a small service run by the Gespan maintainers
  on Cloudflare. It receives reports and metrics over HTTPS, and doesn't log
  their contents.
- **A private storage bucket** (Cloudflare R2) holds the full report.
  Only the maintainers can open it.
- **An error tracker** (Sentry-compatible) receives crash records and error
  lines, to group the same crash across installs and versions.
- **A public GitHub issue** in
  [0xvasanth/gespan](https://github.com/0xvasanth/gespan/issues) is created
  or updated for each report you send. It holds your title and text, your
  app and OS versions, and the top scrubbed error lines. **Your contact email
  is never put in the issue.** It stays in the private report. Automatic
  crash reports only become an issue once the same crash is seen often or is
  new in a release.
- **Fallback**: if the intake can't be reached, Gespan saves the report as a
  zip in your Downloads folder and opens GitHub's new-issue page for you to
  attach it. In that case the zip you attach is public, and you need a GitHub
  account. The report sheet tells you this before you go ahead. The zip is
  built without your contact email or install ID, and you see its contents
  before it is saved.

## How long it is kept

| What | Where | Kept for |
|---|---|---|
| Logs | Your machine | 7 days or 50 MB in total, whichever comes first |
| Crash records | Your machine | Until sent or dismissed, and at most 7 days |
| List of sent report IDs (`sent-reports.json`) | Your machine | 90 days |
| Failure counters | Your machine | 30 days |
| Full reports | Private storage | 90 days, then deleted automatically |
| Crash records and error lines | Error tracker | 90 days |
| Daily error metrics | Intake | 13 months, as daily totals per version |
| Report issues | GitHub (public) | Until a maintainer deletes the issue |

## How to have it deleted

- **On your machine**: delete the logs folder (Settings → Privacy &
  diagnostics → Open logs folder) at any time. Turning off crash reports or
  error metrics deletes anything queued and not yet sent. Turning off both
  also deletes the anonymous install ID.
- **Reports you sent**: every sent report shows a report ID, such as
  `r_2026_10_7XK2M9`. Crash reports sent automatically are listed with their
  IDs in `sent-reports.json` in the logs folder. Open an issue in
  [0xvasanth/gespan](https://github.com/0xvasanth/gespan/issues/new) titled
  "Delete report" with that ID. The ID identifies the report, not you, but
  the request is public and shows your GitHub account. The maintainers then
  delete the private report, its error-tracker events (tagged with the
  report ID) and the report text in the matching GitHub issue.
- **Error metrics** can't be deleted per person, because they are stored only
  as daily totals that no one can trace back to an install.
