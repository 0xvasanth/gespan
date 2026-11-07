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
| `label` | The action's name. Required, except for `npm` tasks, which default to `npm: <script>`. |
| `type` | `shell` (the default) runs `command` as a shell line. `process` quotes `command` and `args` as one program. `npm` runs `script` with the project's package manager. |
| `command`, `args` | What to run. An arg can be a string or `{ "value": "..." }`. Args with spaces are quoted for you. |
| `options.cwd` | The folder to run in, relative to the project. Default: the project root. |
| `options.env` | Environment variables, as `{ "NAME": "value" }`. A name a shell or the loader reads at startup is refused: see [Blocked actions](#blocked-actions). |
| `detail` | A short line shown under the name. |
| `isBackground` | The action keeps running, like a dev server. |
| `icon` | `{ "id": "..." }`: `play`, `test-tube`, `hammer`, `rocket`, `bug` or `package`. Anything else shows `play`. |
| `hide` | Keeps the task out of the list. |
| `windows`, `osx`, `linux` | Overrides for `command`, `args` and `options` on that system. |
| `dependsOn` | Not run. A task with a command still lists, with the note "Dependencies aren't run". |

### Variables

`${workspaceFolder}`, `${workspaceFolderBasename}` and `$ZED_WORKTREE_ROOT` are filled in from the checkout: the project folder, or the session's worktree when it has one. `${env:NAME}` is left for the shell, which fills it in when the action runs, so Gespan never reads your environment while listing. `${env:NAME}` can't be used in `cwd`.

Any other `${...}` blocks the action with "Uses ${…}, which Gespan can't fill in".

### Blocked actions

An action that can't run is still listed, greyed out, with the reason. The reasons are:

- It uses a variable Gespan can't fill in.
- "Runs outside the project": `options.cwd` leaves the project folder.
- "Has no command Gespan can run", such as a `gulp` or `typescript` task that only VS Code can run.
- "Only runs other tasks, which Gespan doesn't do yet": it has `dependsOn` and no command.
- "Sets NAME, which could run code before the command": its environment sets `ZDOTDIR`, `BASH_ENV`, `ENV`, `PROMPT_COMMAND`, `PS4`, `LD_PRELOAD`, `LD_LIBRARY_PATH`, `LD_AUDIT` or any `DYLD_` name. A shell or the loader reads these before the command runs. The check applies to every source, in the menu, the palette and for agents.

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

**Footer.** The Actions button in the footer opens a menu with a section per source: Project, Personal, then each detected file. A long section is collapsed until you open it. Each row shows the command that clicking it runs, after the action's detail when it has one, such as "Ships it — echo ship". An action that sets environment variables adds their names, such as "· +env PORT,MODE". Values are never shown. A blocked row shows its reason instead. Choosing an action runs it in its own terminal in the dock, in the action's folder with its environment. `${env:NAME}` in an environment value is filled in from that terminal's environment, so `${env:PATH}` is your login shell's `PATH`. Running it again reuses that terminal while it sits idle at a prompt.

The Actions button is split in two. The chevron always opens the menu. The main part runs the action you last ran, and shows its name. That holds only while the action is unchanged: if its command, folder or environment changed, or any file Gespan reads actions from changed, such as a `package.json` script body, the main part opens the menu instead. That includes those files in the action's own folder, such as `web/package.json` for a `bun run dev` with `options.cwd` set to `web`. A fresh clone's button always opens the menu.

**Command palette.** Press ⌘P (Ctrl+P), type `>`, then "Run action:" and the action's name. Each row shows the command it runs. This runs the action from the menu as it is now.

Actions are not available in remote projects.

## Adding, sharing and saving

- **Add action** in the menu opens a dialog with a name, a command and an icon. **Share with the project** is on by default and adds the action to `.gespan/tasks.json`. Turn it off to keep the action personal: it is stored in Gespan, outside the repository, and only you see it. A shared add shows "Added to .gespan/tasks.json. Commit it to share it."
- **Edit action** changes a shared or personal action. The rest of the file, including fields Gespan doesn't read, stays as it was. When two tasks in the file share a name, the second is listed as `Build (2)`, and editing or deleting it changes that task.
- **Save to project** in the menu copies a detected action, such as a `package.json` script, into `.gespan/tasks.json`. It keeps the command, folder, environment, detail, icon and background flag. Your checkout's own path, such as a worktree folder that `${workspaceFolder}` was filled in with, is saved as `${workspaceFolder}`, so the shared file works in everyone's clone.
- **Comments.** Gespan edits the file as JSON and can't keep comments. If `tasks.json` has comments, Gespan won't rewrite it: the dialog says so and offers to open the file for a hand edit. Trailing commas are not comments: Gespan rewrites such a file without them. A task with `args`, per-OS overrides or workspace variables can still be renamed, or have its icon or detail changed, in Gespan; only changing its command is done by hand in `tasks.json`.

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
- **Already running.** If the same action is already running in the same folder, `actions_run` returns that run, with its id and recent output, instead of starting another.
- **Waiting.** A run returns when it ends. After about 50 seconds, or about 5 for a background action, it returns with status `running`, and the agent follows it with `actions_output` and ends it with `actions_stop`. A run keeps its last 200 lines, each cut at 2,000 characters.
- **Read-only and plan sessions** get `actions_list` only. They never run actions.
- **Scoped to the checkout.** An agent lists and runs actions in its own session's checkout, the worktree if it has one, and reads output only of runs started there.
- **Approvals.** `actions_run` and `actions_stop` are marked as changing things, so the agent app's own approval prompts apply as for any tool.
- **Mirror tabs.** Each agent run opens a read-only tab named `<action> (agent)` at the end of the project's dock, so you can watch it. When it ends the tab shows `exited <code>` or `stopped`. A closed dock stays closed and the active tab stays active. A project with no window open shows nothing, and the run still goes ahead.

## What Gespan never does

- It runs nothing when a project opens, and it never runs an action in the background on its own. Listing only reads files.
- It reads only regular files inside the project, up to 1 MB. A link that leaves the project, a pipe or a larger file is shown as "Couldn't read" instead.

Return to the [README](../README.md) for a quick start, or see [Agent access](agent-access.md) for the `/operator` CLI.
