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
