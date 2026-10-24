# Native Board Implementation Plan

> **Superseded in part by [`2026-10-24-native-board-v2.md`](2026-10-24-native-board-v2.md).** The Board is now on by default in both apps, and the `boardEnabled` import falls back to on. The `dragging` flag is also cleared whenever GPUI no longer reports an active drag (`BoardView::forget_finished_drag`), which runs any deferred reload. This plan is kept as written when it was carried out.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the Board (already shipped in the Tauri app on this branch) to the native Rust/GPUI app, reading and writing the same `rebase.db` tables, behind a native "Board" setting that is off by default.

**Architecture:** `rebase-core` gets a `board` module with the Tauri DDL byte-for-byte, a `Service` like `automations::Service`, and ports of the auto-move and linked-turn rules. Contract tests pin the shared DDL, error strings, link-id formats and the `boardEnabled` key against the Tauri sources. `rebase-gpui` gets `AppView::Board`, a board view with mouse drag-and-drop built on GPUI `on_drag`/`on_drop`, a "Move to…" card menu instead of keyboard dragging, and turn hooks in `conversation.rs` and `pilots/mod.rs`. The view reloads when it opens, when the window regains focus, and when `PRAGMA data_version` changes, checked every 2 s while the view is open.

**Tech Stack:** Rust, rusqlite, GPUI (`vendor/gpui-pre` 0.3.9), the `crates/rebase-gpui/Cargo.toml` workspace (separate lockfile, profile `native-dev`).

**Spec:** `docs/superpowers/plans/2026-10-23-board.md` (behaviour, Design Decisions, Visual Design). The Tauri implementation on this branch is the reference: `src-tauri/src/board.rs`, `src/features/board/**`. Native-specific decisions are from the 2026-10-24 conversation (user approved the defaults): ship in the same PR; native v1 gets a "Move to…" menu instead of keyboard dragging; changes made in the other app are picked up by reloading on open and on focus, plus a 2 s `data_version` poll while the view is open.

## Global Constraints

