# Board schema (v3)

The Tauri app and Native both keep their board in `gespan.db`, in the same
tables. This page is the contract between them. Any change to it is a change
to both apps.

## Owner

`crates/board-schema` (package `gespan-board-schema`) owns every board
`CREATE` statement, the version marker, the one-time copy of Native's old
data and the shared vocabulary (`ROLES`, `PRIORITIES`, `RELATION_KINDS`,
`FIELD_KINDS`, `LINK_ROLE_PRIMARY`, `COMMENT_AUTHOR_USER`, `BOARD_TABLES`).
Neither app writes board DDL of its own:

- Tauri: `src-tauri/src/board.rs` `ensure_tables`, called from
  `SessionStore::open` → `migrate`.
- Native: `crates/gespan-core/src/board` `ensure_tables`, called from
  `Service::open`.

Both call `gespan_board_schema::ensure(conn)` on every open. `ensure` opens
its own `BEGIN IMMEDIATE` transaction, so callers must not already be in one.
The service logic (moves, positions, activity, validation) is still written
twice, once per app, because the apps live in separate Cargo workspaces with
different connection models. The cross-app tests below keep the two copies in
line.

## Tables

Times are Unix milliseconds. Booleans are `INTEGER` 0/1. Every child row
references its card or board with `ON DELETE CASCADE`, so deleting a card in
either app removes all of its rows, including from tables that app never
reads.

### Since v1 (unchanged)

