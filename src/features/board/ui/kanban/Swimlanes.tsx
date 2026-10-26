import {
  type Announcements,
  closestCorners,
  type CollisionDetection,
  DndContext,
  type DragEndEvent,
  type DragOverEvent,
  DragOverlay,
  type DragStartEvent,
  type KeyboardCoordinateGetter,
  KeyboardSensor,
  PointerSensor,
  type ScreenReaderInstructions,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import {
  type ReactNode,
  useCallback,
  useLayoutEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from "react";
import { createPortal, flushSync } from "react-dom";
import { KEYBOARD_CODES } from "./keyboardCodes";
import type { BoardCard, BoardColumn } from "../../model/board";
import {
  type FocusKey,
  isFocusKey,
  type LaneFocus,
  nextLaneFocus,
} from "../../model/cardFocus";
import {
  LANE_BAND,
  LANE_HEADER_ESTIMATE,
  type Lane,
  laneBandEstimate,
  laneCellKey,
  laneCells,
  type LaneRow,
  laneRows,
} from "../../model/swimlanes";
import { CARD_ESTIMATE, useMeasuredWindow } from "../../model/windowing";

type CellData = { laneKey: string; columnId: string };
type CardData = { cardId: string; laneKey: string };
type Cell = { lane: string; column: string };

/** The header drop cell of a lane and column: always mounted. */
const cellId = (laneKey: string, columnId: string) =>
  `cell:${laneKey}|${columnId}`;
/** One band's part of a cell: a drop there targets the same cell. */
const bandId = (laneKey: string, columnId: string, band: number) =>
  `band:${laneKey}|${columnId}|${band}`;
const rowKey = (row: LaneRow) =>
  row.kind === "header"
    ? `h\u0000${row.lane}`
    : `b\u0000${row.lane}\u0000${row.band}`;

/**
 * How far below the scroller's top the first lane row starts: its 12px top
 * pad, then the 36px column header row and its 4px gap.
 */
export const LANE_TOP_OFFSET = 52;
/** Rows rendered beyond each edge of the view: a band is up to 4 cards a column. */
const ROW_OVERSCAN = 2;
const NO_CARDS: readonly string[] = [];

const LANE_INSTRUCTIONS: ScreenReaderInstructions = {
  draggable:
    "To pick up a card, press Space. Use the arrow keys to move it between " +
    "lanes and columns, Space to drop it, or Escape to cancel.",
};

const STEP: Record<string, [number, number] | undefined> = {
  ArrowRight: [1, 0],
  ArrowLeft: [-1, 0],
  ArrowDown: [0, 1],
  ArrowUp: [0, -1],
};

/** The mounted element matching `attr` = `value` inside `root`, if any. */
function find(
  root: HTMLElement | null,
  selector: string,
  match: (el: HTMLElement) => boolean,
): HTMLElement | null {
  if (!root) return null;
  for (const el of root.querySelectorAll<HTMLElement>(selector)) {
    if (match(el)) return el;
  }
  return null;
}

/**
 * The board as rows: one lane per value, each a header row and then bands of
 * up to four cards per cell, in one windowed scroller. Only the rows in view
 * (and the pinned ones: every lane header, and the rows of the focused,
 * dragged and open cards) are mounted. Each header row carries a drop cell
 * per column, so a keyboard drag reaches any lane; a drop anywhere in a
 * cell's bands reports that cell, and the caller appends the card there and
 * sets what the lane means. Collapsed columns are 36px strips that take drops.
 */
export function Swimlanes({
  columns,
  cards,
  lanes,
  renderCard,
  renderColumnHeader,
  onDrop,
  onCardOpen,
  onColumnExpand,
  selectedIds,
  trailing,
  density = "comfortable",
  focusedCardId = null,
  openCardId = null,
}: {
  columns: readonly BoardColumn[];
  cards: readonly BoardCard[];
  lanes: readonly Lane[];
  renderCard: (card: BoardCard) => ReactNode;
  renderColumnHeader: (column: BoardColumn, count: number) => ReactNode;
  onDrop: (cardId: string, laneKey: string, columnId: string) => void;
  /** Enter on a focused card; Space picks it up. */
  onCardOpen?: (card: BoardCard) => void;
  /** Click on a collapsed column's strip. */
  onColumnExpand?: (column: BoardColumn) => void;
  /** Cards in the bulk selection: each card announces it is pressed. */
  selectedIds?: ReadonlySet<string>;
  trailing?: ReactNode;
  /** Card density: the first-guess height of a card before it is measured. */
  density?: keyof typeof CARD_ESTIMATE;
  /** The card with keyboard focus, kept mounted when scrolled away. */
  focusedCardId?: string | null;
  /** The card whose drawer is open, kept mounted when scrolled away. */
  openCardId?: string | null;
}) {
  const ordered = useMemo(
    () => [...columns].sort((a, b) => a.position - b.position),
    [columns],
  );
  const byId = useMemo(
    () => new Map(cards.map((card) => [card.id, card])),
    [cards],
  );
  // Every cell's cards, collapsed columns too (for counts and drops) ...
  const cells = useMemo(() => laneCells(lanes, cards), [lanes, cards]);
  // ... and the ones it shows: a collapsed strip shows none.
  const shownCells = useMemo(() => {
    const collapsed = ordered.filter((column) => column.collapsed);
    if (!collapsed.length) return cells;
    const out = new Map(cells);
    for (const lane of lanes) {
      for (const column of collapsed) out.delete(laneCellKey(lane.key, column.id));
    }
    return out;
  }, [cells, ordered, lanes]);
  const cellCards = useCallback(
    (lane: string, column: string) =>
      shownCells.get(laneCellKey(lane, column)) ?? NO_CARDS,
    [shownCells],
  );
  const rows = useMemo(() => laneRows(lanes, shownCells), [lanes, shownCells]);
  const rowIds = useMemo(() => rows.map(rowKey), [rows]);
  const rowIndex = useMemo(
    () => new Map(rowIds.map((id, index) => [id, index])),
    [rowIds],
  );
  const estimates = useMemo(() => {
    const cardEstimate = CARD_ESTIMATE[density];
    return rows.map((row) => {
      if (row.kind === "header") return LANE_HEADER_ESTIMATE;
      let most = 0;
      for (const column of ordered) {
        const shown = cellCards(row.lane, column.id).length - row.band * LANE_BAND;
        most = Math.max(most, Math.min(LANE_BAND, shown));
      }
      return laneBandEstimate(most, cardEstimate);
    });
  }, [rows, ordered, cellCards, density]);

  const [active, setActive] = useState<{ card: BoardCard; lane: string } | null>(
    null,
  );
  const [overCell, setOverCell] = useState<string | null>(null);

  /** The band rows showing card `id`, in every lane it sits in. */
  const rowsOfCard = useCallback(
    (id: string, onlyLane?: string): string[] => {
      const out: string[] = [];
      for (const lane of lanes) {
        if (onlyLane !== undefined && lane.key !== onlyLane) continue;
        for (const column of ordered) {
          const at = cellCards(lane.key, column.id).indexOf(id);
          if (at >= 0) {
            out.push(
              rowKey({ kind: "band", lane: lane.key, band: Math.floor(at / LANE_BAND) }),
            );
          }
        }
      }
      return out;
    },
    [lanes, ordered, cellCards],
  );
  const draggedId = active?.card.id ?? null;
  const pinned = useMemo(() => {
    const out = new Set(rowIds.filter((id) => id.startsWith("h\u0000")));
    for (const id of [draggedId, focusedCardId, openCardId]) {
      if (id) for (const row of rowsOfCard(id)) out.add(row);
    }
    return [...out];
  }, [rowIds, rowsOfCard, draggedId, focusedCardId, openCardId]);

  const windowed = useMeasuredWindow({
    ids: rowIds,
    estimate: LANE_HEADER_ESTIMATE,
    estimates,
    gap: 0,
    overscan: ROW_OVERSCAN,
    pinned,
    topOffset: LANE_TOP_OFFSET,
  });
  const { scrollToIndex, ref: windowRef } = windowed;
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const setScroller = useCallback(
    (node: HTMLDivElement | null) => {
      scrollerRef.current = node;
      windowRef(node);
    },
    [windowRef],
  );

  // What the keyboard handlers read, as of the last render.
  const nav = useRef({ lanes, ordered, rowIndex, scrollToIndex });
  useLayoutEffect(() => {
    nav.current = { lanes, ordered, rowIndex, scrollToIndex };
  });

  // A keyboard drag moves header cell to header cell by the data: the cell
  // it goes to is named here as the target, never found from rects, since
  // the lane it goes to may be scrolled far away. Null for a pointer drag.
  const keyboardCell = useRef<Cell | null>(null);
  const [, rerender] = useReducer((n: number) => n + 1, 0);
  const collisionDetection = useCallback<CollisionDetection>(
    (args) =>
      keyboardCell.current
        ? [{ id: cellId(keyboardCell.current.lane, keyboardCell.current.column) }]
        : closestCorners(args),
    [],
  );

  /**
   * Arrow keys step to the neighbouring cell: left and right along the lane,
   * up and down to the next lane in the same column, whose header is
   * scrolled into view first.
   */
  const coordinateGetter = useCallback<KeyboardCoordinateGetter>(
    (event, args) => {
      const dir = STEP[event.code];
      const at = keyboardCell.current;
      if (!dir || !args.context.active || !at) return undefined;
      event.preventDefault();
      const { lanes: all, ordered: cols, rowIndex: index, scrollToIndex: reveal } =
        nav.current;
      const li = all.findIndex((lane) => lane.key === at.lane) + dir[1];
      const ci = cols.findIndex((column) => column.id === at.column) + dir[0];
      if (li < 0 || li >= all.length || ci < 0 || ci >= cols.length) {
        return undefined;
      }
      const next = { lane: all[li].key, column: cols[ci].id };
      if (dir[1] !== 0) {
        flushSync(() =>
          reveal(index.get(rowKey({ kind: "header", lane: next.lane })) ?? -1),
        );
      }
      keyboardCell.current = next;
      rerender();
      const rect = find(
        scrollerRef.current,
        "[data-lane-cell]",
        (el) => el.getAttribute("data-lane-cell") === `${next.lane}|${next.column}`,
      )?.getBoundingClientRect();
      return rect
        ? { x: rect.left + 6, y: rect.top + 6 }
        : args.currentCoordinates;
    },
    [],
  );

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter,
      keyboardCodes: KEYBOARD_CODES,
    }),
  );

  const announcements = useMemo<Announcements>(() => {
    const laneName = (key: string) =>
      lanes.find((lane) => lane.key === key)?.label ?? key;
    const columnName = (id: string) =>
      columns.find((column) => column.id === id)?.name ?? id;
    /** "lane …, column …, position i of n", counted over the whole cell. */
    const place = (cardId: string, lane: string, column: string) => {
      const ids = cells.get(laneCellKey(lane, column)) ?? NO_CARDS;
      const at = ids.indexOf(cardId);
      const [i, total] = at >= 0 ? [at + 1, ids.length] : [ids.length + 1, ids.length + 1];
      return `lane ${laneName(lane)}, column ${columnName(column)}, position ${i} of ${total}`;
    };
    const cardData = (a: { data: { current?: unknown } }) =>
      a.data.current as CardData | undefined;
    const cellData = (o: { data: { current?: unknown } } | null) =>
      o ? (o.data.current as CellData | undefined) : undefined;
    const card = (data: CardData | undefined) =>
      data ? byId.get(data.cardId) : undefined;
    return {
      onDragStart({ active }) {
        const data = cardData(active);
        const c = card(data);
        if (!c || !data) return undefined;
        const where = place(c.id, data.laneKey, c.columnId);
        return `Picked up card #${c.number}, ${c.title}. ${where[0].toUpperCase()}${where.slice(1)}.`;
      },
      onDragOver({ active, over }) {
        const c = card(cardData(active));
        if (!c) return undefined;
        const to = cellData(over);
        return to && "columnId" in to
          ? `Card #${c.number}, ${c.title} is over ${place(c.id, to.laneKey, to.columnId)}.`
          : `Card #${c.number} is no longer over a lane.`;
      },
      onDragEnd({ active, over }) {
        const c = card(cardData(active));
        if (!c) return undefined;
        const to = cellData(over);
        return to && "columnId" in to
          ? `Dropped card #${c.number} in ${place(c.id, to.laneKey, to.columnId)}.`
          : `Card #${c.number} was not moved.`;
      },
      onDragCancel({ active }) {
        const data = cardData(active);
        const c = card(data);
        if (!c || !data) return undefined;
        return `Moving card #${c.number} was cancelled. It is back in ${place(c.id, data.laneKey, c.columnId)}.`;
      },
    };
  }, [lanes, columns, byId, cells]);

  /** ↑/↓/←/→ on a card: scroll the next card's band into view, then focus it. */
  const focusFrom = useCallback(
    (from: LaneFocus, key: FocusKey) => {
      const target = nextLaneFocus(
        lanes.map((lane) => ({
          key: lane.key,
          columns: ordered.map((column) => ({
            id: column.id,
            cardIds: cellCards(lane.key, column.id),
          })),
        })),
        from,
        key,
      );
      if (!target) return;
      const row = rowsOfCard(target.cardId, target.lane)[0];
      const index = row === undefined ? undefined : rowIndex.get(row);
      if (index !== undefined) flushSync(() => scrollToIndex(index));
      find(
        scrollerRef.current,
        "[data-kanban-card]",
        (el) =>
          el.getAttribute("data-kanban-card") === target.cardId &&
          el.getAttribute("data-lane-key") === target.lane,
      )?.focus();
    },
    [lanes, ordered, cellCards, rowsOfCard, rowIndex, scrollToIndex],
  );

  function start({ active: dragged, activatorEvent }: DragStartEvent) {
    const data = dragged.data.current as CardData | undefined;
    const card = data ? byId.get(data.cardId) : undefined;
    keyboardCell.current =
      card && data && activatorEvent instanceof KeyboardEvent
        ? { lane: data.laneKey, column: card.columnId }
        : null;
    setActive(card && data ? { card, lane: data.laneKey } : null);
  }

  function over({ over: target }: DragOverEvent) {
    const to = target?.data.current as CellData | undefined;
    setOverCell(to && "columnId" in to ? laneCellKey(to.laneKey, to.columnId) : null);
  }

  function finish() {
    keyboardCell.current = null;
    setActive(null);
    setOverCell(null);
  }

  function end({ active: dragged, over: target }: DragEndEvent) {
    finish();
    const from = dragged.data.current as CardData | undefined;
    const to = target?.data.current as CellData | undefined;
    if (from && to && "columnId" in to) onDrop(from.cardId, to.laneKey, to.columnId);
  }

  const lanesByKey = useMemo(
    () => new Map(lanes.map((lane) => [lane.key, lane])),
    [lanes],
  );
  const lastBand = useMemo(() => {
    const out = new Map<string, number>();
    for (const row of rows) if (row.kind === "band") out.set(row.lane, row.band);
    return out;
  }, [rows]);

  // Group the slots by lane: every header is pinned, so each skipped run
  // lies inside one lane, after its header.
  const sections: { lane: Lane; children: ReactNode[] }[] = [];
  windowed.slots.forEach((slot, i, slots) => {
    if (slot.kind === "gap") {
      const prev = slots[i - 1];
      const key =
        i === slots.length - 1
          ? "gap:tail"
          : `gap:after:${prev?.kind === "item" ? rowIds[prev.index] : i}`;
      sections[sections.length - 1]?.children.push(
        <div
          key={key}
          aria-hidden="true"
          data-window-gap=""
          className="shrink-0"
          style={{ height: slot.height }}
        />,
      );
      return;
    }
    const row = rows[slot.index];
    const lane = lanesByKey.get(row.lane);
    if (!lane) return;
    const measure = windowed.measureRef(rowIds[slot.index]);
    if (row.kind === "header") {
      sections.push({
        lane,
        children: [
          <div
            key="header"
            ref={measure}
            data-lane-row={slot.index}
            data-lane-header={lane.key}
            className="flex h-9 shrink-0 flex-col justify-end"
          >
            <h3 className="sticky left-0 flex h-[22px] w-max items-center gap-1 text-[12px] font-medium text-content/70">
              {lane.label}{" "}
              <span data-lane-count="" className="tabular-nums text-content/40">
                {lane.cardIds.length}
              </span>
            </h3>
            <div className="flex gap-2">
              {ordered.map((column) => (
                <HeaderCell
                  key={column.id}
                  lane={lane.key}
                  column={column}
                  over={overCell === laneCellKey(lane.key, column.id)}
                />
              ))}
            </div>
          </div>,
        ],
      });
      return;
    }
    const last = lastBand.get(lane.key) === row.band;
    sections[sections.length - 1]?.children.push(
      <div
        key={`band:${row.band}`}
        ref={measure}
        data-lane-row={slot.index}
        className="flex shrink-0 gap-2"
      >
        {ordered.map((column) => {
          const all = cellCards(lane.key, column.id);
          const from = row.band * LANE_BAND;
          return (
            <BandCell
              key={column.id}
              lane={lane}
              column={column}
              band={row.band}
              last={last}
              over={overCell === laneCellKey(lane.key, column.id)}
            >
              {all.slice(from, from + LANE_BAND).map((id, offset) => {
                const card = byId.get(id);
                if (!card) return null;
                return (
                  <LaneCard
                    key={card.id}
                    card={card}
                    laneKey={lane.key}
                    selected={selectedIds?.size ? selectedIds.has(card.id) : undefined}
                    description={`${from + offset + 1} of ${all.length} in ${lane.label}, ${column.name}`}
                    onOpen={active ? undefined : onCardOpen}
                    onArrow={active ? undefined : focusFrom}
                  >
                    {renderCard(card)}
                  </LaneCard>
                );
              })}
            </BandCell>
          );
        })}
      </div>,
    );
  });

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={collisionDetection}
      accessibility={{
        announcements,
        screenReaderInstructions: LANE_INSTRUCTIONS,
      }}
      onDragStart={start}
      onDragOver={over}
      onDragEnd={end}
      onDragCancel={finish}
    >
      <div
        ref={setScroller}
        onScroll={windowed.onScroll}
        data-swimlanes=""
        className="flex h-full min-h-0 flex-col overflow-auto px-4 py-3 [overflow-anchor:none]"
      >
        <div className="flex w-max shrink-0 gap-2 pb-1">
          {ordered.map((column) => {
            const count = cards.filter((card) => card.columnId === column.id).length;
            return column.collapsed ? (
              <div
                key={column.id}
                data-lane-column-head={column.id}
                data-column-collapsed=""
                className="flex h-9 w-9 shrink-0"
              >
                <button
                  type="button"
                  aria-label={`Expand ${column.name}`}
                  title={`${column.name}: expand column`}
                  onClick={() => onColumnExpand?.(column)}
                  className="flex w-full items-center justify-center rounded-lg outline-none hover:bg-content/5 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent/60"
                >
                  <span
                    data-column-count=""
                    className="text-[12px] tabular-nums text-content/45"
                  >
                    {count}
                  </span>
                </button>
              </div>
            ) : (
              <div
                key={column.id}
                data-lane-column-head={column.id}
                className="flex h-9 w-[280px] shrink-0 items-center gap-2 px-2.5"
              >
                {renderColumnHeader(column, count)}
              </div>
            );
          })}
          {trailing}
        </div>
        {sections.map(({ lane, children }) => (
          <section
            key={lane.key}
            aria-label={`Lane ${lane.label}`}
            data-lane={lane.key}
            className="flex w-max shrink-0 flex-col"
          >
            {children}
          </section>
        ))}
      </div>
      {createPortal(
        <DragOverlay>{active ? renderCard(active.card) : null}</DragOverlay>,
        document.body,
      )}
    </DndContext>
  );
}

