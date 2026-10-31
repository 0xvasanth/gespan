<p align="center">
  <img src="docs/assets/logo.png" alt="Gespan logo" width="120" />
</p>

<h1 align="center">Gespan</h1>

<h3 align="center">One desktop app to drive all of your coding agents.</h3>

<p align="center">
  Run Claude Code, Codex, Cursor, OpenCode and eight more coding agents side by side,<br/>
  plan their work on a board, and let a Pilot keep it moving.<br/>
  It uses the subscriptions you already pay for. No new accounts and no token resale.
</p>

<p align="center">
  <a href="https://github.com/0xvasanth/gespan/actions/workflows/ci.yml"><img src="https://github.com/0xvasanth/gespan/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/0xvasanth/gespan/releases"><img src="https://img.shields.io/github/v/release/0xvasanth/gespan?include_prereleases&sort=semver&label=release&color=7c3aed" alt="Latest release" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-a78bfa.svg" alt="License: MIT" /></a>
  <a href="#install"><img src="https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-4c1d95.svg" alt="Platforms" /></a>
  <a href="https://tauri.app"><img src="https://img.shields.io/badge/built%20with-Tauri%20%2B%20Rust-24c8db.svg" alt="Built with Tauri" /></a>
  <a href="https://github.com/0xvasanth/gespan/stargazers"><img src="https://img.shields.io/github/stars/0xvasanth/gespan?style=flat&color=c4b5fd" alt="GitHub stars" /></a>
</p>

<p align="center">
  <sub>Gespan began as a fork of <a href="https://github.com/hardbeat920/monocode">MonoCode</a> and has grown quickly from there. <a href="#acknowledgements">Read the acknowledgements.</a></sub>
</p>

<p align="center">
  <a href="#install"><strong>Install</strong></a> ·
  <a href="#how-it-works"><strong>How it works</strong></a> ·
  <a href="#features"><strong>Features</strong></a> ·
  <a href="#supported-agents"><strong>Providers</strong></a> ·
  <a href="#documentation"><strong>Docs</strong></a> ·
  <a href="#build-from-source"><strong>Build from source</strong></a> ·
  <a href="#faq"><strong>FAQ</strong></a> ·
  <a href="#contributing"><strong>Contribute</strong></a>
</p>

---

> **Gespan** (*guh-SPAHN*) is Dutch for a team of horses or oxen harnessed
> together to pull one wagon. You hold the reins, and a team of agents does the pulling.

## How it works

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/how-it-works-dark.gif" />
  <img src="docs/assets/how-it-works-light.gif" alt="Animation: a Linear ticket lands in the Inbox and becomes a card on the Board. Run with Claude Code starts a session in its own worktree and the card moves to In progress. A daily 09:00 automation starts a Codex run, and the Claude session opens a pull request, which moves its card to Review. A Pilot triages a GitHub issue and starts a Cursor CLI session, finished sessions report back to the Pilot, and the merged card moves to Done." width="100%" />
</picture>

1. **Board.** Tickets from GitHub, GitLab, Linear, Jira and Azure DevOps land in
   the Inbox and become cards.
2. **Sessions.** Run a card with any agent, each in its own Git worktree. The
   card moves as the session starts, opens a pull request and finishes.
3. **Automations.** Agents run on a schedule, like a daily dependency audit, and
   their runs show up on the board.
4. **Pilots.** A long-running agent with memory triages the Inbox, starts
   sessions and gets a report back from each one.

