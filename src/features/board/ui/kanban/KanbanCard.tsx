import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { type ReactNode, useCallback } from "react";
import type { BoardCard } from "../../model/board";
import { type FocusKey, isFocusKey } from "../../model/cardFocus";
import type { KanbanDragData } from "./kanbanMove";

/**
 * Sortable wrapper for one card; the whole card is the drag handle. While it
 * is the active item its slot collapses to a 2px accent line, so the line
 * marks where the card will land as neighbours slide apart. Enter on the
 * focused card opens it; Space picks it up; the arrow keys move focus to a
 * neighbouring card (`onArrow`, left out while a card is lifted, when the
 * arrows belong to the drag).
 */
export function KanbanCard({
  card,
  renderCard,
  selected,
  onOpen,
  onArrow,
  reducedMotion,
  description,
  measureRef,
}: {
  card: BoardCard;
  renderCard: (card: BoardCard) => ReactNode;
  /** Set while a selection exists: whether this card is in it. */
  selected?: boolean;
  onOpen?: (card: BoardCard) => void;
  /** An unmodified arrow key on the card itself: move focus that way. */
  onArrow?: (cardId: string, key: FocusKey) => void;
  reducedMotion: boolean;
  /** "{i} of {total} in {column}", counted over the column's visible cards. */
  description?: string;
  /** The column's window measures the card's height through this. */
  measureRef?: (node: HTMLElement | null) => void;
}) {
  const data: KanbanDragData = { kind: "card", cardId: card.id };
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({
    id: card.id,
    data,
    transition: reducedMotion ? null : undefined,
  });
  const ref = useCallback(
    (node: HTMLElement | null) => {
      setNodeRef(node);
      measureRef?.(node);
    },
    [setNodeRef, measureRef],
  );
  return (
    <div
      ref={ref}
      {...attributes}
      {...listeners}
      onKeyDown={(event) => {
        listeners?.onKeyDown?.(event);
        if (
          onOpen &&
          event.key === "Enter" &&
          event.target === event.currentTarget &&
          !event.defaultPrevented &&
          !event.nativeEvent.isComposing
        ) {
          event.preventDefault();
          onOpen(card);
          return;
        }
        if (
          onArrow &&
          isFocusKey(event.key) &&
          event.target === event.currentTarget &&
          !event.defaultPrevented &&
          !event.nativeEvent.isComposing &&
          !event.altKey &&
          !event.ctrlKey &&
          !event.metaKey &&
          !event.shiftKey
        ) {
          event.preventDefault();
          onArrow(card.id, event.key);
        }
      }}
      aria-roledescription="card"
      aria-pressed={selected}
      aria-label={`Card #${card.number}, ${card.title}`}
      aria-description={description}
      data-kanban-card={card.id}
      data-dragging={isDragging ? "" : undefined}
      style={{
        transform: CSS.Translate.toString(transform),
        transition: transition ?? undefined,
      }}
      className={
        isDragging
          ? "h-0.5 shrink-0 rounded-[2px] bg-accent outline-none"
          : "shrink-0 rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
      }
    >
      {isDragging ? null : renderCard(card)}
    </div>
  );
}
