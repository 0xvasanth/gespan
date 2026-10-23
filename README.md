<p align="center">
  <img src="public/rebase.png" alt="Rebase" width="88" />
</p>

<h1 align="center">Rebase</h1>

<p align="center">
  <strong>A GUI for your coding agents.</strong>
</p>

<p align="center">
  <img width="1680" height="1050" alt="Rebase with agent sessions open side by side" src="https://github.com/user-attachments/assets/2cd4a6ec-eb1e-4b45-8627-a76442ea3874" />
</p>

Works with your subscriptions on Claude Code, Codex, Cursor, Grok Build, OpenCode, Antigravity, Pi, omp, fx, Hermes Agent, and Devin. If they’re installed and logged in, Rebase can run them. Tabs are sessions. The composer is the input. Rebase does not sell tokens.

## Install

Install and sign in to at least one supported agent first. See [provider setup](docs/providers.md) for instructions.

| Platform | Download |
| --- | --- |
| macOS, Apple Silicon | [Rebase.dmg](https://github.com/0xvasanth/rebase/releases/latest) |
| macOS, Intel | [Rebase_x64.dmg](https://github.com/0xvasanth/rebase/releases/latest) |
| Linux, x86_64 | [.deb, .rpm, or AppImage](https://github.com/0xvasanth/rebase/releases/latest) |
| Windows, x86_64 | [Installer](https://github.com/0xvasanth/rebase/releases/latest) |

On macOS, open the DMG and drag Rebase to Applications. On Windows, run the installer. See [Linux setup](docs/install.md) for dependencies and package instructions.

## Get started

Open a project folder, choose an agent, and send a message. Each tab is a session. Split panes to work with sessions side by side.

- Keep conversations organized by project.
- Use separate Git worktrees for parallel tasks.
- Edit files and review changes in the app.
- Start a message with `/operator` to let an agent open sessions and manage worktrees and notes.

Pilots are experimental agents that work across your projects. They keep a memory and can run scheduled tasks you set up with them.

Rebase is still early. If something breaks, [report a bug](https://github.com/0xvasanth/rebase/issues).

## Guides

- [Provider setup](docs/providers.md)
- [Linux installation](docs/install.md)
- [Agent access with /operator](docs/agent-access.md)
- [Remote sessions, experimental](docs/remote-access.md)

## Build from source

You’ll need Node.js 20+ and a stable Rust toolchain. See [build setup](docs/building.md) for platform dependencies and packaging.

The mobile app is a separate Expo project in [`mobile/`](mobile/README.md). Run it with `cd mobile && npm install && npm start`.

```bash
npm install
npm run tauri dev
```

Small, focused contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.

## Contributors

Thanks to everyone who contributes to Rebase!

[![Rebase contributors](https://contrib.rocks/image?repo=0xvasanth/rebase)](https://github.com/0xvasanth/rebase/graphs/contributors)

Contributor image by [contrib.rocks](https://contrib.rocks).

## License

[MIT](LICENSE). Provider names and logos are trademarks of their owners. See [NOTICE](NOTICE).
