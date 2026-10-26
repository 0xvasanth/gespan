import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import {
  type BoardCard,
  type BoardColumn,
  type BoardField,
  type BoardLabel,
  type CardExtras,
  type CardIndexRow,
} from "../model/board";
import type { AgentValues } from "../model/agentFields";
import type { AgentField } from "../model/board";
import { fieldText, formatEstimate } from "../model/cardProps";
import { MIN_COLUMN, ROW_HEIGHT, type TableColumn } from "../model/layout";
import { rowsWithPinned, useWindowedRows } from "../model/windowing";
import { MenuButton, MenuItem } from "./boardControls";
import { LabelPicker, toggled } from "./CardDrawerParts";
import { PriorityItems } from "./CardPropertyMenus";
import { dueChip, dueInputValue, parseDueInput } from "./boardFormat";
import { priorityName } from "../model/boardSettings";
import { usePriorityScheme } from "./priorityScheme";

/** An inline edit of a text-like cell, held here so scrolling or a refresh never loses it. */
type Edit = {
  cardId: string;
  field: "title" | "due";
  /** What the cell held when the edit began; only a different text is written. */
  original: string;
  text: string;
};

const STEP = 10;
const day = (ms: number | null | undefined) =>
  ms == null ? "" : new Date(ms).toLocaleDateString();

/**
 * The Table layout: one fixed-height row per card in the chosen columns,
 * only the rows in view rendered. Title and due edit in place with a draft
 * that waits for Enter (Escape drops it); status, priority and labels edit
 * through menus.
 */
