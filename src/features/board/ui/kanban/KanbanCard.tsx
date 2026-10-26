import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { type ReactNode, useCallback } from "react";
import type { BoardCard } from "../../model/board";
import type { KanbanDragData } from "./kanbanMove";

/**
 * Sortable wrapper for one card; the whole card is the drag handle. While it
 * is the active item its slot collapses to a 2px accent line, so the line
 * marks where the card will land as neighbours slide apart. Enter on the
 * focused card opens it; Space picks it up.
 */
export function KanbanCard({
  card,
  renderCard,
  selected,
  onOpen,
  reducedMotion,
  description,
  measureRef,
}: {
  card: BoardCard;
  renderCard: (card: BoardCard) => ReactNode;
  /** Set while a selection exists: whether this card is in it. */
  selected?: boolean;
  onOpen?: (card: BoardCard) => void;
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
