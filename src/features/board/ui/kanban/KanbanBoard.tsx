import {
  closestCenter,
  closestCorners,
  type CollisionDetection,
  DndContext,
  type DragEndEvent,
  type DragOverEvent,
  DragOverlay,
  type DragStartEvent,
  type KeyboardCoordinateGetter,
  KeyboardSensor,
  type KeyboardSensorProps,
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
  useReducer,
  useRef,
  useState,
} from "react";
import { createPortal, flushSync } from "react-dom";
import type { BoardCard, BoardColumn, BoardDetail } from "../../model/board";
import { type FocusKey, nextCardFocus } from "../../model/cardFocus";
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
import { KEYBOARD_CODES } from "./keyboardCodes";
import { useReducedMotion } from "./useReducedMotion";

export type KanbanBoardProps = {
  detail: BoardDetail;
  renderCard: (card: BoardCard) => ReactNode;
  renderColumnHeader: (column: BoardColumn, count: number) => ReactNode;
  renderColumnFooter: (column: BoardColumn) => ReactNode; // "+ Add card" / composer
  onCardMove: (cardId: string, to: DropTarget) => void;
  onColumnMove: (columnId: string, index: number) => void;
  /** Enter on a focused card; Space picks it up. */
  onCardOpen?: (card: BoardCard) => void;
  /** Click or Enter on a collapsed column's strip. */
  onColumnExpand?: (column: BoardColumn) => void;
  /** Rendered after the last column inside the scroller (the add-column button). */
  trailing?: ReactNode;
  /**
   * The cards follow a sort, so a drop inside the column they came from
   * changes nothing. A drop into another column is still reported; the
   * caller appends it.
   */
  sorted?: boolean;
  /** Cards in the bulk selection: each card announces it is pressed. */
  selectedIds?: ReadonlySet<string>;
  /** Card density: the first-guess height of a card before it is measured. */
  density?: "comfortable" | "compact";
  /**
   * The card with keyboard focus (tracked by the board view), kept mounted
   * when scrolled away so focus never drops to the page.
   */
  focusedCardId?: string | null;
  /** The card whose drawer is open, kept mounted when scrolled away. */
  openCardId?: string | null;
  /**
   * Every card in each column, whatever a search or filter hides; set only
   * while one is active, so a collapsed strip can read "3 of 12 cards".
   */
  columnTotals?: ReadonlyMap<string, number>;
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

/**
 * dnd-kit's keyboard sensor, reporting the instance running the current lift
 * so the board can cancel it directly (dnd-kit has no public cancel). A key
 * event would go through every window listener first, and a popover, modal
 * or sheet that takes Escape would close instead.
 */
type Cancellable = { handleCancel(event: Event): void };

function trackedKeyboardSensor(live: { current: Cancellable | null }) {
  return class TrackedKeyboardSensor extends KeyboardSensor {
    constructor(props: KeyboardSensorProps) {
      super(props);
      // `handleCancel` is private in dnd-kit's types, but it is the sensor's
      // own cancel: it detaches its listeners and calls onCancel.
      live.current = this as unknown as Cancellable;
    }
  };
}

/** The mounted card element for `id` inside `root`, if any. */
function cardElement(root: HTMLElement | null, id: string): HTMLElement | null {
  if (!root) return null;
  for (const el of root.querySelectorAll<HTMLElement>("[data-kanban-card]")) {
    if (el.getAttribute("data-kanban-card") === id) return el;
  }
  return null;
}

/**
 * Where ↑/↓ moves a lifted card, counted from the data: the card the drop
 * goes next to, its column and its index there. `overId` is what the card is
 * over now; a card shown next to it by a cross-column preview (`previewOver`)
 * means "where the preview put it", so the move counts from the card itself.
 */
export function keyboardDropTarget(
  shown: BoardDetail,
  cardId: string,
  overId: string | null,
  previewOver: string | null,
  step: 1 | -1,
): { id: string; columnId: string; index: number } | null {
  const card = shown.cards.find((c) => c.id === cardId);
  if (!card) return null;
  const list = columnCards(shown, card.columnId);
  const overAt =
    overId && overId !== previewOver
      ? list.findIndex((c) => c.id === overId)
      : -1;
  const base = overAt >= 0 ? overAt : list.findIndex((c) => c.id === cardId);
  const next = list[base + step];
  return next ? { id: next.id, columnId: card.columnId, index: base + step } : null;
}

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
  onCardOpen,
  onColumnExpand,
  selectedIds,
  trailing,
  sorted = false,
  density = "comfortable",
  focusedCardId = null,
  openCardId = null,
  columnTotals,
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

  const sessionRef = useRef(session);
  useLayoutEffect(() => {
    sessionRef.current = session;
  });

  const sortedRef = useRef(sorted);
  useLayoutEffect(() => {
    sortedRef.current = sorted;
  });
  const announcements = useMemo(
    () =>
      kanbanAnnouncements({
        current: () => shownRef.current,
        snapshot: () => snapshotRef.current,
        sorted: () => sortedRef.current,
      }),
    [],
  );

  // dnd-kit's multiple-containers pattern: right after a card is previewed in
  // another column its slot is a 2px line, so the card it was dropped next to
  // is still the closest and would read as a sort past it. Until the next
  // frame the card is over itself, which keeps the previewed place.
  const movedToNewColumn = useRef(false);
  // A keyboard drag moves up and down by the data, not by measured rects:
  // the card it goes next to may not be mounted yet (the column is
  // windowed), so it is scrolled into view and named here as the target.
  // Null while ←/→ (or a pointer) leave the target to the rects.
  const keyboardOver = useRef<string | null>(null);
  const keyboardDrag = useRef(false);
  const collisionDetection = useCallback<CollisionDetection>(
    (args) =>
      movedToNewColumn.current
        ? [{ id: args.active.id }]
        : keyboardOver.current
          ? [{ id: keyboardOver.current }]
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

  // Each column's "scroll card `index` into view", for focus and drags.
  const scrollers = useRef(new Map<string, (index: number) => void>());
  const registerScroller = useCallback(
    (columnId: string, scroll: ((index: number) => void) | null) => {
      if (scroll) scrollers.current.set(columnId, scroll);
      else scrollers.current.delete(columnId);
    },
    [],
  );
  const boardRef = useRef<HTMLDivElement | null>(null);
  // Re-renders the board so a new keyboard target is picked up even when
  // the overlay's coordinates didn't change.
  const [, rerender] = useReducer((n: number) => n + 1, 0);

  const coordinateGetter = useCallback<KeyboardCoordinateGetter>(
    (event, args) => {
      const { active, over, collisionRect } = args.context;
      const data = active ? dragDataOf(active) : null;
      const vertical = event.code === "ArrowUp" || event.code === "ArrowDown";
      if (data?.kind === "card" && vertical) {
        const board = shownRef.current;
        const column = board.columns.find(
          (c) =>
            c.id === board.cards.find((card) => card.id === data.cardId)?.columnId,
        );
        // A collapsed column mounts no cards: leave it to the rects.
        if (column && !column.collapsed) {
          event.preventDefault();
          const target = keyboardDropTarget(
            board,
            data.cardId,
            over ? String(over.id) : keyboardOver.current,
            sessionRef.current.drag?.previewOver ?? null,
            event.code === "ArrowDown" ? 1 : -1,
          );
          if (!target) return undefined;
          // Mount the target (and let it register as a droppable) before
          // the collision names it: dnd-kit resolves `over` once per id.
          flushSync(() => {
            scrollers.current.get(target.columnId)?.(target.index);
          });
          keyboardOver.current = target.id;
          rerender();
          const rect = cardElement(boardRef.current, target.id)?.getBoundingClientRect();
          if (!rect || !collisionRect) return args.currentCoordinates;
          const below = event.code === "ArrowDown";
          return {
            x: args.currentCoordinates.x,
            y: below ? rect.bottom - collisionRect.height : rect.top,
          };
        }
      }
      const next = sortableKeyboardCoordinates(event, args);
      if (next) keyboardOver.current = null;
      return next;
    },
    [],
  );

  // The keyboard sensor running the current lift; cleared when it ends.
  const liveKeyboardSensor = useRef<Cancellable | null>(null);
  const [BoardKeyboardSensor] = useState(() =>
    trackedKeyboardSensor(liveKeyboardSensor),
  );
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(BoardKeyboardSensor, {
      coordinateGetter,
      keyboardCodes: KEYBOARD_CODES,
    }),
  );

  const columns = useMemo(() => sortedColumns(shown), [shown]);
  const cardsByColumn = useMemo(
    () => new Map(columns.map((c) => [c.id, columnCards(shown, c.id)])),
    [columns, shown],
  );
  const drag = session.drag;
  const draggedId = drag?.kind === "card" ? drag.id : null;
  // The card focus moves to when the focused one is filtered away.
  const [refocus, setRefocus] = useState<string | null>(null);
  const pinned = useMemo(
    () =>
      [...new Set([draggedId, focusedCardId, openCardId, refocus])].filter(
        (id): id is string => !!id,
      ),
    [draggedId, focusedCardId, openCardId, refocus],
  );

  // The ids each column shows, as of this render and the one before.
  const shownIds = useMemo(
    () =>
      new Map(
        columns.map((column) => [
          column.id,
          column.collapsed
            ? []
            : (cardsByColumn.get(column.id) ?? []).map((card) => card.id),
        ]),
      ),
    [columns, cardsByColumn],
  );
  const lastShownIds = useRef(shownIds);
  useLayoutEffect(() => {
    const before = lastShownIds.current;
    lastShownIds.current = shownIds;
    if (!focusedCardId || before === shownIds) return;
    if ([...shownIds.values()].some((ids) => ids.includes(focusedCardId))) {
      return;
    }
    // Only when focus really went with the card (to the page), never away
    // from wherever the user has put it since.
    const active = document.activeElement;
    if (active && active !== document.body && active.isConnected) return;
    for (const [columnId, ids] of before) {
      const at = ids.indexOf(focusedCardId);
      if (at < 0) continue;
      const now = new Set(shownIds.get(columnId) ?? []);
      // The nearest card still shown, the one that took its place first.
      for (let d = 1; d < ids.length; d++) {
        for (const id of [ids[at + d], ids[at - d]]) {
          if (id && now.has(id)) {
            setRefocus(id);
            return;
          }
        }
      }
      const first = shownIds.get(columnId)?.[0];
      if (first) setRefocus(first);
      return;
    }
  }, [shownIds, focusedCardId]);
  useLayoutEffect(() => {
    if (!refocus) return;
    for (const [columnId, ids] of shownIds) {
      const index = ids.indexOf(refocus);
      if (index >= 0) scrollers.current.get(columnId)?.(index);
    }
    cardElement(boardRef.current, refocus)?.focus();
    setRefocus(null);
  }, [refocus, shownIds]);

  /** ↑/↓/←/→ on a card: scroll the next card into view, then focus it. */
  const focusFrom = useCallback(
    (from: string, key: FocusKey) => {
      const target = nextCardFocus(
        columns.map((column) => ({
          id: column.id,
          // A collapsed strip shows no cards, so it has none to focus.
          cardIds: column.collapsed
            ? []
            : (cardsByColumn.get(column.id) ?? []).map((card) => card.id),
        })),
        from,
        key,
      );
      if (!target) return;
      const columnId = shown.cards.find((card) => card.id === target)?.columnId;
      const index = (columnId ? cardsByColumn.get(columnId) : undefined)?.findIndex(
        (card) => card.id === target,
      );
      if (columnId && index !== undefined && index >= 0) {
        flushSync(() => scrollers.current.get(columnId)?.(index));
      }
      cardElement(boardRef.current, target)?.focus();
    },
    [columns, cardsByColumn, shown],
  );

  // A click anywhere but the lifted card abandons a keyboard drag, as
  // Escape would (the Native rule: focus moving off the card cancels it).
  const keyboardLift = drag && keyboardDrag.current ? drag.id : null;
  useEffect(() => {
    if (!keyboardLift) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest("[data-kanban-card]")?.getAttribute("data-kanban-card") ===
          keyboardLift
      )
        return;
      // Straight to the sensor: no key event for other surfaces to take.
      liveKeyboardSensor.current?.handleCancel(new Event("cancel"));
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    return () =>
      document.removeEventListener("pointerdown", onPointerDown, true);
  }, [keyboardLift]);
  const activeCard =
    drag?.kind === "card"
      ? (shown.cards.find((c) => c.id === drag.id) ?? null)
      : null;

  function update(next: DragSession) {
    movedToNewColumn.current = false;
    if (!next.drag) {
      keyboardOver.current = null;
      keyboardDrag.current = false;
      liveKeyboardSensor.current = null;
    }
    snapshotRef.current = next.lastSnapshot;
    setSession(next);
  }

  function handleDragStart({ active: dragged, activatorEvent }: DragStartEvent) {
    const data = dragDataOf(dragged);
    keyboardDrag.current = activatorEvent instanceof KeyboardEvent;
    // A lifted card starts over its own place.
    keyboardOver.current =
      keyboardDrag.current && data?.kind === "card" ? data.cardId : null;
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
      if (next.local !== s.local) {
        movedToNewColumn.current = true;
        // A keyboard card shown in another column goes on from its new
        // place there, unless that column is a strip with no cards mounted.
        const dragged = next.drag?.kind === "card" ? next.drag.id : null;
        const into = dragged
          ? shownBoard(next).cards.find((c) => c.id === dragged)?.columnId
          : undefined;
        if (
          keyboardDrag.current &&
          dragged &&
          !shownBoard(next).columns.find((c) => c.id === into)?.collapsed
        ) {
          keyboardOver.current = dragged;
        }
      }
      return next;
    });
  }

  function handleDragEnd({ over }: DragEndEvent) {
    const { session: next, commit } = sessionEnd(session, toDropRef(over));
    if (sorted && commit?.kind === "card") {
      const from = session.drag?.snapshot.cards.find(
        (card) => card.id === commit.cardId,
      )?.columnId;
      if (from === commit.to.columnId) {
        update(sessionCancel(session));
        return;
      }
    }
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
        ref={boardRef}
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
              selectedIds={selectedIds}
              onCardOpen={drag ? undefined : onCardOpen}
              onCardArrow={drag ? undefined : focusFrom}
              registerScroller={registerScroller}
              renderColumnHeader={renderColumnHeader}
              renderColumnFooter={renderColumnFooter}
              onExpand={onColumnExpand}
              density={density}
              pinned={pinned}
              total={columnTotals?.get(column.id)}
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
