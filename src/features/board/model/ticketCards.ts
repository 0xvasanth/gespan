import { fetchTicket } from "../../sessions/model/ticketLinks";
import type { TicketRef } from "../../sessions/model/session";
import {
  createCardFromTicketRef,
  getBoard,
  getCardTicket,
  retitleCardFromTicket,
  setCardTicket,
  type BoardColumn,
} from "./board";

/** The first To do column, else the first column. */
export function newCardColumn(
  columns: readonly BoardColumn[],
): BoardColumn | undefined {
  const ordered = [...columns].sort((a, b) => a.position - b.position);
  return ordered.find((column) => column.role === "todo") ?? ordered[0];
}

/**
 * Saves a fetched ticket onto the card, unless the card no longer links that
 * ticket: the user unlinked it, or linked another, while the fetch was out.
 * The card's current link is re-read here, since the caller's copy may be
 * stale. While the card's title is still the ticket's key, the fetched title
 * replaces it. Returns whether anything was saved.
 */
export async function applyFetchedTicket(
  card: { id: string; boardId: string },
  requested: TicketRef,
  filled: TicketRef,
): Promise<boolean> {
  const current = await getCardTicket(card.id);
  if (
    !current ||
    current.tracker !== requested.tracker ||
    current.key !== requested.key
  ) {
    return false;
  }
  await setCardTicket(card.id, filled);
  const title = filled.title?.trim();
  if (!title) return true;
  const found = (await getBoard(card.boardId)).cards.find(
    (candidate) => candidate.id === card.id,
  );
  if (found && found.title === requested.key) {
    // The store re-checks the title and the link in the same write, and
    // marks the title as ticket text so the "Run with…" prompt leaves it out.
    await retitleCardFromTicket(card.id, requested.key, title);
  }
  return true;
}

/**
 * Makes a card from a ticket: titled with its key and linked at once, then
 * retitled from the ticket once it is fetched, unless the user has changed
 * the title meanwhile. A failed fetch leaves the key as the title; a later
 * refresh retitles it.
 */
export async function createCardFromTicket(
  column: BoardColumn,
  ticket: TicketRef,
): Promise<string> {
  const card = await createCardFromTicketRef(column.id, ticket);
  let filled: TicketRef;
  try {
    filled = await fetchTicket(ticket);
  } catch {
    return card.id;
  }
  await applyFetchedTicket(card, ticket, filled);
  return card.id;
}
