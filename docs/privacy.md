# Privacy: diagnostics and issue reports

This page covers what Gespan and Gespan Native record when something goes
wrong, what can leave your machine, where it goes, how long it is kept and
how to have it deleted. It covers errors and failures only: Gespan does no
product analytics (no page views, feature usage, funnels or retention
tracking).

**Gespan never sends diagnostics by itself.** There is no telemetry, no
automatic crash upload and no background reporting, so there is nothing to
opt in to. A report leaves your machine only when you save it and send it on
yourself, by email or in a GitHub issue.

## What is collected

### Kept on your machine

These stay on your machine. Nothing in Gespan uploads them:

- **Logs**: error, warning and info lines from the app, in the format in
  [diagnostics/log-schema.md](diagnostics/log-schema.md).
- **Crash records**: the panic message, where in Gespan's source it
  happened, the thread name, a backtrace, and the app and OS versions.
- **Failure counters**: counts of failures per day by kind, such as
  "worktree creation failed 3 times, `not_found`". The kind is a fixed code,
  never error text. Settings → Diagnostics shows the last 7 days.

### In a report you save

"Report an issue" (Help menu, the command palette, Settings → Diagnostics,
"Report" on an error, or "Report…" on the crash notice) builds a zip in your
Downloads folder. It holds what you type (title, what happened, optional
steps), your app and OS versions, architecture and locale, and the IDs of the
log lines the report is about. If "Attach diagnostics" is ticked, it also
holds the last 2,000 log lines or 24 hours of them, unsent crash records, the
last 7 days of failure counters, and which provider CLIs are installed with
their versions and how many projects and sessions you have.

The sheet lists every file in the zip. You can open each one to read exactly
what it holds, and untick anything you don't want to include, before the zip
is saved. The format is in
[diagnostics/bundle-format.md](diagnostics/bundle-format.md).

## What is never collected

Gespan is built not to record these in its diagnostics, and scrubs what it
does record:

- Your code, file contents, diffs, prompts, transcripts, voice recordings,
  terminal output or provider responses.
- API keys, tokens or passwords, including anything Gespan reads from the
  keychain or your accounts.
- Your full home-directory path, project names or repository URLs.

Before anything is shown in the report preview or saved, Gespan scrubs it
**on your machine**: your home folder becomes `~`, project folders become
`<project-1>` and so on (the file name is kept), email addresses become
`<email>`, and anything that looks like a key or token becomes
`<redacted>`, as do the tracker and Codex sign-in tokens Gespan knows about.
Log fields with these names are dropped entirely: `prompt`, `content`,
`diff`, `output`, `body`, `env`, `audio`, `transcript`.

Error messages are free text, and scrubbing works by known patterns, so it
can miss something unusual. That is why you see the exact contents of a
report before it is saved. What **you type** into a report (title, what
happened and steps) is scrubbed the same way, so a pasted path, token or
email address is hidden, but everything else stays as you wrote it.

## Where it goes

- **Your machine**: the logs folder for each app (Settings → Diagnostics →
  Open logs folder). On macOS that is `~/Library/Logs/dev.gespan.native/`
  for Gespan Native and `~/Library/Logs/dev.gespan.desktop/` for Gespan.
  Saved reports go to your Downloads folder.
- **By email, if you choose "Save zip and email"**: Gespan opens your mail
  app with a message to the Gespan maintainer (`itparser+gespan@gmail.com`)
  holding your title, what happened, steps, app and OS versions and the log
  line IDs, all scrubbed. You attach the zip and press Send yourself. Only
  the maintainer sees it.
- **On GitHub, if you choose "Save zip and open GitHub"**: Gespan opens
  GitHub's new-issue page in
  [0xvasanth/gespan](https://github.com/0xvasanth/gespan/issues) with your
  title, what happened, steps and app and OS versions filled in. You need a
  GitHub account, and the issue and any zip you attach to it are
  **public**.

## How long it is kept

| What | Where | Kept for |
|---|---|---|
| Logs | Your machine | 7 days or 50 MB in total, whichever comes first |
| Crash records | Your machine | Until saved in a report or dismissed, and at most 7 days |
| Failure counters | Your machine | 30 days |
| Saved report zips | Your Downloads folder | Until you delete them |
| Emailed reports | The maintainer's mailbox | Until you ask for deletion |
| Report issues | GitHub (public) | Until a maintainer deletes the issue |

## How to have it deleted

- **On your machine**: delete the logs folder (Settings → Diagnostics →
  Open logs folder) and any saved report zips at any time.
- **A report you emailed**: reply to your email asking for it to be deleted,
  and the maintainer deletes the message and its attachment.
- **A GitHub issue**: ask in the issue, or email the maintainer, and the
  issue or the attachment is deleted.
