# Native preview limitations

The native app is still experimental. A feature being present in source does not
establish runtime or platform parity with the production React/Tauri app. See
[verification](verification.md) for recorded checks.

## Providers

All eleven providers have discovery, model selection and native transports.
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
| Files and Git | Platform visual/IME checks, Linux/Windows media hosting, non-Git checkpoints, per-file checkpoint Undo/Keep and orchestration checkpoint integration |
| Accounts and settings | Live OAuth/MCP authentication checks and remaining platform-specific settings behavior |
| Inbox | Runtime/visual checks, inline video, linked-update notices and richer CI evidence/repair progress |
| Notes | Runtime/visual checks and composer note-card presentation |
| Automations | Scheduler, provider and recovery checks; current implementation has source review only |
| Board | Runtime/visual checks in `bun run native:dev` and the Tauri app (see [verification](verification.md#board--october-24-2026)). Both apps read and write the same board tables, so boards, cards and everything on them show in both. Details and known differences are listed under the table in [Board details](#board-details) |
| Pilots | Existing-data checks, HTML accessibility, floating-document interactions and durable orchestration recovery |
| Remote connections | Real-machine installation/update, existing-data restoration and cross-platform checks; the existing host's feature limits still apply |
| Integrations | Completion/approval notifications, sounds and Dock badges; Quick Composer attachments/shortcut checks, MCP/skills checks and remaining app-control endpoints. macOS reminder delivery and click routing passed on Oct 22 |
| Shared data | Concurrent Tauri/native use with active scheduled tasks or phone connections remains unverified and deferred |

### Board details

Shared by both apps:

- Board switcher, quick edits on the card face (priority, due, labels), column menu, collapsed columns, quick add, Undo toast (8 s) for moves, sorts, archives and deletes, held deletes, Archive view, and Delete column only when the column is empty.
- Card detail drawer with a Full page toggle (Escape or the toggle goes back to the drawer) and an "Open cards in: Drawer / Full page" setting.
- Start and due dates with quick picks (Today, Tomorrow, Next week, In 2 weeks, Clear) and a date input.
- Estimate, entered as `90`, `90m`, `1h`, `1h 30m` or `1.5h` and shown as `1h 30m`.
- Relations: Blocked by, Blocks and Relates to, with cycles refused.
- Values for a board's existing custom fields (text, number, date, select, checkbox). Managing the fields themselves is not built yet.
- A links list (session, pilot, PR URL, worktree path).
- A Markdown toolbar (bold, italic, heading, bulleted and numbered list, link, code); the stored text stays plain Markdown.
- Subtask reorder and Convert to card; comment authors shown as You or Agent.
- A More menu with Open, Status…, Priority…, Labels…, Move to board…, Copy link, Copy `#N title`, Duplicate, Archive and Delete. It opens from the drawer's "…", from a card's right-click, and from "More…" in the card's "…" menu.
- A New card modal on ⌘N (Ctrl+N) with Create more and ⌘↵ to submit.

Known differences:

| Area | Native | Tauri |
|---|---|---|
| "Open cards in" setting | Board header menu (setting `board-card-open`) | Settings → General (flag `gespan.boardCardOpen`) |
| Subtask reorder | Menu only (Move up, Move down) | Menu, plus Alt+↑ and Alt+↓ |
| Keyboard dragging | Space lifts, arrows move, Space or Enter drops, Escape cancels | Same keys, through dnd-kit |
| Collapsed column strip | Stacks the letters one per line (first 18 characters) | Rotates the text (vertical writing mode) |
| "Pick a date…" on a card face | Opens the drawer's date picker | Shows a date field in the menu |
| Copy link | Copies `gespan://board/{board}/card/{card}`; the app does not open these links yet | Same; not opened yet |

## Platform and distribution

macOS is the primary verified preview target. Remaining verification includes
accessibility, IME and scroll profiling, crash/recovery testing and actual
packaged upgrades.

Linux and Windows GUI builds and interaction have not been verified. In
particular, HTML panels currently use a text fallback, native notification
delivery is unavailable, and media hosting still needs platform support.
Windows close-to-tray behavior also needs implementation/verification.
Desktop webview preferences (theme, projects, model defaults, and related
settings) are read from the installed app on Linux as well as macOS. The Windows
WebView2 reader has synthetic fixture coverage on Linux; real Windows profile
compatibility and Windows execution remain unverified. The reader opens only a
temporary copy; copying an actively written profile is best-effort.

The preview workflow checks core on macOS, Linux and Windows, but checks/builds
the GUI only on macOS.