> [!NOTE]
> This is where Gespan is heading, and parts of the loop are still being built:
> adding Inbox items to the board, pull requests moving cards, and Pilots and
> automations working the board directly. Progress is tracked in
> [#211](https://github.com/0xvasanth/gespan/issues/211).

## Why Gespan?

Coding agents are good now, and you probably use more than one. Each one comes
with its own terminal, its own keybindings and its own idea of what a session
is. Keeping track of five agents across three repos is a job of its own.

Gespan gives them one home:

- **Bring your own agents.** Gespan runs the CLIs you have already installed and
  signed in to. Your existing plans, logins, skills and MCP setup carry over.
- **Tabs are sessions.** Every conversation is a tab. Split panes to watch
  several agents work side by side, each in its own Git worktree if you like.
- **Native and local.** Gespan is built in Rust, on Tauri and GPUI. Sessions are stored on
  your machine, and the app has [no telemetry](docs/privacy.md).
- **You stay in charge.** You can review diffs, keep or undo what each turn
  changed, approve tools and stop runaway agents from a single window.

> [!NOTE]
> Gespan does not sell tokens or proxy your requests. It is a GUI over agents
> you already have, and your usage stays between you and your provider.

## Supported agents

If an agent is installed and signed in, Gespan can run it. One is enough to get
started. Gespan checks for each CLI at startup and points you to the install
steps for any that are missing.

| | | | |
| :-- | :-- | :-- | :-- |
| **[Claude Code](https://claude.com/product/claude-code)** | **[Codex](https://developers.openai.com/codex/cli)** | **[Cursor CLI](https://cursor.com/cli)** | **[Grok Build](https://docs.x.ai/build/overview)** |
| **[OpenCode](https://opencode.ai)** | **[Antigravity](https://antigravity.google/docs/cli-install)** | **[Pi](https://pi.dev/)** | **[omp](https://omp.sh)** |
| **[fx](https://fx.sh)** | **[Hermes Agent](https://github.com/NousResearch/hermes-agent)** | **[Kiro CLI](https://kiro.dev/docs/cli/)** | **[Devin CLI](https://docs.devin.ai/cli)** |

Install and login commands for each agent are in [provider setup](docs/providers.md).

## Features

<table>
<tr>
<td colspan="2" valign="top">

### 📋 Board
Plan work as cards across boards and columns, linked to tracker tickets, pull
requests and worktrees. Choose **Run with…** on a card to start a session with
any agent, and the card moves to In progress when it starts and to Review when
it finishes. See the [Board guide](docs/board.md).

</td>
</tr>
<tr>
<td width="50%" valign="top">

### 🗂️ Multi-agent workspace
Organize conversations by project, open each one as a tab, and split panes to
run several agents at once. You can switch models, effort levels and permission
modes for each session.

</td>
<td width="50%" valign="top">

### 🌿 Worktrees built in
Give parallel tasks their own Git worktree so agents never step on each other.
A searchable Git history graph sits in the side panel.

</td>
</tr>
<tr>
<td valign="top">

### 🔍 Review and undo
Turn checkpoints record what each turn changed, including edits made through
shell commands. You can keep or undo a turn's diff and review all changes in a
fast diff viewer with syntax highlighting.

</td>
<td valign="top">

### ✍️ Editor and terminal
Edit files in the built-in code editor, preview Markdown, and open terminal tabs
next to your sessions, all in the same window.

</td>
</tr>
<tr>
<td valign="top">

### 🎛️ Agents that drive Gespan
Start a message with `/operator` to let an agent open sessions, split panes,
create worktrees and read your notes. You can also use `/plan` and
`/orchestrator` to change how a turn runs.

</td>
<td valign="top">

### 🧭 Pilots *(experimental)*
Pilots are long-running agents that work across your projects. They keep a
memory, run scheduled habits, write Markdown and HTML reports, and report back
on the sessions they start.

</td>
</tr>
<tr>
<td valign="top">

### 📥 Inbox and automations
Bring in GitHub, GitLab, Linear, Jira and Azure DevOps work items. Start a
session from an issue, or run agents on a schedule or when an inbox event
arrives.

</td>
<td valign="top">

### 🌐 Remote machines *(experimental)*
Connect a Mac, Linux or Windows host over SSH and run sessions there. They keep
running when your laptop closes. An invite-only iPhone companion lets you check
in from your phone.

</td>
</tr>
</table>


## Install

> [!IMPORTANT]
> Install and sign in to at least one [supported agent](docs/providers.md) first.

Gespan is in public beta. The current release is
[**0.0.1-beta**](https://github.com/0xvasanth/gespan/releases/tag/v0.0.1-beta).
It comes as two desktop apps that share the same data: **Gespan Native**, the
fast native app recommended on macOS, and **Gespan**, recommended on Windows and Linux.

| Platform | Recommended | Also available |
| :-- | :-- | :-- |
| macOS, Apple Silicon | [Gespan Native (.dmg)](https://github.com/0xvasanth/gespan/releases/download/v0.0.1-beta/GespanNative_0.0.1-beta_aarch64.dmg) | [Gespan (.dmg)](https://github.com/0xvasanth/gespan/releases/download/v0.0.1-beta/Gespan_0.0.1-beta_aarch64.dmg) |
| macOS, Intel | [Gespan Native (.dmg)](https://github.com/0xvasanth/gespan/releases/download/v0.0.1-beta/GespanNative_0.0.1-beta_x64.dmg) | [Gespan (.dmg)](https://github.com/0xvasanth/gespan/releases/download/v0.0.1-beta/Gespan_0.0.1-beta_x64.dmg) |
| Windows, x64 | [Gespan (installer)](https://github.com/0xvasanth/gespan/releases/download/v0.0.1-beta/Gespan_0.0.1-beta_x64-setup.exe) | [Gespan Native (installer)](https://github.com/0xvasanth/gespan/releases/download/v0.0.1-beta/GespanNative_0.0.1-beta_x64-setup.exe) |
| Linux, x64 | [Gespan (AppImage)](https://github.com/0xvasanth/gespan/releases/download/v0.0.1-beta/Gespan_0.0.1-beta_amd64.AppImage) · [.deb](https://github.com/0xvasanth/gespan/releases/download/v0.0.1-beta/Gespan_0.0.1-beta_amd64.deb) · [.rpm](https://github.com/0xvasanth/gespan/releases/download/v0.0.1-beta/Gespan-0.0.1-0.beta.x86_64.rpm) | [Gespan Native (AppImage)](https://github.com/0xvasanth/gespan/releases/download/v0.0.1-beta/GespanNative_0.0.1-beta_amd64.AppImage) |

Gespan Native on Windows and Linux is experimental and less tested than on macOS.

**Opening it the first time.** The beta is not yet signed with an Apple
Developer ID or a Windows code-signing certificate, so your system warns you once.

- **macOS:** open the DMG and drag the app to Applications. The first time,
  Control-click the app, choose **Open**, then confirm. If macOS still refuses, run
  `xattr -dr com.apple.quarantine "/Applications/Gespan Native.app"` (or `Gespan.app`).
- **Windows:** in the SmartScreen prompt, choose **More info → Run anyway**.
- **Linux:** make the AppImage executable (`chmod +x`) and run it. See
  [Linux installation](docs/install.md) for dependencies and the `.deb` and `.rpm` packages.

Every release is listed on the
[Releases page](https://github.com/0xvasanth/gespan/releases).

### Move from Gespan to Gespan Native

Gespan Native is a separate app, so Gespan can't update itself into it. Your
data doesn't need copying: Gespan Native opens the same `dev.gespan.desktop`
folder, so your conversations, notes and Pilots are there on first launch.

1. Install Gespan Native from the table above.
2. Quit Gespan, and check it isn't still running in the menu bar or system tray.
3. Back up your data folder:

   | Platform | Data folder |
   | :-- | :-- |
   | macOS | `~/Library/Application Support/dev.gespan.desktop` |
   | Windows | `%APPDATA%\dev.gespan.desktop` |
   | Linux | `~/.local/share/dev.gespan.desktop` (or under `$XDG_DATA_HOME`) |

4. Open Gespan Native.
5. Once you're happy, uninstall Gespan. On Windows, leave **Delete the
   application data** unticked: that folder is now Gespan Native's data too.

Don't keep the same conversation open in both apps. Gespan saves whole
conversations and can overwrite turns added in Gespan Native.

## Quick start

1. **Open a project folder.** Gespan groups your sessions by project.
2. **Pick an agent and a model.** Any installed agent appears in the picker.
3. **Send a message.** The session opens as a tab, and replies stream in.
4. **Go parallel.** Split the pane, start a second agent, and put it in its own
   worktree so the two never clash.
5. **Review.** Look through the diff, then keep or undo each turn's changes.

> [!TIP]
> Try `/operator start two Codex sessions: one to inspect the API and one to review the UI`.
> The agent will open and arrange both sessions for you.

Gespan is still early. If something breaks, please [open an issue](https://github.com/0xvasanth/gespan/issues).

## Documentation

| Guide | What's inside |
| :-- | :-- |
| [Provider setup](docs/providers.md) | Install and log in to each supported agent |
| [Board](docs/board.md) | Cards, columns and tracker tickets, with sessions started from them |
| [Linux installation](docs/install.md) | System dependencies and `.deb` / `.rpm` / AppImage packages |
| [Agent access with `/operator`](docs/agent-access.md) | Let agents open sessions and manage worktrees and notes |
| [Remote sessions](docs/remote-access.md) | Run agents on another machine over SSH *(experimental)* |
| [Jira inbox](docs/jira.md) | Connect Jira Cloud to the Inbox |
| [Building from source](docs/building.md) | Platform prerequisites and packaging |
| [Privacy](docs/privacy.md) | What is recorded locally and what never leaves your machine |
| [Gespan Native](docs/native/README.md) | The Rust + GPUI app: building, checks and current limitations |

## Build from source

You'll need **Node.js 20+**, **[Bun](https://bun.sh)** and a stable
**Rust** toolchain. See [build setup](docs/building.md) for platform
dependencies and packaging.

```bash
git clone https://github.com/0xvasanth/gespan.git
cd gespan
bun install
bun run tauri dev
```

Before opening a pull request, run the same checks CI runs:

```bash
bun run check   # vitest, tsc, cargo fmt, cargo clippy, cargo test
```

The mobile app is a separate Expo project in [`mobile/`](mobile/README.md).
Run it with `cd mobile && bun install && bun run start`.

<details>
<summary><strong>Project layout</strong></summary>

<br/>

| Path | What lives there |
| :-- | :-- |
| `src/app/` | App composition, startup, window shell, release and update UI |
| `src/features/` | Product features: sessions, workspace, pilots, inbox, automations and more |
| `src/integrations/harness/` | The provider-independent harness core and one adapter per agent |
| `src/platform/tauri/` | Browser-to-Tauri adapters for filesystem, PTY and clipboard |
| `src-tauri/` | Rust backend: PTYs, filesystem and Git, session storage, native window |
| `crates/` | Shared Rust crates and the experimental GPUI app |
| `host/` | The remote host that runs sessions on another machine |
| `mobile/` | The Expo iPhone companion |

</details>

## Gespan Native

Gespan Native is a rewrite of Gespan in Rust and [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
It is the recommended app on macOS. On Windows and Linux, the React/Tauri app is
still the main app, and Native there is experimental.

<details>
<summary><strong>Run Gespan Native from source</strong></summary>

<br/>

On macOS you need Node.js 22+, Rust and Xcode:

```sh
bun run native:dev     # launch with a separate development database
bun run native:build   # create an optimized app using installed Gespan data
```

In the development terminal, type `r` and press Enter to rebuild and restart.
There is no hot reload. The release app saves changes to your installed Gespan
data. See the [native guide](docs/native/README.md) for prerequisites, build
output, checks and current limitations.

</details>

> [!WARNING]
> Gespan Native shares `~/Library/Application Support/dev.gespan.desktop/gespan.db`
> with Gespan. Close both apps and back up `gespan.db` before you first use
> Gespan Native. Do not open the same conversation in both apps at once: Gespan
> saves whole conversations and can overwrite turns added in Gespan Native.

## FAQ

<details>
<summary><strong>Is Gespan free?</strong></summary>

<br/>

Yes. Gespan is open source under the [MIT License](LICENSE). You pay only for
the agent plans you already have, directly to their providers.

</details>

<details>
<summary><strong>Do I need API keys?</strong></summary>

<br/>

No. Gespan runs the agent CLIs you have already installed and signed in to, so
it uses the same login and plan as your terminal. If an agent is set up to use
an API key, Gespan uses that setup too.

</details>

<details>
<summary><strong>Does my code or usage go through Gespan?</strong></summary>

<br/>

No. Each agent talks to its own provider, exactly as it does in your terminal.
Sessions are stored on your machine, and the app has no telemetry. See
[Privacy](docs/privacy.md).

</details>

<details>
<summary><strong>Why not just run the agents in terminal tabs?</strong></summary>

<br/>

You can, and Gespan runs those same CLIs. What it adds is the layer around
them: sessions as tabs and split panes, a worktree for each parallel task, a
diff you can keep or undo for each turn, approvals in one place, and a board,
inbox, automations and Pilots that hand work to the agents.

</details>

<details>
<summary><strong>Which app should I install, Gespan or Gespan Native?</strong></summary>

<br/>

On macOS, Gespan Native. On Windows and Linux, Gespan. They share the same data,
so you can switch later. See [Install](#install).

</details>

## Contributing

Contributions are welcome, and small, focused pull requests get merged fastest.
Bug reports, protocol fixes and tests for the existing agents help the most.

- Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.
- For changes to product direction, open an issue first.
- New agent adapters are on hold for now while the existing ones are aligned.
  [CONTRIBUTING.md](CONTRIBUTING.md#new-providers) explains why.
- Be kind: [Code of Conduct](CODE_OF_CONDUCT.md). Report security issues
  privately as described in [SECURITY.md](SECURITY.md).

### Contributors

Thanks to everyone who has helped build Gespan.

<a href="https://github.com/0xvasanth/gespan/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=0xvasanth/gespan" alt="Gespan contributors" />
</a>

## Acknowledgements

Gespan is a fork of [MonoCode](https://github.com/hardbeat920/monocode), the
open-source GUI for coding agents created by
[hardbeat920](https://github.com/hardbeat920) and its contributors, and it would
not exist without their work. Gespan's core ideas come from that project, as
does much of its code. These include tabs as sessions, a provider-independent
harness, and running the agents and subscriptions you already have.

Gespan builds on that foundation and has expanded rapidly in its own direction,
with new features, more agents and many fixes. Fixes still come in from
upstream regularly. The original MIT copyright notice is kept in
[LICENSE](LICENSE). If Gespan is useful to you, please give the original
project a star too.

## License

Gespan is released under the [MIT License](LICENSE).

Provider names and logos are trademarks of their respective owners. See [NOTICE](NOTICE).

<p align="center">
  <sub>If Gespan saves you a few terminal windows, consider giving it a ⭐</sub>
</p>
