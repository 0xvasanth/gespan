import { useDroppable } from "@dnd-kit/core";
import {
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { type ReactNode, useCallback, useEffect, useMemo } from "react";
import type { BoardCard, BoardColumn } from "../../model/board";
import type { FocusKey } from "../../model/cardFocus";
import {
  CARD_ESTIMATE,
  CARD_GAP,
  CARD_OVERSCAN,
  useMeasuredWindow,
} from "../../model/windowing";
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

const NO_IDS: readonly string[] = [];

/**
 * One column: sortable by its header, with a droppable body (so empty columns
 * accept cards) holding a vertical sortable list of cards. The body is
 * windowed: only the cards in view (plus overscan and the `pinned` ones) are
 * mounted, with spacers standing in for the rest, while the sortable list
 * keeps every card id so drops resolve among all of them.
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
  density = "comfortable",
  pinned = NO_IDS,
  onCardArrow,
  registerScroller,
  total,
}: {
  /**
   * Every card in the column, whatever a search or filter hides; set only
   * while one is active.
   */
  total?: number;
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
  /** Sets the first-guess card height until each card is measured. */
  density?: keyof typeof CARD_ESTIMATE;
  /** Card ids kept mounted wherever they are (focus, a drag, an open drawer). */
  pinned?: readonly string[];
  /** An arrow key on a card (left out while a card is lifted). */
  onCardArrow?: (cardId: string, key: FocusKey) => void;
  /** Hands the board this column's "scroll card `index` into view". */
  registerScroller?: (
    columnId: string,
    scrollToIndex: ((index: number) => void) | null,
  ) => void;
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
  const ids = useMemo(() => cards.map((card) => card.id), [cards]);
  // A collapsed strip mounts no cards, so it has nothing to window.
  const windowed = useMeasuredWindow({
    ids: column.collapsed ? NO_IDS : ids,
    estimate: CARD_ESTIMATE[density],
    gap: CARD_GAP,
    overscan: CARD_OVERSCAN,
    pinned,
  });
  const { scrollToIndex } = windowed;
  useEffect(() => {
    if (!registerScroller) return;
    registerScroller(column.id, scrollToIndex);
    return () => registerScroller(column.id, null);
  }, [registerScroller, column.id, scrollToIndex]);
  const { setNodeRef: setBodyNodeRef } = body;
  const { ref: windowRef } = windowed;
  const bodyRef = useCallback(
    (node: HTMLDivElement | null) => {
      setBodyNodeRef(node);
      windowRef(node);
    },
    [setBodyNodeRef, windowRef],
  );

  if (column.collapsed) {
    const shown = cards.length;
    const narrowed = total !== undefined;
    const all = total ?? shown;
    const label = narrowed
      ? `${column.name}, ${shown} of ${all} ${all === 1 ? "card" : "cards"}`
      : `${column.name}, ${shown} ${shown === 1 ? "card" : "cards"}`;
    // A 36px strip. Its body stays the droppable, so a card dropped on it
    // (mouse or keyboard) lands at the end; the cards themselves aren't
    // mounted, so none of them can be the drop's target.
    return (
      <section
        ref={setNodeRef}
        aria-label={label}
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
              {narrowed ? `${shown}/${all}` : shown}
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
        ref={bodyRef}
        onScroll={windowed.onScroll}
        className="flex min-h-10 flex-1 flex-col gap-1.5 overflow-y-auto p-1.5 [overflow-anchor:none]"
      >
        <SortableContext items={ids} strategy={verticalListSortingStrategy}>
          {windowed.slots.map((slot, i, slots) => {
            if (slot.kind === "gap") {
              const prev = slots[i - 1];
              // Keyed by place, so scrolling resizes a spacer, never remounts it.
              const key =
                i === 0
                  ? "gap:lead"
                  : i === slots.length - 1
                    ? "gap:tail"
                    : `gap:after:${prev?.kind === "item" ? cards[prev.index].id : i}`;
              return (
                <div
                  key={key}
                  aria-hidden="true"
                  data-window-gap=""
                  className="shrink-0"
                  style={{ height: slot.height }}
                />
              );
            }
            const card = cards[slot.index];
            return (
              <KanbanCard
                key={card.id}
                card={card}
                renderCard={renderCard}
                // Pressed only while a selection exists (R82).
                selected={selectedIds?.size ? selectedIds.has(card.id) : undefined}
                onOpen={onCardOpen}
                onArrow={onCardArrow}
                reducedMotion={reducedMotion}
                description={`${slot.index + 1} of ${cards.length} in ${column.name}`}
                measureRef={windowed.measureRef(card.id)}
              />
            );
          })}
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
