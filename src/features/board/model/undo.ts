/**
 * Undo for board actions, matching the native app. Each undoable action
 * records its inverse before it runs, then shows one toast for 8 seconds with
 * an Undo button. A new action replaces the toast. A delete is held: the card
 * is archived at once and only deleted when its toast goes (expiry, a new
 * action or a board switch). If the board goes away first (the window
 * closes), the timer goes with it and the card stays archived.
 */

/** How long a toast, and so a held delete, lasts. */
export const UNDO_WINDOW_MS = 8000;

/** Where a card was: card, column, index among the column's live cards. */
export type Place = [cardId: string, columnId: string, index: number];

/** The inverse of an undoable action, captured before it ran. */
export type UndoAction =
  | { kind: "move"; cardId: string; columnId: string; index: number }
  | { kind: "moveMany"; items: Place[] }
  | { kind: "archive"; items: Place[] }
  | { kind: "delete"; cardId: string; columnId: string; index: number }
  /** A held delete of several cards (a bulk delete). */
  | { kind: "deleteMany"; items: Place[] }
  | { kind: "sort"; columnId: string; order: string[] };

/** The toast: an action that can be undone, or a message (`action` null). */
export type UndoToast = {
  /** Bumped per toast, so a list can key on it. */
  id: number;
  label: string;
  action: UndoAction | null;
};

export type Undo = {
  /**
   * Shows the toast for an action that just ran; a held delete is committed
   * first. Ignored while disposed.
   */
  push: (label: string, action: UndoAction) => void;
  /**
   * Shows a message with no Undo, for 8 seconds, but only when no toast is
   * showing: it never replaces one, so it never cuts short a held delete.
   * False when it showed nothing.
   */
  notify: (message: string) => boolean;
  /** Closes the toast and hands back its inverse, for the caller to run. */
  undo: () => UndoAction | null;
  /** Closes the toast; a held delete is carried out now. */
  commitPending: () => Promise<void>;
  /**
   * Drops the toast and its timer without committing (the card stays
   * archived) and ignores later pushes until `attach` is called again.
   */
  dispose: () => void;
  /** Accepts pushes again; returns `dispose`. For a mount effect. */
  attach: () => () => void;
  current: () => UndoToast | null;
  /** The card whose delete is being held, if any. */
  pendingDelete: () => string | null;
  /** Every card whose delete is being held: one, several, or none. */
  pendingDeletes: () => string[];
  subscribe: (listener: () => void) => () => void;
};