const CELL_CLASS =
  "shrink-0 border-x border-stroke bg-selection-subtle data-[over]:border-accent/50";

/** A lane's drop cell for one column in its header row: the top of the cell. */
function HeaderCell({
  lane,
  column,
  over,
}: {
  lane: string;
  column: BoardColumn;
  over: boolean;
}) {
  const data: CellData = { laneKey: lane, columnId: column.id };
  const { setNodeRef } = useDroppable({ id: cellId(lane, column.id), data });
  return (
    <div
      ref={setNodeRef}
      data-lane-cell={`${lane}|${column.id}`}
      data-lane-column={column.id}
      data-column-collapsed={column.collapsed ? "" : undefined}
      data-over={over ? "" : undefined}
      className={`${CELL_CLASS} h-3.5 rounded-t-lg border-t ${column.collapsed ? "w-9" : "w-[280px]"}`}
    />
  );
}

/** One band's part of a cell: up to four cards, or an empty drop area. */
function BandCell({
  lane,
  column,
  band,
  last,
  over,
  children,
}: {
  lane: Lane;
  column: BoardColumn;
  band: number;
  last: boolean;
  over: boolean;
  children: ReactNode;
}) {
  const data: CellData = { laneKey: lane.key, columnId: column.id };
  const { setNodeRef } = useDroppable({
    id: bandId(lane.key, column.id, band),
    data,
  });
  return (
    <div
      ref={setNodeRef}
      data-lane-band={`${lane.key}|${column.id}|${band}`}
      data-lane-column={column.id}
      data-column-collapsed={column.collapsed ? "" : undefined}
      data-over={over ? "" : undefined}
      className={`${CELL_CLASS} flex min-h-10 flex-col gap-1.5 ${column.collapsed ? "w-9" : "w-[280px] px-1.5 pb-1.5"} ${last ? "rounded-b-lg border-b" : ""}`}
    >
      {column.collapsed ? null : children}
    </div>
  );
}

