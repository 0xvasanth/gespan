# Native verification

Status recorded on October 22, 2026, on macOS Apple Silicon. These are local
results, not a release certification. See [preview limitations](parity.md) for
remaining runtime and platform work.

## Recorded results

| Check | Result |
| --- | --- |
| Complete native check (`bun run native:check`) | Passed |
| Native script tests (`bun run native:test:dev`) | 19 passed, including development restarts, bundle data modes and dependency guards |
| Native Rust formatting and strict Clippy | Passed for core and GPUI, including all targets |
| Native core tests | 547 passed, 1 ignored |
| GPUI tests | 551 passed, 11 ignored |
| Editor asset regeneration and smoke checks | Passed, including large-file diff highlighting |
| Remote checks | 50 core tests, 25 native runtime/UI tests, Connections and secret-input regressions passed; isolated Node-host and loopback SSH fixtures also passed |
| Host suite | 112 passed, 6 skipped |
| Production frontend | 5,560 tests passed, 13 skipped; TypeScript and production build passed under Node 24 |
| Production Rust workspace | 698 tests passed, 1 SSH fixture ignored; formatting and strict Clippy passed |
| Production macOS packaging | Tauri release `.app` built and its local ad-hoc signature verified |
| Native macOS commands | `native:dev` built and launched; `native:build` produced an optimized app without launching it; both bundle signatures verified |
| Native database defaults | Dev opened `Gespan Development/gespan.db`; release `--check-data` read the production database without migration |
| Native UI and chat | Provider order, live Codex streaming, Stop, saved history after relaunch, resumed provider context, and file preview/source passed |
| Native release UI | Restored existing workspace and conversation history from a consistent production database snapshot |
| macOS notifications | Reminder appeared in Notification Center; opening it returned from Settings to the matching chat and cleared the reminder |

The ignored tests require explicit SSH/host fixtures, a Metal device or a manual
frame-cost measurement. Cargo still reports an upstream future-compatibility
advisory for `block 0.1.6`. The preview workflow has not been run as part of these
local checks. Linux/Windows GUI checks and broad live-provider verification
remain outstanding.

Production source, lockfiles and release workflows were checked against `main`;
the native dependency graph remains separate. Live provider requests used the
development database and a temporary project. The release GUI used a disposable
production snapshot with background agents and automations disabled; only the
read-only data check opened the live production database. Notification delivery
used a separate test bundle identity. Notification sounds, Dock badges and
completion/approval alerts were not manually checked.

Reopening Tauri after native use of the live production database remains a manual
check. Concurrent shared-data startup, scheduled tasks and phone connections
remain unverified and deferred.

## Desktop preference migration — October 23, 2026

On Linux, all 27 automated desktop preference tests and strict core Clippy (`--all-targets`,
`--profile native-dev`, `-D warnings`) passed. The installed Linux WebKitGTK
preference store was readable. Synthetic fixtures verified that WebView2 reads
preserve source filenames and contents with the source database both open/locked
and closed, and that empty origins do not hide saved preferences. A WebKitGTK
fixture also verified eight saved projects reach the restored native workspace.
The full core suite passed serially: 557 passed, 1 ignored. The initial parallel
run failed two provider-process fixtures; both passed in a serial rerun and in
the full serial suite.

The Windows-only module's conditional-compilation path passed an isolated
`-D warnings` check on Linux. This was not a native Windows build. Real WebView2
profile compatibility, a Windows build and Windows execution remain unverified.
Copies of actively written LevelDB files are best-effort and may be incomplete
or unreadable; close Tauri before first-launch preference restoration.

The installed-profile integration test is ignored by default because it reads
personal storage. To run that read-only check explicitly:

```sh
cargo test --manifest-path crates/gespan-gpui/Cargo.toml --locked -p gespan-core --profile native-dev installed_desktop_dir_reads -- --ignored
```

## Board #174 verification script

Both apps read and write the same `boards`, `board_columns`, `board_cards`
and `board_activity` tables, plus the shared label, project, subtask, comment
and ticket-link tables, through the `gespan-board-store` crate;
`crates/gespan-core/tests/shared_contracts.rs` pins the DDL, error strings and
the `gespan.boardEnabled` key to Tauri. The old `board_native_*` tables are
left in place for one release; after the one-time copy nothing reads them.
Deleting a card in either app removes its own rows, including which labels it
carries; the board's label definitions stay until the board is deleted. The
Board setting is on by default in both apps.