export function createUndo({
  onCommitDelete,
  timeoutMs = UNDO_WINDOW_MS,
}: {
  onCommitDelete: (cardId: string) => Promise<void> | void;
  timeoutMs?: number;
}): Undo {
  let toast: UndoToast | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let nextId = 0;
  let disposed = false;
  const listeners = new Set<() => void>();
  const emit = () => {
    for (const listener of [...listeners]) listener();
  };

  const clearTimer = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };

  /** The cards a toast's delete holds. */
  const heldBy = (shown: UndoToast | null): string[] => {
    const action = shown?.action;
    if (action?.kind === "delete") return [action.cardId];
    if (action?.kind === "deleteMany") return action.items.map(([id]) => id);
    return [];
  };

  /** Closes the toast; returns the cards of the delete it held, if any. */
  const close = (): string[] => {
    const held = heldBy(toast);
    clearTimer();
    toast = null;
    return held;
  };

  const commitPending = async () => {
    const hadToast = toast !== null;
    const held = close();
    if (hadToast) emit();
    for (const id of held) await onCommitDelete(id);
  };

  const dispose = () => {
    disposed = true;
    const had = toast !== null;
    close();
    if (had) emit();
  };

  const show = (label: string, action: UndoAction | null) => {
    toast = { id: ++nextId, label, action };
    timer = setTimeout(() => {
      timer = null;
      void commitPending();
    }, timeoutMs);
    emit();
  };

  return {
    push: (label, action) => {
      if (disposed) return;
      const held = close();
      for (const id of held) void onCommitDelete(id);
      show(label, action);
    },
    notify: (message) => {
      if (disposed || toast) return false;
      show(message, null);
      return true;
    },
    undo: () => {
      const action = toast?.action ?? null;
      if (!action) return null;
      clearTimer();
      toast = null;
      emit();
      return action;
    },
    commitPending,
    dispose,
    attach: () => {
      disposed = false;
      return dispose;
    },
    current: () => toast,
    pendingDelete: () =>
      toast?.action?.kind === "delete" ? toast.action.cardId : null,
    pendingDeletes: () => heldBy(toast),
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

/** "Moved #N to {column}" for one card, "Moved N cards to {column}" for more. */
export function movedLabel(numbers: readonly number[], column: string): string {
  return numbers.length === 1
    ? `Moved #${numbers[0]} to ${column}`
    : `Moved ${numbers.length} cards to ${column}`;
}

export function archivedLabel(numbers: readonly number[]): string {
  return numbers.length === 1
    ? `Archived #${numbers[0]}`
    : `Archived ${numbers.length} cards`;
}

export function deletedLabel(number: number): string {
  return `Deleted #${number}`;
}

export function deletedManyLabel(numbers: readonly number[]): string {
  return numbers.length === 1
    ? deletedLabel(numbers[0]!)
    : `Deleted ${numbers.length} cards`;
}

export function sortedLabel(column: string): string {
  return `Sorted ${column}`;
}

/** What the toast says when an undo hit a store error. */
export function undoFailure(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  if (text === "Card not found") return "Couldn't undo: the card was deleted";
  if (text === "Column not on this board" || text === "Column not found") {
    return "Couldn't undo: the column was deleted";
  }
  return `Couldn't undo: ${text}`;
}

/** The writes an undo needs, so it can run against the backend or a test. */
export type UndoApi = {
  moveCard: (cardId: string, columnId: string, index: number) => Promise<void>;
  moveCards: (
    cardIds: string[],
    columnId: string,
    index: number,
  ) => Promise<void>;
  restoreCard: (
    cardId: string,
    columnId: string,
    index: number,
  ) => Promise<void>;
  /** The board as it is now: live cards in order, and archived card ids. */
  snapshot: () => Promise<{
    live: readonly { id: string; columnId: string }[];
    archivedIds: readonly string[];
  }>;
};

/**
 * Puts cards back one at a time, in ascending (column, index) order, so each
 * lands at its old index. Every card is tried; the first error is thrown.
 */
async function putBack(
  places: readonly Place[],
  place: (cardId: string, columnId: string, index: number) => Promise<void>,
) {
  const sorted = [...places].sort((a, b) =>
    a[1] === b[1] ? a[2] - b[2] : a[1] < b[1] ? -1 : 1,
  );
  let first: { error: unknown } | null = null;
  for (const [cardId, columnId, index] of sorted) {
    try {
      await place(cardId, columnId, index);
    } catch (error) {
      first ??= { error };
    }
  }
  if (first) throw first.error;
}

/** Runs an inverse. Restores everything it can and throws the first error. */
export async function runUndo(action: UndoAction, api: UndoApi) {
  switch (action.kind) {
    case "move":
      return api.moveCard(action.cardId, action.columnId, action.index);
    case "moveMany":
      return putBack(action.items, api.moveCard);
    case "archive":
      return putBack(action.items, api.restoreCard);
    case "delete":
      return api.restoreCard(action.cardId, action.columnId, action.index);
    case "deleteMany":
      return putBack(action.items, api.restoreCard);
    case "sort": {
      // Cards that left the column since are skipped; a deleted one is reported.
      const { live, archivedIds } = await api.snapshot();
      const still = action.order.filter((id) =>
        live.some(
          (card) => card.id === id && card.columnId === action.columnId,
        ),
      );
      if (still.length) await api.moveCards(still, action.columnId, 0);
      const deleted = action.order.some(
        (id) =>
          !live.some((card) => card.id === id) && !archivedIds.includes(id),
      );
      if (deleted) throw new Error("Card not found");
      return;
    }
  }
}
