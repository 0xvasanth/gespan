# Board Schema v3 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Both apps read and write one shared board schema (v3) in `gespan.db`. Existing Native `board_native_*` data is copied into it once, with nothing lost, and cross-app tests pin the contract.

**Architecture:** A new small crate, `crates/board-schema` (`gespan-board-schema`), owns every board `CREATE` statement, the v3 additions, the one-time copy from `board_native_*` and the shared vocabulary. Both `src-tauri/src/board.rs` and `crates/gespan-core/src/board` call its `ensure()` and keep their own service logic. That logic is pinned by contract tests and by committed cross-app fixtures: each app writes one, and the other app reads it.

**Tech Stack:** Rust, rusqlite 0.40.2 (bundled, same version in both Cargo workspaces), SQLite window functions, Vitest for the one TS model change.

**Spec:** GitHub issue #85 (approved design), background in epic #48 Part 1, working rules in #60.

## Decisions (record these in the PR)

1. **Tauri does not depend on `gespan-core` for board storage.** `gespan-core` lives in a separate Cargo workspace with its own lock file. It pulls in tokio, quinn, rquickjs and image, and it owns its own `Connection`, while Tauri shares `SessionStore`'s mutex-guarded one. So the two apps keep parallel service code, pinned by contract tests. **Schema and migration are not duplicated:** they live in `crates/board-schema`, following the `crates/mobile-link` / `crates/tracker-secrets` precedent from #60. The data-copying migration then exists exactly once.
2. **v3 is additive only.** The four original tables keep their exact DDL. v3 adds columns with `ALTER TABLE … ADD COLUMN` and creates new tables. A Tauri or Native build from before v3 can still open a v3 database.
3. **Version marker:** `board_schema_migrations(version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)`, with version `3`. The DDL is `IF NOT EXISTS` and runs on every open. The data copy runs only when there is no `3` row. It runs in one `BEGIN IMMEDIATE` transaction that rechecks the marker, so the two apps opening the file at the same moment copy once. A rerun never brings back rows the user deleted after the migration.
4. **`board_native_*` tables are left in place, untouched and no longer written.** They are kept for rollback during this release; dropping them is a follow-up. A pre-v3 Native build that keeps writing to them after the migration writes data v3 does not see. The contract doc records this as a downgrade caveat.
5. **Old link columns:** `board_cards.link_kind/link_id` stay the read source for this release (`cards_for_link`, auto-move). The migration backfills them into `board_card_links` with role `primary`. Both apps' `link_card` also keep the `primary` row in step through the shared `sync_primary_link`.
6. **Orphan native rows are skipped.** These are rows whose card or label no longer exists. They are already invisible, and copying them would break the FK constraint and stop the app from opening. A copy failure of any other kind rolls back the whole transaction and fails the open. It never drops data quietly.
7. **Values are copied verbatim** (priority, due date, `done`, bodies, `created_at`). The only exception is positions: subtask positions per card and label positions per board are renumbered to 0..n-1. Labels are numbered in `lower(name)` order, so Native's alphabetical label order is unchanged. `labels()` keeps sorting by name until the label manager (#94) switches it to position.
8. **Activity:** subtask add, toggle and delete, and comment add, now write exactly one `edited` row each; Native skips these today (epic Part 1). The existing detail strings (`priority: High`, …) stay unchanged. Changing them to `from → to` is left as a follow-up for the UI tickets. The migration itself writes no activity.
9. **`cancelled` role** is accepted by both backends, shown in both role pickers ("Cancelled") and in the TS `ColumnRole`. A card in a `cancelled` column never auto-moves, the same as one in `done`. The change is mirrored in `auto_move.rs` and `autoMove.ts`.
10. **No write APIs yet** for relations, custom fields, extra links, the new `card_meta` fields or column colour/WIP/collapse. Their tickets (#94/#93, #95, #90, #103) add them. This ticket ships their tables, the contract and the reads of the new column fields (`Column` gains `color`, `wip_limit`, `collapsed` in both apps).
11. **Tauri's write connection gets `PRAGMA busy_timeout = 5000` before `migrate`.** Without it, Tauri opening the file while Native holds the `IMMEDIATE` migration lock fails with `SQLITE_BUSY` instead of waiting.

## Global Constraints

- Gespan names only; `src/test/legacyNames.test.ts` must pass. No tool or AI attribution in commits or the PR.
- Branch `board/85-schema-v3`; never push `main` or tags; force-push only with `--force-with-lease`.
- UI tooling via Bun (`bun install`, `bun run`, `bunx vitest`); Rust via `cargo`.
- Hotspot files (`Cargo.toml` member lists, `Cargo.lock`s): keep edits additive and regenerate the lock files with cargo; never hand-merge them.
- Positions stay 0..n-1 after every move, delete and the migration.
- Each field change writes exactly one `board_activity` row, and a no-op edit writes none.
- Long checks run one at a time, in the foreground. Rerun the known flaky tests before treating them as failures.

## Review Focus

1. **Both apps open the file at the same time on first v3 launch.** Expected: the data is copied once, neither app errors, and the second app waits on the lock. Test: Task 2 `concurrent_ensure_copies_once`; Task 5 `open_waits_for_a_held_migration_lock`.
2. **A Native user deletes a migrated label or subtask, then restarts.** Expected: it stays deleted. Test: Task 2 `rerun_does_not_resurrect_deleted_rows`.
3. **The v2 data holds orphan rows or gaps in positions.** Expected: the open still succeeds, orphans are skipped, and positions come out 0..n-1. Test: Task 2 `orphans_are_skipped_and_positions_renumbered`.
4. **The copy fails partway through.** Expected: nothing half-copied, no marker written, and the next open retries. Test: Task 2 `failed_copy_rolls_back_and_retries`.
5. **A card is deleted in Tauri, which never reads the v3 tables.** Expected: its meta, labels, subtasks, comments, links and field values go with it. Test: Task 5 `deleting_a_card_removes_its_v3_rows`.

---

## File structure

- Create `crates/board-schema/Cargo.toml`, a package named `gespan-board-schema`. It depends on `rusqlite = { version = "0.40.2", default-features = false, features = ["bundled"] }`. It has a feature `testing = []`, and `tempfile = "3"` as a dev-dependency.
- Create `crates/board-schema/src/lib.rs`, the public API and vocabulary.
- Create `crates/board-schema/src/ddl.rs`, holding `BASE_DDL` (the existing 4 tables, verbatim from `src-tauri/src/board.rs:85-136`) and `V3_DDL`.
- Create `crates/board-schema/src/migrate.rs`, holding `ensure`, the column adds, the native copy and the link backfill.
- Create `crates/board-schema/src/testing.rs` (`#[cfg(feature = "testing")]`), holding the v2 seed and assert helpers and `dump`/`load`.
- Create `crates/board-schema/fixtures/written_by_native.sql` and `written_by_tauri.sql`.
- Create `docs/board-schema.md`, the contract doc.
- Modify the root `Cargo.toml` (`members` += `"crates/board-schema"`), `src-tauri/Cargo.toml`, `crates/gespan-core/Cargo.toml`, `Cargo.lock` and `crates/gespan-gpui/Cargo.lock`.
- Modify `crates/gespan-core/src/board/{mod.rs,extras.rs,auto_move.rs}`, `crates/gespan-core/tests/shared_contracts.rs`, `crates/gespan-gpui/src/board.rs` (ROLES) and `crates/gespan-gpui/src/board/tests.rs` (table names).
- Modify `src-tauri/src/board.rs`, `src-tauri/src/session_store.rs` (busy timeout), `src/features/board/model/{board.ts,autoMove.ts}` plus its test, and `src/features/board/ui/boardFormat.ts`.
- Modify `docs/native/parity.md` (Board row) and `docs/native/verification.md:76`.

## The v3 contract (exact DDL for `V3_DDL`)

```sql
CREATE TABLE IF NOT EXISTS board_schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS board_card_meta (
  card_id TEXT PRIMARY KEY REFERENCES board_cards(id) ON DELETE CASCADE,
  priority INTEGER NOT NULL DEFAULT 0,          -- index into PRIORITIES, 0 = None
  due_at INTEGER, start_at INTEGER, estimate_minutes INTEGER,
  archived_at INTEGER, completed_at INTEGER);
CREATE TABLE IF NOT EXISTS board_labels (
  id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name TEXT NOT NULL, color TEXT NOT NULL, position INTEGER NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS board_labels_name_idx ON board_labels (board_id, lower(name));
CREATE TABLE IF NOT EXISTS board_card_labels (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  label_id TEXT NOT NULL REFERENCES board_labels(id) ON DELETE CASCADE,
  PRIMARY KEY (card_id, label_id));
CREATE INDEX IF NOT EXISTS board_card_labels_label_idx ON board_card_labels (label_id);
CREATE TABLE IF NOT EXISTS board_subtasks (
  id TEXT PRIMARY KEY, card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0, position INTEGER NOT NULL,
  converted_card_id TEXT REFERENCES board_cards(id) ON DELETE SET NULL);  -- room for "Convert to card"
CREATE INDEX IF NOT EXISTS board_subtasks_card_idx ON board_subtasks (card_id, position);
CREATE TABLE IF NOT EXISTS board_comments (
  id TEXT PRIMARY KEY, card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  body TEXT NOT NULL, author TEXT NOT NULL DEFAULT 'user',  -- user | agent:<session> | external:<provider>
  created_at INTEGER NOT NULL, edited_at INTEGER);
CREATE INDEX IF NOT EXISTS board_comments_card_idx ON board_comments (card_id, created_at);
CREATE TABLE IF NOT EXISTS board_card_projects (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  project TEXT NOT NULL, PRIMARY KEY (card_id, project));
CREATE TABLE IF NOT EXISTS board_card_relations (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  related_card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  kind TEXT NOT NULL, created_at INTEGER NOT NULL,
  PRIMARY KEY (card_id, related_card_id, kind));
CREATE INDEX IF NOT EXISTS board_card_relations_related_idx ON board_card_relations (related_card_id);
CREATE TABLE IF NOT EXISTS board_fields (
  id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name TEXT NOT NULL, kind TEXT NOT NULL, options_json TEXT NOT NULL DEFAULT '{}', position INTEGER NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS board_fields_name_idx ON board_fields (board_id, lower(name));
CREATE TABLE IF NOT EXISTS board_card_field_values (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  field_id TEXT NOT NULL REFERENCES board_fields(id) ON DELETE CASCADE,
  value_json TEXT NOT NULL, PRIMARY KEY (card_id, field_id));
CREATE INDEX IF NOT EXISTS board_card_field_values_field_idx ON board_card_field_values (field_id);
CREATE TABLE IF NOT EXISTS board_card_links (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  kind TEXT NOT NULL, ref TEXT NOT NULL, role TEXT NOT NULL DEFAULT 'primary', created_at INTEGER NOT NULL,
  PRIMARY KEY (card_id, kind, ref));
CREATE INDEX IF NOT EXISTS board_card_links_ref_idx ON board_card_links (kind, ref);
```

Columns added to `board_columns` when absent (checked through `pragma_table_info`): `color TEXT`, `wip_limit INTEGER` and `collapsed INTEGER NOT NULL DEFAULT 0`.

The `-- …` comments are documentation for this plan; leave them out of the Rust string, or keep them, but write the same text into `docs/board-schema.md`.

---

### Task 1: `gespan-board-schema` crate with `ensure()` for fresh and v1 databases

**Files:**
- Create: `crates/board-schema/{Cargo.toml,src/lib.rs,src/ddl.rs,src/migrate.rs}`
- Modify: `Cargo.toml` (members), `Cargo.lock` (via `cargo build -p gespan-board-schema`)
- Test: `crates/board-schema/src/migrate.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces, in `lib.rs`:
  - `pub const SCHEMA_VERSION: i64 = 3;`
  - `pub const ROLES: [&str; 5] = ["todo", "doing", "review", "done", "cancelled"];`
  - `pub const PRIORITIES: [&str; 5] = ["None", "Low", "Medium", "High", "Urgent"];`
  - `pub const RELATION_KINDS: [&str; 3] = ["blocks", "relates", "duplicates"];`
  - `pub const FIELD_KINDS: [&str; 7] = ["text", "number", "select", "multi-select", "date", "checkbox", "url"];`
  - `pub const LINK_ROLE_PRIMARY: &str = "primary";` and `pub const COMMENT_AUTHOR_USER: &str = "user";`
  - `pub const BOARD_TABLES: [&str; 14]`, every shared table in FK-safe insert order: `boards, board_columns, board_cards, board_card_meta, board_labels, board_card_labels, board_subtasks, board_comments, board_card_projects, board_card_relations, board_fields, board_card_field_values, board_card_links, board_activity` (the migrations marker is not included).
  - `pub fn ensure(conn: &rusqlite::Connection) -> rusqlite::Result<()>`. It must not be called inside an open transaction, because it opens its own `IMMEDIATE` one.
  - `pub fn sync_primary_link(conn: &Connection, card_id: &str, link: Option<(&str, &str)>, now: i64) -> rusqlite::Result<()>`. It deletes the card's `role='primary'` rows, then inserts `(card_id, kind, ref, 'primary', now)` when `link` is `Some`. Callers run it inside their own transaction.

- [ ] **Step 1: Write failing tests** in `migrate.rs`:
  - `fresh_database_gets_every_table_and_the_marker`: after `ensure` on an in-memory connection with `PRAGMA foreign_keys=ON`, the `sqlite_master` tables `LIKE 'board%'` sorted equal `BOARD_TABLES` sorted plus `board_schema_migrations`, and `SELECT version FROM board_schema_migrations` is `[3]`.
  - `ensure_twice_changes_nothing`: `PRAGMA schema_version` and the row count of `board_schema_migrations` (1) are the same after a second `ensure`.
  - `v1_database_gains_columns_and_keeps_rows`: run `BASE_DDL` only, insert a board, a column (`role='done'`) and a card, then `ensure`. The column reads `color IS NULL, wip_limit IS NULL, collapsed = 0`; the card and its `link_kind/link_id` are unchanged.
  - `ref_and_role_columns_round_trip`: insert into `board_card_links (card_id, kind, ref, created_at)` and read back `role == "primary"`.
  - `sync_primary_link_replaces_and_clears`: link `("session","a")`, then `("pilot","b")`; exactly one primary row `pilot/b`; then `None` leaves zero primary rows. A row with `role='origin'` survives all three calls.
- [ ] **Step 2: Run** `cargo test -p gespan-board-schema`. Expected: the tests fail to compile (`ensure` not found).
- [ ] **Step 3: Implement** `ddl.rs` (`BASE_DDL` verbatim from `src-tauri/src/board.rs:86-135`, and `V3_DDL` as above), `ensure` (begin `IMMEDIATE`, run `BASE_DDL`, add any missing `board_columns` columns, run `V3_DDL`; if there is no version-3 row, call `copy_from_native` (stub returning `Ok(())` until Task 2) and `backfill_links`, then insert the marker; commit) and `sync_primary_link`. `backfill_links`: `INSERT OR IGNORE INTO board_card_links (card_id, kind, ref, role, created_at) SELECT id, link_kind, link_id, 'primary', updated_at FROM board_cards WHERE link_kind IS NOT NULL AND link_id IS NOT NULL`. Add `"crates/board-schema"` to the root `members`.
- [ ] **Step 4: Run** `cargo test -p gespan-board-schema`. Expected: `test result: ok. 5 passed`.
- [ ] **Step 5: Commit** with `git add crates/board-schema Cargo.toml Cargo.lock && git commit -m "Add the shared board schema crate with v3 tables"`.

### Task 2: One-time copy from `board_native_*`, plus v2 test helpers

**Files:**
- Modify: `crates/board-schema/src/migrate.rs`, `crates/board-schema/src/lib.rs`
- Create: `crates/board-schema/src/testing.rs`

**Interfaces:**
- Consumes: `ensure` (Task 1).
- Produces, in `testing` (behind the `testing` feature, so the app crates can use it as a dev-dependency):
  - `pub const NATIVE_V2_DDL: &str`, verbatim from the current `crates/gespan-core/src/board/extras.rs:19-57`. Copy it before Task 4 deletes that file's DDL.
  - `pub struct NativeV2Seed { pub board_id: String, pub card_ids: Vec<String>, pub label_ids: Vec<String> }`
  - `pub fn seed_native_v2(conn: &Connection) -> NativeV2Seed`. It runs `BASE_DDL` and `NATIVE_V2_DDL` with raw SQL, but no v3 schema. It inserts one board, the 4 default columns, and cards A, B and C in Backlog at positions 0..2 (A is linked to `session/s-1`). It also inserts: A meta `priority=4, due_at=1792929600000`; B meta `priority=2, due_at=NULL`; A projects `/work/gespan` and `/work/site`; labels `Bug`/`d73a4a` and `docs`/`0075ca`, both attached to A and `Bug` to B; A subtasks `One` (done, position 0), `Two` (position 5) and `Three` (position 9), to give gapped positions; A comments `First` (created_at 1000) and `Second` (2000). It finally inserts the orphan rows with `PRAGMA foreign_keys=OFF` around them: a subtask, comment, meta and project row for card `ghost`, and a card_label naming label `ghost-label`.
  - `pub fn assert_native_v2_migrated(conn: &Connection, seed: &NativeV2Seed)`. It asserts every seeded value is in the v3 tables (exact values above). Subtask positions must be `[0,1,2]` in the original order, label positions `Bug=0, docs=1`, comment authors `user` with unchanged `created_at`, and no `ghost` rows. Card positions per column stay `0..n-1`, `board_card_links` has `(A, session, s-1, primary)`, and every `board_native_*` table has the same row count as before.

- [ ] **Step 1: Write failing tests** in `migrate.rs` (the crate's own tests enable `testing` through `#[cfg(any(test, feature = "testing"))]` on the module):
  - `native_v2_data_is_copied_with_every_value`: `seed_native_v2` then `ensure`, then `assert_native_v2_migrated`.
  - `orphans_are_skipped_and_positions_renumbered`: covered by the assertions above. Also assert `ensure` returned `Ok` with `foreign_keys=ON`.
  - `rerun_does_not_resurrect_deleted_rows`: after migrating, `DELETE FROM board_labels WHERE name='docs'` and delete subtask `Two`, then run `ensure` again. Both stay gone and `board_card_labels` has no `docs` row.
  - `failed_copy_rolls_back_and_retries`: seed, then create the trigger `CREATE TRIGGER fail BEFORE INSERT ON board_comments BEGIN SELECT RAISE(ABORT, 'boom'); END`. This trigger is created after `V3_DDL`, so run `V3_DDL` first in the test and then `ensure`. `ensure` must return an error. Then 0 rows in `board_card_meta` and no marker row. Drop the trigger; `ensure` → `assert_native_v2_migrated` passes.
  - `database_without_native_tables_only_backfills_links`: run `BASE_DDL` with one linked card, then `ensure`. Expect one link row, no `board_native_*` tables created, and the marker present.
  - `concurrent_ensure_copies_once`: in a temp file DB (WAL, `busy_timeout=5000`), seed, then two threads each open their own connection and call `ensure`. Both return `Ok`; `board_subtasks` has 3 rows and `board_schema_migrations` 1.
- [ ] **Step 2: Run** `cargo test -p gespan-board-schema --features testing`. Expected: the tests fail.
- [ ] **Step 3: Implement `copy_from_native(tx: &Connection) -> rusqlite::Result<()>`.** It runs each step only when the source table exists in `sqlite_master`, and copies only rows whose card (and label) exists (an `INNER JOIN board_cards`):
  - meta: `(card_id, priority, due_at)`;
  - projects;
  - labels: `position = ROW_NUMBER() OVER (PARTITION BY board_id ORDER BY lower(name), rowid) - 1`;
  - card_labels: joined to both `board_cards` and `board_labels`;
  - subtasks: `position = ROW_NUMBER() OVER (PARTITION BY card_id ORDER BY position, rowid) - 1`;
  - comments: `author = 'user'`, `edited_at = NULL`.

  Use plain `INSERT`, not `OR IGNORE`, so a conflict fails loudly; the v3 tables are empty inside this transaction. Then add `testing.rs`.
- [ ] **Step 4: Run** `cargo test -p gespan-board-schema --features testing`. Expected: all tests pass. Quote the `test result:` line.
- [ ] **Step 5: Commit** with `git commit -m "Copy Native board data into the shared v3 tables once"`.

### Task 3: Normalized `dump` / `load` for cross-app fixtures

**Files:**
- Modify: `crates/board-schema/src/testing.rs`

**Interfaces:**
- Produces:
  - `pub fn dump(conn: &Connection) -> String`. For each table in `BOARD_TABLES` order, it emits one `INSERT INTO <table> (<cols in pragma order>) VALUES (...);` line per row, in `rowid` order. Values are SQL literals: `NULL`, integers, and `'…'` strings with `'` doubled. Normalization: every 36-character UUID-shaped string (hyphens at 8, 13, 18 and 23, the rest hex) becomes `id-0001`, `id-0002`, … by first appearance across the whole dump. Every `*_at` column except `due_at` and `start_at` becomes `0`.
  - `pub fn load(conn: &Connection, sql: &str) -> rusqlite::Result<()>`, which runs `sql` in one transaction. The caller has already opened the database through its app's own open path.
  - `pub fn fixture_matches(path: &Path, actual: &str)`. When `GESPAN_BLESS_BOARD_FIXTURES=1`, it writes `actual` to `path`. Otherwise it asserts equality and the failure message names that env var.

- [ ] **Step 1: Write failing tests:**
  - `dump_normalizes_ids_and_times_but_keeps_due`: two identical scenarios with different UUIDs and `now` values give an equal `dump`, and `due_at=1792929600000` survives.
  - `load_of_a_dump_dumps_the_same`: dump DB X, `ensure` a fresh DB Y, `load` the dump into Y, and `dump(Y) == dump(X)`.
  - `strings_with_quotes_round_trip`: a card titled `It's "done"`.
- [ ] **Step 2: Run** `cargo test -p gespan-board-schema --features testing dump`. Expected: FAIL.
- [ ] **Step 3: Implement** `dump`, `load` and `fixture_matches`.
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Commit** with `git commit -m "Add normalized board dumps for cross-app fixtures"`.

### Task 4: Native (`gespan-core`) on schema v3

**Files:**
- Modify: `crates/gespan-core/Cargo.toml` (`gespan-board-schema = { path = "../board-schema" }`; dev-dependency with `features = ["testing"]`), `crates/gespan-gpui/Cargo.lock` (via cargo)
- Modify: `crates/gespan-core/src/board/mod.rs`, `extras.rs` and `auto_move.rs`; `crates/gespan-gpui/src/board.rs` (ROLES); `crates/gespan-gpui/src/board/tests.rs`

**Interfaces:**
- Consumes: `gespan_board_schema::{ensure, ROLES, PRIORITIES, sync_primary_link, LINK_ROLE_PRIMARY}`; in tests, `testing::{seed_native_v2, assert_native_v2_migrated}`.
- Produces:
  - `Column` gains `pub color: Option<String>, pub wip_limit: Option<i64>, pub collapsed: bool` (read in `column_from_row`, with `collapsed` read as integer `!= 0`).
  - `extras::PRIORITIES` stays as a `pub use gespan_board_schema::PRIORITIES`, so gpui callers are unchanged.
  - The public `Service` method signatures are unchanged.

- [ ] **Step 1: Write failing tests** in `extras.rs` / `mod.rs` tests:
  - `native_v2_database_opens_with_every_value`: `seed_native_v2` on a temp file, then `Service::open`, then `assert_native_v2_migrated`. Through the API: `extras(board)[A]` has `priority 4`, `due Some(1792929600000)`, 2 projects, 2 labels, `subtasks_done 1/total 3` and `comments 2`; `subtasks(A)` titles are `[One, Two, Three]`, positions `[0,1,2]`; and `labels(board)` names are `[Bug, docs]`.
  - `each_field_change_writes_exactly_one_activity_row`: a table over `set_priority`, `set_due`, `set_projects`, `set_labels`, `add_subtask`, `toggle_subtask`, `delete_subtask`, `add_comment` and `card_update`. Each call adds exactly 1 row (kind `edited`); repeating the same `set_*`/`toggle_subtask` value adds 0. Subtask and comment details: `subtask added: <title>`, `subtask done: <title>`, `subtask reopened: <title>`, `subtask removed: <title>`, `comment added`.
  - `deleting_a_subtask_keeps_positions_contiguous`: add 3 subtasks, delete the middle one, and the positions are `[0,1]`.
  - `new_labels_append_positions`: create 2 labels, positions `[0,1]`.
  - `card_link_keeps_the_primary_link_row_in_step`: link `session/x`, then `pilot/y`, then unlink; the `board_card_links` primary rows are `[(session,x)]`, then `[(pilot,y)]`, then `[]`.
  - `cancelled_role_is_accepted` (`column_create(.., Some("cancelled"))` returns Ok, and a second one fails with `already has the cancelled role`). In `auto_move.rs`: `cards_in_a_cancelled_column_never_move` (Started and Completed both give `None` from a `cancelled` column placed left of doing/review).
  - Retarget the existing `native_rows` / delete-cascade tests to the v3 tables, and add `board_card_links` to the rows that must cascade.
- [ ] **Step 2: Run** `cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-core --profile native-dev board`. Expected: the new tests fail.
- [ ] **Step 3: Implement.**
  - `ensure_tables` calls `gespan_board_schema::ensure` and nothing else; delete `extras::ensure_tables`, every board `CREATE` statement in `gespan-core` and the local `ROLES`.
  - Retarget the extras SQL: `board_native_meta`→`board_card_meta`, `_projects`→`board_card_projects`, `_labels`→`board_labels`, `_card_labels`→`board_card_labels`, `_subtasks`→`board_subtasks`, `_comments`→`board_comments` (which inserts `author = COMMENT_AUTHOR_USER`). `create_label` sets `position = COALESCE(MAX(position)+1, 0)` for the board.
  - `delete_subtask` renumbers the card's remaining subtasks with the same pattern as `write_card_positions`.
  - Subtask and comment writers call `log_edit` once.
  - `link_card` calls `sync_primary_link` inside its transaction.
  - In `auto_move_target`, return `None` when `current.role` is `done` or `cancelled`.
  - Add `("cancelled", "Cancelled")` to gpui `ROLES`.
  - Retarget the three `board_native_*` references in `crates/gespan-gpui/src/board/tests.rs` (lines ~274, ~1253, ~1544) to `board_labels` / `board_comments`.
- [ ] **Step 4: Run** the Step 2 command, then `cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-gpui --profile native-dev board -- --test-threads=1`. Expected: both report `test result: ok`.
- [ ] **Step 5: Commit** with `git commit -m "Native board reads and writes the shared v3 tables"`.

### Task 5: Tauri on schema v3

**Files:**
- Modify: `src-tauri/Cargo.toml` (dependency and dev-dependency `features = ["testing"]`), `Cargo.lock`
- Modify: `src-tauri/src/board.rs`, `src-tauri/src/session_store.rs:41-44`
- Modify: `src/features/board/model/board.ts` (`ColumnRole` += `"cancelled"`), `src/features/board/ui/boardFormat.ts` (`ROLE_LABEL.cancelled = "Cancelled"`, `ROLES` += `"cancelled"`), `src/features/board/model/autoMove.ts` plus its existing test file

**Interfaces:**
- Consumes: the same shared-crate items as Task 4.
- Produces: Tauri `Column` gains `color: Option<String>, wip_limit: Option<i64>, collapsed: bool` (serialized camelCase `color`, `wipLimit`, `collapsed`; the TS `Column` type gains them as optional fields).

- [ ] **Step 1: Write failing tests** in `src-tauri/src/board.rs` tests and `session_store.rs` tests:
  - `native_v2_database_opens_in_tauri_with_every_value`: `seed_native_v2` on a temp file, then `SessionStore::open(path)`, then `assert_native_v2_migrated` on its connection; `get_board` returns cards A, B and C at positions `[0,1,2]` with A still linked `session/s-1`.
  - `opening_twice_is_a_no_op`: open, close and reopen; `PRAGMA schema_version` is unchanged and there is one marker row.
  - `deleting_a_card_removes_its_v3_rows`: insert meta, label link, subtask, comment, link, relation (both directions) and field value rows for a card, then `delete_card`; every one is gone, and the other card's rows remain.
  - `link_card_keeps_the_primary_link_row_in_step`: the same expectations as Task 4.
  - `cancelled_role_is_accepted_and_unique`.
  - `open_waits_for_a_held_migration_lock` (`session_store.rs`): a second connection runs `BEGIN IMMEDIATE`, and a thread commits after 300 ms; `SessionStore::open` on the same file returns Ok.
  - Vitest in the autoMove test: `"never moves a card out of a cancelled column"`.
- [ ] **Step 2: Run** `cargo test -p gespan board::` and `cargo test -p gespan session_store::tests::open_waits`, and `bunx vitest run src/features/board`. Expected: the new tests fail.
- [ ] **Step 3: Implement.**
  - `ensure_tables` calls `gespan_board_schema::ensure`; delete Tauri's board DDL and its local `ROLES` (use the shared one).
  - `column_from_row` reads the new fields.
  - `link_card` calls `sync_primary_link` in its transaction.
  - In `SessionStore::open`, the write connection's pragma batch gains `PRAGMA busy_timeout = 5000;` before `migrate`.
  - In `autoMove.ts`, return no target when `current.role` is `"done"` or `"cancelled"`, mirroring Task 4.
- [ ] **Step 4: Run** the Step 2 commands again. Expected: `test result: ok` / Vitest `passed`.
- [ ] **Step 5: Commit** with `git commit -m "Tauri board opens schema v3 and keeps links in step"`.

### Task 6: Cross-app fixtures and shared contract tests

**Files:**
- Create: `crates/board-schema/fixtures/written_by_native.sql`, `written_by_tauri.sql` (blessed by the tests below)
- Modify: `crates/gespan-core/tests/shared_contracts.rs`, `src-tauri/src/board.rs` (tests)

**Interfaces:**
- Consumes: `testing::{dump, load, fixture_matches}`, plus both apps' board APIs.

- [ ] **Step 1: Write the tests.**
  - Core, `native_writes_the_committed_fixture` (`shared_contracts.rs`). `Service::open` on a temp file, then:
    - `create("Fixture")`, cards `First` and `Second` in Backlog, `Third` moved to In Progress;
    - on `First`: `set_priority(3)`, `set_due(Some(1792929600000))`, `set_projects(["/work/gespan"])`, `create_label("Bug","d73a4a")` attached through `set_labels`;
    - subtasks `Write test` (toggled done) and `Ship`, comment `Looks good`, `card_link("session","session-fixture")`.

    Then call `fixture_matches(<board-schema>/fixtures/written_by_native.sql, &dump(conn))`, reading `conn` through a raw `rusqlite::Connection` on the same file.
  - Tauri, `tauri_writes_the_committed_fixture`. `SessionStore::open` on a temp file, then through `board.rs` functions:
    - `create_board("Fixture")`, cards `Alpha` and `Beta`, Beta moved to Review;
    - rename the Backlog column to `Queue`, `create_column("Dropped", Some("cancelled"))`;
    - `link_card(Alpha, pilot, pilot-fixture)`, `update_card(Beta, "Beta", "Notes")`.

    Then `fixture_matches(.../written_by_tauri.sql, …)`.
  - Tauri, `tauri_opens_a_database_written_by_native`. `SessionStore::open` (empty temp file), then `load(written_by_native.sql)`, reopen, then:
    - `get_board` has 3 cards with contiguous positions per column, `First` linked `session/session-fixture`, and `board_activity` lists every row;
    - raw reads find `First`'s meta `(3, 1792929600000)`, label `Bug`, 2 subtasks (positions 0 and 1, the first done) and comment author `user`.
  - Core, `native_opens_a_database_written_by_tauri` (`shared_contracts.rs`). `Service::open`, then `load(written_by_tauri.sql)`, reopen, then:
    - `get` shows columns `Queue, In Progress, Review, Done, Dropped`, with `Dropped.role == Some("cancelled")`;
    - `cards_for_link("pilot","pilot-fixture")` returns `Alpha`, `Beta.body == "Notes"`, and the `board_card_links` primary row is present.
  - Replace `board_tables_match_tauri` with `both_apps_use_the_shared_board_schema`: both `src-tauri/src/board.rs` and the native board sources contain `gespan_board_schema::ensure`, and neither contains any match of `CREATE (?:UNIQUE )?(?:TABLE|INDEX) IF NOT EXISTS board`.
  - Replace the 4-table test with `opening_the_board_service_creates_exactly_the_v3_tables`: after `Store::open_gespan` and `Service::open`, the `board%` tables equal `BOARD_TABLES` plus `board_schema_migrations` and no `board_native_*`. Each table's `pragma_table_info` column names equal an explicit expected list written out in the test, one entry per table in the DDL above (`board_columns` = `id, board_id, name, position, role, color, wip_limit, collapsed`). Changing the contract must mean editing this test on purpose.
  - Extend `board_error_strings_match_tauri` with `"Unknown link kind: {kind}"` and `"A link needs both a kind and an id"`. Both sources already contain them; this pins them.
- [ ] **Step 2: Bless** the fixtures once with `GESPAN_BLESS_BOARD_FIXTURES=1 cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-core --profile native-dev --test shared_contracts native_writes` and `GESPAN_BLESS_BOARD_FIXTURES=1 cargo test -p gespan board::tests::tauri_writes`. Read both `.sql` files and check by eye that they contain the scenario values and only `id-NNNN` ids.
- [ ] **Step 3: Run without the bless variable:** `cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-core --profile native-dev --test shared_contracts` and `cargo test -p gespan board::`. Expected: `test result: ok` for both. Then change one byte in a fixture, confirm the matching test fails, and restore it.
- [ ] **Step 4: Commit** with `git commit -m "Pin the board schema v3 contract across both apps"`.

### Task 7: Contract doc, parity docs and full verification

**Files:**
- Create: `docs/board-schema.md`
- Modify: `docs/native/parity.md:44`, `docs/native/verification.md:76`

- [ ] **Step 1: Write `docs/board-schema.md`.** It should cover:
  - the owner crate, and that both apps call `ensure` on open;
  - every table and column with its encoding: priority index → `PRIORITIES`, `done`/`collapsed` 0/1, timestamps in ms since the epoch, `value_json` holding a JSON value per `FIELD_KINDS`, comment `author` format, link `role`, and the relation `kind` direction (`card_id` blocks `related_card_id`);
  - the marker and versioning rules (add a `v4` block that checks its own marker row; never edit v3);
  - the copy semantics: one-time, orphans skipped, positions renumbered, `board_native_*` kept and frozen;
  - the old link columns as the read source for this release;
  - the downgrade caveat;
  - how to re-bless the fixtures.

  Update the parity row: Native data now lives in shared tables that Tauri reads too, and Tauri shows them once #86 lands. Update the verification line so it no longer says extras live in `board_native_*`.
- [ ] **Step 2: Run** `bun install` if `node_modules` is missing, then, one at a time: `cargo test -p gespan-board-schema --features testing`, `bun run check`, `bun run native:check`, `bunx vitest run src/test/legacyNames.test.ts`. Expected: each one finishes with passing results; quote the summary lines. Rerun the known flaky tests before treating them as real failures.
- [ ] **Step 3: Commit** with `git commit -m "Document the board schema v3 contract"`.

## Follow-ups (outside #85)

- Drop the `board_native_*` tables one release after v3 ships.
- Switch reads from `board_cards.link_kind/link_id` to `board_card_links`, then re-backfill and stop dual-writing (#95).
- Write APIs and activity for relations, custom fields, extra links, `start_at`/`estimate`/archive/complete, and column colour/WIP/collapse (#90, #93/#94, #95).
- Readable `from → to` activity details, and position-ordered labels (#88/#89, #94).
