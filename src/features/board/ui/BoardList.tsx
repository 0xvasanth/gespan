import { useMemo, type MouseEvent } from "react";
import { ChevronDown, ChevronRight } from "../../../shared/ui/icons";
import {
  PRIORITIES,
  type BoardCard,
  type BoardColumn,
  type BoardLabel,
  type CardExtras,
} from "../model/board";
import {
  ROW_HEIGHT,
  buildGroups,
  type Group,
  type GroupBy,
} from "../model/layout";
import { useWindowedRows } from "../model/windowing";
import { dueChip } from "./boardFormat";

type Row =
  | { kind: "header"; key: string; group: Group; collapsed: boolean }
  | { kind: "card"; key: string; group: Group; card: BoardCard };

/**
 * The List layout: cards under collapsible groups, one fixed-height row each,
 * only the rows in view rendered. A card in two groups (two labels) has a row
 * in each; row keys include the group, so they stay unique.
 */
export function BoardList({
  cards,
  columns,
  extras,
  labels,
  projects,
  groupBy,
  collapsed,
  selected,
  onToggleGroup,
  onRowClick,
}: {
  /** The cards shown, in display order. */
  cards: readonly BoardCard[];
  columns: readonly BoardColumn[];
  extras: Readonly<Record<string, CardExtras | undefined>>;
  labels: readonly BoardLabel[];
  projects: readonly { path: string; name: string }[];
  groupBy: GroupBy;
  collapsed: readonly string[];
  selected: ReadonlySet<string>;
  onToggleGroup: (key: string) => void;
  /** A click on a card row: the board decides between selecting and opening. */
  onRowClick: (event: MouseEvent<HTMLElement>, card: BoardCard) => void;
}) {
  const rows = useMemo(() => {
    const byId = new Map(cards.map((card) => [card.id, card]));
    const groups = buildGroups({ by: groupBy, cards, columns, extras, labels, projects });
    const out: Row[] = [];
    for (const group of groups) {
      const closed = collapsed.includes(group.key);
      out.push({
        kind: "header",
        key: `group:${group.key}`,
        group,
        collapsed: closed,
      });
      if (closed) continue;
      for (const id of group.cardIds) {
        out.push({
          kind: "card",
          key: `${group.key}/${id}`,
          group,
          card: byId.get(id)!,
        });
      }
    }
    return out;
  }, [cards, columns, extras, labels, projects, groupBy, collapsed]);

  const win = useWindowedRows({ count: rows.length, rowHeight: ROW_HEIGHT });
  const columnName = (id: string) =>
    columns.find((column) => column.id === id)?.name ?? "";

  return (
    <div
      ref={win.ref}
      onScroll={win.onScroll}
      data-board-list=""
      role="group"
      aria-label="Cards"
      className="h-full min-h-0 overflow-y-auto"
    >
      <div style={{ height: win.totalHeight, position: "relative" }}>
        {rows.slice(win.start, win.end).map((row, offset) => {
          const style = {
            position: "absolute" as const,
            top: (win.start + offset) * ROW_HEIGHT,
            height: ROW_HEIGHT,
            left: 0,
            right: 0,
          };
          if (row.kind === "header") {
            const Chevron = row.collapsed ? ChevronRight : ChevronDown;
            return (
              <div key={row.key} style={style}>
                <button
                  type="button"
                  data-list-group={row.group.key}
                  aria-expanded={!row.collapsed}
                  onClick={() => onToggleGroup(row.group.key)}
                  className="flex h-full w-full items-center gap-1.5 border-b border-stroke bg-background-base/60 px-4 text-left text-[12px] font-medium text-content/80 hover:bg-content/5"
                >
                  <Chevron className="size-3.5 shrink-0" strokeWidth={1.75} />
                  <span className="truncate">{row.group.label}</span>
                  <span className="tabular-nums text-content/45">
                    {row.group.cardIds.length}
                  </span>
                </button>
              </div>
            );
          }
          const { card } = row;
          const mine = extras[card.id];
          const due = mine?.dueAt != null ? dueChip(mine.dueAt) : null;
          const isSelected = selected.has(card.id);
          return (
            <div
              key={row.key}
              style={style}
              role="button"
              data-list-row={row.key}
              data-card-id={card.id}
              data-selected={isSelected ? "" : undefined}
              aria-pressed={isSelected}
              aria-label={`Card #${card.number}, ${card.title}`}
              tabIndex={0}
              onClick={(event) => onRowClick(event, card)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && event.target === event.currentTarget) {
                  event.preventDefault();
                  onRowClick(event as unknown as MouseEvent<HTMLElement>, card);
                }
              }}
              className={`flex cursor-pointer items-center gap-3 border-b border-stroke/60 px-4 text-[13px] outline-none hover:bg-content/5 focus-visible:ring-2 focus-visible:ring-accent/60 ${
                isSelected ? "bg-accent/10" : ""
              }`}
            >
              <span className="w-10 shrink-0 font-mono text-[11px] text-content/45">
                #{card.number}
              </span>
              <span className="min-w-0 flex-1 truncate">{card.title}</span>
              {groupBy !== "status" ? (
                <span className="shrink-0 text-[11px] text-content/55">
                  {columnName(card.columnId)}
                </span>
              ) : null}
              {mine?.priority && groupBy !== "priority" ? (
                <span className="shrink-0 text-[11px] text-content/70">
                  {PRIORITIES[mine.priority]}
                </span>
              ) : null}
              {due && groupBy !== "due" ? (
                <span className="shrink-0 text-[11px] text-content/60">
                  {due.text}
                </span>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
