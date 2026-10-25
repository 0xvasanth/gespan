import { fetchTicket } from "../../sessions/model/ticketLinks";
import type { TicketRef } from "../../sessions/model/session";
import {
  createCard,
  getBoard,
  setCardTicket,
  updateCard,
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
 * Makes a card from a ticket: titled with its key and linked at once, then
 * retitled from the ticket once it is fetched, unless the user has changed
 * the title meanwhile. A failed fetch leaves the key as the title.
 */
export async function createCardFromTicket(
  column: BoardColumn,
  ticket: TicketRef,
): Promise<string> {
  const card = await createCard(column.id, ticket.key);
  await setCardTicket(card.id, ticket);
  let filled: TicketRef;
  try {
    filled = await fetchTicket(ticket);
  } catch {
    return card.id;
  }
  await setCardTicket(card.id, filled);
  const title = filled.title?.trim();
  if (!title) return card.id;
  const current = (await getBoard(card.boardId)).cards.find(
    (candidate) => candidate.id === card.id,
  );
  if (current && current.title === ticket.key) {
    await updateCard(card.id, title, current.body);
  }
  return card.id;
}