| Table | Columns |
|---|---|
| `boards` | `id`, `name`, `card_seq` (last card number handed out), `created_at`, `updated_at` |
| `board_columns` | `id`, `board_id`, `name`, `position` (0..n-1 per board), `role` (one of `ROLES` or NULL, unique per board) |
| `board_cards` | `id`, `board_id`, `column_id`, `number` (per board, never reused), `title`, `body`, `position` (0..n-1 per column), `link_kind`, `link_id`, `created_at`, `updated_at` |
| `board_activity` | `id`, `board_id`, `card_id` (NULL for a deleted card's tombstone), `kind`, `actor` (`user` or `system`), `from_column`, `to_column`, `detail`, `created_at` |

### Added in v3

| Table / column | Meaning |
|---|---|
| `board_columns.color` | Column colour, six hex digits without `#`, or NULL |
| `board_columns.wip_limit` | Work-in-progress limit, or NULL for none |
| `board_columns.collapsed` | 0/1, default 0 |
| `board_card_meta` | One optional row per card: `priority` (index into `PRIORITIES`: 0 None, 1 Low, 2 Medium, 3 High, 4 Urgent), `due_at`, `start_at` (both the day the user picked), `estimate_minutes`, `archived_at`, `completed_at` |
| `board_labels` | Per board: `id`, `name` (unique per board, case-insensitive), `color` (six hex digits), `position` (0..n-1) |
| `board_card_labels` | `(card_id, label_id)` |
| `board_subtasks` | `id`, `card_id`, `title`, `done` 0/1, `position` (0..n-1 per card), `converted_card_id` (the card this subtask became, set to NULL if that card is deleted) |
| `board_comments` | `id`, `card_id`, `body`, `author`, `created_at`, `edited_at` (NULL until edited). `author` is `user`, `agent:<session id>` or `external:<provider>` |
| `board_card_projects` | `(card_id, project)`, a project path |
| `board_card_relations` | `(card_id, related_card_id, kind)`, `created_at`. `kind` is one of `RELATION_KINDS` and reads left to right: `card_id` *blocks* / *relates to* / *duplicates* `related_card_id` |
| `board_fields` | Custom fields per board: `id`, `name` (unique per board, case-insensitive), `kind` (one of `FIELD_KINDS`), `options_json` (a JSON object, `{}` by default; select options live here), `position` |
| `board_card_field_values` | `(card_id, field_id)`, `value_json`: one JSON value whose shape follows the field kind (`text`/`url`/`select`: string, `number`: number, `multi-select`: array of strings, `date`: ms number, `checkbox`: boolean) |
| `board_card_links` | `(card_id, kind, ref)`, `role` (default `primary`), `created_at`. A card's link to a session, Pilot, Flow run or anything else |
| `board_schema_migrations` | `(version, applied_at)`: one row per board schema version applied |

The exact DDL is `BASE_DDL` and `V3_DDL` in `crates/board-schema/src/ddl.rs`.
`opening_the_board_service_creates_exactly_the_v3_tables` in
`crates/gespan-core/tests/shared_contracts.rs` lists every table and column,
so a schema change also has to change that test.

### What this release reads and writes

- `board_cards.link_kind/link_id` is still where both apps read a card's link
  (`cards_for_link`, auto-move). Both apps' `link_card` also keep a matching
  `primary` row in `board_card_links` up to date through `sync_primary_link`,
  and the v3 migration backfills one for every card that already had a link.
- Native reads and writes meta (priority and due date), projects, labels,
  subtasks and comments. Tauri does not show them yet (#86), but cascades
  them on delete.
- Nothing writes relations, custom fields, links other than `primary`,
  `start_at`/`estimate_minutes`/`archived_at`/`completed_at`, or column
  colour/WIP limit/collapsed yet. Their tickets add the writers. Both apps
  already read the column fields.
- The `cancelled` role is accepted by both apps. A card in a `done` or
  `cancelled` column never auto-moves.
- Each field change writes exactly one `edited` activity row; a change to
  the value already stored writes none. Subtask add, toggle and delete, and
  comment add, are logged as `subtask added: <title>`,
  `subtask done: <title>`, `subtask reopened: <title>`,
  `subtask removed: <title>` and `comment added`.
- Labels are numbered by creation (new labels are appended), but `labels()`
  still lists them by name.

## Versioning

`ensure` runs every `CREATE … IF NOT EXISTS` and every missing
`ALTER TABLE … ADD COLUMN` on each open, and then, only if
`board_schema_migrations` has no row for version 3, runs the one-time data
steps and writes that row. All of this happens in one `BEGIN IMMEDIATE`
transaction, so two apps opening the file at the same moment take turns. The
second one sees the marker and copies nothing. A failure rolls the whole
transaction back, fails the open and is retried on the next open.

Rules for later versions:

- Never edit `BASE_DDL` or the v3 statements. Older builds of either app
  still open this file.
- Only add. New tables use `IF NOT EXISTS`; new columns go through the same
  "add if missing" check.
- Data steps for v4 go in their own block that checks for a `4` row and
  writes it, in the same transaction.
- Raise `SCHEMA_VERSION` and update this page and the column-list test.

## The v3 copy from Native's `board_native_*` tables

Native builds before v3 kept extras in `board_native_meta`, `_projects`,
`_labels`, `_card_labels`, `_subtasks` and `_comments`. The first v3 open
copies them into the shared tables once:

- Values are copied as they are: priority, due date, `done`, titles, bodies
  and comment times. Comments get `author = 'user'` and no `edited_at`.
- Positions are renumbered to 0..n-1: subtasks per card in their old order,
  labels per board by `lower(name)`, matching Native's old alphabetical order.
- Orphan rows, whose card, label or board is gone, are skipped. They were
  already invisible, and copying them would break the foreign keys and fail
  the open. Any other copy error fails the whole migration rather than
  dropping rows.
- The copy runs only once. Deleting a migrated label or subtask and then
  reopening does not bring it back.
- The `board_native_*` tables are left exactly as they were and are no longer
  written, so a rollback is possible for this release. Dropping them is a
  follow-up.

### Downgrade caveat

A pre-v3 build can still open a v3 file: it only sees the four v1 tables,
which are unchanged. But a pre-v3 **Native** build still reads and writes
`board_native_*`. Anything it writes there after the migration is not seen
by v3 builds, and anything written in v3 is not seen by it.

## Cross-app fixtures

`crates/board-schema/fixtures/written_by_native.sql` and
`written_by_tauri.sql` are normalized dumps of a fixed scenario written
through each app's real API. UUIDs become `id-0001`, `id-0002`, … in order of
first appearance, and every `*_at` time except `due_at`/`start_at` becomes 0.

- `native_writes_the_committed_fixture` (gespan-core `tests/shared_contracts.rs`)
  and `tauri_writes_the_committed_fixture` (`src-tauri/src/board.rs`) fail when
  what an app writes no longer matches its fixture.
- `tauri_opens_a_database_written_by_native` and
  `native_opens_a_database_written_by_tauri` load the other app's fixture
  through their own open path and read it back.

When a change to what an app writes is intended, re-bless and review the diff:

```sh
GESPAN_BLESS_BOARD_FIXTURES=1 cargo test --manifest-path crates/gespan-gpui/Cargo.toml \
  -p gespan-core --profile native-dev --test shared_contracts native_writes
GESPAN_BLESS_BOARD_FIXTURES=1 cargo test -p gespan board::tests::tauri_writes
```

The Native fixture contains the activity detail `due: 2026-10-25`, the
local-time day of a noon-UTC due date. Blessing it on a machine more than 11
hours ahead of UTC would change that line.
