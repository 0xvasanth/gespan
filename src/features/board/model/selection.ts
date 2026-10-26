/** Bulk selection: the cards picked, in pick order, and where a Shift-click range starts. */
export type Selection = {
  ids: readonly string[];
  anchor: string | null;
};

export const EMPTY_SELECTION: Selection = { ids: [], anchor: null };

/** ⌘/Ctrl-click: flips one card and makes it the range's anchor. */
export function toggleSelect(selection: Selection, id: string): Selection {
  const has = selection.ids.includes(id);
  return {
    ids: has
      ? selection.ids.filter((other) => other !== id)
      : [...selection.ids, id],
    anchor: id,
  };
}

/**
 * Shift-click: the cards from the anchor to `id` in `order` (the order shown
 * now), replacing the selection. With no anchor, or one no longer in the
 * order, only `id` is selected and becomes the anchor.
 */
export function rangeSelect(
  selection: Selection,
  id: string,
  order: readonly string[],
): Selection {
  const to = order.indexOf(id);
  const from = selection.anchor === null ? -1 : order.indexOf(selection.anchor);
  if (to < 0 || from < 0) return { ids: to < 0 ? [] : [id], anchor: id };
  const [low, high] = from < to ? [from, to] : [to, from];
  return { ids: order.slice(low, high + 1), anchor: selection.anchor };
}

/**
 * Keeps only the cards still shown. The anchor goes only when its card is
 * gone from the board (`exists`), so a range can still start from a card a
 * filter has hidden. Returns the same object when nothing changed.
 */
export function pruneSelection(
  selection: Selection,
  shown: ReadonlySet<string>,
  exists: ReadonlySet<string>,
): Selection {
  const ids = selection.ids.filter((id) => shown.has(id));
  const anchor =
    selection.anchor !== null && exists.has(selection.anchor)
      ? selection.anchor
      : null;
  if (ids.length === selection.ids.length && anchor === selection.anchor) {
    return selection;
  }
  return { ids, anchor };
}
