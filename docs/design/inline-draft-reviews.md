# Inline draft reviews

Status: built. Written 2026-11-12. Builds on [review-setup.md](review-setup.md).

## What it does

Someone with write access comments `/gespan review` on a GitHub pull request or
a GitLab merge request. A review automation runs. Its findings go on the pull or
merge request as a **draft**: a GitHub pending review, or GitLab draft notes.
Each finding is an inline comment on its file, its side of the diff (old or
new) and its line. Nothing is submitted. My tasks asks **Submit review** or
**Keep as draft**.

## The trigger

- The command is the words `/gespan review` at the start of a comment's first
  non-blank line, in any case. Text after them is the asker's note, and it
  reaches the agent as untrusted data like any comment. `/review` alone is not
  used, because other review bots already answer it.
- Two events: `pull_request_review_command` (GitHub) and
  `merge_request_review_command` (GitLab). A command comment raises the
  command event *instead of* the plain comment event, so a "respond to
  comments" automation does not also answer it.
- Only an author with write access starts it (`Occurrence.trusted`: push,
  maintain or admin on GitHub, Developer or above on GitLab). Naming an author
  on the trigger does not waive this. The connected account's own command
  counts: Gespan never posts one (see "Inert text").
- The review setup sheet offers it as "gets a "/gespan review" comment from
  someone who can push". This saves the GitHub trigger for the chosen repo plus
  a GitLab trigger for any merge request in the project.

## Results: "leaves a draft review on the PR for you to submit"

A new `review_results` value, `pending`.

- **The agent never writes.** The run stays read-only. Under `pending`,
  `review_policy::verdict` denies `gh pr review` and `gh pr comment` even on the
  target, along with every `gh api` write. The agent ends its reply with a
  fenced `gespan-review` JSON block:
  `{"summary": …, "comments": [{"path", "side": "new"|"old", "line", "body"}]}`.
- **Gespan posts, to the event's pull request only.** The run's target comes
  from the claimed event key (`ReviewTarget::from_event_key`), never from the
  reply. Fields such as `repo` or `number` in the reply are ignored.
- Every harness supports it, Pi and omp included, since no shell is needed.
- GitLab runs are always prefetched (MR text and diff), because the reviewer's
  only forge tool is `gh`. With a GitLab target, every `gh` read and write is
  denied: a GitHub repo with the same name is someone else's. Draft and Post
  results clamp to "keep in Gespan" on GitLab.

## Placing comments

`review_draft::diff_map` parses the unified diff: `gh pr diff` on GitHub, and
on GitLab the patch built from `/merge_requests/:iid/diffs`. Each hunk is read
by its line counts, so a removed line starting with `--` is never taken for a
header. Renames, added and deleted files, `\ No newline` markers, CRLF and
quoted paths are all handled.

| Line kind | GitHub | GitLab position |
|---|---|---|
| added | `side: RIGHT`, `line` = new number | `new_line` only |
| removed | `side: LEFT`, `line` = old number | `old_line` only |
| unchanged, inside a hunk | either side | both `old_line` and `new_line` |

A finding goes inline only when its path (either name of a renamed file) and
its line on that side are in a hunk. Anything else goes into the review's
summary under "Notes on lines outside the diff", with the place the agent
meant. There is a cap of 50 inline comments.

**Fallbacks:**

- If GitHub refuses the inline comments (422), Gespan retries once with
  every finding in the summary.
- If a pending review already exists, Gespan says so and does not retry.
- If GitLab refuses one position, that finding moves into the summary note.
- With no diff, or no `diff_refs`, every finding goes in the summary.

## Inert text

GitLab runs quick actions (`/merge`, `/approve`, …) from a note when it is
published, bots act on `/commands`, and a `/gespan review` line would start
another review. Every line Gespan posts that starts with `/` is escaped as
`\/`, which renders the same, fenced or not, since forges disagree on what
a fence is. The whole summary is escaped, not just the bodies, and file paths
from the reply lose control characters and backticks. Draft and Post runs,
where the agent posts with `gh`, are denied any body with such a line. That
is why the account's own `/gespan review` can count: no Gespan reviewer can
post one.

## Asking the user

The draft is saved in `review_drafts` (the automations database) with status
`waiting`. My tasks lists it as a **Draft reviews** task. The card shows the
summary and every inline comment with its place. The choices:

- **Submit review**: on GitHub, `POST …/reviews/{id}/events` with
  `event: COMMENT`. On GitLab, each of Gespan's own draft notes is published
  with `PUT …/draft_notes/{id}/publish`, never `bulk_publish`, which would also
  send drafts the user wrote by hand.
- **Keep as draft**: nothing is sent and Gespan stops asking. The draft
  stays on the forge.

A draft is settled once. A second click does nothing. Waiting drafts survive
a restart.
