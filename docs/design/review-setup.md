# Review setup in two clicks

Status: design, not built. Written 2026-11-12.

## Why

Gespan can already review pull requests automatically, but it is hard to set up.
A user who wants "review my PRs" opens Automations, picks a Code Review card, and
lands in the full automation editor with about eight decisions left to make:

- project and repositories
- model
- access level (Supervised, Auto-accept edits, Auto, Full access)
- working copy (this checkout or a fresh worktree)
- conversation (start fresh or continue the last run)
- session folder
- what the run does (start a session or wake a Pilot)
- catch-up window for missed runs

A template only fills in the trigger and the prompt (`Automations::begin` in
`crates/gespan-gpui/src/automations.rs`). Everything else is left at generic
automation defaults, and some of those defaults are wrong for a reviewer:

- Access defaults to `auto` (`crates/gespan-core/src/automations.rs:1415`), so a
  reviewer can edit files and run commands.
- Nothing says where the review goes. The prompt says "Leave a structured
  review", and whether that ends up posted on GitHub depends on what the agent
  decides to do.
- If GitHub is not connected, the trigger menu just says "Not connected". The
  fix lives in Settings → Inbox, which tells the user to install the `gh` CLI and
  run `gh auth login` in a terminal.
- "Review requested", the case most people mean by "review my PRs", is a
  trigger but has no card. The "Review pull requests" and "Review draft PRs"
  cards overlap.

The goal is that a user can understand what a review automation will do and
turn it on in two clicks, without seeing the full editor.

## Goals

1. **Two clicks from the gallery to a running reviewer:** pick the card, then
   Turn on. This holds whenever GitHub is already connected and the project has
   a GitHub remote.
2. **The user reads what will happen before turning it on.** It is shown as one
   plain sentence, and every choice in that sentence can be changed inline.
3. **Safe by default.** A reviewer never edits the working tree. It never posts
   to GitHub unless the user chose posting.
4. **Connecting GitHub happens in place.** If `gh` is missing or not signed in,
   the setup screen says so and offers the fix there.
5. **Where results go is an explicit setting** with three values.

## Non-goals

- Changing the full automation editor. It stays as it is, as the "Customize…" path.
- Setup screens for GitLab, Azure DevOps or other providers. This design is
  shaped so they can follow, but only GitHub is in scope.
- Webhooks or faster event delivery. Events keep arriving through the existing
  Inbox refresh.
- Changing Pilot side reviews or the turn review card.
- Changing the review prompt's substance beyond what the results setting needs.

## Anti-goals (failure even if it works)

- The agent posts to a PR when the user chose Inbox only or Draft.
- The quick setup quietly saves different values from the ones its sentence
  shows.
- A user who already has review automations finds them changed or migrated
  without asking.
- The setup screen grows its own copy of the editor, with every field shown
  again.

## The flow

### Click 1: the card

The gallery's Code Review category shows a single **Review pull requests**
card. It replaces today's "Review pull requests" and "Review draft PRs" cards.
"Find critical bugs" and "Add test coverage" stay as they are. Clicking the card
opens the **review setup sheet** instead of the editor.

### The review setup sheet

One screen with a sentence and two buttons. The sentence, with defaults filled in:

> When a pull request in **acme/app** **asks for your review**, Gespan reviews it
> with **Claude Sonnet**, without changing any files, and
> **keeps the review in Gespan**.

Each bold phrase is an inline dropdown:

| Phrase | Choices | Default |
|---|---|---|
| Repository | Repos of the current project, plus "any repo in this project" | the project's GitHub repo if it has exactly one, else "any repo" |
| Which PRs | asks for your review · is opened · is opened, drafts too | asks for your review |
| Model | the same list the editor's model picker shows | the user's default new-session model (`installed_providers::new_session_choice`) |
| Results | keeps the review in Gespan · drafts a PR review for you to approve · posts the review on the PR | keeps the review in Gespan |

Under the sentence:

- **A readiness line.** "GitHub connected as @login ✓", or a problem with a fix
  button (see below).
- **Buttons:** **Turn on** (primary), **Customize…**, Cancel.

"Customize…" opens the full editor with the draft the sheet built, so nothing
already chosen is lost.

**Turn on** saves and enables the automation, then shows it selected in the
Automations list, the same as saving from the editor.

### Readiness and connecting GitHub

The sheet runs the same check Settings → Inbox uses
(`inbox::Service::status("github")`: `gh` found, `gh auth status` succeeds).
It runs off the UI thread when the sheet opens. That check doesn't return the
login. *(My call)* The sheet says "GitHub connected ✓" without a login rather
than adding another `gh` call.

