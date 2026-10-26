import type { Session } from "../../sessions/model/session";

/**
 * The reply a completed turn writes back to its linked cards: the turn's
 * last non-blank assistant or plan block (a plan turn's plan, or a Codex
 * `<proposed_plan>` lifted out of a build turn's reply, counts as a reply).
 * Reasoning, tool, approval and system text never counts, and only blocks
 * after the turn's user message do (a message queued while the turn ran is
 * skipped). Returned as is: the store trims it.
 */
export function writeBackReply(session: Session | undefined): string {
  if (!session) return "";
  const queued = new Set(
    session.queuedMessages?.map((message) => message.blockId) ?? [],
  );
  for (let index = session.blocks.length - 1; index >= 0; index -= 1) {
    const block = session.blocks[index];
    if (queued.has(block.id)) continue;
    if (block.role === "user") break;
    if (
      (block.role === "assistant" || block.role === "plan") &&
      block.text.trim()
    )
      return block.text;
  }
  return "";
}
