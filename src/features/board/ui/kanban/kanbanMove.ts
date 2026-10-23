import type { BoardCard, BoardColumn, BoardDetail } from "../../model/board";

export type DragKind = "card" | "column";
export type DropTarget = { columnId: string; index: number };

/** What the board puts in dnd-kit `data` for each draggable / droppable. */
export type KanbanDragData =
  | { kind: "card"; cardId: string }
  | { kind: "column"; columnId: string }
  | { kind: "column-body"; columnId: string };

/** A droppable under the pointer, reduced to what drop resolution needs. */
export type DropRef = { overId: string; overKind: "card" | "column" };

/** Card place in its column: `index` is 0-based, `total` counts the card. */
export type CardPlace = { column: BoardColumn; index: number; total: number };

/** Droppable id for a column's card list; distinct from the column's own id. */
export function columnBodyId(columnId: string): string {
  return `column-body:${columnId}`;
}

export function sortedColumns(detail: BoardDetail): BoardColumn[] {
  return [...detail.columns].sort((a, b) => a.position - b.position);
}

export function columnCards(
  detail: BoardDetail,
  columnId: string,
): BoardCard[] {
  return detail.cards
    .filter((card) => card.columnId === columnId)
    .sort((a, b) => a.position - b.position);
}

export function placeOfCard(
  detail: BoardDetail,
  cardId: string,
): CardPlace | null {
  const card = detail.cards.find((c) => c.id === cardId);
  const column = card && detail.columns.find((c) => c.id === card.columnId);
  if (!card || !column) return null;
  const cards = columnCards(detail, column.id);
  return {
    column,
    index: cards.findIndex((c) => c.id === cardId),
    total: cards.length,
  };
}

/** Reads the board's drag data off a dnd-kit `active` / `over` entry. */
export function dragDataOf(entry: {
  data: { current?: unknown };
}): KanbanDragData | null {
  const data = entry.data.current as Partial<KanbanDragData> | undefined;
  switch (data?.kind) {
    case "card":
    case "column":
    case "column-body":
      return data as KanbanDragData;
    default:
      return null;
  }
}

/** Maps a dnd-kit `over` entry to a card or a column (header or body). */
export function toDropRef(
  over: { id: string | number; data: { current?: unknown } } | null,
): DropRef | null {
  if (!over) return null;
  const data = dragDataOf(over);
  if (!data) return null;
  if (data.kind === "card")
    return { overId: String(over.id), overKind: "card" };
  return { overId: data.columnId, overKind: "column" };
}

/**
 * Where `cardId` lands when dropped over `overId`. Over a card: that card's
 * index (in its column, as dnd-kit's arrayMove would place it). Over a column:
 * the end of that column. Returns null when the card would stay put.
 */
export function resolveCardDrop(
  detail: BoardDetail,
  cardId: string,
  overId: string,
  overKind: "card" | "column",
): DropTarget | null {
  const card = detail.cards.find((c) => c.id === cardId);
  if (!card) return null;
  let target: DropTarget;
  if (overKind === "card") {
    if (overId === cardId) return null;
    const over = detail.cards.find((c) => c.id === overId);
    if (!over) return null;
    const index = columnCards(detail, over.columnId).findIndex(
      (c) => c.id === overId,
    );
    target = { columnId: over.columnId, index };
  } else {
    if (!detail.columns.some((c) => c.id === overId)) return null;
    const others = columnCards(detail, overId).filter((c) => c.id !== cardId);
    target = { columnId: overId, index: others.length };
  }
  if (target.columnId === card.columnId) {
    const current = columnCards(detail, card.columnId).findIndex(
      (c) => c.id === cardId,
    );
    if (current === target.index) return null;
  }
  return target;
}

function renumber<T extends { position: number }>(
  list: T[],
  into: Map<T, T>,
): void {
  list.forEach((item, i) => {
    into.set(item, item.position === i ? item : { ...item, position: i });
  });
}

/**
 * Moves `cardId` to `to` (index clamped to the column), renumbering the source
 * and target columns 0..n-1. Other cards keep their order and identity.
 */
export function applyCardMove(
  detail: BoardDetail,
  cardId: string,
  to: DropTarget,
): BoardDetail {
  const card = detail.cards.find((c) => c.id === cardId);
  if (!card || !detail.columns.some((c) => c.id === to.columnId)) {
    return detail;
  }
  const source = columnCards(detail, card.columnId).filter(
    (c) => c.id !== cardId,
  );
  const target =
    to.columnId === card.columnId ? source : columnCards(detail, to.columnId);
  const index = Math.min(Math.max(to.index, 0), target.length);
  const moved = { ...card, columnId: to.columnId };
  const next = new Map<BoardCard, BoardCard>();
  renumber([...target.slice(0, index), moved, ...target.slice(index)], next);
  if (to.columnId !== card.columnId) renumber(source, next);
  next.set(card, next.get(moved)!);
  return { ...detail, cards: detail.cards.map((c) => next.get(c) ?? c) };
}

/** Moves a column to `index` (clamped) and renumbers columns 0..n-1. */
export function applyColumnMove(
  detail: BoardDetail,
  columnId: string,
  index: number,
): BoardDetail {
  const columns = sortedColumns(detail);
  const column = columns.find((c) => c.id === columnId);
  if (!column) return detail;
  const rest = columns.filter((c) => c.id !== columnId);
  const at = Math.min(Math.max(index, 0), rest.length);
  const next = new Map<BoardColumn, BoardColumn>();
  renumber([...rest.slice(0, at), column, ...rest.slice(at)], next);
  return { ...detail, columns: detail.columns.map((c) => next.get(c) ?? c) };
}

/** Index a column dragged over `overColumnId` lands at, or null if unchanged. */
export function resolveColumnDrop(
  detail: BoardDetail,
  columnId: string,
  overColumnId: string,
): number | null {
  const ids = sortedColumns(detail).map((c) => c.id);
  const from = ids.indexOf(columnId);
  const to = ids.indexOf(overColumnId);
  if (from < 0 || to < 0 || from === to) return null;
  return to;
}

/**
 * `current` with the final `over` applied: where the card ends up. With no
 * `over` the card stays where the preview has it.
 */
export function landCard(
  current: BoardDetail,
  cardId: string,
  over: DropRef | null,
): BoardDetail {
  const target =
    over && resolveCardDrop(current, cardId, over.overId, over.overKind);
  return target ? applyCardMove(current, cardId, target) : current;
}

/**
 * The move to commit at drag end: `current` is the board as previewed during
 * the drag, `snapshot` the board at drag start. Null when the card ends up
 * exactly where it started.
 */
export function finalCardTarget(
  snapshot: BoardDetail,
  current: BoardDetail,
  cardId: string,
  over: DropRef | null,
): DropTarget | null {
  const from = placeOfCard(snapshot, cardId);
  const to = placeOfCard(landCard(current, cardId, over), cardId);
  if (!from || !to) return null;
  if (from.column.id === to.column.id && from.index === to.index) return null;
  return { columnId: to.column.id, index: to.index };
}
