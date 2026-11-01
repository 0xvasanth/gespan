# Gespan — project rules

## UI tooling: use Bun

For the TypeScript side (`host/`, `mobile/`, `scripts/`, `tests/`), use Bun
instead of npm/npx:

- Install dependencies: `bun install`, not `npm install` / `npm ci`
- Run package scripts: `bun run <script>` (e.g. `bun run native:dev`,
  `bun run check`), not `npm run <script>`
- One-off binaries: `bunx <tool>` (e.g. `bunx vitest run <file>`,
  `bunx tsc --noEmit`), not `npx <tool>`
- Add or remove packages: `bun add` / `bun add -d` / `bun remove`

Tests still run through Vitest (`bun run test:host` for the host,
`bunx vitest run --config mobile/vitest.config.ts` for mobile), not `bun test`,
because the suites rely on Vitest APIs and config.

The Rust side (`crates/`) keeps using `cargo`.
