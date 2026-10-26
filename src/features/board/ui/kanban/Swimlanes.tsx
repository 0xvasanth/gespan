import {
  type Announcements,
  closestCorners,
  DndContext,
  type DragEndEvent,
  DragOverlay,
  type KeyboardCoordinateGetter,
  KeyboardSensor,
  PointerSensor,
  type ScreenReaderInstructions,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import { type ReactNode, useMemo, useState } from "react";
import { KEYBOARD_CODES } from "./keyboardCodes";
import { createPortal } from "react-dom";
import type { BoardCard, BoardColumn } from "../../model/board";
import type { Lane } from "../../model/swimlanes";

type CellData = { laneKey: string; columnId: string };
type CardData = { cardId: string; laneKey: string };

const cellId = (laneKey: string, columnId: string) =>
  `cell:${laneKey}|${columnId}`;

const LANE_INSTRUCTIONS: ScreenReaderInstructions = {
  draggable:
    "To pick up a card, press Space. Use the arrow keys to move it between " +
    "lanes and columns, Space to drop it, or Escape to cancel.",
};

/**
 * Arrow keys step to the neighbouring cell: left and right along the lane,
 * up and down to the next lane in the same column. The dragged card lands
 * at the target cell's top-left.
 */
const moveBetweenCells: KeyboardCoordinateGetter = (
  event,
  { context: { active, droppableRects, droppableContainers, collisionRect } },
) => {
  const step: Record<string, [number, number] | undefined> = {
    ArrowRight: [1, 0],
    ArrowLeft: [-1, 0],
    ArrowDown: [0, 1],
    ArrowUp: [0, -1],
  };
  const dir = step[event.code];
  if (!dir || !active || !collisionRect) return undefined;
  event.preventDefault();
  let best: { x: number; y: number; score: number } | null = null;
  for (const container of droppableContainers.getEnabled()) {
    if (!String(container.id).startsWith("cell:")) continue;
    const rect = droppableRects.get(container.id);
    if (!rect) continue;
    // A cell wholly beyond the card, in the direction of the key and in line with it.
    const overlapsRow =
      rect.top < collisionRect.bottom && rect.bottom > collisionRect.top;
    const overlapsColumn =
      rect.left < collisionRect.right && rect.right > collisionRect.left;
    let gap: number | null = null;
    if (dir[0] > 0 && overlapsRow) gap = rect.left - collisionRect.right;
    else if (dir[0] < 0 && overlapsRow) gap = collisionRect.left - rect.right;
    else if (dir[1] > 0 && overlapsColumn) gap = rect.top - collisionRect.bottom;
    else if (dir[1] < 0 && overlapsColumn) gap = collisionRect.top - rect.bottom;
    if (gap === null || gap < -1) continue;
    if (!best || gap < best.score) {
      best = { x: rect.left + 6, y: rect.top + 6, score: gap };
    }
  }
  return best ? { x: best.x, y: best.y } : undefined;
};

/**
 * The board as rows: one lane per value, each with the board's columns. A
 * drop into a cell reports the card, the lane and the column; the caller sets
 * what the lane means. Cards drag by pointer or by keyboard (Space, arrows).
 */
export function Swimlanes({
  columns,
  cards,
  lanes,
  renderCard,
  renderColumnHeader,
  onDrop,
  onCardOpen,
  selectedIds,
  trailing,
}: {
  columns: readonly BoardColumn[];
  cards: readonly BoardCard[];
  lanes: readonly Lane[];
  renderCard: (card: BoardCard) => ReactNode;
  renderColumnHeader: (column: BoardColumn, count: number) => ReactNode;
  onDrop: (cardId: string, laneKey: string, columnId: string) => void;
  /** Enter on a focused card; Space picks it up. */
  onCardOpen?: (card: BoardCard) => void;
  /** Cards in the bulk selection: each card announces it is pressed. */
  selectedIds?: ReadonlySet<string>;
  trailing?: ReactNode;
}) {
  const ordered = useMemo(
    () => [...columns].sort((a, b) => a.position - b.position),
    [columns],
  );
  const byId = useMemo(
    () => new Map(cards.map((card) => [card.id, card])),
    [cards],
  );
  const [active, setActive] = useState<BoardCard | null>(null);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: moveBetweenCells,
      keyboardCodes: KEYBOARD_CODES,
    }),
  );
  const announcements = useMemo<Announcements>(() => {
    const laneName = (key: string) =>
      lanes.find((lane) => lane.key === key)?.label ?? key;
    const columnName = (id: string) =>
      columns.find((column) => column.id === id)?.name ?? id;
    const card = (data: CardData | undefined) =>
      data ? byId.get(data.cardId) : undefined;
    const name = (c: BoardCard) => `card #${c.number}, ${c.title}`;
    const cell = (data: CellData | undefined) =>
      data && "columnId" in data
        ? `lane ${laneName(data.laneKey)}, column ${columnName(data.columnId)}`
        : null;
    const cardData = (a: { data: { current?: unknown } }) =>
      a.data.current as CardData | undefined;
    const cellData = (o: { data: { current?: unknown } } | null) =>
      o ? (o.data.current as CellData | undefined) : undefined;
    return {
      onDragStart({ active }) {
        const data = cardData(active);
        const c = card(data);
        if (!c || !data) return undefined;
        const from = byId.get(c.id);
        return `Picked up ${name(c)}. Lane ${laneName(data.laneKey)}, column ${columnName(from?.columnId ?? "")}.`;
      },
      onDragOver({ active, over }) {
        const c = card(cardData(active));
        if (!c) return undefined;
        const where = cell(cellData(over));
        return where
          ? `${name(c)[0].toUpperCase()}${name(c).slice(1)} is over ${where}.`
          : `Card #${c.number} is no longer over a lane.`;
      },
      onDragEnd({ active, over }) {
        const c = card(cardData(active));
        if (!c) return undefined;
        const where = cell(cellData(over));
        return where
          ? `Dropped card #${c.number} in ${where}.`
          : `Card #${c.number} was not moved.`;
      },
      onDragCancel({ active }) {
        const data = cardData(active);
        const c = card(data);
        if (!c || !data) return undefined;
        return `Moving card #${c.number} was cancelled. It is back in lane ${laneName(data.laneKey)}, column ${columnName(c.columnId)}.`;
      },
    };
  }, [lanes, columns, byId]);

  function end({ active: dragged, over }: DragEndEvent) {
    setActive(null);
    const from = dragged.data.current as CardData | undefined;
    const to = over?.data.current as CellData | undefined;
    if (from && to && "columnId" in to) onDrop(from.cardId, to.laneKey, to.columnId);
  }

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCorners}
      accessibility={{
        announcements,
        screenReaderInstructions: LANE_INSTRUCTIONS,
      }}
      onDragStart={({ active: dragged }) =>
        setActive(
          byId.get((dragged.data.current as CardData | undefined)?.cardId ?? "") ??
            null,
        )
      }
      onDragEnd={end}
      onDragCancel={() => setActive(null)}
    >
      <div
        data-swimlanes=""
        className="flex h-full min-h-0 flex-col overflow-auto px-4 py-3"
      >
        <div className="flex w-max gap-2 pb-1">
          {ordered.map((column) => (
            <div
              key={column.id}
              data-lane-column-head={column.id}
              className="flex h-9 w-[280px] shrink-0 items-center gap-2 px-2.5"
            >
              {renderColumnHeader(
                column,
                cards.filter((card) => card.columnId === column.id).length,
              )}
            </div>
          ))}
          {trailing}
        </div>
        {lanes.map((lane) => (
          <section
            key={lane.key}
            aria-label={`Lane ${lane.label}`}
            data-lane={lane.key}
            className="flex w-max flex-col border-t border-stroke py-2"
          >
            <h3 className="sticky left-0 pb-1.5 text-[12px] font-medium text-content/70">
              {lane.label}{" "}
              <span className="tabular-nums text-content/40">
                {lane.cardIds.length}
              </span>
            </h3>
            <div className="flex gap-2">
              {ordered.map((column) => (
                <LaneCell
                  key={column.id}
                  lane={lane}
                  column={column}
                  cards={lane.cardIds
                    .map((id) => byId.get(id))
                    .filter(
                      (card): card is BoardCard =>
                        !!card && card.columnId === column.id,
                    )
                    .sort((a, b) => a.position - b.position)}
                  renderCard={renderCard}
                  selectedIds={selectedIds}
                  onCardOpen={active ? undefined : onCardOpen}
                />
              ))}
            </div>
          </section>
        ))}
      </div>
      {createPortal(
        <DragOverlay>{active ? renderCard(active) : null}</DragOverlay>,
        document.body,
      )}
    </DndContext>
  );
}

