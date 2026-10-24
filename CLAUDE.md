# Gespan — project rules

## UI tooling: use Bun

For everything on the UI side (the React/Vite frontend in `src/`, `tests/`,
`host/`, and `mobile/`), use Bun instead of npm/npx:

- Install dependencies: `bun install`, not `npm install` / `npm ci`
- Run package scripts: `bun run <script>` (e.g. `bun run dev`, `bun run check`,
  `bun run tauri dev`), not `npm run <script>`
- One-off binaries: `bunx <tool>` (e.g. `bunx vitest run <file>`,
  `bunx tsc --noEmit`), not `npx <tool>`
- Add or remove packages: `bun add` / `bun add -d` / `bun remove`

Tests still run through Vitest (`bun run test` / `bunx vitest`), not
`bun test`, because the suites rely on Vitest APIs and config.

The Rust side (`src-tauri/`, `crates/`) keeps using `cargo`.
