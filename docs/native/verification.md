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

## Board — October 24, 2026

Both apps read and write the same `boards`, `board_columns`, `board_cards`
and `board_activity` tables, plus the shared label, project, subtask, comment
and ticket-link tables, through the `gespan-board-store` crate;
`crates/gespan-core/tests/shared_contracts.rs` pins the DDL, error strings and
the `gespan.boardEnabled` key to Tauri. The old `board_native_*` tables are
left in place for one release; after the one-time copy nothing reads them.
Deleting a card in either app removes its own rows, including which labels it
carries; the board's label definitions stay until the board is deleted. The
Board setting is on by default in both apps.
Automated coverage: storage, ordering, extras and concurrent writes (core);
auto-move and turn rules (core); the setting and app view; the board switcher,
search and filters; card and step styling; drag and drop, keyboard dragging and
Move to…; the task drawer; card-face quick edits, the column menu, collapsed columns,
undo, held deletes and the Archive view; and turn hooks (GPUI tests).

These runtime checks are pending; none has been run by hand yet:

- Board is on by default; turning it off in Settings → General hides the rail,
  sidebar and title-tab entries, and turning it on again shows the same board.
- The header's board switcher creates, renames and deletes boards. Deleting
  the open board opens another, and deleting the last one creates a new
  default board. The board you opened last is still open after quitting and
  reopening.
- Search narrows cards (also by `#12`), Filter narrows by project, label and
  priority, counts read `shown/total`, and New task opens the add field in
  the first step.
- Cards show label chips, subtask progress, priority, due date (red when
  past), comment count and linked work; steps add, rename, reorder and delete.
- Drag cards within a step, across steps and onto an empty step, including
  while a filter is on.
- Drag a card from the keyboard: Tab to a card, Space lifts it, arrows move
  it, Space or Enter drops it, and Escape puts it back with nothing written.
- Use a card's "Move to…" menu, both by clicking "…" and from the keyboard
  (Tab to a card, Enter, arrow keys, Enter).
- Open a task: the drawer sits beside the board and never covers the header.
  Change status, priority, due date (including "Pick a date…"), projects,
  labels (including creating one), linked work, description, subtasks and
  comments; Mark complete, copy reference and Delete.
- Card face: click priority, due or labels and a small menu opens in place
  without dragging or opening the card; a card with no priority shows a faint
  target on hover. In Native, "Pick a date…" opens the drawer's date picker.
- Column menu: Collapse column, Sort by priority (Urgent first), Sort by due
  date (soonest first, no date last) and Mark all as done (hidden on the done
  column). Archive completed shows only on the done column. Sort with a filter
  on still sorts the whole column.
- Collapse a column: it becomes a thin strip with its name and count (Native
  stacks the name top to bottom). Drop a card on it by mouse and from the
  keyboard; the card lands at the end. Click or Enter expands it, and it stays
  collapsed after reopening the board.
- Quick add: type a title and press Enter; the field stays open for the next
  card, and Escape closes it.
- Undo a move: drag a card to another step, press Undo in the toast ("Moved #N
  to {column}") within 8 s, and it returns to its old place. Do the same for a
  multi-card move ("Moved N cards to {column}").
- Undo a sort: sort a column and press Undo ("Sorted {column}"); the previous
  order returns exactly.
- Undo an archive: Archive completed ("Archived N cards"), then Undo; the cards
  return to their columns and positions.
- Undo a delete: delete a card ("Deleted #N"); it leaves the board but is not
  in the Archive view. Undo brings it back at its old place. Let the toast
  expire instead and the card is gone for good.
- Quit during a held delete: delete a card, quit within 8 s and reopen. The
  card is in the Archive view, not lost; Restore puts it back.
- Move or delete the card in the other app during the 8 s, then press Undo:
  the toast says what could not be undone and nothing is duplicated.
- Archive view: open it from the board; archived cards are newest first.
  Restore puts a card at the end of its column, and Delete forever asks first.
- Delete column is disabled with "Move or delete its N cards first" while the
  column holds live or archived cards, and works once both are gone.
- Edit a board in Tauri and see the change in native within about 2 s. Both
  apps show the same boards, tasks, labels, priority, due dates, subtasks,
  comments and ticket links.
- In Tauri, the same task carries priority, due date, projects, labels,
  subtasks and comments in the docked drawer; search and filters show
  `shown/total`, and cards drag with the keyboard (Space lifts, arrows move,
  Space or Enter drops, Escape cancels).
- Card detail: toggle Full page in the drawer's top bar (Escape or the toggle
  goes back). Set "Open cards in" to Full page (Native: board header menu;
  Tauri: Settings → General), click a card, and it opens full page; set it back
  to Drawer.
- Dates: set start and due with each quick pick (Today, Tomorrow, Next week,
  In 2 weeks), pick one with the date input, and Clear.
- Estimate: enter `90`, `90m`, `1h`, `1h 30m` and `1.5h`; each shows as `1h 30m`
  style text. `0` and text that is not a duration are refused.
- Relations: add Blocked by, Blocks and Relates to from the picker (cards show
  as `#N Title`). Making a cycle (A blocks B, then B blocks A) shows the
  refusal and writes nothing.
- Custom fields: a board that already has text, number, date, select and
  checkbox fields shows them in the drawer; edit each and reopen the card.
- Links: add a session, a Pilot, a PR URL and a worktree path; Open and Remove
  each. The ticket keeps its own row.
- Markdown toolbar: bold, italic, heading, bulleted and numbered list, link and
  code on a selection, on several lines and on an empty selection; the cursor
  lands between the markers, and the saved description is plain Markdown.
- Subtasks: Move up and Move down from the subtask menu (in Tauri also
  Alt+↑ and Alt+↓); the first has no Move up and the last no Move down.
  Convert to card puts a new card right after the parent, linked with Relates
  to, and removes the subtask.
- Comments show "You" for yours and "Agent" for an agent's, with different
  avatars.
- More menu: from the drawer's "…", a card's right-click and "More…" in the
  card's "…" menu. Check Open, Status…, Priority…, Labels…, Move to board…,
  Copy link, Copy `#N title`, Duplicate, Archive and Delete. Duplicate adds
  "{title} (copy)" after the original without links, comments or relations.
  Move to board… lands in the other board's first To do column. Archive and
  Delete show the Undo toast. Copy link copies a `gespan://board/…/card/…`
  link; the app does not open it yet.
- New card: press ⌘N (Ctrl+N) with focus on the board, including in the search
  box; the modal opens and no "n" is typed. A title is required; ⌘↵ creates,
  Escape closes. With Create more on, the modal stays open with the title
  cleared and the other fields kept.
- Send a message to a Pilot with Board on and with Board off; a Pilot-linked
  card moves when Board is on, and nothing breaks either way.
- Link a card to a session, send a prompt, and watch the card move to In
  Progress, then Review.

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