The repository default needs the project's list of GitHub repos, not just the
checkout's one repo. Use `github_repositories` (`crates/gespan-core/src/inbox.rs:728`),
made public, rather than `scm::github_repo`.

*(My call)* Remote-workspace projects are out of scope. The readiness check and
sign-in are local. For a remote project the sheet says "Quick setup isn't
available for remote projects yet" and offers Customize….

| State | What the sheet shows | Turn on |
|---|---|---|
| Checking | "Checking GitHub…" with a spinner | disabled |
| `gh` not installed | "GitHub CLI isn't installed." Buttons: **Get GitHub CLI** (opens cli.github.com), **Check again** | disabled |
| Not signed in | "Sign in to GitHub to continue." Button: **Sign in**, which opens a Gespan terminal pane running `gh auth login --web`, then re-checks when that command exits. **Check again** is also shown. | disabled |
| Connected | "GitHub connected ✓" (with the login when it is cheap to get) | enabled |
| Project has no GitHub remote | "This project isn't on GitHub." Repository dropdown shows "Choose a repository…" | disabled until a repo is picked |

### What gets saved

Turn on saves an ordinary `Automation`. Nothing new is needed in storage except
the results setting. Fields:

- `name`: "Review pull requests"
- `cwd`: current project
- `harness` / `model`: from the Model phrase
- `workspace_mode`: `current` (see "What code the reviewer reads")
- `reuse_session`: false (start fresh)
- `action`: `Session`
- `session_folder_id`: none
- `missed_run_grace_minutes`: the editor's default
- `triggers`: one GitHub trigger:
  - "asks for your review" → `pull_request_review_requested`
  - "is opened" → `pull_request_opened`
  - "is opened, drafts too" → two triggers, `pull_request_opened` and
    `draft_opened`
  - `repos` holds the chosen repo, or is empty for "any repo in this project"
    (empty already means every repo, `automations.rs` around line 1726)
- `runtime_mode`: set from Results (see below)
- `prompt`: the review prompt plus the results instruction (see below)
- **new:** `review_results`: `"gespan"` | `"draft"` | `"post"`

## Where results go

This setting fixes the "where does the review end up" problem. It sets both
what the agent is told and what it is allowed to do. Prompt text alone is not
the guard.

