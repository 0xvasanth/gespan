import { useCallback, useEffect, useRef, useState } from "react";
import {
  deleteArchivedCard,
  listArchivedCards,
  subscribeBoard,
  unarchiveCard,
  type BoardCard,
} from "../model/board";
import { CloseButton } from "./CardDrawerParts";
import { confirmDeleteCard } from "./boardControls";
import { errorText } from "./boardFormat";

const ROW_BUTTON =
  "inline-flex h-[22px] shrink-0 items-center rounded-md px-2 text-[11px] text-content/60 hover:bg-content/10 hover:text-content";

/**
 * The open board's archived cards, newest first, each with Restore (back to
 * the end of its column) and Delete forever. A card whose delete is being
 * held by the undo toast is not listed.
 */
export function ArchiveView({
  boardId,
  columnNames,
  pendingDelete,
  pendingDeletes = [],
  onClose,
  onError,
}: {
  boardId: string;
  columnNames: ReadonlyMap<string, string>;
  /** The card the undo toast is holding a delete for. */
  pendingDelete: string | null;
  /** Cards of a held bulk delete, hidden like `pendingDelete`. */
  pendingDeletes?: readonly string[];
  onClose: () => void;
  onError: (message: string) => void;
}) {
  // Null until the first load answers.
  const [cards, setCards] = useState<BoardCard[] | null>(null);
  const onErrorRef = useRef(onError);
  useEffect(() => {
    onErrorRef.current = onError;
  });
  const request = useRef(0);

  const load = useCallback(() => {
    const ticket = ++request.current;
    listArchivedCards(boardId)
      .then((list) => {
        if (ticket === request.current) setCards(list);
      })
      .catch((error) => {
        if (ticket !== request.current) return;
        onErrorRef.current(`Couldn't load the archive: ${errorText(error)}`);
      });
  }, [boardId]);

  useEffect(() => {
    setCards(null);
    load();
    return subscribeBoard(boardId, load);
  }, [boardId, load]);

  const restore = (card: BoardCard) =>
    void unarchiveCard(card.id).catch((error) =>
      onError(`Couldn't restore card #${card.number}: ${errorText(error)}`),
    );

  const deleteForever = async (card: BoardCard) => {
    if (!(await confirmDeleteCard(card))) return;
    // Only while still archived: a card restored elsewhere meanwhile is left
    // alone, and the reload shows it gone from here.
    deleteArchivedCard(card.id)
      .then(load)
      .catch((error) =>
        onError(`Couldn't delete card #${card.number}: ${errorText(error)}`),
      );
  };

  const held = new Set(pendingDeletes);
  const shown = (cards ?? []).filter(
    (card) => card.id !== pendingDelete && !held.has(card.id),
  );

  return (
    <aside
      data-board-archive
      aria-label="Archive"
      onKeyDown={(event) => {
        if (event.key === "Escape" && !event.nativeEvent.isComposing) {
          event.preventDefault();
          onClose();
        }
      }}
      className="flex min-h-0 w-[320px] shrink-0 flex-col border-l border-stroke bg-background-base/40"
    >
      <div className="flex h-10 shrink-0 items-center gap-2 border-b border-stroke px-3">
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-content">
          Archive
        </span>
        <CloseButton onClick={onClose} />
      </div>
      <ul className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-3">
        {shown.map((card) => (
          <li
            key={card.id}
            data-archived-card={card.id}
            className="flex flex-col gap-1.5 rounded-lg border border-content/10 px-2.5 py-2"
          >
            <span className="line-clamp-2 text-[13px] text-content">
              {card.title}
            </span>
            <div className="flex items-center gap-1">
              <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-content/45">
                #{card.number} · {columnNames.get(card.columnId) ?? ""}
              </span>
              <button
                type="button"
                aria-label={`Restore card #${card.number}`}
                className={ROW_BUTTON}
                onClick={() => restore(card)}
              >
                Restore
              </button>
              <button
                type="button"
                aria-label={`Delete card #${card.number} forever`}
                className={`${ROW_BUTTON} text-red-300/90 hover:bg-red-500/15 hover:text-red-300`}
                onClick={() => void deleteForever(card)}
              >
                Delete forever
              </button>
            </div>
          </li>
        ))}
        {cards && !shown.length ? (
          <li className="flex justify-center py-6 text-[12px] text-content/45">
            No archived cards
          </li>
        ) : null}
      </ul>
    </aside>
  );
}
