import {
  closestCenter,
  closestCorners,
  type CollisionDetection,
  DndContext,
  type DragEndEvent,
  type DragOverEvent,
  DragOverlay,
  type DragStartEvent,
  KeyboardSensor,
  MeasuringStrategy,
  type Modifier,
  PointerSensor,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import {
  horizontalListSortingStrategy,
  SortableContext,
  sortableKeyboardCoordinates,
} from "@dnd-kit/sortable";
import {
  type ReactNode,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import type { BoardCard, BoardColumn, BoardDetail } from "../../model/board";
import {
  kanbanAnnouncements,
  kanbanScreenReaderInstructions,
} from "./announcements";
import { KanbanColumn } from "./KanbanColumn";
import {
  applyCardMove,
  applyColumnMove,
  columnCards,
  type DragKind,
  type DropTarget,
  dragDataOf,
  finalCardTarget,
  resolveCardDrop,
  resolveColumnDrop,
  sortedColumns,
  toDropRef,
} from "./kanbanMove";
import { useReducedMotion } from "./useReducedMotion";

export type KanbanBoardProps = {
  detail: BoardDetail;
  renderCard: (card: BoardCard) => ReactNode;
  renderColumnHeader: (column: BoardColumn, count: number) => ReactNode;
  renderColumnFooter: (column: BoardColumn) => ReactNode; // "+ Add card" / composer
  onCardMove: (cardId: string, to: DropTarget) => void;
  onColumnMove: (columnId: string, index: number) => void;
  /** Rendered after the last column inside the scroller (the add-column button). */
  trailing?: ReactNode;
};

type ActiveDrag = { kind: DragKind; id: string };

// Cards collide with cards and column bodies; columns only with columns.
const collisionDetection: CollisionDetection = (args) => {
  const kind = dragDataOf(args.active)?.kind;
  const droppableContainers = args.droppableContainers.filter((container) => {
    const over = dragDataOf(container)?.kind;
    return kind === "column"
      ? over === "column"
      : over === "card" || over === "column-body";
  });
  return kind === "column"
    ? closestCenter({ ...args, droppableContainers })
    : closestCorners({ ...args, droppableContainers });
};

// Columns slide along the row only, so the scroller never clips them.
const lockColumnsToRow: Modifier = ({ active, transform }) =>
  active && dragDataOf(active)?.kind === "column"
    ? { ...transform, y: 0 }
    : transform;

const MEASURING = { droppable: { strategy: MeasuringStrategy.Always } };

const OVERLAY_CLASS =
  "rounded-lg rotate-[1.5deg] shadow-[0_16px_36px_-10px_rgb(0_0_0/0.45)] motion-reduce:rotate-0";

/**
 * Columns and cards on dnd-kit. Owns drag state only: while a card is dragged
 * across columns it renders a local preview; the final move is reported once
 * through `onCardMove` / `onColumnMove` and shown optimistically until a new
 * `detail` arrives.
 */
export function KanbanBoard({
  detail,
  renderCard,
  renderColumnHeader,
  renderColumnFooter,
  onCardMove,
  onColumnMove,
  trailing,
}: KanbanBoardProps) {
  const reducedMotion = useReducedMotion();
  const [local, setLocal] = useState<BoardDetail | null>(null);
  const [active, setActive] = useState<ActiveDrag | null>(null);
  const [seen, setSeen] = useState(detail);
  // A fresh `detail` replaces the optimistic copy, except mid-drag.
  if (seen !== detail) {
    setSeen(detail);
    if (!active) setLocal(null);
  }
  const shown = local ?? detail;

  const snapshotRef = useRef<BoardDetail | null>(null);
  const shownRef = useRef(shown);
  useLayoutEffect(() => {
    shownRef.current = shown;
  });

  const announcements = useMemo(
    () =>
      kanbanAnnouncements({
        current: () => shownRef.current,
        snapshot: () => snapshotRef.current,
      }),
    [],
  );

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );

  const columns = useMemo(() => sortedColumns(shown), [shown]);
  const cardsByColumn = useMemo(
    () => new Map(columns.map((c) => [c.id, columnCards(shown, c.id)])),
    [columns, shown],
  );
  const activeCard =
    active?.kind === "card"
      ? (shown.cards.find((c) => c.id === active.id) ?? null)
      : null;

  function finish() {
    setActive(null);
    snapshotRef.current = null;
  }

  // Back to the board as it was at drag start.
  function restore(snapshot: BoardDetail) {
    setLocal(snapshot === detail ? null : snapshot);
  }

  function handleDragStart({ active: dragged }: DragStartEvent) {
    const data = dragDataOf(dragged);
    if (!data || data.kind === "column-body") return;
    snapshotRef.current = shown;
    setActive({
      kind: data.kind,
      id: data.kind === "card" ? data.cardId : data.columnId,
    });
  }

  function handleDragOver({ active: dragged, over }: DragOverEvent) {
    const data = dragDataOf(dragged);
    const ref = toDropRef(over);
    if (data?.kind !== "card" || !ref) return;
    // Only cross-column moves need a preview; SortableContext animates the rest.
    setLocal((prev) => {
      const current = prev ?? detail;
      const card = current.cards.find((c) => c.id === data.cardId);
      const target = resolveCardDrop(
        current,
        data.cardId,
        ref.overId,
        ref.overKind,
      );
      if (!card || !target || target.columnId === card.columnId) return prev;
      return applyCardMove(current, data.cardId, target);
    });
  }

  function handleDragEnd({ active: dragged, over }: DragEndEvent) {
    const data = dragDataOf(dragged);
    const snapshot = snapshotRef.current ?? detail;
    finish();
    if (data?.kind === "card") {
      const to = finalCardTarget(snapshot, shown, data.cardId, toDropRef(over));
      if (!to) {
        restore(snapshot);
        return;
      }
      setLocal(applyCardMove(snapshot, data.cardId, to));
      onCardMove(data.cardId, to);
    } else if (data?.kind === "column") {
      const ref = toDropRef(over);
      const index = ref && resolveColumnDrop(shown, data.columnId, ref.overId);
      if (index == null) return;
      setLocal(applyColumnMove(shown, data.columnId, index));
      onColumnMove(data.columnId, index);
    }
  }

  function handleDragCancel() {
    restore(snapshotRef.current ?? detail);
    finish();
  }

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={collisionDetection}
      measuring={MEASURING}
      modifiers={[lockColumnsToRow]}
      accessibility={{
        announcements,
        screenReaderInstructions: kanbanScreenReaderInstructions,
      }}
      onDragStart={handleDragStart}
      onDragOver={handleDragOver}
      onDragEnd={handleDragEnd}
      onDragCancel={handleDragCancel}
    >
      <div
        data-kanban-board=""
        data-dragging={active ? active.kind : undefined}
        className="flex h-full min-h-0 items-start gap-2 overflow-x-auto px-4 py-3"
      >
        <SortableContext
          items={columns.map((c) => c.id)}
          strategy={horizontalListSortingStrategy}
        >
          {columns.map((column) => (
            <KanbanColumn
              key={column.id}
              column={column}
              cards={cardsByColumn.get(column.id) ?? []}
              isDropTarget={activeCard?.columnId === column.id}
              reducedMotion={reducedMotion}
              renderCard={renderCard}
              renderColumnHeader={renderColumnHeader}
              renderColumnFooter={renderColumnFooter}
            />
          ))}
        </SortableContext>
        {trailing}
      </div>
      {createPortal(
        <DragOverlay dropAnimation={reducedMotion ? null : undefined}>
          {activeCard ? (
            <div className={OVERLAY_CLASS}>{renderCard(activeCard)}</div>
          ) : null}
        </DragOverlay>,
        document.body,
      )}
    </DndContext>
  );
}
