import type { BoardColumn, BoardLabel } from "../model/board";
import { LabelDot } from "./CardDrawerParts";
import { MenuButton, MenuItem } from "./boardControls";
import { PriorityItems } from "./CardPropertyMenus";

const BTN =
  "inline-flex h-7 items-center rounded-md px-2.5 text-[12px] text-content/85 hover:bg-content/10 hover:text-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/60";

/** The floating bar over a bulk selection. */
export function SelectionBar({
  count,
  columns,
  labels,
  labelCounts,
  onStatus,
  onPriority,
  onLabel,
  onArchive,
  onDelete,
  onClear,
}: {
  count: number;
  columns: readonly Pick<BoardColumn, "id" | "name">[];
  labels: readonly BoardLabel[];
  /** How many selected cards hold each label (by label id). */
  labelCounts: ReadonlyMap<string, number>;
  onStatus: (columnId: string) => void;
  onPriority: (priority: number) => void;
  /** Adds the label to every selected card, or removes it from all (`on` false). */
  onLabel: (labelId: string, on: boolean) => void;
  onArchive: () => void;
  onDelete: () => void;
  onClear: () => void;
}) {
  return (
    <div
      role="toolbar"
      aria-label="Selected cards"
      data-selection-bar=""
      className="pointer-events-auto absolute bottom-16 left-1/2 z-20 flex -translate-x-1/2 items-center gap-1 rounded-xl border border-stroke bg-background-base px-2 py-1.5 shadow-lg"
    >
      <span
        data-selection-count=""
        className="px-2 text-[12px] font-medium tabular-nums text-content"
      >
        {count} selected
      </span>
      <MenuButton label="Status for selected" className={BTN} trigger="Status…">
        {(close) => (
          <>
            {columns.map((column) => (
              <MenuItem
                key={column.id}
                onSelect={() => {
                  close();
                  onStatus(column.id);
                }}
              >
                {column.name}
              </MenuItem>
            ))}
          </>
        )}
      </MenuButton>
      <MenuButton
        label="Priority for selected"
        className={BTN}
        trigger="Priority…"
        width={160}
      >
        {(close) => (
          <PriorityItems priority={-1} onChoose={onPriority} close={close} />
        )}
      </MenuButton>
      <MenuButton
        label="Labels for selected"
        className={BTN}
        trigger="Labels…"
        width={220}
      >
        {(close) =>
          labels.length ? (
            labels.map((label) => {
              const all = labelCounts.get(label.id) === count;
              return (
                <MenuItem
                  key={label.id}
                  kind="checkbox"
                  checked={all}
                  onSelect={() => {
                    close();
                    onLabel(label.id, !all);
                  }}
                >
                  <LabelDot color={label.color} />
                  <span className="min-w-0 truncate">{label.name}</span>
                </MenuItem>
              );
            })
          ) : (
            <MenuItem disabled onSelect={() => undefined}>
              No labels yet
            </MenuItem>
          )
        }
      </MenuButton>
      <button type="button" className={BTN} onClick={onArchive}>
        Archive
      </button>
      <button
        type="button"
        className={`${BTN} text-red-300/90 hover:bg-red-500/15 hover:text-red-300`}
        onClick={onDelete}
      >
        Delete
      </button>
      <button type="button" className={BTN} onClick={onClear}>
        Clear
      </button>
    </div>
  );
}
