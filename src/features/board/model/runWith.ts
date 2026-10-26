import type { Session } from "../../sessions/model/session";
import { newSession } from "../../sessions/model/session";
import type { TicketRef } from "../../sessions/model/session";
import type { RunRequest } from "../ui/boardFormat";

/** What the store is asked for; nothing here talks to a tracker. */
export type RunIo = {
  /** The prompt the store builds for the card. */
  getRunPrompt: (cardId: string) => Promise<string>;
  /** The ticket the card holds in the store (cached; no network). */
  getCardTicket: (cardId: string) => Promise<TicketRef | null>;
};

/**
 * The session "Run with…" starts and the prompt it sends. The prompt is the
 * store's, untouched: it is never assembled or extended here, and no ticket
 * text is added. The card's cached ticket rides on the session, so a
 * disconnected tracker still starts it (the ticket tools then report "not
 * connected"). A ticket that can't be read starts the session without one.
 */
export async function prepareRun(
  request: RunRequest,
  io: RunIo,
): Promise<{ session: Session; prompt: string }> {
  const [prompt, ticket] = await Promise.all([
    io.getRunPrompt(request.cardId),
    io.getCardTicket(request.cardId).catch(() => null),
  ]);
  const session: Session = {
    ...newSession(request.harness, request.cwd, request.model),
    ...(ticket ? { linkedWorkItem: { kind: "ticket" as const, ticket } } : {}),
    ...(request.worktree
      ? { workspaceMode: "worktree" as const, worktreeBase: "HEAD" }
      : {}),
  };
  return { session, prompt };
}
