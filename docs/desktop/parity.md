# Known limitations

A feature being present in source does not establish that it has been tried by
hand on every platform. See [checks](verification.md) for the commands to
check it locally.

The Tauri app is retired, after every feature it had was ported (epic #217). Its
checklist and the old side-by-side comparisons are in this file's history.

## Providers

All twelve providers have discovery, model selection and native transports.
The live results below are historical spot checks, not certification of every
provider feature or the current revision.

| Provider | Transport | Last recorded live coverage |
| --- | --- | --- |
| Claude Code | stream-json | Streaming, resume and cancellation during the initial native slice |
| Codex | app-server | Streaming, relaunch persistence, resumed context and cancellation (Oct 22) |
| Cursor | ACP | Catalog, streaming, two-turn resume and read-only Plan mode (Oct 19) |
| Grok Build | ACP | Catalog, streaming and two-turn resume (Oct 19) |
| OpenCode | HTTP/SSE v1/v2 | Current transport has source review only; earlier ACP checks do not cover it |
| Pi | RPC | Catalog, streaming and two-turn resume (Oct 19) |
| omp | RPC | Catalog, streaming and two-turn resume (Oct 19) |
| fx | ACP | Catalog, streaming and two-turn resume (Oct 19) |
| Hermes Agent | ACP | Catalog; prompt blocked by upstream account credit (Oct 19) |
| Kiro CLI | ACP | No live check; protocol fixtures and a fake `kiro-cli` only |
| Antigravity | ACP | Protocol fixtures only; no installed server for a live check |
| Devin | ACP | Catalog, streaming, two-turn resume and supervised write approval (Oct 19) |

Changes to provider lifecycles on Oct 21 still need live checks, especially
steering, compaction, rewind, warm resume, questions and MCP confirmations.
Command discovery before the first turn is incomplete outside Pi/omp.
Provider permissions depend on the CLI: Cursor's default policy did not request
approval for every edit. Native Supervised mode does not add that guarantee.

## Application work

| Area | Remaining verification or work |
| --- | --- |
| Chat | Live attachment/steering/rewind checks, complete keyboard selection and screen-reader navigation, richer nested task artifacts and provider-specific task controls |
| History and workspace | Transcript/database paging, cache runtime checks, remaining desktop layout consumers, explicit import/export |
| Files and Git | Platform visual/IME checks and Linux/Windows media hosting |
| Accounts and settings | Live OAuth/MCP authentication checks and remaining platform-specific settings behavior |
| Inbox | Runtime/visual checks and linked-update notices |
| Notes | Runtime/visual checks |
| Automations | Scheduler, provider and recovery checks; current implementation has source review only |
| Board | Runtime/visual checks in `bun run dev`. Details and known differences are listed under the table in [Board details](#board-details) |
| Pilots | Existing-data checks, HTML accessibility and floating-document interactions |
| Remote connections | Real-machine installation/update, existing-data restoration and cross-platform checks; the existing host's feature limits still apply |
| Integrations | Completion/approval notifications, sounds and Dock badges; Quick Composer attachments/shortcut checks, MCP/skills checks and remaining app-control endpoints. macOS reminder delivery and click routing passed on Oct 22 |

### Board details

Shared by both apps:

- Board switcher, quick edits on the card face (priority, due, labels), column menu, collapsed columns, quick add, Undo toast (8 s) for moves, sorts, archives and deletes, held deletes, Archive view, and Delete column only when the column is empty.
- Card detail drawer with a Full page toggle (Escape or the toggle goes back to the drawer) and an "Open cards in: Drawer / Full page" setting.
- Start and due dates with quick picks (Today, Tomorrow, Next week, In 2 weeks, Clear) and a date input.
- Estimate, entered as `90`, `90m`, `1h`, `1h 30m`, `1.5h`, `.5h` or `0.5h` and shown as `1h 30m`. Both apps use one grammar, rounded to the minute: `1.m`, `0x10`, `1e3`, negative values and zero-length units are refused with "Use minutes or hours, like 90, 90m, 1h 30m or 1.5h", and the typed text stays in the field.
- Relations: Blocked by, Blocks and Relates to, with cycles refused.
- Values for a board's custom fields, which are managed in board settings (see below).
- A links list (session, pilot, PR URL, worktree path).
- A Markdown toolbar (bold, italic, heading, bulleted and numbered list, link, code); the stored text stays plain Markdown.
- Subtask reorder and Convert to card; comment authors shown as You or Agent.
- A More menu with Open, Status…, Priority…, Labels…, Move to board…, Copy link, Copy `#N title`, Duplicate, Archive and Delete. It opens from the drawer's "…", from a card's right-click, and from "More…" in the card's "…" menu.
- A New card modal on ⌘N (Ctrl+N) with Create more and ⌘↵ to submit.
- One query engine (`crates/board-store/src/views.rs`) behind search, filters and sort. Due buckets use each date's own local day, so they stay right across midnight and DST; `failed` follows a card across boards.
- Filter builder: "Where" and "and" rows with is / is not, value chips and Clear all. Rows cover custom fields too; text, number and date fields list only the values cards hold. A deleted label, project or field is dropped from the row and never errors or empties the results.
- Sort by manual, priority, due, start, title, created or updated, ascending or descending; the header reads "Sorted by {key}". While a sort is on, dragging inside a column is off, and moving a card to another column appends it.
- Saved-view tabs: "All cards" comes first and cannot be deleted. Rename, Duplicate (copies unsaved edits) and Delete. A "•" on the tab marks unsaved changes, with Save and Revert. A view changed in the other app re-applies unless this app has local edits.
- List layout grouped by status, priority, label, project or due, with collapsible groups and "No status", "No label", "No project" and "No due date" groups. Collapse keys are `mode:value` and are shared across apps.
- Table layout: a column picker, resizable columns (60 px minimum), and inline edits of title, status, priority, due and labels. A draft title survives refreshes, and pressing Enter without typing does not overwrite a rename from the other app.
- List and table render only the visible rows. The board layout is windowed too: an expanded column mounts only the cards in view plus a 240 px margin, and always keeps the focused, dragged and open-drawer cards mounted. A collapsed column mounts none. A reload that leaves a column's cards unchanged keeps its scroll position.
- Swimlanes are windowed as rows: a lane header, then bands of 4 cards per cell. Lane headers stay mounted and carry a drop target per column, which keyboard drags use to reach another lane. A drop anywhere in a cell's bands appends to that cell. Collapsed columns are thin strips showing the count and taking drops.
- Keyboard focus at scale: cards are Tab stops. ↑/↓ move to the previous or next visible card in the column and ←/→ to the same index in the next column with cards. In swimlanes ↑/↓ also cross into the neighbouring lane. Moving to a card that is not mounted scrolls it into view first.
- Keyboard drags announce "Picked up card #N, {title}. {column}, position i of {total}.", the "is over" line, "Dropped card #N in …" and "Moving card #N was cancelled…". `{total}` is the column's visible count, never the mounted count. The column label reads "{name}, {total} cards".
- Shift ranges, bulk actions and announcements count from the data in display order, not from mounted elements. With swimlanes the order is lane by lane, then column by column, then position.
- Search queries as you type. Filter rows, sort and view changes apply at once.
- Column display changes (collapse, colour, WIP limit) are written as patches, so collapsing a column never resets a colour or WIP limit the other app just set.
- Activity authors read You, Agent or System.
- Bulk select: ⌘-click (Ctrl-click) toggles a card and Shift-click selects a range in on-screen order. The selection holds only visible cards. A floating bar offers Status…, Priority…, Labels…, Archive, Delete and Clear; a bulk delete is a held delete with Undo. Esc clears the selection last, after the drawer and menus.
- Shortcuts N (new card), `/` (search), E (open the focused card) and Delete / Backspace (held delete of the focused card or the selection) work only when not typing and not inside the open drawer.
- Command palette: Create card, Go to board and Switch board….
- Board settings page: name, icon and colour, and default view. Columns have a colour, a WIP limit shown as `{total}/{limit}` (`{shown}/{total}` while filtered), and the toast "Over the WIP limit ({n}/{limit})" on a drop over it. Delete column offers "Move cards to…", and is refused up front while archived cards remain.
- Labels: create, rename (unique ignoring case), recolour, reorder, merge (with confirmation) and delete. Priority levels can be renamed and hidden; a card that already has a hidden level still shows it.
- Custom fields in 7 kinds (text, number, date, select, multi-select, checkbox, url). Renaming an option keeps the values. Deleting a field removes its filter rows and table columns. Filter rows for text, number and date fields offer card values plus free-form; select and multi-select offer the options; url offers values plus free-form.
- Display options: card-face toggles and density, set as the board default; a saved view can override them. Swimlanes group by priority, label, project or a select field. A drop into another lane changes the column, and sets the lane value only for priority and select lanes; those value-setting drops have no Undo. Cards can't be reordered inside a lane cell.
- Templates: Blank, Agent pipeline, Bug triage and Release checklist, plus "Save as template". Built-ins can't be deleted.
- Auto-move rules per event (started, needs input, completed, failed, PR opened, PR merged). Each is default, stay or a chosen column.
- Link status chips on cards (session, Pilot, PR, worktree). They refresh when the board opens and when a linked turn settles, never on a timer.
- Blocked cards show a lock, and "Run with…" asks first. "Run with…" starts a session with the store-built prompt, links it to the card and inherits the card's ticket.
- Agent fields (provider, model, cost, run time) on cards and in the table; cost stays empty.
- Agent write-back posts a comment marked Agent when a linked turn finishes (a plan turn posts its plan), and agent card tools manage subtasks.
- Automation toggles in settings: "Run with…", write-back, and agent may change ticket status. A session's ticket-status permission is its session override, then the strictest linked board, then on; an unreadable setting shows an error state. The linked ticket is read when each call is answered.
- If settings can't be read, the board still loads, the sections show "Couldn't load settings" with Retry, and nothing is saved.

Known behavior:

| Area | Behavior |
|---|---|
| "Open cards in" setting | Board header menu (setting `board-card-open`) |
| Keyboard dragging | Space lifts, arrows move, Space or Enter drops, Escape cancels |
| Collapsed column strip | Stacks the letters one per line (first 18 characters) |
| Copy link | Copies `gespan://board/{board}/card/{card}`; the app does not open these links yet |
| "Running" filter and chip | Counts sessions waiting for input; the chip says waiting |
| Cost | No data source; stays empty |
| "Run with…" | Refuses remote projects ("Run with… works with local projects for now") |
| "Run with…" agents and worktree | Lists agents that aren't installed apart. New boards default to a new worktree, named after the card and linked to it, with a board setting |
| Agent write-back and turn tracking | Local sessions only; remote turns have no ticket bridge or turn tracking |
| Ticket chips on remote sessions | Don't show the ticket's title and status |
| Quick edits on the card face | Not undoable (moves, sorts, archives and deletes are) |
| PR state | Known only for the current branch's PR, otherwise "unknown"; remote workspaces show "unknown" |
| Swimlane card in two lanes | Focus follows the card by id (its first appearance); arrows skip the other copy |
| A reload moves the focused card to another lane | Focus stays on the card |
| Header "Move cards to…" before Delete column | Moves the cards, then deletes the column as two separate writes; not atomic and no Undo |
| Keyboard drag in swimlanes | Steps one lane at a time |
| "New session from ticket…" | Asks for the ticket, then opens a new session linked to it and drafts its key |

## Platform and distribution

macOS is the primary verified preview target. Remaining verification includes
accessibility, IME and scroll profiling, crash/recovery testing and actual
packaged upgrades.

On October 25, a Linux ARM64 source build was launched and exercised in an
Ubuntu 24.04 Noodle VM using Rust 1.98 and a VM-specific graphics workaround.
This covered launch and basic UI interaction, not live provider requests or
installation/upgrades of the downloadable Linux x64 packages. Windows GUI
execution remains unverified.

On Linux under Wayland, HTML panels open in the browser instead of inside the app,
and media hosting still needs platform support. Windows close-to-tray behavior needs verification.
Desktop webview preferences (theme, projects, model defaults, and related
settings) are read from the installed app on Linux as well as macOS. The Windows
WebView2 reader has synthetic fixture coverage on Linux; real Windows profile
compatibility and Windows execution remain unverified. The reader opens only a
temporary copy; copying an actively written profile is best-effort.

CI checks core and the GUI on macOS, Linux and Windows.
