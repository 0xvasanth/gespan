import type { BoardCard, CardExtras } from "./board";

export type SortKey = "priority" | "due";

/**
 * The ids of `cards` (a whole column, in order) sorted once by `key`:
 * priority puts Urgent first, due puts the soonest first and undated last.
 * Ties keep their current order.
 */
export function sortedCardIds(
  cards: readonly BoardCard[],
  extras: Readonly<Record<string, CardExtras | undefined>>,
  key: SortKey,
): string[] {
  const rank = (card: BoardCard) =>
    key === "priority"
      ? -(extras[card.id]?.priority ?? 0)
      : (extras[card.id]?.dueAt ?? Number.POSITIVE_INFINITY);
  return cards
    .map((card, index) => ({ id: card.id, rank: rank(card), index }))
    .sort((a, b) =>
      a.rank === b.rank ? a.index - b.index : a.rank < b.rank ? -1 : 1,
    )
    .map((entry) => entry.id);
}
