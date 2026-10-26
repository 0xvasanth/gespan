import type {
  Active,
  Announcements,
  Over,
  ScreenReaderInstructions,
} from "@dnd-kit/core";
import type { BoardCard, BoardColumn, BoardDetail } from "../../model/board";
import {
  type CardPlace,
  dragDataOf,
  landCard,
  placeOfCard,
  resolveColumnDrop,
  sortedColumns,
  toDropRef,
} from "./kanbanMove";

export const kanbanScreenReaderInstructions: ScreenReaderInstructions = {
  draggable:
    "To pick up a card or column, press Space. Use the arrow keys to move it, " +
    "Space to drop it, or Escape to cancel.",
};

function where(place: CardPlace): string {
  return `${place.column.name}, position ${place.index + 1} of ${place.total}`;
}

export function cardPickedUp(card: BoardCard, place: CardPlace): string {
  return `Picked up card #${card.number}, ${card.title}. ${where(place)}.`;
}

export function cardOver(card: BoardCard, place: CardPlace): string {
  return `Card #${card.number} is over ${where(place)}.`;
}

export function cardLeft(card: BoardCard): string {
  return `Card #${card.number} is no longer over a column.`;
}

export function cardDropped(card: BoardCard, place: CardPlace): string {
  return `Dropped card #${card.number} in ${where(place)}.`;
}

export function cardCancelled(card: BoardCard, place: CardPlace): string {
  return `Moving card #${card.number} was cancelled. It is back in ${where(place)}.`;
}

type ColumnPlace = { index: number; total: number };

function at(place: ColumnPlace): string {
  return `position ${place.index + 1} of ${place.total}`;
}

export function columnPickedUp(column: BoardColumn, place: ColumnPlace) {
  return `Picked up column ${column.name}, ${at(place)}.`;
}

export function columnOver(column: BoardColumn, place: ColumnPlace) {
  return `Column ${column.name} is over ${at(place)}.`;
}

export function columnLeft(column: BoardColumn) {
  return `Column ${column.name} is no longer over a drop position.`;
}

export function columnDropped(column: BoardColumn, place: ColumnPlace) {
  return `Dropped column ${column.name} at ${at(place)}.`;
}

export function columnCancelled(column: BoardColumn, place: ColumnPlace) {
  return `Moving column ${column.name} was cancelled. It is back at ${at(place)}.`;
}

export type KanbanAnnouncementSource = {
  /** The board as rendered right now (including any drag preview). */
  current: () => BoardDetail;
  /** The board when the drag started; null before the first drag. */
  snapshot: () => BoardDetail | null;
  /**
   * The cards follow a sort: a drop inside the column the card came from is
   * cancelled, and a drop into another column goes to the end of it.
   */
  sorted?: () => boolean;
};

/**
 * dnd-kit announcements for the board. Reads the board lazily so the object
 * can be created once and still describe the latest preview.
 */
export function kanbanAnnouncements(
  source: KanbanAnnouncementSource,
): Announcements {
  const startBoard = () => source.snapshot() ?? source.current();

  function cardAt(detail: BoardDetail, active: Active) {
    const data = dragDataOf(active);
    if (data?.kind !== "card") return null;
    const card = detail.cards.find((c) => c.id === data.cardId);
    const place = placeOfCard(detail, data.cardId);
    return card && place ? { card, place } : null;
  }

  function cardLanding(active: Active, over: Over | null) {
    const data = dragDataOf(active);
    if (data?.kind !== "card") return null;
    return cardAt(
      landCard(source.current(), data.cardId, toDropRef(over)),
      active,
    );
  }

  function columnAt(detail: BoardDetail, active: Active) {
    const data = dragDataOf(active);
    if (data?.kind !== "column") return null;
    const columns = sortedColumns(detail);
    const index = columns.findIndex((c) => c.id === data.columnId);
    if (index < 0) return null;
    return { column: columns[index], place: { index, total: columns.length } };
  }

  function columnLanding(active: Active, over: Over | null) {
    const found = columnAt(source.current(), active);
    const ref = toDropRef(over);
    if (!found || !ref) return null;
    const index =
      resolveColumnDrop(source.current(), found.column.id, ref.overId) ??
      found.place.index;
    return { column: found.column, place: { ...found.place, index } };
  }

  return {
    onDragStart({ active }) {
      const card = cardAt(startBoard(), active);
      if (card) return cardPickedUp(card.card, card.place);
      const column = columnAt(startBoard(), active);
      if (column) return columnPickedUp(column.column, column.place);
      return undefined;
    },
    onDragOver({ active, over }) {
      const landed = cardLanding(active, over);
      if (landed)
        return over
          ? cardOver(landed.card, landed.place)
          : cardLeft(landed.card);
      const column = columnLanding(active, over);
      if (column) return columnOver(column.column, column.place);
      const leaving = columnAt(source.current(), active);
      return leaving ? columnLeft(leaving.column) : undefined;
    },
    onDragEnd({ active, over }) {
      const landed = cardLanding(active, over);
      if (landed && source.sorted?.()) {
        const start = cardAt(startBoard(), active);
        if (start && start.card.columnId === landed.card.columnId) {
          return cardCancelled(start.card, start.place);
        }
        return cardDropped(landed.card, {
          ...landed.place,
          index: landed.place.total - 1,
        });
      }
      if (landed) return cardDropped(landed.card, landed.place);
      const column =
        columnLanding(active, over) ?? columnAt(startBoard(), active);
      return column ? columnDropped(column.column, column.place) : undefined;
    },
    onDragCancel({ active }) {
      const card = cardAt(startBoard(), active);
      if (card) return cardCancelled(card.card, card.place);
      const column = columnAt(startBoard(), active);
      return column ? columnCancelled(column.column, column.place) : undefined;
    },
  };
}
