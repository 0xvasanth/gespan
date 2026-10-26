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

const COLLAPSED_CLASS = COLUMN_CLASS.replace("w-[280px]", "w-9");

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
  selectedIds,
  onCardOpen,
  renderColumnHeader,
  renderColumnFooter,
  onExpand,
}: {
  column: BoardColumn;
  cards: BoardCard[];
  isDropTarget: boolean;
  reducedMotion: boolean;
  renderCard: (card: BoardCard) => ReactNode;
  selectedIds?: ReadonlySet<string>;
  onCardOpen?: (card: BoardCard) => void;
  renderColumnHeader: (column: BoardColumn, count: number) => ReactNode;
  renderColumnFooter: (column: BoardColumn) => ReactNode;
  /** Click or Enter on a collapsed column's strip. */
  onExpand?: (column: BoardColumn) => void;
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

  if (column.collapsed) {
    // A 36px strip. Its body stays the droppable, so a card dropped on it
    // (mouse or keyboard) lands at the end; the cards themselves aren't
    // mounted, so none of them can be the drop's target.
    return (
      <section
        ref={setNodeRef}
        aria-label={column.name}
        data-kanban-column={column.id}
        data-column-collapsed=""
        data-over={isDropTarget ? "" : undefined}
        style={{
          transform: CSS.Translate.toString(transform),
          transition: transition ?? undefined,
        }}
        className={COLLAPSED_CLASS}
      >
        <ColumnStripe color={column.color} />
        <div
          ref={body.setNodeRef}
          data-column-drop=""
          className="flex min-h-40 flex-1"
        >
          <button
            type="button"
            aria-label={`Expand ${column.name}`}
            title="Expand column"
            onClick={() => onExpand?.(column)}
            className="flex w-full flex-col items-center gap-2 rounded-lg py-2.5 outline-none hover:bg-content/5 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent/60"
          >
            <span
              data-column-count=""
              className="text-[12px] tabular-nums text-content/45"
            >
              {cards.length}
            </span>
            <span
              data-column-name=""
              className="truncate text-[13px] font-medium text-content [writing-mode:vertical-rl]"
            >
              {column.name}
            </span>
          </button>
        </div>
      </section>
    );
  }

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
      <ColumnStripe color={column.color} />
      <div
        ref={setActivatorNodeRef}
        {...attributes}
        {...listeners}
        aria-roledescription="column"
        aria-label={`Column ${column.name}`}
        className="group/column-header flex h-9 shrink-0 cursor-grab items-center gap-2 rounded-t-lg px-2.5 outline-none has-[[data-wip-over]]:bg-red-500/10 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent/60 data-[dragging]:cursor-grabbing"
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
              selected={selectedIds?.has(card.id)}
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

/** A 2px bar in the column's palette colour, across its top. */
function ColumnStripe({ color }: { color: string | null }) {
  if (!color) return null;
  return (
    <div
      data-column-color={color}
      aria-hidden="true"
      className="mx-px h-0.5 shrink-0 rounded-t-lg"
      style={{ backgroundColor: `#${color}` }}
    />
  );
}
