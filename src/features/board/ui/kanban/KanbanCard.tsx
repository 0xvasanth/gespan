import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import type { ReactNode } from "react";
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
  onOpen,
  reducedMotion,
}: {
  card: BoardCard;
  renderCard: (card: BoardCard) => ReactNode;
  onOpen?: (card: BoardCard) => void;
  reducedMotion: boolean;
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
  return (
    <div
      ref={setNodeRef}
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
      aria-label={`Card #${card.number}, ${card.title}`}
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
