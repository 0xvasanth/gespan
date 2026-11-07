# Project actions

Actions are a project's everyday commands: test, build, start the dev server. Gespan lists them in the footer, in the command palette and to agents, so you and your agents run the same commands the same way.

Gespan reads actions from files; it never runs anything when you open a project. An action runs only when you choose it, or when an agent asks to run it.

## `.gespan/tasks.json`

Shared actions live in `.gespan/tasks.json` in the project, so everyone who clones the repository gets them. The file uses VS Code's `tasks.json` format (`"version": "2.0.0"`), and it can have comments and trailing commas.

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "Test core",
      "type": "shell",
      "command": "cargo test --lib",
      "options": { "cwd": "crates/gespan-core" },
      "detail": "gespan-core unit tests",
      "icon": { "id": "test-tube" }
    },
    {
      "label": "Run app",
      "type": "shell",
      "command": "bun run dev",
      "isBackground": true,
      "icon": { "id": "rocket" }
    }
  ]
}
```

### Fields

Gespan reads these fields of each task and ignores the rest.

| Field | Meaning |
| --- | --- |
| `label` | The action's name. Required. |
| `type` | `shell` (the default) runs `command` as a shell line. `process` quotes `command` and `args` as one program. `npm` runs `script` with the project's package manager. |
| `command`, `args` | What to run. An arg can be a string or `{ "value": "..." }`. Args with spaces are quoted for you. |
| `options.cwd` | The folder to run in, relative to the project. Default: the project root. |
| `options.env` | Environment variables, as `{ "NAME": "value" }`. |
| `detail` | A short line shown under the name. |
| `isBackground` | The action keeps running, like a dev server. |
| `icon` | `{ "id": "..." }`: `play`, `test-tube`, `hammer`, `rocket`, `bug` or `package`. Anything else shows `play`. |
| `hide` | Keeps the task out of the list. |
| `windows`, `osx`, `linux` | Overrides for `command`, `args` and `options` on that system. |
| `dependsOn` | Not run. A task with a command still lists, with the note "Dependencies aren't run". |

### Variables

`${workspaceFolder}`, `${workspaceFolderBasename}` and `$ZED_WORKTREE_ROOT` are filled in from the folder the action runs in. `${env:NAME}` is left for the shell, which fills it in when the action runs, so Gespan never reads your environment while listing. `${env:NAME}` can't be used in `cwd`.

Any other `${...}` blocks the action with "Uses ${…}, which Gespan can't fill in".

### Blocked actions

An action that can't run is still listed, greyed out, with the reason. The reasons are:

- It uses a variable Gespan can't fill in.
- "Runs outside the project": `options.cwd` leaves the project folder.
- "Has no command Gespan can run", such as a `gulp` or `typescript` task that only VS Code can run.
- "Only runs other tasks, which Gespan doesn't do yet": it has `dependsOn` and no command.

A file that can't be parsed, such as one you are halfway through editing, shows as one "Couldn't read" row. The other sources still list.

## Where else Gespan finds actions

Gespan also lists commands from other tools' files. These are listed in this order, and the first file that exists in each row is the one read:

1. `.gespan/tasks.json`, then your personal actions
2. Conductor: `.conductor/settings.toml`, `conductor.json`
3. Superset: `.paradev/config.json`, `.superset/config.json`
4. Cursor: `.cursor/environment.json`
5. Codex: `.codex/environments/environment.toml`
6. Claude Code: `.claude/launch.json`
7. VS Code: `.vscode/tasks.json`
8. Zed: `.zed/tasks.json`
9. `package.json` scripts, run with `bun`, `pnpm`, `yarn` or `npm` according to the lockfile
10. `deno.json`, `deno.jsonc`
11. `mise.toml`, `.mise.toml`
12. `justfile`
13. `.cargo/config.toml` aliases
14. `Procfile`
15. `Makefile`, `makefile`, `GNUmakefile`

### De-duplication

The same command often appears in several files, so Gespan drops repeats:

- A detected action with the same name as an action in `.gespan/tasks.json` is dropped. Your repository's file can never hide your personal actions.
- An action that runs the same command in the same folder as an earlier one is dropped, so the earlier source wins. Blocked actions are left out of this check.
- Two actions that still share an id get a number, such as `Build (2)`.

## Running actions

**Footer.** The Actions button in the footer opens a menu with a section per source: Project, Personal, then each detected file. A long section is collapsed until you open it. Choosing an action runs it in its own terminal in the dock, in the action's folder with its environment. Running it again reuses that terminal while it sits idle at a prompt.

Once you have run an action, the button shows it and runs it again with one click. That holds only while the action is unchanged: if its command, folder or environment changed, or any file Gespan reads actions from changed, such as a `package.json` script body, the button opens the menu instead. A fresh clone's button always opens the menu.

**Command palette.** Press ⌘P (Ctrl+P), type `>`, then "Run action:" and the action's name. This runs the action from the menu as it is now.

Actions are not available in remote projects.

## Adding, sharing and saving

- **Add action** in the menu opens a dialog with a name, a command and an icon. **Share with the project** is on by default and adds the action to `.gespan/tasks.json`. Turn it off to keep the action personal: it is stored in Gespan, outside the repository, and only you see it. A shared add shows "Added to .gespan/tasks.json. Commit it to share it."
- **Edit action** changes a shared or personal action. The rest of the file, including fields Gespan doesn't read, stays as it was.
- **Save to project** in the menu copies a detected action, such as a `package.json` script, into `.gespan/tasks.json`. It keeps the command, folder, environment, detail, icon and background flag.
- **Comments.** Gespan edits the file as JSON and can't keep comments. If `tasks.json` has comments, Gespan won't rewrite it: the dialog says so and offers to open the file for a hand edit. A task with `args`, per-OS overrides or workspace variables is also edited by hand.

## Agents

An agent session can list and run actions through four tools of the `actions` toolkit:

| Tool | What it does |
| --- | --- |
| `actions_list` | Lists every action in the session's checkout with its `id`, `label`, `detail`, `source`, `command`, `cwd`, `background` and `blocked` reason, plus files that couldn't be read. Detected actions are included. |
| `actions_run` | Runs an action. Takes `id` and `command`. Returns the exit code and output. |
| `actions_output` | Takes a `runId` and returns the output printed since the last read, once the run ends or after about 50 seconds. |
| `actions_stop` | Takes a `runId` and stops the run and everything it started. |

How it behaves:

- **`{id, command}`.** `actions_run` needs the `command` that `actions_list` showed. If the project's files changed that action since, nothing runs and the agent is told to list again. A blocked action or an unknown `id` also runs nothing.
- **Waiting.** A run returns when it ends. After about 50 seconds, or about 5 for a background action, it returns with status `running`, and the agent follows it with `actions_output` and ends it with `actions_stop`. A run keeps its last 200 lines, each cut at 2,000 characters.
- **Read-only and plan sessions** get `actions_list` only. They never run actions.
- **Scoped to the checkout.** An agent lists and runs actions in its own session's checkout, the worktree if it has one, and reads output only of runs started there.
- **Approvals.** `actions_run` and `actions_stop` are marked as changing things, so the agent app's own approval prompts apply as for any tool.
- **Mirror tabs.** Each agent run opens a read-only tab named `<action> (agent)` at the end of the project's dock, so you can watch it. When it ends the tab shows `exited <code>` or `stopped`. A closed dock stays closed and the active tab stays active. A project with no window open shows nothing, and the run still goes ahead.

## What Gespan never does

- It runs nothing when a project opens, and it never runs an action in the background on its own. Listing only reads files.
- It reads only regular files inside the project, up to 1 MB. A link that leaves the project, a pipe or a larger file is shown as "Couldn't read" instead.

Return to the [README](../README.md) for a quick start, or see [Agent access](agent-access.md) for the `/operator` CLI.
