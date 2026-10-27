# Board

The board keeps your work in one place: cards you move through columns, with sessions, Pilots, pull requests and tracker tickets linked to them. It is the same in the Tauri app and in Gespan Native. Both read and write the same data, so a change in one shows up in the other. Where the two apps differ, the difference is noted in the text, and the full list is in [Board details](native/parity.md#board-details).

## Boards

Boards are global. A board is not tied to one project. Each card is tagged with the project it belongs to, and you can filter or group by project.

- **Switcher.** The board header lists your boards. Pick one to open it.
- **New board.** Choose "New board", give it a name and pick a template (see [Templates](#templates)). Gespan Native does not offer a template picker on the empty state; use the switcher there.
- **Rename and delete.** "Rename board" and "Delete board" are in the board actions menu (the switcher in Native). Deleting a board asks first, then removes it with its columns, cards and activity. To keep cards, move them first with Move to board… in the card's More menu.
- **Board settings.** Also in the header menu. See [Board settings](#board-settings).
- **Open cards in.** Choose whether a card opens in a side drawer or on a full page. In Native it is in the board header menu. In the Tauri app it is under Settings, General.

## Cards

### Adding cards

- **Quick add.** Use "Add card" at the foot of a column (or the plus in the column header), type a title and press Enter.
- **New card.** Press ⌘N (Ctrl+N), or press N when no field is focused, or use "Create card" in the command palette. Fill in the details, then press ⌘↵. "Create more" keeps the form open for the next card.
- **From a ticket.** "New card from ticket…" creates a card linked to a Jira, Linear or Plane ticket. See [Ticket links](#ticket-links).

### The card face

A card shows its number, title, priority, labels, due date and the link status chips. You can change priority, due date and labels from the card face without opening the card. Quick edits on the face are not undoable. Display options in board settings choose what the face shows.

### The drawer and the full page

Open a card to see all of it in a drawer. The full page toggle expands it, and Escape or the toggle takes you back to the drawer. The card's "…" menu opens the More menu.

- **Properties.** Status, priority, labels, project, start and due dates, estimate and the board's custom fields. Dates have quick picks (Today, Tomorrow, Next week, In 2 weeks, Clear) and a date input. An estimate takes `90`, `90m`, `1h`, `1h 30m` or `1.5h` and is shown as `1h 30m`. Anything else is refused with "Use minutes or hours, like 90, 90m, 1h 30m or 1.5h", and what you typed stays in the field.
- **Description.** Markdown, with a toolbar for bold, italic, heading, bulleted and numbered lists, link and code. What is stored is plain Markdown.
- **Subtasks.** Add, tick off, reorder, and "Convert to card". Tauri also reorders with Alt+↑ and Alt+↓; Native uses Move up and Move down in the menu.
- **Comments.** Each comment shows who wrote it: You or Agent.
- **Relations.** Blocked by, Blocks and Relates to. A relation that would create a cycle is refused.
- **Links.** The card's session, Pilot, pull request URL and worktree path.
- **Activity.** Moves and edits, labelled You, Agent or System.

### More menu

Open, Status…, Priority…, Labels…, Move to board…, Copy link, Copy `#N title`, Duplicate, Archive and Delete. It opens from the drawer's "…", from a card's right-click, and from "More…" in a card's "…" menu. "Copy link" copies a `gespan://board/…` link. Gespan does not open these links yet.

### Archive, delete and Undo

Archive hides a card without losing it. The Archive view lists archived cards and restores them. Delete is held for a few seconds: moves, sorts, archives and deletes show an Undo toast for 8 seconds, and a held delete is only final when it expires. A column can only be deleted when it has no cards, and you can move its cards to another column first.

## Views

A board has three layouts.

- **Board.** Columns of cards. Drag a card to move it, or use the keyboard (see [Keyboard](#keyboard)). Collapse a column to a thin strip. Cards you drop on a collapsed strip go to the end of that column.
- **List.** Rows grouped by status, priority, label, project or due date. Groups collapse, and cards without a value land in "No status", "No label", "No project" or "No due date".
- **Table.** Pick the columns you want and resize them. Edit title, status, priority, due and labels in place.

### Swimlanes

In the board layout, group rows by priority, label, project or a select field. Dropping a card into another lane changes its column, and sets the lane value too for priority and select lanes (those drops have no Undo). You can't reorder cards inside a lane cell; a drop appends.

### Saved views

A view is a layout plus its search, filters, sort and display choices. "All cards" is always first and can't be deleted. You can save, rename, duplicate and delete your own views. A dot on a tab means unsaved changes, with Save and Revert. A view saved in the other app re-applies unless you have unsaved edits here.

## Search, filters and sort

Press `/` to search. The query matches a card's title and description, and `#12` finds card 12. In the Tauri app the search runs 150 ms after you stop typing.

The filter builder uses "Where" and "and" rows with "is" and "is not" and value chips. Rows cover the card's properties, its link and run state (such as running or failed) and custom fields. "Clear all" removes every row. If a label, project or field you filtered on is deleted, its row is dropped and the results still load.

Sort by manual order, priority, due, start, title, created or updated, ascending or descending. The header reads "Sorted by {key}". While a sort is on, a card dragged within a column can't be placed, and a card moved to another column goes to the end.

## Keyboard

| Keys | What happens |
|---|---|
| ↑ / ↓ | Move focus to the previous or next card in the column. In swimlanes they continue into the next lane |
| ← / → | Move to the same position in the next column that has cards |
| Tab | Moves between cards. Tauri cards, and Native board cards, are Tab stops |
| Space | Lifts the focused card, then drops it |
| Arrows while lifted | Move the card between columns and positions |
| Enter | Drops a lifted card, or opens the focused card |
| Escape | Cancels a lift. Otherwise it closes, in order, menus, the open drawer, then clears a selection last |
| N | New card, when no field is focused |
| `/` | Focus search |
| E | Open the focused card |
| Delete / Backspace | Delete the focused card, or the selection (a held delete with Undo) |
| ⌘-click / Ctrl-click | Add or remove a card from the selection |
| Shift-click | Select a range, in on-screen order |

Single-letter shortcuts do nothing while you type in a field and do nothing inside the open drawer. With cards selected, a bar offers Status…, Priority…, Labels…, Archive, Delete and Clear. The command palette has Create card, Go to board, Switch board… and "New session from ticket…". In Native it also has "New card from ticket…" and "Link ticket…".

Keyboard drags announce each step to screen readers, for example "Picked up card #12, Fix login. Todo, position 3 of 40." and "Dropped card #12 in Done, position 1 of 8."

## Board settings

Open "Board settings" from the board header menu.

- **General.** Name, icon, colour and the default view.
- **Columns.** Name, colour and an optional WIP limit. A column shows `{total}/{limit}`, or `{shown}/{total}` while filtered. A drop over the limit works and shows "Over the WIP limit ({n}/{limit})". Deleting a column offers "Move cards to…", and is refused while archived cards remain in it.
- **Labels.** Create, rename (names are unique ignoring case), recolour, reorder, merge and delete.
- **Priority.** Rename or hide levels. A card that already has a hidden level still shows it.
- **Fields.** Custom fields of seven kinds: text, number, date, select, multi-select, checkbox and url. Renaming a select option keeps the values. Deleting a field removes its filter rows and table columns.
- **Display.** What the card face shows, and density. This is the board default, and a saved view can override it.
- **Automation.** "Run with…", "Agent write-back", "Agent may change ticket status", and the auto-move rules. See [Agent work](#agent-work).
- **Templates.** See [Templates](#templates).

If the settings can't be read, the board still opens, the sections say "Couldn't load settings" with Retry, and nothing is saved until they load.

## Templates

New boards start from a template: Blank, Agent pipeline, Bug triage or Release checklist. Use "Save as template" in board settings to keep a board's columns and settings for later. Built-in templates can't be deleted.

## Agent work

### Run with…

"Run with…" is on a card's drawer and menus. Pick a provider, a model and where to work, and Gespan starts a session from the card, links the two and gives the session a prompt built from the card. If the card has a ticket, the session inherits it. Turn it off per board under Automation. It works with local projects only; for a remote project it says "Run with… works with local projects for now". A card blocked by another unfinished card shows a lock, and "Run with…" asks before starting.

### Link status chips

Cards show chips for the linked session, Pilot, pull request and worktree. They refresh when the board opens and when a linked turn finishes, never on a timer. A pull request's state is only known for the branch's current pull request. Otherwise the chip says "unknown". The "Running" filter and chip count sessions that are waiting for input too.

### Write-back

When a linked turn finishes, Gespan can post a comment on the card marked Agent. A plan turn posts its plan. Write-back covers local sessions only, in both apps.

### Auto-move rules

Each event can move the card: session started, needs input, completed, failed, pull request opened and pull request merged. For every event choose Default, Stay, or a column.

### Agent fields

Cards and the table can show the provider, model and run time of the linked session. Cost is not recorded; the column stays empty.

### Agent card tools

A session linked to a card can list the card's subtasks (`card_subtasks`) and tick one off (`card_subtask_done`). A read-only session can list subtasks but not tick them.

## Ticket links

A card, or a session, can link to one ticket in Jira, Linear or Plane. The link is one way: Gespan reads the ticket's key, title and status. Board edits are never copied to the tracker. Only an agent's ticket tools write there (see Agents and tickets).

Connect a tracker first in Settings, Inbox. [Jira setup](jira.md) shows how for Jira; Linear and Plane connect from the same Settings, Inbox page. Gespan keeps your tracker tokens in the system keychain.

- **Link.** "Link ticket…" in the card drawer, or on a session's ticket chip. Paste a key or a URL. A URL for a different site than the one you connected is refused. A saved `http://` site and a pasted `https://` URL count as different sites.
- **Chip.** Shows the tracker, key, title and status, and opens the ticket in the tracker. A chip that says the tracker is disconnected means its token is missing; reconnect in Settings.
- **Refresh and unlink.** The chip menu has Open, Refresh and Unlink. Tickets are read when linked, when a card opens and when you refresh, never on a timer.
- **New card from ticket…** is a button in the board header in the Tauri app and a command-palette entry in Native. It makes a card in the first To do column (or the first column) and fills in the title.
- **New session from ticket…** is in the command palette. It starts a local session in the current project, links the ticket and puts the ticket key, then its title, in the message box as a draft. Nothing is sent until you send it. In the Tauri app a blank session saves its ticket link with its first message.

### Agents and tickets

A session with a linked ticket gives its agent three tools: `ticket_get`, `ticket_comment` and `ticket_status`. They only act on that session's linked ticket and refuse any other key. Read-only and plan sessions can read but can't comment or change status.

Status changes are separate. "Agent may change ticket status" decides whether `ticket_status` may move the ticket. It is set per board in Automation, and per session from the chip menu (Board default, On or Off). The session setting wins, then the strictest linked board, then On.

Ticket titles are not put into the prompts Gespan sends automatically. They carry the key only, because titles and descriptions are text anyone can write in a tracker.

## Large boards

Boards are built and tested to stay responsive with 1,000 cards. Each expanded column draws only the cards you can see, with a few extra above and below. Search, filters and sort run in one shared query engine over a single board-wide read, never one read per card. Focus, drags, selections and screen-reader positions still count from the whole board. A card you focus, drag or have open is never taken off the screen. Reloads from the other app keep your place in a scrolled column.

## Known limits

- Cost is not recorded.
- "Run with…" and write-back are for local projects and sessions only.
- Pull request state is known for the current branch's pull request only.
- Cards can't be reordered inside a swimlane cell.
- In Native, list and table rows don't take keyboard focus. Tauri rows do.
- In Native, a card shown in two lanes is focused by its id, so the arrows skip its other copy.
- `gespan://` links are copied but not opened yet.
- Remote sessions don't show a ticket's title and status.
- Two apps editing the same display settings or saved-view layout at the same moment can lose one edit.
- A Linear URL from another workspace links the ticket with the same key in the connected workspace.
- The Archive panel is not windowed.

## Security

- Tracker tokens stay in the system keychain. They are never in the board, in prompts or in the agent's environment.
- An agent only reaches the ticket linked to its own session, with a token that belongs to that session alone.
- Read-only and plan sessions can read tickets and cards but can't write to them (Gespan's write-back may still post a plan turn's plan as a card comment).
- Ticket titles and descriptions are never added to automatic prompts.

## Upgrading

Both apps share one database, so a newer build can leave data an older build doesn't know about.

- **Ticket links on sessions.** A build from before ticket links doesn't know a session's linked ticket. It drops the link the next time it saves that session. If you switch between builds, use the newer one for sessions you've linked, and re-link a ticket if it disappears.
- **Board data from before version 3.** Anything an older (pre-v3) build writes to its old board tables after the upgrade is not carried over.
- **Unlinking in an older build.** While an older build is still in use, unlinking a card there doesn't remove an agent's access granted through the newer build's link.

Opening the board in a newer build upgrades the board data in place. Keep a copy of `gespan.db` first if you plan to go back to an older build.
