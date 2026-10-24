# Native Board v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rework the native Board into one gr8r-style board. It has steps you define, richer tasks (projects, labels, priority, due date, subtasks, comments), a gr8r-style detail drawer that never overlaps the header, and it is on by default in both apps.

**Architecture:**
- **Storage.** The shared Tauri tables (`boards`, `board_columns`, `board_cards`, `board_activity`) stay exactly as they are. The new task data lives in native-only tables keyed by `board_cards.id` with `ON DELETE CASCADE`, so a card deleted in either app takes its extras with it.
- **One board.** Native always shows one board: the oldest one, created if none exists.
- **Header.** The board header becomes a gr8r header: title, search, filter, New task.
- **Cards and columns.** Thinner, quieter styling.
- **Detail drawer.** It docks beside the board, which gets narrower, instead of floating over it.

**Tech Stack:** Rust, rusqlite, GPUI (`vendor/gpui-pre`), `crates/rebase-gpui/Cargo.toml` workspace. The Tauri side changes only one default.

**Spec:** the 2026-10-24 review conversation. The user's screenshots compare the native board with gr8r (https://gr8r-taskmanagement.vercel.app/, a project → Board → open a task). Their answers, binding:
- native only, except that both apps have Board on by default;
- task fields: Projects (multiple), Labels, Priority, Due date;
- drawer sections: Linked work, Subtasks, Comments and Activity tabs, Mark complete and copy reference;
- "things on top" means the header row;
- one board, no project boards; projects are tags on a task; users add their own steps;
- nothing may overlap the drawer.

Earlier plans: `2026-10-23-board.md` (Visual Design) and `2026-10-24-native-board.md` (the native port this reworks).

## Global Constraints

- **Shared tables.** Tauri's board tables and their DDL stay unchanged, and `crates/rebase-core/tests/shared_contracts.rs` keeps checking the shared DDL and error strings as before. Its only intended edits are the `boardEnabled` import default, which Task 1 turns on, and setting the `board_native_*` tables apart from the four shared ones. New tables are prefixed `board_native_` and are created by `board::ensure_tables` after the shared DDL.
- **New tables:**
  - `board_native_meta(card_id PK, priority INTEGER NOT NULL DEFAULT 0, due_at INTEGER)`
  - `board_native_projects(card_id, project TEXT, PK(card_id, project))`
  - `board_native_labels(id PK, board_id → boards, name, color)`
  - `board_native_card_labels(card_id, label_id → board_native_labels, PK(card_id, label_id))`
  - `board_native_subtasks(id PK, card_id, title, done INTEGER, position)`
  - `board_native_comments(id PK, card_id, body, created_at)`

  Every `card_id` references `board_cards(id) ON DELETE CASCADE`.
- **Priority** is 0 = None, 1 = Low, 2 = Medium, 3 = High, 4 = Urgent. **Due date** is unix ms at local midnight, or NULL.
- **Projects** are Rebase workspace project paths (`workspace.projects`). The name shown is the project appearance name, or the folder name if there isn't one.
- **The task description is `board_cards.body`**, the field Tauri calls notes, so both apps show the same text.
- **One board in native:** `Service::default_board()` returns the oldest board by `created_at`, creating "Board" with the default columns if none exists. Native never shows a board switcher or "New board".
- **Board on by default:** the Tauri `BOARD_ENABLED_DEFAULT = true`, the native `Preferences.board` default is `true`, and the import default is `("board","boardEnabled",true)`.
- **Activity kinds** in Tauri's `board_activity` table stay limited to Tauri's kinds. Native-only edits (priority, labels and so on) log `edited` with a `detail`, such as `"priority: High"`. Subtask toggles and comments write no activity rows.
- **Look:** gr8r-style, with native tokens from `theme.rs`. Body text 13 px, meta 11 to 12 px. Hairline borders at `content(0.07)`. No heavy bold. Hover-only actions.
- **No new crates.** `Cargo.lock` stays unchanged. Commits carry no attribution and never use the old product name.
- **Gate:** the native gate (fmt, clippy `-D warnings`, core and GPUI tests), plus `bun run check:web` and `bun run check:rust` for Task 1 and at the end.

## Review Focus

