# Gespan — project rules

## UI tooling: use Bun

For the TypeScript side (`host/`, `mobile/`, `scripts/`, `tests/`), use Bun
instead of npm/npx:

- Install dependencies: `bun install`, not `npm install` / `npm ci`
- Run package scripts: `bun run <script>` (e.g. `bun run dev`,
  `bun run check`), not `npm run <script>`
- One-off binaries: `bunx <tool>` (e.g. `bunx vitest run <file>`,
  `bunx tsc --noEmit`), not `npx <tool>`
- Add or remove packages: `bun add` / `bun add -d` / `bun remove`

Tests still run through Vitest (`bun run test:host` for the host,
`bunx vitest run --config mobile/vitest.config.ts` for mobile), not `bun test`,
because the suites rely on Vitest APIs and config.

The Rust side (`crates/`) keeps using `cargo`.

## Databases: always Toasty

Every table Gespan owns is a Toasty model (`toasty = "=0.11.0"`), opened through
`gespan_core::orm`. Never write `CREATE TABLE` / `ALTER TABLE`, never open a
Gespan SQLite file with rusqlite, and never probe columns at runtime.

- A new table or column: add or change the model, register it in
  `crates/gespan-core/src/orm/kind.rs`, then generate the migration with
  `cargo run --manifest-path crates/gespan-core/Cargo.toml --bin toasty-migrate -- <kind> <name>`.
- What Toasty's DDL can't express (foreign keys, cascades, CHECKs, partial or
  expression indexes) goes in that kind's constraints, with a test.
- Every query goes through Toasty's typed API: `Model::filter_by_*`, `get_by_*`,
  `create()`, `update()`, `delete()`, `exec`. Never write a raw `SELECT`,
  `INSERT`, `UPDATE` or `DELETE` against a Gespan table, whether through
  `toasty::sql`, `Store::sql()`, board-store's `query_sql` / `execute_sql` or a
  rusqlite connection.
- Raw SQL is only for what Toasty 0.11 cannot express: conditional upserts
  (`ON CONFLICT ... WHERE`), updates that must return a count, CTEs, `json_*`
  and window functions, a projection that must stay on a covering index, and
  queries against FTS5 and sqlite-vec virtual tables, which Toasty cannot model.
  Run it through `toasty::sql` (`Store::sql()` in store code) on the same
  database, with a comment on the statement saying which of these it is.
- Existing raw queries are debt. When you change a file that has them, move its
  queries to the typed API and lower its number in `QUERIES` in
  `tests/toasty_only.rs`. Never raise a number or add a file to make that test
  pass.
- CI guards this: `models_match_the_last_migration` fails when a model changes
  without a migration, and `tests/toasty_only.rs` fails when production code
  adds raw schema SQL, opens a database directly, or adds a raw query line.
- Only other apps' databases (Cursor, Codex, WebKit, the Tauri importer) stay on
  rusqlite. See `docs/toasty.md`.
