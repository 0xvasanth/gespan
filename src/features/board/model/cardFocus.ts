/** Arrow-key focus between board cards, decided from the data (never the mounted cards). */

export type FocusKey = "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight";

const FOCUS_KEYS: ReadonlySet<string> = new Set([
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
]);

export function isFocusKey(key: string): key is FocusKey {
  return FOCUS_KEYS.has(key);
}

/**
 * The card an arrow key moves focus to from `from` (the Native C1-5 rule):
 * ↑/↓ the previous or next card in the column, ←/→ the same index in the
 * nearest column that has cards, clamped to its last card. Columns with no
 * visible cards (empty, or collapsed and passed empty) are skipped. Null when
 * there is nowhere to go, or `from` isn't on the board.
 */
export function nextCardFocus(
  columns: readonly { id: string; cardIds: readonly string[] }[],
  from: string,
  key: FocusKey,
): string | null {
  let at = -1;
  let index = -1;
  for (let c = 0; c < columns.length && at < 0; c++) {
    const found = columns[c].cardIds.indexOf(from);
    if (found >= 0) {
      at = c;
      index = found;
    }
  }
  if (at < 0) return null;
  const here = columns[at].cardIds;
  if (key === "ArrowUp") return here[index - 1] ?? null;
  if (key === "ArrowDown") return here[index + 1] ?? null;
  const step = key === "ArrowLeft" ? -1 : 1;
  for (let c = at + step; c >= 0 && c < columns.length; c += step) {
    const cards = columns[c].cardIds;
    if (cards.length) return cards[Math.min(index, cards.length - 1)];
  }
  return null;
}

/** A card in one lane: a card with several labels sits in several lanes. */
export type LaneFocus = { lane: string; cardId: string };

const SEP = "\u0000";

/**
 * The card an arrow key moves focus to in swimlanes, from the card `from`
 * in its lane. ↑/↓ go to the previous or next card in the cell, crossing
 * into the nearest lane above or below that has cards in the same column.
 * ←/→ stay in the lane: the same index in the nearest column with cards,
 * clamped (the Native C1-5 rule). Collapsed columns are passed empty.
 */
export function nextLaneFocus(
  lanes: readonly {
    key: string;
    columns: readonly { id: string; cardIds: readonly string[] }[];
  }[],
  from: LaneFocus,
  key: FocusKey,
): LaneFocus | null {
  const lane = lanes.find((l) => l.key === from.lane);
  if (!lane) return null;
  if (key === "ArrowLeft" || key === "ArrowRight") {
    const id = nextCardFocus(lane.columns, from.cardId, key);
    return id ? { lane: lane.key, cardId: id } : null;
  }
  // Each column read top to bottom through every lane.
  const columns = lane.columns.map((column) => ({
    id: column.id,
    cardIds: lanes.flatMap(
      (l) =>
        l.columns
          .find((c) => c.id === column.id)
          ?.cardIds.map((id) => `${l.key}${SEP}${id}`) ?? [],
    ),
  }));
  const next = nextCardFocus(columns, `${from.lane}${SEP}${from.cardId}`, key);
  if (!next) return null;
  const cut = next.indexOf(SEP);
  return { lane: next.slice(0, cut), cardId: next.slice(cut + 1) };
}
