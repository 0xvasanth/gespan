import {
  cardsForLink,
  getBoard,
  moveCard,
  recordCardEvent,
  type BoardColumn,
  type CardLinkKind,
  type ColumnRole,
  type LinkedWorkEvent,
} from "./board";

/** Forward-only: pick the first column with `role` strictly right of `current`. */
function forwardColumn(
  columns: BoardColumn[],
  current: BoardColumn,
  role: ColumnRole,
): BoardColumn | null {
  return (
    columns
      .filter((column) => column.role === role)
      .sort((a, b) => a.position - b.position)
      .find((column) => column.position > current.position) ?? null
  );
}

/**
 * Where a card should go when its linked work reports `event`, or null to stay.
 * Cards only ever move right: started -> doing, completed -> review (or done
 * when the board has no review column). Failed work never moves a card, and
 * neither does any event for a card already in a done or cancelled column.
 */
export function autoMoveTarget(
  columns: BoardColumn[],
  current: BoardColumn,
  event: LinkedWorkEvent,
): BoardColumn | null {
  if (current.role === "done" || current.role === "cancelled") return null;
  if (event === "started") return forwardColumn(columns, current, "doing");
  if (event === "completed") {
    const hasReview = columns.some((column) => column.role === "review");
    return forwardColumn(columns, current, hasReview ? "review" : "done");
  }
  return null;
}

/**
 * Applies the auto-move rule to every card linked to this work. The event is
 * always recorded on the card: after a move (which logs its own "moved" row)
 * and when the card stays. One failing card does not stop the others;
 * the first error is rethrown after all cards were handled.
 */
export async function onLinkedWork(
  kind: CardLinkKind,
  id: string,
  event: LinkedWorkEvent,
): Promise<void> {
  const cards = await cardsForLink(kind, id);
  const boardColumns = new Map<string, Promise<BoardColumn[]>>();
  const columnsFor = (boardId: string) => {
    let columns = boardColumns.get(boardId);
    if (!columns) {
      columns = getBoard(boardId).then((detail) => detail.columns);
      boardColumns.set(boardId, columns);
    }
    return columns;
  };

  let firstError: unknown;
  let failed = false;
  for (const card of cards) {
    try {
      const columns = await columnsFor(card.boardId);
      const current = columns.find((column) => column.id === card.columnId);
      const target = current ? autoMoveTarget(columns, current, event) : null;
      if (target) {
        await moveCard(card.id, target.id, Number.MAX_SAFE_INTEGER, "system");
      }
      await recordCardEvent(card.id, event);
    } catch (error) {
      if (!failed) {
        failed = true;
        firstError = error;
      }
    }
  }
  if (failed) throw firstError;
}