function LaneCard({
  card,
  laneKey,
  selected,
  description,
  onOpen,
  onArrow,
  children,
}: {
  card: BoardCard;
  laneKey: string;
  selected?: boolean;
  /** "{i} of {total} in {lane}, {column}", counted over the whole cell. */
  description: string;
  onOpen?: (card: BoardCard) => void;
  /** An unmodified arrow key on the card itself: move focus that way. */
  onArrow?: (from: LaneFocus, key: FocusKey) => void;
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
        const own =
          event.target === event.currentTarget &&
          !event.defaultPrevented &&
          !event.nativeEvent.isComposing;
        // Enter on the card itself opens it; on a control inside, the control acts.
        if (onOpen && event.key === "Enter" && own) {
          event.preventDefault();
          onOpen(card);
          return;
        }
        if (
          onArrow &&
          isFocusKey(event.key) &&
          own &&
          !event.altKey &&
          !event.ctrlKey &&
          !event.metaKey &&
          !event.shiftKey
        ) {
          event.preventDefault();
          onArrow({ lane: laneKey, cardId: card.id }, event.key);
        }
      }}
      aria-roledescription="card"
      aria-pressed={selected}
      aria-label={`Card #${card.number}, ${card.title}`}
      aria-description={description}
      data-kanban-card={card.id}
      data-lane-key={laneKey}
      data-dragging={isDragging ? "" : undefined}
      style={{ opacity: isDragging ? 0.4 : undefined }}
      className="shrink-0 rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
    >
      {children}
    </div>
  );
}
