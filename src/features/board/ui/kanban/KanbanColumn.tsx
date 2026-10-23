import { useDroppable } from "@dnd-kit/core";
import {
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import type { ReactNode } from "react";
import type { BoardCard, BoardColumn } from "../../model/board";
import { KanbanCard } from "./KanbanCard";
import { columnBodyId, type KanbanDragData } from "./kanbanMove";

// Target tint is layered over the normal fill, so it is a background image.
const COLUMN_CLASS = [
  "flex max-h-full w-[280px] shrink-0 flex-col rounded-lg border border-stroke bg-selection-subtle",
  "[--kanban-tint:color-mix(in_srgb,var(--color-accent)_6%,transparent)]",
  "data-[over]:border-[color-mix(in_srgb,var(--color-accent)_38%,transparent)]",
  "data-[over]:bg-[linear-gradient(var(--kanban-tint),var(--kanban-tint))]",
  "data-[dragging]:z-10 data-[dragging]:opacity-90",
].join(" ");

/**
 * One column: sortable by its header, with a droppable body (so empty columns
 * accept cards) holding a vertical sortable list of cards.
 */
export function KanbanColumn({
  column,
  cards,
  isDropTarget,
  reducedMotion,
  renderCard,
  onCardOpen,
  renderColumnHeader,
  renderColumnFooter,
}: {
  column: BoardColumn;
  cards: BoardCard[];
  isDropTarget: boolean;
  reducedMotion: boolean;
  renderCard: (card: BoardCard) => ReactNode;
  onCardOpen?: (card: BoardCard) => void;
  renderColumnHeader: (column: BoardColumn, count: number) => ReactNode;
  renderColumnFooter: (column: BoardColumn) => ReactNode;
}) {
  const data: KanbanDragData = { kind: "column", columnId: column.id };
  const {
    attributes,
    listeners,
    setNodeRef,
    setActivatorNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({
    id: column.id,
    data,
    transition: reducedMotion ? null : undefined,
  });
  const bodyData: KanbanDragData = { kind: "column-body", columnId: column.id };
  const body = useDroppable({ id: columnBodyId(column.id), data: bodyData });

  return (
    <section
      ref={setNodeRef}
      aria-label={column.name}
      data-kanban-column={column.id}
      data-dragging={isDragging ? "" : undefined}
      data-over={isDropTarget ? "" : undefined}
      style={{
        transform: CSS.Translate.toString(transform),
        transition: transition ?? undefined,
      }}
      className={COLUMN_CLASS}
    >
      <div
        ref={setActivatorNodeRef}
        {...attributes}
        {...listeners}
        aria-roledescription="column"
        aria-label={`Column ${column.name}`}
        className="group/column-header flex h-9 shrink-0 cursor-grab items-center gap-2 rounded-t-lg px-2.5 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent/60 data-[dragging]:cursor-grabbing"
        data-dragging={isDragging ? "" : undefined}
      >
        {renderColumnHeader(column, cards.length)}
      </div>
      <div
        ref={body.setNodeRef}
        className="flex min-h-10 flex-1 flex-col gap-1.5 overflow-y-auto p-1.5"
      >
        <SortableContext
          items={cards.map((card) => card.id)}
          strategy={verticalListSortingStrategy}
        >
          {cards.map((card) => (
            <KanbanCard
              key={card.id}
              card={card}
              renderCard={renderCard}
              onOpen={onCardOpen}
              reducedMotion={reducedMotion}
            />
          ))}
        </SortableContext>
      </div>
      {renderColumnFooter(column)}
    </section>
  );
}
