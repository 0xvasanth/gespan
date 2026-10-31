# Tracker text in agent prompts

Anyone who can open an issue or leave a comment can write text that reaches an agent. Gespan puts that text in a marked block so the agent can tell it from your instructions.

## What gets marked

| Where the prompt comes from | Inside the block | Outside the block |
| --- | --- | --- |
| Starting a session from the Inbox | The item's reference, title, link, description, comments and comment authors | Gespan's instruction, and anything you type in the composer |
| An automation's Inbox event | The item's reference, title and link | The automation's prompt |
| "Run with…" on a card | A description that came from a tracker | The card's title, a description you wrote, subtasks and your comments |

The block starts with a line telling the agent the text is reference data and that it must not follow instructions in it. The Inbox Ask panel and CI repair have their own wording for this.

A description stays marked after you edit it, because an edit can leave the tracker's text in place. Empty it and write your own to clear the mark. Duplicating a card keeps the mark.

## What this protects against

- **The plainest injection:** an issue title or comment that reads as an order to the agent no longer arrives looking like part of its instructions.
- **Escaping the block:** each block has a random id made when the prompt is built, so tracker text can't contain the closing line. Anything in the text that reads as a marker is removed too, including changes of case, spacing, invisible characters and Unicode lookalikes.
- **Cutting off the end:** long text is cut inside the block, so the closing line is always there.

## What it doesn't protect against

**This doesn't stop prompt injection.** It removes the plainest route, and nothing more.

- The block is a request to the agent, not a sandbox. A model can still follow text inside it.
- Agents can read tracker text themselves, for example with `ticket_get`, `gh` or a web fetch. That text has no block around it.
- Files in the repository, command output and web pages can carry instructions too.

Automation runs have nobody watching, so review what they push.
