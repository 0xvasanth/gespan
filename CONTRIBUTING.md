# Contributing

Gespan is early and I’m the only maintainer, so small and focused lands much faster than large and ambitious. Past that, the door is open - bug reports and fixes are genuinely welcome.

Please don’t open PRs that add a new provider right now. The existing harnesses still need to agree on a few patterns, and a new adapter would copy whatever is there today. See [New providers](#new-providers).

## Get it running

You need Node.js 22+, [Bun](https://bun.sh), a current stable Rust toolchain, and at least one provider CLI installed and logged in. See [provider setup](docs/providers.md) for installation and login instructions.

macOS, Linux, and Windows are supported targets. See [building from source](docs/building.md) for platform prerequisites and packaging commands.

```bash
bun run native:dev
```

## Where things live

- `crates/gespan-gpui/` - the desktop app: shell, composer, transcript, editor, settings, and platform UI
- `crates/gespan-core/` - provider transports, persistence, accounts, Git and files, and remote connections
- `crates/` - the other shared Rust crates, such as the board store, tracker clients, and the MCP hub
- `host/` - the remote host, in TypeScript, which runs sessions on another machine
- `mobile/` - the Expo companion app

`crates/gespan-core/src/provider/` is the most useful place to start if you want to fix something real. Each provider has a transport that turns the CLI's output into Gespan's own events, so you can fix a Codex parsing bug with only Claude Code installed. That's for the providers we already ship - please don't add a new one yet.

See the [native guide](docs/native/README.md) and [architecture](docs/native/migration.md) for more.

## Before you push

```bash
bun run check
```

That runs the host tests, then `cargo fmt`, `cargo clippy`, and `cargo test` for both Rust workspaces. If it's green locally it should be green on GitHub. `bun run test:host`, `bun run check:rust`, and `bun run native:check` run the parts separately when you only touched one side.

## New providers

I’m pausing new harnesses until the current ones share the same patterns - session lifecycle, catalog probes, usage, approvals, and how slash commands and skills are wired. A PR that adds another provider will be closed for now, even if the work is good. Fixes, tests, and protocol bugs on Claude, Codex, Cursor, Grok, OpenCode, Antigravity, Pi, omp, fx, Hermes Agent, and Devin are still the best kind of contribution.

When the pause lifts, this section goes away.

## Pull requests

`main` is protected: changes land through a pull request with passing CI. Keep a PR to one thing, and say what changed and why. The [PR template](.github/pull_request_template.md) covers the rest. If it changes the UI, a before/after screenshot helps a lot.

For anything that moves product direction - a new surface, new provider behavior, a refactor that changes the shape of the app - open an issue first. That’s not gatekeeping, I’d just rather you hear “I’m already halfway through that” before you write it than after. New providers are the exception: don’t send the adapter, even from an issue, until the pause above is gone.

I might close a PR, ask you to shrink it, or end up implementing the idea differently. That’s a call about scope and timing, not about you or the quality of your work.

Be kind: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). Security reports: [SECURITY.md](SECURITY.md).