- **Shared data contract.** Native table and index DDL is byte-identical to `src-tauri/src/board.rs` `ensure_tables`: tables `boards`, `board_columns`, `board_cards`, `board_activity`; indexes `board_columns_role_idx`, `board_cards_column_idx`, `board_cards_link_idx`, `board_activity_board_idx`, `board_activity_card_idx`. No numbered migration.
- **Error strings** match Tauri exactly: `"Column not on this board"`, `"Column has N cards"`, `"Board not found"`, and role errors containing `"role"`.
- **Link ids.** A `session` link stores the persisted session id string (`Store::persisted_session_id`), never the native `i64`. A `pilot` link stores `Pilot.id`. Link kinds are `session | pilot | flow-run`. Actors are `user | system`. Record kinds are `started | completed | failed`.
- **Activity rows** store column **names** in `from_column` / `to_column`, as Tauri does.
- **Auto-move** is forward-only. It records `started` / `completed` / `failed` after a move and when the card stays. A stopped turn sends no event. Pilot resolution is own chat → habit run → spawning Pilot, decided at turn start.
- **Setting.** The native preference is `board: bool`, default `false`. It is imported once from the Tauri `boardEnabled` key. Its row sits in General, after Notes, with this copy: `"A Kanban board for tracking work. Cards can link to sessions and Pilots, and move on their own as that work starts and finishes."`
- **No Flow runs** (Flows isn't built), and no keyboard dragging in native v1.
- **Look.** Use the Visual Design table in the spec, with values mapped to native tokens in `crates/rebase-gpui/src/theme.rs`. Don't invent new colors.
- **Busy timeout.** Use `busy_timeout` 5 s and `PRAGMA foreign_keys=ON` per connection, as `automations::Service::open` does.
- **No new crates.** Add no dependencies to `crates/rebase-gpui/Cargo.lock`. Build with `--locked`.
- **Gate.** Run `bun run native:check` (it shells to npm internally; leave that as it is). The Tauri side must stay green too: `bun run check:web` and `bun run check:rust`.
- **Commits** carry no Claude or co-author attribution. Use "rebase" naming only, never the old product name.

## Review Focus

1. **Tauri and native write the same board at once.** Positions stay contiguous and no write is lost. Task 1 interleaves moves from two `Service` connections on one temp DB.
2. **A card linked to a session in Tauri resolves in native, and the reverse.** This only works because both store the persisted string id. Tasks 4 and 6 test round-tripping a link through `persisted_session_id`.
3. **Tauri edits a board while the native Board view is open.** Native reflects the edit within about 2 s, and a reload never cancels a drag in progress (the reload is deferred until the drop). Task 4 tests the deferral.
4. **Board is turned off while the native view is open.** The view closes, its title tab is removed, and the data is kept. Task 3 tests this.
5. **A card is dropped on the column it came from, at the same index.** No command call and no activity row. Task 5 tests this.

---

### Task 1: Core board storage and shared-contract tests

**Files:**
- Create: `crates/rebase-core/src/board/mod.rs` (Task 2 adds submodules beside it)
- Modify: `crates/rebase-core/src/lib.rs` (`pub mod board;`), `crates/rebase-core/tests/shared_contracts.rs`

**Interfaces:**
- Produces, in `rebase_core::board`:
  - Types: `Board { id, name, created_at, updated_at }`, `Column { id, board_id, name, position, role: Option<String> }`, `Card { id, board_id, column_id, number, title, body, position, link_kind: Option<String>, link_id: Option<String>, created_at, updated_at }`, `Activity { id, board_id, card_id: Option<String>, kind, actor, from_column: Option<String>, to_column: Option<String>, detail: Option<String>, created_at }`, `BoardDetail { board, columns, cards }`.
  - `pub(crate) fn ensure_tables(conn: &Connection) -> rusqlite::Result<()>`
  - `pub struct Service` with:
    - `open(path: &Path) -> Result<Self, String>`
    - `list() -> Result<Vec<Board>, String>`, `get(board_id) -> Result<BoardDetail, String>`
    - `create(name) -> Result<Board, String>`, `rename(board_id, name)`, `delete(board_id)`
    - `column_create(board_id, name, role) -> Result<Column, String>`, `column_update(column_id, name, role)`, `column_move(column_id, index)`, `column_delete(column_id)`
    - `card_create(column_id, title, body) -> Result<Card, String>`, `card_update(card_id, title, body)`, `card_move(card_id, column_id, index, actor)`, `card_link(card_id, kind, id)`, `card_delete(card_id)`
    - `cards_for_link(kind, id) -> Result<Vec<Card>, String>`
    - `activity(board_id, card_id, limit) -> Result<Vec<Activity>, String>`
    - `card_record(card_id, kind, detail)`
    - `data_version() -> Result<i64, String>`
  - Every method takes `&mut self` and mirrors the Tauri function of the same role. Method names are snake_case without the `board_` prefix.

- [ ] **Step 1: Write the failing tests.** Port every test in `src-tauri/src/board.rs` `mod tests` (22 tests), keeping the same names and assertions, to `#[cfg(test)]` in `board/mod.rs`. They run against `Connection::open_in_memory()` + `PRAGMA foreign_keys=ON` + `ensure_tables`. Add:
  - `two_connections_interleaving_moves_keep_positions_contiguous`: a temp-dir DB, two `Service::open`s, 20 seeded alternating moves, then every column is `0..n-1`.
  - `data_version_changes_after_another_connection_writes`: `a.data_version()`, `b.card_create(...)`, then `a.data_version()` differs.

  In `tests/shared_contracts.rs`, add:
  - `board_tables_match_tauri`: extract every `CREATE TABLE IF NOT EXISTS board…` and `CREATE … INDEX IF NOT EXISTS board…` statement (up to its closing `;`) from `tauri("src-tauri/src/board.rs")` and from the native source, normalize whitespace runs to a single space, and assert the two lists are equal.
  - `board_error_strings_match_tauri`: assert each Global Constraints error string appears in both sources.
  - Also assert that `Store::open_rebase` on a fresh temp DB followed by `board::Service::open` creates exactly the four board tables.
- [ ] **Step 2: Run:** `cargo test --locked --manifest-path crates/rebase-gpui/Cargo.toml -p rebase-core --profile native-dev board` → FAIL (module missing).
- [ ] **Step 3: Implement.** Copy the DDL string from Tauri verbatim. Port the transaction and renumbering logic. `Service::open` follows `automations::Service::open` (`automations.rs:663-676`). `data_version` follows `store/pilots.rs:83`.
- [ ] **Step 4: Run:** the same command, plus `cargo test … -p rebase-core --test shared_contracts` → PASS.
- [ ] **Step 5: Commit:** `git commit -m "Add native board storage sharing the Tauri board tables"`

### Task 2: Core auto-move and linked-turn rules

**Files:**
- Create: `crates/rebase-core/src/board/auto_move.rs`, `crates/rebase-core/src/board/linked_turns.rs`

**Interfaces:**
- Consumes: Task 1's `Service`, `Column`, `Card`.
- Produces:
  - `pub enum LinkedWorkEvent { Started, Completed, Failed }` with `as_str()` → `"started" | "completed" | "failed"`
  - `pub fn auto_move_target<'a>(columns: &'a [Column], current: &Column, event: LinkedWorkEvent) -> Option<&'a Column>`
  - `pub fn on_linked_work(service: &mut Service, kind: &str, id: &str, event: LinkedWorkEvent) -> Result<(), String>`. For each card from `cards_for_link`: move with actor `"system"` to the end of the target column, then `card_record`; or only `card_record` when there's no target. It processes every card, then returns the first error.
  - `pub fn turn_pilot_id(own: Option<&str>, habit: Option<&str>, spawned_by: Option<&str>) -> Option<String>` (first `Some` wins)
  - `pub struct LinkedTurns`, keyed by `(session_key: String, gen: u64)`:
    - `started(session_key, gen, pilot_id: Option<String>) -> Vec<(kind, id, LinkedWorkEvent)>`
    - `settled(session_key, gen, ok: bool) -> Vec<(…)>`
    - `cancelled(session_key, gen)`

    It keeps no I/O. Callers dispatch the returned calls.

- [ ] **Step 1: Write the failing tests.** Port the cases from `src/features/board/model/autoMove.test.ts` and `src/features/board/model/linkedTurns.test.ts` with the same scenarios and expectations: forward-only targets, no-review fallback, a `role:null` column between doing and review, a missing doing column, move-then-record order, record-only when staying, failure isolation, one started and one terminal per turn, a late settle after cancel ignored, the Pilot precedence cases, and session-only when no Pilot. Run `on_linked_work` against a real in-memory `Service`, not a mock.
- [ ] **Step 2: Run:** `cargo test --locked --manifest-path crates/rebase-gpui/Cargo.toml -p rebase-core --profile native-dev board::` → FAIL.
- [ ] **Step 3: Implement**, comparing columns by `position`.
- [ ] **Step 4: Run:** the same command → PASS.
- [ ] **Step 5: Commit:** `git commit -m "Port board auto-move and linked-turn rules to native core"`

### Task 3: Native Board setting, AppView and shell wiring

**Files:**
- Modify:
  - `crates/rebase-gpui/src/settings/preferences.rs` (`board: bool`, default `false`, next to `notes`)
  - `crates/rebase-core/src/desktop_preferences/workspace_preferences.rs` (add `("board", "boardEnabled", false)`)
  - `crates/rebase-gpui/src/settings/pages/general.rs` (row after Notes, ~:89-94), `settings/model.rs` (search entry `id "board"`, label `"Board"`, keywords `"kanban cards columns tasks tickets track"`, ~:240), `settings/widgets.rs` (id list, ~:255)
  - `crates/rebase-gpui/src/workspace.rs` (`AppView::Board`, label `"Board"`, icon `"kanban"`)
  - `crates/rebase-gpui/src/shell/app_views.rs` (label, render, focus, `open_board`, like `open_notes`), `shell/project_rail.rs` (~:835-882), `shell/title_tabs.rs` (gate at ~:71: `view != Board || preferences.board`), `shell/classic_surfaces.rs`, `shell/classic_sidebar.rs`, `shell/classic_surface_tests.rs` (~:846-923)
  - `crates/rebase-gpui/src/shell.rs` (lazy `board: Option<Entity<BoardView>>`)
- Create:
  - `crates/rebase-gpui/assets/hugeicons/kanban.svg`: Hugeicons free `KanbanIcon`, converted to the same SVG format as the neighbouring files. The license is already in `hugeicons.LICENSE`.
  - `crates/rebase-gpui/src/board.rs`: a stub `BoardView` that renders "No boards yet" and a "Create board" button. Task 4 fills it in.

**Interfaces:**
- Produces: `AppView::Board`, `Shell::open_board(&mut self, cx)`, `preferences.board`, and `BoardView::new(db_path: PathBuf, cx) -> Self` in `crates/rebase-gpui/src/board.rs`.

- [ ] **Step 1: Write the failing tests:**
  - `preferences.rs`: `board` defaults to `false`.
  - The desktop-preferences import test (find the existing one for `pilotsEnabled`): a Tauri flag `boardEnabled=true` imports as `board: true`, and a missing key gives `false`.
  - `classic_surface_tests.rs`: extend the per-surface enumeration so Board is reachable when `board` is on and absent when it's off.
  - A `#[gpui::test]`: with Board open, flipping `preferences.board` to `false` closes the view and removes its title tab.
  - A settings search test that `"kanban"` finds the Board row.
  - In `shared_contracts.rs`: assert `tauri("src/features/settings/model/settings.ts")` contains `"rebase.boardEnabled"`, and the native import list contains `"boardEnabled"`.
- [ ] **Step 2: Run:** `bun run native:test` → FAIL.
- [ ] **Step 3: Implement** by mirroring `notes` and `Notes` in every file listed. Make surgical edits only.
- [ ] **Step 4: Run:** `bun run native:check` → PASS.
- [ ] **Step 5: Commit:** `git commit -m "Add a native Board setting and app view"`

### Task 4: Native board view (columns, cards, composer, refresh)

**Files:**
- Modify: `crates/rebase-gpui/src/board.rs`
- Create: `crates/rebase-gpui/src/board/` submodules if `board.rs` would exceed about 800 lines (`header.rs`, `column.rs`, `card.rs`), following the `notes/` folder pattern.

**Interfaces:**
- Consumes: `rebase_core::board::Service` (Task 1), `AppView::Board` wiring (Task 3).
- Produces: `BoardView` state: `boards`, `open_board_id`, `detail: Option<BoardDetail>`, `dragging: bool`, `pending_reload: bool`, `last_data_version: i64`. Also `BoardView::reload(&mut self, cx)`, called by Tasks 5–6.

UI behaviour (copy the Tauri behaviour; the visuals follow the spec's Visual Design table):
- **Board header:** a switcher (reuse the app's existing picker/popover component; find what `notes.rs` or `automations.rs` uses for a select), "New board", and a trailing `+` that adds a column.
- **Column header:** role icon (14 px; draw the rings in GPUI with the role colours mapped to native tokens), name, count, and on hover `+` and `…`. The `…` menu has Rename, Set role and Delete. Delete is disabled with the tooltip `"Column has N cards"` while the column isn't empty.
- **Card:** title (two-line clamp), fixed-width `#N`, link chip, and relative updated time. A card in a `done` column shows its title at 60 %.
- **Composer:** "+ Add card" turns into an input with the hint `"Enter to add · Esc to cancel"`. Enter adds the trimmed title (an empty title adds nothing), Esc cancels.
- **Remembered board:** the last open board id is kept in the native preferences or workspace state, not localStorage.
- **Refresh:**
  - reload when the view opens, and when the window becomes active;
  - while the view is open, a 2 s timer compares `Service::data_version()` with `last_data_version` and reloads when it changes;
  - while `dragging` is true, a reload is deferred (`pending_reload = true`) and runs right after the drop or cancel.
- **Errors** go through the app's existing toast or notification mechanism (find what `notes.rs` uses).

- [ ] **Step 1: Write the failing `#[gpui::test]`s** (with `VisualTestContext`, a temp DB and `Service`):
  - It renders 4 default columns with names and counts after "Create board".
  - Enter in the composer creates a card with the trimmed title. An empty title creates nothing. Esc closes the composer.
  - The column Delete item is disabled while the column has cards.
  - A write from a second `Service` connection appears after the poll tick. Advance the test executor's clock by 2 s.
  - With `dragging = true`, the poll tick doesn't reload. After the drag ends, it reloads once.
  - A session-linked card whose persisted id resolves through the store shows a chip with the session title. An unknown id shows `"Linked work no longer exists"`.
- [ ] **Step 2: Run:** `bun run native:test` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run:** `bun run native:check` → PASS.
- [ ] **Step 5: Commit:** `git commit -m "Add the native board view with columns, cards and live refresh"`

### Task 5: Native drag-and-drop and "Move to…"

**Files:**
- Modify: `crates/rebase-gpui/src/board.rs` (or its `card.rs` / `column.rs`)
- Create: `crates/rebase-gpui/src/board/drop.rs`, containing the pure drop-index logic

**Interfaces:**
- Consumes: Task 4 `BoardView`, `Service::card_move` / `column_move`.
- Produces:
  - `pub(crate) fn card_drop_index(card_tops: &[Pixels], card_heights: &[Pixels], pointer_y: Pixels) -> usize`: the insertion index is the first card whose vertical midpoint is below the pointer, otherwise the end.
  - `pub(crate) fn resolve_card_drop(detail: &BoardDetail, card_id: &str, column_id: &str, index: usize) -> Option<(String, usize)>`: returns `None` for a same-place drop. Mirror `finalCardTarget` / `resolveCardDrop` in `src/features/board/ui/kanban/kanbanMove.ts`, including the within-column index shift.

Behaviour:
- **Card drag:** GPUI `on_drag` with a `DraggedBoardCard { card_id, from_column }` payload and a preview view. The preview is the card at full size, tilted 1.5° if GPUI supports a transform; otherwise it gets the spec's shadow only. Note in the report which one you used.
- **Drop targets:** each column body takes `on_drag_move` and `on_drop`. While hovering, the column gets the 6 % accent tint and 38 % accent border, and a 2 px accent line marks the insertion index. On drop, resolve the target. When it isn't `None`, apply the move optimistically to `detail` and call `card_move(…, "user")`. On error, reload and toast.
- **Column reorder** by dragging the header, hand-rolled like `review.rs:781-860` (`start_tab_drag` / `move_tab_drag` / `finish_tab_drag`), calling `column_move`.
- **"Move to…"** in the card `…` menu lists the other columns. Choosing one moves the card to the end of that column with actor `"user"`. This is the keyboard-accessible path, so the menu must work with the keyboard the same way the other menus do.
- **The `dragging` flag** from Task 4 is set on drag start and cleared on drop or cancel. A release outside the board fires neither, so the flag is also cleared whenever GPUI stops reporting an active drag, and any deferred reload runs then.

- [ ] **Step 1: Write the failing tests:**
  - Unit tests for `card_drop_index`: above the first midpoint → 0; between → 1; below all → len; an empty column → 0.
  - Unit tests for `resolve_card_drop`: same place → `None`; within a column 0 → 2 of 4 gives the shifted index the backend expects; across columns gives the hovered index; an empty column gives 0.
  - A `#[gpui::test]` that simulates a drag (`cx.simulate_mouse_down` / `simulate_mouse_move` / `simulate_mouse_up`; see `explorer.rs` tests or `review.rs` tab drag tests for the helpers) moves a card across columns, and the DB shows the new column.
  - A same-place drop writes no activity row.
  - "Move to… → Done" moves the card and logs a `moved` row with actor `user`.
  - A rejected move (point the card at a deleted column) reloads and shows the error.
- [ ] **Step 2: Run:** `bun run native:test` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run:** `bun run native:check` → PASS.
- [ ] **Step 5: Commit:** `git commit -m "Add drag-and-drop and Move to… for native board cards"`

### Task 6: Card detail, linking, and linked-turn hooks

**Files:**
- Modify:
  - `crates/rebase-gpui/src/board.rs` (card detail: title and body edit, link picker, "Open linked work", activity timeline, Delete with confirmation)
  - `crates/rebase-gpui/src/conversation.rs` (turn start ~:1061, settle ~:1292)
  - `crates/rebase-gpui/src/pilots/mod.rs` (Pilot chat finish ~:995)
  - wherever native habit runs start (`rebase_core::pilot::habits`)
- Modify: `docs/native/parity.md` (add a Board row), `docs/native/verification.md` (a Board section listing the manual checks below as pending)

**Interfaces:**
- Consumes:
  - Task 2's `LinkedTurns`, `turn_pilot_id` and `on_linked_work`.
  - Task 1's `Service`.
  - `Store::persisted_session_id(i64)`, `Store::started_links()` (child → caller), and `Pilot { id, session_id }`.

Behaviour:
- **Detail view.** It mirrors the Tauri `CardDetail`:
  - The link picker lists sessions and Pilots (no Flow runs). Archived sessions aren't offered, but an existing link to one still resolves.
  - "Open linked work" opens the session or Pilot using the shell's existing navigation, and reports a failure through the toast.
  - The timeline reads `Moved from <from> to <to> · <actor> · <relative time>`, and a failed load shows `"Couldn't load activity"`.
  - Delete asks for confirmation first, using the app's existing confirm modal (`modal.rs`).
- **Turn hooks.** `LinkedTurns` lives on one app-level owner (shell or app state), so its keys outlive conversation views. At turn start in `conversation.rs`:
  - The session key is `persisted_session_id(summary.id)`.
  - The Pilot is `turn_pilot_id(own, habit, spawned)`:
    - own = a Pilot whose `session_id == Some(id)`;
    - habit = the Pilot that owns a habit-run session;
    - spawned = `started_links()[id]` → that caller's Pilot.
  - Dispatch `started`.
- **At settle** (`finished`):
  - If `stopping` was set, call `cancelled`.
  - Otherwise call `settled(ok)`, where `ok` is false when the turn ended with an error status.
  - Dispatch the result on a background executor. Board errors are logged and never surface into the turn.
- **Pilot chat path.** Hook `pilots/mod.rs` the same way, unless it already goes through `conversation.rs`. If it does, note that in the report and don't hook it twice.
- **Remote sessions** send no events, matching Tauri.

- [ ] **Step 1: Write the failing tests:**
  - **Turn events** (`#[gpui::test]` or core-level, whichever can drive a fake turn; follow the existing `conversation` tests that stub `run_turn`):
    - a turn on a session linked to a card moves it to In Progress, then to Review on a successful finish;
    - an error finish logs `failed` and the card stays;
    - a stopped turn logs only `started`;
    - a session spawned by a Pilot also moves that Pilot's card.
  - **Detail view:**
    - a card linked in Tauri form (persisted string id) resolves and opens;
    - a missing session shows `"Linked work no longer exists"` and hides "Open linked work";
    - Delete is cancelled when the confirm is declined;
    - a failed activity load shows `"Couldn't load activity"`.
- [ ] **Step 2: Run:** `bun run native:test` → FAIL.
- [ ] **Step 3: Implement**, with surgical edits in `conversation.rs` and `pilots/mod.rs`.
- [ ] **Step 4: Run:** `bun run native:check`, `bun run check:web` and `bun run check:rust` → all PASS.
- [ ] **Step 5: Manual checks**, recorded as pending in `docs/native/verification.md` for a human in `bun run native:dev`. Don't mark any as run unless you actually ran them:
  - Turn Board on.
  - Create a board in native and see it in Tauri; edit it in Tauri and see it in native within about 2 s.
  - Drag cards within and across columns, and onto an empty column.
  - Reorder columns.
  - Use "Move to…".
  - Link a card to a session, send a prompt, and watch the card move.
  - Turn Board off with the view open.
- [ ] **Step 6: Commit:** `git commit -m "Link native board cards to sessions and Pilots and move them with turns"`
