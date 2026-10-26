import type { Session } from "../../sessions/model/session";

/**
 * The reply a completed turn writes back to its linked cards: the turn's
 * plan when it produced one (a plan turn's final block, or a Codex
 * `<proposed_plan>` lifted out of a build turn's reply), else its last
 * non-blank assistant reply. Only assistant and plan blocks count, never
 * reasoning, tool, approval or system text, and only blocks after the
 * turn's user message (a message queued while the turn ran is skipped).
 * Returned as is: the store trims it.
 */
export function writeBackReply(session: Session | undefined): string {
  if (!session) return "";
  const queued = new Set(
    session.queuedMessages?.map((message) => message.blockId) ?? [],
  );
  let reply = "";
  for (let index = session.blocks.length - 1; index >= 0; index -= 1) {
    const block = session.blocks[index];
    if (queued.has(block.id)) continue;
    if (block.role === "user") break;
    if (block.role === "plan" && block.text.trim()) return block.text;
    if (!reply && block.role === "assistant" && block.text.trim())
      reply = block.text;
  }
  return reply;
}
