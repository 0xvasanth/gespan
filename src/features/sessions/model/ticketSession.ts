import type { LinkedWorkItem, TicketRef } from "./session";
import { refreshLinkedTicket } from "./ticketLinks";

/** The command palette's "New session from ticket…". */
export const NEW_SESSION_FROM_TICKET_ACTION = {
  id: "session-from-ticket",
  label: "New session from ticket…",
} as const;

/** What the app provides to start a session from a ticket. */
export type TicketSessionHost = {
  /**
   * Open a new local session in the active project with `draft` in its
   * composer, and return its id. Nothing is sent.
   */
  createSession: (draft: string) => string;
  /** Save the session's link (the app saves it with `setSessionLinkedWorkItem`). */
  link: (sessionId: string, item: LinkedWorkItem) => void;
  /** The session's link right now. */
  linkedWorkItem: (sessionId: string) => LinkedWorkItem | undefined;
  /** The session's composer text right now. */
  draft: (sessionId: string) => string | undefined;
  /** Replace the session's composer text. Never sends it. */
  setDraft: (sessionId: string, text: string) => void;
};

/**
 * Start a local session linked to `ticket`, with the key as its draft. Once
 * the ticket is fetched, its title replaces the draft unless the user has
 * edited it. A failed fetch keeps the key. Nothing is sent: the user reads
 * and edits the draft first. Resolves with the session id once the fetch
 * has settled; the session itself opens at once.
 */
export async function startSessionFromTicket(
  ticket: TicketRef,
  host: TicketSessionHost,
): Promise<string> {
  const key = ticket.key;
  const id = host.createSession(key);
  host.link(id, { kind: "ticket", ticket });
  let filled: TicketRef;
  try {
    filled = await refreshLinkedTicket({
      ticket,
      current: () => host.linkedWorkItem(id),
      save: (item) => host.link(id, item),
    });
  } catch {
    return id;
  }
  const title = filled.title?.trim();
  const linked = host.linkedWorkItem(id);
  const stillLinked =
    linked?.kind === "ticket" &&
    linked.ticket.tracker === ticket.tracker &&
    linked.ticket.key === key;
  if (title && stillLinked && (host.draft(id) ?? key) === key) {
    host.setDraft(id, title);
  }
  return id;
}