| Results | Agent is told | Enforcement |
|---|---|---|
| Keeps the review in Gespan | Write the review as your final reply. Do not post to GitHub. | Posting or editing commands are refused, not just discouraged. |
| Drafts a review for you to approve | Post the review with `gh pr review`. | The `gh pr review` / `gh pr comment` / `gh api` write call goes through the approval notice (PRs #454/#455 show approvals as readable commands), so the user sees the exact review text and approves or declines. |
| Posts the review on the PR | Post the review with `gh pr review`. | That one class of call is allowed without asking. Edits to files are still refused. |

In every mode the reviewer must be able to read freely (`gh pr view`,
`gh pr diff`, `gh pr checks`, read-only `gh api`, reading files in the
project), so a run never stalls on an approval for a read.

### Enforcement: build on the existing read-only access

`AccessMode::ReadOnly` (`crates/gespan-core/src/agent.rs:7`) already exists and
is wired into the provider launch paths. Automations can't use it today because
validation rejects `"read-only"` (`crates/gespan-core/src/automations.rs:341`).
The plan:

1. Allow `"read-only"` as an automation `runtime_mode`. The full editor's Access
   dropdown gains it too.
2. Widen the Claude read-only Bash allowlist
   (`crates/gespan-core/src/provider/claude/read_only.rs`) to the `gh` reads
   above: `gh pr view|diff|checks|list`, and `gh api` with GET only.
3. All three Results values run with `read-only`. Draft and Post add one narrow
   exception on top: `gh pr review` / `gh pr comment` aimed at **the PR that
   triggered the run** (the repo and number come from the run's event, not from
   the agent).
   - Draft: that exception raises an approval instead of being denied outright,
     which is today's read-only behavior (`provider.rs:590`).
   - Post: it is allowed.
   - Any other write, including any `gh api` write and any write to another PR,
     is still denied. *(My call: PR text is untrusted input, so Post must not
     let it steer the agent into merging, labeling or commenting elsewhere.)*

### Draft means approval in Gespan (decided by the user)

"Drafts a review for you to approve" means the agent asks to run
`gh pr review` and Gespan shows that request as an approval notice, with the
exact review text. Nothing reaches GitHub until the user approves. It is not a
GitHub pending review.

When nobody is watching, the run waits on the approval and the notice stays in
the notifications list. *(My call)* There is no timeout. If the app quits while
the run waits, the run ends the way any interrupted session does, and nothing
is posted.

### Every agent at launch (decided by the user)

The quick setup must work with every agent the model dropdown offers, not just
Claude. For each harness (Claude, Codex, ACP, Pi, opencode), read-only access
must:

- allow the `gh` reads listed above, with network access where the harness's
  sandbox needs it,
- deny every other write, and
- support the one narrow `gh pr review` / `gh pr comment` exception for the
  triggering PR (approval for Draft, allow for Post).

Today only Claude has command-level filtering for read-only mode
(`provider/claude/read_only.rs`), so each other harness needs its own
equivalent. If one harness turns out to be unable to enforce the narrow
exception, it may ship offering "keeps the review in Gespan" only. That is the
only acceptable fallback, and it must be stated in the PR.

### What code the reviewer reads

*(My call)* The reviewer reads the PR through `gh pr diff` / `gh pr view`, and
reads surrounding code from the project checkout without changing it. It does
not check out the PR. So runs use `workspace_mode: current`, not a worktree.
Checking out the PR would mean writing to the working copy, which read-only
forbids. A fresh worktree would show the local branch, not the PR, so it would
add cost without showing the right code.

The review appears where automation runs already appear: as a session under the
automation's Recent runs and in Runs. Whether "keeps the review in Gespan" should
also raise a notification is left to the builder. Follow what other finished
automation runs do today.

## Existing automations

Existing review automations are not migrated and not changed. If
`review_results` is missing on load, it reads as unset, and the run behaves
exactly as it does today. Only automations made through the sheet carry the
setting. The full editor shows a Results row only when `review_results` is set.

If the user opens the sheet while a review automation for the same project and
trigger already exists, the sheet opens it in the editor instead of making a
duplicate. Builder's call how to match ("same project + a GitHub review
trigger" is enough).

## Constraints

- The UI lives in `crates/gespan-gpui` (GPUI). The Native app is the only desktop
  app, and it must work on macOS, Windows and Linux.
- Work that touches the disk or runs `gh` stays off the UI thread, as the rest
  of Automations does.
- The sheet uses the existing dropdown, button and theme pieces from
  `automations/` and `popover`. It does not get a separate visual language.
- Accessibility: every inline dropdown is a button with a role and a label that
  reads the whole choice ("Results: keeps the review in Gespan").

## Tests the change needs

These follow the repo rule: tests that exercise real behavior.

- The draft the sheet builds, for each Which PRs × Results combination: the
  triggers, the repos, `workspace_mode`, `reuse_session`, `runtime_mode`,
  `review_results`, and the prompt's results instruction.
- Repository default: a single GitHub remote gives that repo, while none or
  several give "any repo".
- Readiness states map to the right message and a disabled or enabled Turn on,
  driven by a stubbed `gh` (`GESPAN_*` test seams already exist for binaries).
- Enforcement, run for every harness:
  with Results = Gespan, a `gh pr review` call is refused. With
  Draft it raises an approval. With Post it runs. In all three, `gh pr diff`
  runs without asking. A `gh pr review` on a different PR number, a `gh pr merge`
  and a `gh api -X POST` are denied in every mode.
- Validation accepts `read-only` as an automation run mode. The read-only Bash
  allowlist accepts the listed `gh` reads and rejects `gh api` with a
  non-GET method.
- Old automations without `review_results` load and run unchanged
  (serde round-trip plus a run).
- Opening the sheet when a matching automation exists opens the editor and does
  not create a second one.
- A layout test, like `automations/layout_tests.rs`, for the sheet at the
  narrow and wide widths the Automations page supports.

## Calls made in this design (not discussed with the user)

These were my defaults. Check them while reading:

1. The default trigger is **asks for your review**, not "PR opened".
2. The default result is **keep in Gespan**. Posting is opt-in.
3. The two PR cards merge into one. "Find critical bugs" and "Add test coverage"
   are untouched.
4. Sign-in runs `gh auth login --web` in a Gespan terminal pane, not a custom
   OAuth flow.
5. Existing automations are left alone. There is no migration.
6. Runs read the PR through `gh` and use the current checkout read-only, with no worktree. Each run starts fresh.
7. Post and Draft may write only to the PR that triggered the run. Every other write is denied.
8. The sheet doesn't show the GitHub login, and remote-workspace projects get Customize… only.