1. **A card deleted in Tauri** takes its native extras with it, and nothing breaks if Tauri never created the native tables. Task 2 deletes a card through a raw Tauri-style DELETE and checks every `board_native_*` row is gone.
2. **Tauri has made several boards.** Native shows only the oldest, still offers no switcher, and leaves the others alone. Task 2 covers `default_board` with three boards.
3. **Search and filter together** narrow the cards, and clearing them restores the board. A card that matches by `#number` shows up. Task 3.
4. **The drawer is open while a poll reloads.** It keeps any unsaved description, and the step moves if another app moved the card. Task 5.
5. **A narrow window with the drawer open.** The board scrolls horizontally within its narrower column beside the docked drawer, with nothing behind the drawer, and the header stays clickable. Task 5 checks the header bounds don't intersect the drawer bounds.

---

### Task 1: Board on by default in both apps

**Files:** `src/features/settings/model/settings.ts` (`BOARD_ENABLED_DEFAULT = true`), `src/features/settings/model/settings.flags.test.ts`, `src/features/settings/ui/SettingsView.test.ts`, `crates/rebase-gpui/src/settings/preferences.rs` (`board: true`), `crates/rebase-gpui/src/settings/model.rs` test, `crates/rebase-core/src/desktop_preferences/workspace_preferences.rs` (import default `true`), `crates/rebase-core/src/desktop_preferences.rs` test, `crates/rebase-core/tests/shared_contracts.rs` (the expected import tuple).

- [ ] **Step 1: Update the tests first.**
  - Tauri: `BOARD_ENABLED_DEFAULT` is `true`, and `loadBoardEnabled()` with empty storage is `true`. The SettingsView toggle test starts on, so its first click writes `"0"`.
  - Native: `Preferences::default().board` is `true` (rename the test to `board_is_on_until_turned_off`). An unset desktop key imports as `true`; an explicit `false` imports as `false`. The contract expects `("board", "boardEnabled", true)`.
- [ ] **Step 2: Run** `bunx vitest run src/features/settings` and the native core and GPUI settings tests. Expect FAIL.
- [ ] **Step 3: Flip the four defaults.**
- [ ] **Step 4: Run** the native gate, `bun run check:web` and `bun run check:rust`. Expect PASS. Any Tauri or native test that assumed Board starts off must set the flag explicitly; fix those tests, not the code.
- [ ] **Step 5: Commit:** "Turn the Board on by default in both apps"

### Task 2: Native task data and the single board (core)

**Files:** `crates/rebase-core/src/board/mod.rs` (`ensure_tables` adds the native DDL; `Service::default_board`), create `crates/rebase-core/src/board/extras.rs`.

**Interfaces:**

```rust
pub struct CardExtras { pub priority: u8, pub due_at: Option<i64>, pub projects: Vec<String>, pub labels: Vec<String /*label id*/>, pub subtasks_done: usize, pub subtasks_total: usize, pub comments: usize }
pub struct Label { pub id: String, pub board_id: String, pub name: String, pub color: String }
pub struct Subtask { pub id: String, pub card_id: String, pub title: String, pub done: bool, pub position: i64 }
pub struct Comment { pub id: String, pub card_id: String, pub body: String, pub created_at: i64 }
impl Service {
    pub fn default_board(&mut self) -> Result<Board, String>;
    pub fn extras(&mut self, board_id: &str) -> Result<HashMap<String, CardExtras>, String>; // one query per table
    pub fn set_priority(&mut self, card_id: &str, priority: u8) -> Result<(), String>;      // 0..=4
    pub fn set_due(&mut self, card_id: &str, due_at: Option<i64>) -> Result<(), String>;
    pub fn set_projects(&mut self, card_id: &str, projects: &[String]) -> Result<(), String>;
    pub fn labels(&mut self, board_id: &str) -> Result<Vec<Label>, String>;
    pub fn create_label(&mut self, board_id: &str, name: &str, color: &str) -> Result<Label, String>;
    pub fn set_labels(&mut self, card_id: &str, label_ids: &[String]) -> Result<(), String>;
    pub fn subtasks(&mut self, card_id: &str) -> Result<Vec<Subtask>, String>;
    pub fn add_subtask(&mut self, card_id: &str, title: &str) -> Result<Subtask, String>;
    pub fn toggle_subtask(&mut self, subtask_id: &str, done: bool) -> Result<(), String>;
    pub fn delete_subtask(&mut self, subtask_id: &str) -> Result<(), String>;
    pub fn comments(&mut self, card_id: &str) -> Result<Vec<Comment>, String>;            // oldest first
    pub fn add_comment(&mut self, card_id: &str, body: &str) -> Result<Comment, String>;
}
```

