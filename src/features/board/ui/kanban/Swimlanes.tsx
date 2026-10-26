import {
  closestCorners,
  DndContext,
  type DragEndEvent,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import { type ReactNode, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import type { BoardCard, BoardColumn } from "../../model/board";
import type { Lane } from "../../model/swimlanes";

type CellData = { laneKey: string; columnId: string };
type CardData = { cardId: string; laneKey: string };

const cellId = (laneKey: string, columnId: string) =>
  `cell:${laneKey}|${columnId}`;

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
  trailing,
}: {
  columns: readonly BoardColumn[];
  cards: readonly BoardCard[];
  lanes: readonly Lane[];
  renderCard: (card: BoardCard) => ReactNode;
  renderColumnHeader: (column: BoardColumn, count: number) => ReactNode;
  onDrop: (cardId: string, laneKey: string, columnId: string) => void;
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
    useSensor(KeyboardSensor),
  );

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
}: {
  lane: Lane;
  column: BoardColumn;
  cards: BoardCard[];
  renderCard: (card: BoardCard) => ReactNode;
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
        <LaneCard key={card.id} card={card} laneKey={lane.key}>
          {renderCard(card)}
        </LaneCard>
      ))}
    </div>
  );
}

function LaneCard({
  card,
  laneKey,
  children,
}: {
  card: BoardCard;
  laneKey: string;
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
      data-kanban-card={card.id}
      {...attributes}
      {...listeners}
      style={{ opacity: isDragging ? 0.4 : undefined }}
    >
      {children}
    </div>
  );
}