export function BoardTable({
  cards,
  existing,
  columns,
  boardColumns,
  extras,
  index,
  labels,
  fields,
  projects,
  selected,
  focusedCardId = null,
  onRowClick,
  onResize,
  onCommitTitle,
  onSetStatus,
  onSetPriority,
  onSetDue,
  onSetLabels,
  onCreateLabel,
  agent,
}: {
  /** The agent fields of a card, worked out from its linked sessions. */
  agent?: (cardId: string) => AgentValues;
  /** The cards shown, in display order. */
  cards: readonly BoardCard[];
  /** Every card id on the board: an edit closes when its card is gone from it. */
  existing: ReadonlySet<string>;
  columns: readonly TableColumn[];
  boardColumns: readonly BoardColumn[];
  extras: Readonly<Record<string, CardExtras | undefined>>;
  index: ReadonlyMap<string, CardIndexRow>;
  labels: readonly BoardLabel[];
  fields: readonly BoardField[];
  projects: readonly { path: string; name: string }[];
  selected: ReadonlySet<string>;
  /** The card whose row holds focus: kept mounted when scrolled away (R81). */
  focusedCardId?: string | null;
  onRowClick: (event: ReactMouseEvent<HTMLElement>, card: BoardCard) => void;
  onResize: (columnId: string, width: number) => void;
  onCommitTitle: (card: BoardCard, title: string) => void;
  onSetStatus: (card: BoardCard, columnId: string) => void;
  onSetPriority: (card: BoardCard, priority: number) => void;
  onSetDue: (card: BoardCard, dueAt: number | null) => void;
  onSetLabels: (card: BoardCard, labelIds: string[]) => void;
  onCreateLabel: (card: BoardCard, name: string) => void;
}) {
  const scheme = usePriorityScheme();
  const win = useWindowedRows({
    count: cards.length,
    rowHeight: ROW_HEIGHT,
    // The sticky header sits above the first row inside the scroller.
    topOffset: ROW_HEIGHT,
  });
  const [edit, setEdit] = useState<Edit | null>(null);
  const editRef = useRef<Edit | null>(null);
  editRef.current = edit;
  const width = useMemo(
    () => columns.reduce((sum, column) => sum + column.width, 0),
    [columns],
  );

  // A card deleted in the other app takes its edit with it.
  useEffect(() => {
    if (edit && !existing.has(edit.cardId)) setEdit(null);
  }, [edit, existing]);

  const pinned = useMemo(() => {
    if (!focusedCardId) return [];
    const at = cards.findIndex((card) => card.id === focusedCardId);
    return at >= 0 ? [at] : [];
  }, [cards, focusedCardId]);

  const cardsById = useMemo(
    () => new Map(cards.map((card) => [card.id, card])),
    [cards],
  );

  const finish = (save: boolean) => {
    const current = editRef.current;
    if (!current) return;
    editRef.current = null;
    setEdit(null);
    const card = cardsById.get(current.cardId);
    if (!save || !card || current.text === current.original) return;
    if (current.field === "title") {
      const title = current.text.trim();
      if (title && title !== current.original) onCommitTitle(card, title);
    } else if (current.text === "") {
      onSetDue(card, null);
    } else {
      const due = parseDueInput(current.text);
      if (due !== null) onSetDue(card, due);
    }
  };

  const editInput = (card: BoardCard, field: Edit["field"]) => (
    <input
      autoFocus
      aria-label={
        field === "title"
          ? `Title of #${card.number}`
          : `Due date of #${card.number}`
      }
      type={field === "due" ? "date" : "text"}
      value={edit?.text ?? ""}
      onChange={(event) =>
        setEdit((current) =>
          current ? { ...current, text: event.target.value } : current,
        )
      }
      onClick={(event) => event.stopPropagation()}
      onPointerDown={(event) => event.stopPropagation()}
      onKeyDown={(event) => {
        event.stopPropagation();
        if (event.nativeEvent.isComposing) return;
        if (event.key === "Enter") {
          event.preventDefault();
          finish(true);
        } else if (event.key === "Escape") {
          // Drops the edit alone: the selection under it stays.
          event.preventDefault();
          finish(false);
        }
      }}
      onBlur={() => finish(true)}
      className="h-6 w-full min-w-0 rounded border border-stroke bg-background-base px-1.5 text-[13px] text-content outline-none focus:border-accent/60"
    />
  );

  const startEdit = (card: BoardCard, field: Edit["field"]) => {
    const original =
      field === "title" ? card.title : dueInputValue(extras[card.id]?.dueAt ?? null);
    setEdit({ cardId: card.id, field, original, text: original });
  };

  const cell = (card: BoardCard, column: TableColumn) => {
    const mine = extras[card.id];
    const row = index.get(card.id);
    const editing = edit?.cardId === card.id ? edit.field : null;
    switch (column.id) {
      case "title":
        return editing === "title" ? (
          editInput(card, "title")
        ) : (
          <>
            <span className="min-w-0 flex-1 truncate">{card.title}</span>
            <button
              type="button"
              aria-label={`Edit title of #${card.number}`}
              onClick={(event) => {
                event.stopPropagation();
                startEdit(card, "title");
              }}
              onKeyDown={(event) => event.stopPropagation()}
              className="shrink-0 rounded px-1 text-[11px] text-content/40 hover:bg-content/10 hover:text-content"
            >
              Edit
            </button>
          </>
        );
      case "status": {
        const name =
          boardColumns.find((other) => other.id === card.columnId)?.name ?? "";
        return (
          <MenuButton
            label={`Status of #${card.number}`}
            width={180}
            className="max-w-full truncate rounded px-1 text-left hover:bg-content/10"
            trigger={name}
          >
            {(close) =>
              boardColumns.map((other) => (
                <MenuItem
                  key={other.id}
                  checked={other.id === card.columnId}
                  onSelect={() => {
                    close();
                    if (other.id !== card.columnId) onSetStatus(card, other.id);
                  }}
                >
                  {other.name}
                </MenuItem>
              ))
            }
          </MenuButton>
        );
      }
      case "priority":
        return (
          <MenuButton
            label={`Priority of #${card.number}`}
            width={160}
            className="rounded px-1 text-left hover:bg-content/10"
            trigger={priorityName(scheme, mine?.priority ?? 0)}
          >
            {(close) => (
              <PriorityItems
                priority={mine?.priority ?? 0}
                onChoose={(priority) => onSetPriority(card, priority)}
                close={close}
              />
            )}
          </MenuButton>
        );
      case "due":
        return editing === "due" ? (
          editInput(card, "due")
        ) : (
          <button
            type="button"
            aria-label={`Edit due date of #${card.number}`}
            onClick={(event) => {
              event.stopPropagation();
              startEdit(card, "due");
            }}
            onKeyDown={(event) => event.stopPropagation()}
            className="truncate rounded px-1 text-left hover:bg-content/10"
          >
            {mine?.dueAt != null ? dueChip(mine.dueAt).text : "—"}
          </button>
        );
      case "labels": {
        const held = mine?.labels ?? [];
        const shown = labels.filter((label) => held.includes(label.id));
        return (
          <MenuButton
            label={`Labels of #${card.number}`}
            width={240}
            className="flex min-w-0 max-w-full gap-1 truncate rounded px-1 text-left hover:bg-content/10"
            trigger={shown.length ? shown.map((l) => l.name).join(", ") : "—"}
          >
            {(close) => (
              <LabelPicker
                labels={labels}
                selected={held}
                onToggle={(id) => onSetLabels(card, toggled(held, id))}
                onCreate={(name) => onCreateLabel(card, name)}
                close={close}
              />
            )}
          </MenuButton>
        );
      }
      case "start":
        return <span className="truncate">{day(row?.startAt)}</span>;
      case "projects":
        return (
          <span className="truncate">
            {(mine?.projects ?? [])
              .map(
                (path) =>
                  projects.find((project) => project.path === path)?.name ??
                  path,
              )
              .join(", ")}
          </span>
        );
      case "estimate":
        return (
          <span className="truncate">
            {row?.estimateMinutes != null
              ? formatEstimate(row.estimateMinutes)
              : ""}
          </span>
        );
      case "created":
        return <span className="truncate">{day(card.createdAt)}</span>;
      case "updated":
        return <span className="truncate">{day(card.updatedAt)}</span>;
      case "agent:provider":
      case "agent:model":
      case "agent:cost":
      case "agent:runTime": {
        const key = column.id.slice("agent:".length) as AgentField;
        return (
          <span data-agent-field={key} className="truncate">
            {agent?.(card.id)[key] ?? ""}
          </span>
        );
      }
      default: {
        const field = fields.find((found) => `field:${found.id}` === column.id);
        const raw = field ? row?.fields[field.id] : undefined;
        return (
          <span className="truncate">
            {field && raw !== undefined ? (fieldText(field.kind, raw) ?? "") : ""}
          </span>
        );
      }
    }
  };

  return (
    <div
      ref={win.ref}
      onScroll={win.onScroll}
      data-board-table=""
      role="grid"
      aria-label="Cards"
      aria-rowcount={cards.length + 1}
      className="h-full min-h-0 overflow-auto"
    >
      <div style={{ width, minWidth: "100%" }}>
        <div
          role="row"
          aria-rowindex={1}
          className="sticky top-0 z-10 flex border-b border-stroke bg-background-base text-[11px] font-medium text-content/55"
          style={{ height: ROW_HEIGHT }}
        >
          {columns.map((column) => (
            <div
              key={column.id}
              role="columnheader"
              data-table-column={column.id}
              style={{ width: column.width }}
              className="relative flex shrink-0 items-center truncate px-3"
            >
              <span className="truncate">{column.label}</span>
              <ResizeHandle
                label={column.label}
                width={column.width}
                onResize={(next) =>
                  onResize(column.id, Math.max(MIN_COLUMN, Math.round(next)))
                }
              />
            </div>
          ))}
        </div>
        <div style={{ height: win.totalHeight, position: "relative" }}>
          {rowsWithPinned(win.start, win.end, pinned).map((at) => {
            const card = cards[at];
            const isSelected = selected.has(card.id);
            return (
              <div
                key={card.id}
                role="row"
                data-table-row={card.id}
                data-selected={isSelected ? "" : undefined}
                aria-selected={isSelected}
                aria-rowindex={at + 2}
                aria-label={`Card #${card.number}, ${card.title}`}
                tabIndex={0}
                onClick={(event) => onRowClick(event, card)}
                onKeyDown={(event: ReactKeyboardEvent<HTMLElement>) => {
                  if (
                    event.key === "Enter" &&
                    event.target === event.currentTarget
                  ) {
                    event.preventDefault();
                    onRowClick(event as unknown as ReactMouseEvent<HTMLElement>, card);
                  }
                }}
                style={{
                  position: "absolute",
                  top: at * ROW_HEIGHT,
                  height: ROW_HEIGHT,
                  width: "100%",
                }}
                className={`flex cursor-pointer items-center border-b border-stroke/60 text-[13px] outline-none hover:bg-content/5 focus-visible:ring-2 focus-visible:ring-accent/60 ${
                  isSelected ? "bg-accent/10" : ""
                }`}
              >
                {columns.map((column) => (
                  <div
                    key={column.id}
                    role="gridcell"
                    data-table-cell={column.id}
                    style={{ width: column.width }}
                    className="flex min-w-0 shrink-0 items-center gap-1 px-3"
                  >
                    {cell(card, column)}
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

/** The drag handle on a column's right edge; Left and Right resize it by 10 px from the keyboard. */
function ResizeHandle({
  label,
  width,
  onResize,
}: {
  label: string;
  width: number;
  onResize: (width: number) => void;
}) {
  const drag = useRef<{ x: number; width: number } | null>(null);
  const resize = useRef(onResize);
  resize.current = onResize;
  useEffect(() => {
    const move = (event: MouseEvent | PointerEvent) => {
      const start = drag.current;
      if (start) resize.current(start.width + event.clientX - start.x);
    };
    const stop = () => {
      drag.current = null;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
    };
  }, []);
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={`Resize ${label}`}
      aria-valuenow={width}
      aria-valuemin={MIN_COLUMN}
      tabIndex={0}
      onClick={(event) => event.stopPropagation()}
      onPointerDown={(event: ReactPointerEvent) => {
        event.preventDefault();
        drag.current = { x: event.clientX, width };
      }}
      onKeyDown={(event) => {
        if (event.key === "ArrowLeft") {
          event.preventDefault();
          onResize(width - STEP);
        } else if (event.key === "ArrowRight") {
          event.preventDefault();
          onResize(width + STEP);
        }
      }}
      className="absolute right-0 top-0 h-full w-1.5 cursor-col-resize hover:bg-accent/50 focus-visible:bg-accent/60 focus-visible:outline-none"
    />
  );
}
