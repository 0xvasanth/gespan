# Agent access with /operator

## Enable access

Type `/operator` at the start of a composer message to enable Gespan access in that thread. For example, `/operator start two Codex sessions: one to inspect the API and one to review the UI`, or `/operator list my notes`. The slash picker also offers this command. The transcript shows only the request text in a translucent amber bubble; Gespan removes the command from the request sent to the agent and supplies the local `app` CLI path and instructions on that turn. Later turns in the same thread can use the CLI without repeating `/operator`; other threads receive no CLI instructions or app access. The CLI can act only during an active agent turn. The agent can run the shown `app --help` command for the exact JSON input fields.

## CLI actions

Sessions started by an ordinary Operator remain visible in project history.
Pilots use the same project actions, with their **Show started sessions** setting
controlling whether their launches appear in the user's history. This is
independent of navigation: `reveal` defaults to `false`, so a background launch
can stay visible in history without switching the user's current conversation.

- `models.list` shows available providers, models, settings, and permission modes.
- `sessions.start` creates a session in the current project with a prompt. Set `reveal: true` to open its tab, or `placement: "right"` / `placement: "down"` to split the calling session's pane; `besideSessionId` selects another visible session pane in the project. Reuse the returned session ID as the next `besideSessionId` to build nested layouts. By default it submits the prompt; set `draft: true` to save it unsent without starting an agent turn. It accepts a provider, model, effort or other model settings, permission mode, and current checkout or new worktree choice. Set `worktreeCwd` to a path from `worktrees.list` for a specific existing checkout. Use `worktrees.create` to create a worktree on a named new or existing local branch, then pass its path as `worktreeCwd`. Omit `runtimeMode` to inherit the calling session's permission mode, or set it explicitly to override. It returns the new session ID as soon as the pane and prompt are accepted, so the agent can move it into a folder immediately.
- `sessions.list` shows project sessions and their archived status. `sessions.read` returns up to three recent user/assistant exchanges, with a cursor for older exchanges and a per-message character cap. `sessions.send` submits a follow-up to an idle session, while `sessions.draft` saves an unsent message for the user to review. `folders.list` and `folders.move` organize project sessions in sidebar folders, including a new folder.
- `sessions.stop`, `sessions.archive` and `sessions.delete` take a `sessionId` to manage another session in the project. Pilots can select an assigned project with `project`. Stop cancels the current turn and pauses queued messages; archive stops and saves the conversation for later restoration; delete stops and permanently removes the conversation. Open files, terminals and worktrees are kept. These actions cannot target the caller, Pilot chats, habit runs or orchestration workers. Reuse the same request ID when retrying a call.
- When a Pilot successfully stops, archives or deletes a session it is monitoring, its pending completion report for that session is dismissed, including any queued report. The Pilot confirms the action in its current reply. Reports for other sessions and other Pilots are kept. Rejected launches or follow-ups return a CLI error without a later completion report.
- `notes.list` returns titles and short previews; `notes.read` returns one full note by ID.
- `notes.list` accepts `limit` and `offset`. `notes.write` creates a note from its
  `body`, with optional title and tags; pass an existing note `id` to update just
  its title, body or tags while preserving the other fields.

Retry an uncertain mutation with the same request ID and identical input.
Unknown fields and invalid options produce errors instead of being ignored.

## Sibling sessions

Every local session, with or without `/operator`, knows about the other sessions working in the same repository: in its main checkout or any of its worktrees. Nothing needs enabling.

- **At the start of a turn** the agent is told about its siblings, but only when that has changed since it was last told. Each sibling gets one line: its title, its agent, its status (working, waiting on the user, on hold, idle), where it runs (`main checkout` or `worktree <branch>`), its goal (the first line of its first message) and the files it is changing. A started session also names the chat that started it. A `⚠` line flags a file that a sibling changes and this session changes too. The block is capped at 12 sessions and 2,000 characters. The chat shows only what the user typed.
- **`sessions_list`** returns the same roster as data. With `scope: "all"` it also lists sessions in other projects, by title, agent, status and project folder name.
- **`sessions_read`** returns a sibling's recent exchanges, paginated like `sessions.read`. It only reads sessions in the caller's own repository.

A session's files are the paths its own Edit and Write calls touched that are still uncommitted in its checkout. Another session's edits in the same checkout are not counted as its own. Archived sessions, remote sessions and new chats nothing was sent from are left out.

## Starting sessions

Any local chat can start work that runs on its own: "do this in a new worktree" or "do this in the main repo". The agent calls `sessions_start`, and the new session appears in the sidebar at once, under the chat that started it, without taking focus.

- **`sessions_start`** takes `prompt` and `workspace`: `"worktree"` makes a new worktree of the repository (from `base`, or the default branch), `"main"` uses the main checkout, and `"existing:<path>"` uses a worktree already on disk. Optional fields: `harness`, `model`, `title`, `access`, `wake` and `requestId`. It answers with the new session's id, title, folder and permissions.
- **Permissions never widen.** A started session runs in its parent's mode unless it asks for a narrower one (`access`). Asking for a broader mode is refused with `permission_escalation`. A user can still widen a started session from its own composer.
- **Limits.** Sessions can start sessions two levels below a chat (`depth_limit` past that), and at most 5 of one session's started sessions run at once (`fanout_limit`).
- **Retries are safe.** The same `requestId` from the same session returns the first answer and starts nothing new, also after Gespan restarts.
- **Reporting back.** When a started session's turn ends, its parent hears about it as a new turn labelled `[From session "<title>" (id N)]`, with the end of its reply. Nothing interrupts the parent: reports wait until it is idle, and reports that arrive meanwhile come as one turn. `wake` chooses when: `settled` (the default) when it finishes or fails, `always` also when it waits on the user or is stopped, `never` not at all. Approvals and questions go to the user, not the parent. A report not yet delivered survives a restart.
- **Watching.** The parent's Watching chip lists the started sessions it will hear back from. Stop waiting there, and the session keeps running without reporting.
- **`sessions_cancel`** stops a session the caller started, or one they started, and everything below it, leaves first. It can't reach other sessions (`out_of_scope`) or the caller itself (`self_target`).
- **In the sidebar,** a chat with started sessions still running offers **Stop the sessions it started**. Stopping, archiving or deleting a chat leaves its started sessions running; deleting it makes them chats of their own.

Bad input is refused with `invalid_input`, naming the field. Every refusal starts with its code.

## Project actions

Agents can also list and run a project's commands with the `actions` tools; see [Project scripts](actions.md#agents).

## Orchestration workers

Orchestration workers keep their existing scoped `control` workflow and do not receive this app access.

Return to the [README](../README.md) for a quick start.