function LaneCell({
  lane,
  column,
  cards,
  renderCard,
  selectedIds,
  onCardOpen,
}: {
  lane: Lane;
  column: BoardColumn;
  cards: BoardCard[];
  renderCard: (card: BoardCard) => ReactNode;
  selectedIds?: ReadonlySet<string>;
  onCardOpen?: (card: BoardCard) => void;
}) {
  const data: CellData = { laneKey: lane.key, columnId: column.id };
  const { setNodeRef, isOver } = useDroppable({
    id: cellId(lane.key, column.id),
    data,
  });
  return (
    <div
      ref={setNodeRef}
      data-lane-cell={`${lane.key}|${column.id}`}
      data-over={isOver ? "" : undefined}
      className="flex min-h-12 w-[280px] shrink-0 flex-col gap-1.5 rounded-lg border border-stroke bg-selection-subtle p-1.5 data-[over]:border-accent/50"
    >
      {cards.map((card) => (
        <LaneCard
          key={card.id}
          card={card}
          laneKey={lane.key}
          selected={selectedIds?.has(card.id)}
          onOpen={onCardOpen}
        >
          {renderCard(card)}
        </LaneCard>
      ))}
    </div>
  );
}

function LaneCard({
  card,
  laneKey,
  selected,
  onOpen,
  children,
}: {
  card: BoardCard;
  laneKey: string;
  selected?: boolean;
  onOpen?: (card: BoardCard) => void;
  children: ReactNode;
}) {
  const data: CardData = { cardId: card.id, laneKey };
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: `${laneKey}|${card.id}`,
    data,
  });
  return (
    <div
      ref={setNodeRef}
      {...attributes}
      {...listeners}
      onKeyDown={(event) => {
        listeners?.onKeyDown?.(event);
        // Enter on the card itself opens it; on a control inside, the control acts.
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
      data-kanban-card={card.id}
      data-dragging={isDragging ? "" : undefined}
      style={{ opacity: isDragging ? 0.4 : undefined }}
      className="shrink-0 rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
    >
      {children}
    </div>
  );
}