Writes use `write_tx` (IMMEDIATE). Priority, due-date, project and label changes each log one `edited` activity row with a detail: `priority: High`, `due: 2026-11-02` or `due: none`, `projects: 2`, `labels: Bug, Docs`. Titles are trimmed and non-empty. Each label name is unique per board, case-insensitive. Colors come from the fixed palette in Task 4.

- [ ] **Step 1: Write the failing tests:**
  - `default_board_creates_one_board_once`
  - `default_board_is_the_oldest_of_several`
  - each setter round-trips through `extras`
  - priority 5 → `Err`
  - `set_projects` replaces the set, and duplicates collapse
  - `create_label` rejects a duplicate name, case-insensitively
  - subtasks keep their order, toggle, and count into `extras`
  - comments come back oldest first
  - `deleting_a_card_with_a_raw_delete_removes_its_native_rows`: run `DELETE FROM board_cards` on a second plain connection with `foreign_keys=ON`, then check every `board_native_*` table has no rows for the card
  - `extras_tables_leave_the_shared_ddl_untouched`: the existing contract tests still pass
- [ ] **Step 2: Run** `cargo test … -p rebase-core board`. Expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same tests and the contract tests. Expect PASS.
- [ ] **Step 5: Commit:** "Store native board task fields beside the shared board tables"

### Task 3: One board, gr8r header, search and filter

**Files:** `crates/rebase-gpui/src/board.rs` (load via `default_board`; remove the board switcher, New board, board rename/delete and `Menu::Switcher`), create `crates/rebase-gpui/src/board/header.rs`, update `board/tests.rs`. `shell/app_views.rs` passes the projects (`set_projects(paths, appearances)`) when opening the view and when they change.

**Behaviour:**
- **Header row** (`PANE_HEADER_HEIGHT`, bottom hairline):
  - kanban icon and "Board" (13 px medium);
  - a search field (220 px, placeholder "Search tasks") that matches the title or `#number`;
  - a "Filter" button with a count badge when active. Its popover has three sections: Projects (the projects in use plus all workspace projects), Labels and Priority. Each row toggles. A "Clear filters" row appears at the bottom.
  - a spacer, then a "New task" accent button that opens the composer in the first step.
- **Filtering** hides cards and leaves the counts showing visible / total, for example `2/5`, when a filter is active.
- **The empty state** is gone: the board always exists.

- [ ] **Step 1: Write the failing tests** (GPUI):
  - opening the view with no boards shows four steps, with no "board-switcher" and no "board-new";
  - typing in search narrows the cards, and "#2" finds card 2;
  - a Filter by project shows only tagged cards, and Clear restores them;
  - "New task" opens the composer in the first step;
  - the counts read `1/3` while filtered.
- [ ] **Step 2: Run** `board::` tests. Expect FAIL.
- [ ] **Step 3: Implement.** Remove the multi-board code paths and their tests, replacing them with the tests above.
- [ ] **Step 4: Run** the native gate. Expect PASS.
- [ ] **Step 5: Commit:** "Show one native board with a search and filter header"

### Task 4: gr8r card and column styling

**Files:** `board/card.rs`, `board/column.rs`, create `board/style.rs` (the label palette and the priority glyph).

**Spec:**
- **Columns.** 280 px wide, radius 10, fill `content(0.02)`, hairline `content(0.06)`, no heavier border. Header 36 px: role ring 13 px, name 13 px medium at `content(0.9)`, count 12 px at `content(0.4)`; `+` and `…` appear on hover only.
- **Cards.** Radius 8, fill `pane_fill()`, hairline `content(0.07)`, hover hairline `content(0.14)`, padding 10/12, gap 6.
  1. Label chips row, only when there are labels: each chip is 11 px text on `color@0.14` fill with a 6 px colour dot, radius 4, padding 1/6.
  2. Title: 13 px, `content(0.92)`, two-line clamp.
  3. Subtask progress row, only when there are subtasks: a checklist glyph, `done/total` in 11 px, and a 3 px bar (accent fill, `content(0.08)` track).
  4. Meta row (11 px, `content(0.45)`): mono `#N`; the priority glyph (three 2 px bars, filled to the level, `Urgent` = a red `!` box); a calendar glyph with the due date (`Today`, `Tomorrow`, weekday within 6 days, else `Nov 12`, red when overdue); a comment glyph with its count; the link chip; then the relative updated time on the right.
- **Label palette** (8 colours): gray `0x8a867e`, red `0xe5534b`, orange `0xe0823d`, yellow `0xe8c547`, green `0x3fb950`, teal `0x2bb3a8`, blue `0x4da3f5`, purple `0xa371f7`.
- **Done step:** the title at `content(0.6)` and a green check glyph before it.