Automated coverage: storage, ordering, extras, concurrent writes, statement
counts per board-wide read and a 1,000-card query budget (board store and
core); auto-move and turn rules (core); windowing, keyboard focus and
announcements at 1,000 cards (Vitest and GPUI tests); and every feature in
[Board details](parity.md#board-details).

The steps below have not been run by hand yet. Record each result as pass or
fail with the date and build. Run each step in the app or apps named; where it
says both, run it once in each.

### Setup

1. Quit both apps. Back up `gespan.db` (paths are in the
   [native guide](README.md#data-and-configuration): release is
   `~/Library/Application Support/dev.gespan.desktop/gespan.db` on macOS,
   development is under `Gespan Development/`).
2. Add a 1,000-card board to it. This always creates a new board from the blank
   template, never edits an existing one, and refuses a file that is not a
   Gespan database (it has no `boards` table) without changing it. It prints the
   new board's id.

   ```sh
   cargo run -p gespan-board-store --example seed_board -- --db "<path to gespan.db>"
   ```

   Optional flags: `--cards 1000` and `--name "<board>"`.
3. Build both apps in release mode (`bun run native:build` for Native and a
   Tauri release build) and open both. Delete the seeded board when you are
   done.

### Steps

Result column: pass or fail, date, build.

#### 1. The #174 manual checks

| # | App | Action | Expected | Result |
| --- | --- | --- | --- | --- |
| 1.1 | Native, then Tauri | Set a label on a card in Native. | Within about 2 s the label shows on the card in Tauri. | |
| 1.2 | Tauri, then Native | Set a label on a card in Tauri. | It shows in Native. | |
| 1.3 | Both | Create a board from the "Agent pipeline" template. On a card, "Run with…" (pick a provider, model and a local project). | The card gets a session link and inherits its ticket, if it has one, and moves Queued, Running, Needs review. Link chips refresh on board open and when the turn settles, not on a timer. Native refuses a remote project. | |
| 1.4 | Both | Collapse a column. Drop a card onto its strip with the mouse. Then lift another card with the keyboard (Space), move onto the strip and drop. | Each card lands at the end of the collapsed column. Click or Enter expands it, and it stays collapsed after reopening the board. | |
| 1.5 | Both | Collapse a column in one app, then change its colour or WIP limit in the other. | The collapse does not reset the colour or WIP limit just set in the other app. | |

#### 2. 1,000 cards, release builds

Open the seeded board in each app. Use a screen recording or the app's frame
timing.

| # | App | Action | Expected | Result |
| --- | --- | --- | --- | --- |
| 2.1 | Both | Switch to the seeded board from another board. | The cards show within 300 ms. | |
| 2.2 | Both | Type a search that has results. | Results show within 100 ms (in Tauri, within 100 ms after its 150 ms wait, so about 250 ms after the last key). | |
| 2.3 | Both | Drag a card across columns, and within a scrolled column. | The drag stays smooth, with no visible stutter. The card lands where the line showed. | |
| 2.4 | Both | Lift a card with the keyboard and move it down 50 places. | The view follows the card and the announcement reads "position 51 of 250". | |
| 2.5 | Both | Click a card, then Shift-click a card far below, scrolling between. | Exactly the cards in display order between them are selected, including cards that were off screen. | |
| 2.6 | Both | Group by label (swimlanes). Scroll, drag a card between lanes, and open a card. | Scrolling and dragging stay smooth. A drop in a cell appends to it. | |
| 2.7 | Both | Scroll deep into a column, then change a card in the other app. | The same card stays at the top of the view. It does not jump to the top or leave a gap. | |
| 2.8 | Both | Toggle Comfortable and Compact density while scrolled. | Nothing overlaps and no gap appears. | |
| 2.9 | Native | Press an arrow key on a card, then scroll away and back. | Focus lands on the right card, and stays on it after the wheel scroll. | |

#### 3. Screen reader (VoiceOver)

| # | App | Action | Expected | Result |
| --- | --- | --- | --- | --- |
| 3.1 | Both | With VoiceOver on, Tab to a card, press Space, move and drop. | VoiceOver reads "Picked up card #N, {title}. {column}, position i of 250 (the column's count).", the "over" announcements as you move, and "Dropped card #N in {column}, position i of 250.". Escape reads the "was cancelled" line. | |
| 3.2 | Tauri | Focus a card in WebKit. | VoiceOver reads the card's `aria-description` ("{i} of {total} in {column}"). | |
| 3.3 | Both | Open a card with E or Enter, then close it. | The drawer takes focus, and closing it gives focus back to the card. | |
| 3.4 | Native | Focus a column. | It reads "{name}, {total} cards" ("1 card" for one). | |

#### 4. Focus and scrolling in real WebKit

| # | App | Action | Expected | Result |
| --- | --- | --- | --- | --- |
| 4.1 | Tauri | Press ⌘N, create a card, and let the New card modal close. | Focus returns to the board, and a second ⌘N works. | |
| 4.2 | Tauri | Scroll a long column with the trackpad (momentum), while cards measure in. | No scroll jitter. | |
| 4.3 | Tauri | Keyboard-drag a card down a long column with Space and arrows. | The column scrolls to follow the drop line once, with no fight between dnd-kit's scrolling and ours. | |
| 4.4 | Tauri | Drop a card with the mouse, then click a card within 50 ms. | The click right after a drop is swallowed (no drawer opens). A click after that opens it. | |
| 4.5 | Tauri | Group by a label. Collapse a column. | The collapsed column draws as a 36 px strip showing only its count, and it takes drops. | |
| 4.6 | Tauri | In swimlanes, keyboard-drag a card to a lane far below. | The view scrolls to the target lane as the line moves from header to header. | |
| 4.7 | Tauri | Focus a card, then delete it or archive it from the other app. Click empty space on the board. | Focus does not fall to the page body, and arrow keys keep working from where it lands. | |
| 4.8 | Native | Keyboard-drag or arrow to a card 50 places down, and across a swimlane band edge. | The card reveals on the first or a following frame, never a far jump, and focus lands on it. | |

#### 5. Checks carried from C5

| # | App | Action | Expected | Result |
| --- | --- | --- | --- | --- |
| 5.1 | Both | Board settings: change name, icon, colour and default view. Give a column a colour and WIP limit; drop a card over it. | Both apps show the changes. The header reads `{total}/{limit}` (`{shown}/{total}` while filtered). The toast reads "Over the WIP limit ({n}/{limit})". "Move cards to…" moves cards on delete; archived cards left refuse it up front. | |
| 5.2 | Both | Labels: create, rename (a name differing only in case is refused), recolour, reorder, merge (confirm first) and delete. Rename and hide priority levels. | Cards and saved-view label rows follow a merge. A card on a hidden level still shows it. | |
| 5.3 | Both | Custom fields: add all 7 kinds, set values, rename a select option, delete a field. Type an estimate `90`, `90m`, `1h`, `1h 30m`, `1.5h`, `.5h`. Then try `1.m`, `0x10`, `1e3`, `-5`. | Values stay after an option rename. Deleting a field removes its filter rows and table columns. The first set is accepted. The second is refused with "Use minutes or hours, like 90, 90m, 1h 30m or 1.5h", and the typed text stays in the field. | |
| 5.4 | Both | Display options: toggle card-face items and density, override them in a saved view. Group by priority, label, project and a field. Drop across lanes. | "All cards" keeps the board default. Priority and select lanes set the value; the others change only the column. Value-setting drops have no Undo. | |
| 5.5 | Both | Create a board from Blank, Agent pipeline, Bug triage and Release checklist. "Save as template", create from it, then try to delete a built-in. | The new board has no cards. The error reads "Built-in templates can't be deleted". | |
| 5.6 | Both | Auto-move: for each event set default, stay and a chosen column, and run a linked session through started, needs input, completed and failed. Check PR opened and merged. | The card follows each setting. A deleted target column means stay. | |
| 5.7 | Both | Block a card with a live card, then "Run with…" it. | A lock shows, and the dialog asks "#{n} is blocked by …. Run anyway?" with Run anyway and Cancel. | |
| 5.8 | Both | Write-back: with it on, finish a linked turn. With it off, finish another. Turn ticket status changes off (board toggle or session override) and have an agent change status. | A comment marked Agent appears only when it is on (a plan turn posts its plan). The status change is refused when off. Activity rows read You, Agent or System. | |
| 5.9 | Both | Make the settings read fail. | The board still loads, the sections show "Couldn't load settings" with Retry, and nothing is saved until Retry works. | |
| 5.10 | Tauri | Use the command palette's "New session from ticket…". | A local session opens in the active project with the ticket linked and its key (later its title) as an unsent draft. Nothing is sent. | |

For the rest of the board (filters, saved views, list, table, bulk select,
shortcuts, undo, the Archive view, relations, links and the drawer), run the
features listed in [Board details](parity.md#board-details) in both apps and
compare. Check due filters once around midnight or a DST change.

## Local checks

Run the complete native check from the repository root:

```sh
bun run native:check
```

Core tests run in parallel. GPUI tests run serially to isolate rendering state
and avoid CPU contention with the interpreter's wall-clock limits.

For focused work, run the relevant part:

```sh
bun run native:test:dev
cargo fmt --manifest-path crates/gespan-gpui/Cargo.toml --all --check
cargo clippy --manifest-path crates/gespan-gpui/Cargo.toml --locked -p gespan-core -p gespan-gpui --all-targets --profile native-dev -- -D warnings
bun run native:test
```

On macOS with a Metal device, the renderer regression opens no window:

```sh
cargo test --manifest-path crates/gespan-gpui/Cargo.toml --locked -p gespan-gpui --profile native-dev overlapping_shell_pane -- --ignored
```

## Optional runtime checks

The provider smoke example uses an existing CLI login. `--models` only discovers
the catalog; without it, the example makes two real provider requests and checks
streaming and resumed context:

```sh
cargo run --manifest-path crates/gespan-gpui/Cargo.toml --locked -p gespan-core --profile native-dev --example provider_smoke -- cursor --models
cargo run --manifest-path crates/gespan-gpui/Cargo.toml --locked -p gespan-core --profile native-dev --example provider_smoke -- cursor
```

For long-history interaction, use the isolated fixture from the
[native guide](README.md#checks-and-fixtures). Check typing and scrolling while a
separate real session streams, then switch tabs and verify draft/viewport state,
Stop, and saved history after relaunch. Synthetic history is not sent to providers.
