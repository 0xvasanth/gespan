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
  useCallback,
  useEffect,
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
  columnCards,
  type DropTarget,
  dragDataOf,
  sortedColumns,
  toDropRef,
} from "./kanbanMove";
import {
  createSession,
  type DragSession,
  sessionCancel,
  sessionDetailChanged,
  sessionEnd,
  sessionOver,
  sessionStart,
  shownBoard,
} from "./kanbanSession";
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

// Cards collide with cards and column bodies; columns only with columns.
const boardCollisions: CollisionDetection = (args) => {
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
  const [session, setSession] = useState(() => createSession(detail));
  if (session.detail !== detail) {
    setSession(sessionDetailChanged(session, detail));
  }
  const shown = shownBoard(session);

  // Announcements run inside dnd-kit's event, after our handler but before
  // React commits: `current` is the board as rendered, and `snapshot` is set
  // synchronously at drag start and kept until the next one.
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

  // dnd-kit's multiple-containers pattern: right after a card is previewed in
  // another column its slot is a 2px line, so the card it was dropped next to
  // is still the closest and would read as a sort past it. Until the next
  // frame the card is over itself, which keeps the previewed place.
  const movedToNewColumn = useRef(false);
  const collisionDetection = useCallback<CollisionDetection>(
    (args) =>
      movedToNewColumn.current
        ? [{ id: args.active.id }]
        : boardCollisions(args),
    [],
  );
  useEffect(() => {
    if (!movedToNewColumn.current) return;
    const frame = requestAnimationFrame(() => {
      movedToNewColumn.current = false;
    });
    return () => cancelAnimationFrame(frame);
  }, [session.local]);

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
  const drag = session.drag;
  const activeCard =
    drag?.kind === "card"
      ? (shown.cards.find((c) => c.id === drag.id) ?? null)
      : null;

  function update(next: DragSession) {
    movedToNewColumn.current = false;
    snapshotRef.current = next.lastSnapshot;
    setSession(next);
  }

  function handleDragStart({ active: dragged }: DragStartEvent) {
    const data = dragDataOf(dragged);
    if (data?.kind === "card") {
      update(sessionStart(session, { kind: "card", id: data.cardId }));
    } else if (data?.kind === "column") {
      update(sessionStart(session, { kind: "column", id: data.columnId }));
    }
  }

  function handleDragOver({ over }: DragOverEvent) {
    const ref = toDropRef(over);
    setSession((s) => {
      const next = sessionOver(s, ref);
      if (next.local !== s.local) movedToNewColumn.current = true;
      return next;
    });
  }

  function handleDragEnd({ over }: DragEndEvent) {
    const { session: next, commit } = sessionEnd(session, toDropRef(over));
    update(next);
    if (commit?.kind === "card") onCardMove(commit.cardId, commit.to);
    else if (commit?.kind === "column") {
      onColumnMove(commit.columnId, commit.index);
    }
  }

  function handleDragCancel() {
    update(sessionCancel(session));
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
        data-dragging={drag ? drag.kind : undefined}
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