- [ ] **Step 1: Write the failing tests:**
  - unit tests for `due_label(due_at, now) -> (String, bool overdue)`: today, tomorrow, a weekday within 6 days, a far date, and overdue;
  - unit tests for `priority_bars(p) -> usize` (0..=3, with Urgent shown separately);
  - a GPUI test that a card with 2 labels, priority High, a due date and 1/3 subtasks shows `board-card-labels-1`, `board-card-subtasks-1` and `board-card-due-1`, and a bare card shows none of them.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** the native gate → PASS.
- [ ] **Step 5: Commit:** "Restyle native board cards and steps like gr8r"

### Task 5: gr8r task drawer

**Files:** rewrite `board/detail.rs`, create `board/detail/properties.rs` and `board/detail/pickers.rs`; `board.rs` layout (the drawer docks).

**Layout:** the board view is a row of `[board column (flex 1, min 0)] [drawer (width 440, left hairline)]`. The header row lives inside the board column only, so the drawer never covers it. The drawer is full height.

**Drawer, top to bottom:**
1. **Top bar** (36 px): `Board / #12` in 12 px (`#12` mono), then a spacer, then actions:
   - "Mark complete": a circle glyph and label; once done it reads "Completed" with a green check, and clicking moves the card to the done-role step;
   - copy reference: writes `#12 Title` to the clipboard;
   - `…`: Delete task, with a confirm;
   - close `×`.
2. **Title:** an inline input, 18 px semibold, saved on Enter or blur.
3. **Properties:** a two-column grid with 120 px label cells (12 px, icon plus text) and 32 px rows, click to edit:
   - Status: the step, chosen in a Menu;
   - Priority: a Menu of None, Low, Medium, High, Urgent, with glyphs;
   - Due date: a Menu of Today, Tomorrow, Next week, "Pick a date…" (an inline `YYYY-MM-DD` input) and Clear;
   - Projects: a multi-select popover with search, showing chips;
   - Labels: a multi-select popover with search and "Create label "x"" (next palette colour), showing chips;
   - Linked work: the chip, then Open or Change in the existing picker.

   An empty value shows `Empty` at `content(0.35)`.
4. **Description:** a section label, then a borderless multi-line input with placeholder "Add a description…", saved on blur. It writes `board_cards.body`.
5. **Subtasks** `done/total` with a progress bar: checkbox rows (toggle; Enter on the add row creates; delete on hover), then "+ Add subtask".
6. **Tabs:** `Comments N` | `Activity N`.
   - Comments: a list (avatar initial, "You", relative time, body), then a composer ("Leave a comment…", ⌘Enter or a Comment button).
   - Activity: the existing timeline.

Escape closes, and saves unsaved edits. Edits made elsewhere refresh fields you haven't changed (keep the `loaded_*` rule).

- [ ] **Step 1: Write the failing tests** (GPUI). Rewrite the existing detail tests to the new selectors.
  - Opening a card shows `board-drawer-status`, `-priority`, `-due`, `-projects`, `-labels` and `-linked`.
  - Picking Priority High updates the card's priority glyph, and the activity log gains `edited` with `priority: High`.
  - Due → Tomorrow shows "Tomorrow" on the card.
  - Toggling two projects stores both, and the Filter can then find the card by either one.
  - Creating label "Bug" adds the chip to the card.
  - Adding and toggling a subtask updates the card's `1/1` progress.
  - Adding a comment shows it in the list, and the card's comment count reads 1.
  - Mark complete moves the card to Done.
  - Copy reference puts `#1 Title` on the clipboard.
  - Delete with confirm, and a declined confirm keeps the card.
  - **No overlap:** with the drawer open, `board-header`'s bounds and `board-drawer`'s bounds don't intersect, and "New task" is still clickable.
  - Open linked work, the missing-link state and the failed-activity state still behave (port the old tests).
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** the native gate → PASS.
- [ ] **Step 5: Commit:** "Rebuild the native task drawer like gr8r"

### Task 6: Docs and full verification

**Files:** `docs/native/parity.md` (Board row: one board, task fields, on by default), `docs/native/verification.md` (Board section: new manual checks for the header, filters, every drawer property, subtasks, comments, and no overlap), and the native README feature list if it mentions Board.

- [ ] **Step 1:** Update the docs.
- [ ] **Step 2: Run** the native gate, `bun run check:web` and `bun run check:rust`. Expect PASS.
- [ ] **Step 3: Commit:** "Document the native Board v2"
