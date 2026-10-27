import { useEffect, useState } from "react";
import { Plus } from "../../../shared/ui/icons";
import {
  columnCardCount,
  type BoardColumn,
  type ColumnRole,
} from "../model/board";
import { wipCount } from "../model/boardSettings";
import type { SortKey } from "../model/columnActions";
import {
  ICON_BUTTON,
  MenuButton,
  MenuItem,
  MenuLabel,
  MenuSeparator,
  NameInput,
  noDrag,
} from "./boardControls";
import { ROLE_LABEL, ROLES } from "./boardFormat";
import { ColumnRoleIcon } from "./kanban/ColumnRoleIcon";

/**
 * Contents of a column's 36px header row: role icon, name and count, with
 * add-card and the column menu revealed on hover or focus.
 */
export function BoardColumnHeader({
  column,
  count,
  shown,
  takenRoles,
  onAddCard,
  onRename,
  onSetRole,
  onDelete,
  onCollapse,
  onSort,
  onMarkAllDone,
  onArchiveCompleted,
  heldCards = 0,
  moveTargets = [],
  onMoveAndDelete,
}: {
  /** The board's other columns, which a Delete can move this one's cards to. */
  moveTargets?: readonly { id: string; name: string }[];
  /** Moves the column's cards to the end of `targetId`, then deletes it. */
  onMoveAndDelete?: (targetId: string) => void;
  column: BoardColumn;
  /** Every card in the column, whatever a search or filter hides. */
  count: number;
  /** Cards a search or filter leaves showing; set only while one is active. */
  shown?: number;
  /** Roles held by the board's other columns. */
  takenRoles: ReadonlySet<ColumnRole>;
  onAddCard: () => void;
  onRename: (name: string) => void;
  onSetRole: (role: ColumnRole | null) => void;
  onDelete: () => void;
  onCollapse: () => void;
  onSort: (key: SortKey) => void;
  /** Absent in the done column, and when the board has none. */
  onMarkAllDone?: () => void;
  /** Present in the done column only. */
  onArchiveCompleted?: () => void;
  /**
   * Archived cards here whose delete the undo toast is holding: they go
   * before the column does, so they don't block Delete column.
   */
  heldCards?: number;
}) {
  const [renaming, setRenaming] = useState(false);
  const wip = wipCount({ total: count, shown, limit: column.wipLimit ?? null });
  return (
    <>
      <ColumnRoleIcon role={column.role} className="shrink-0" />
      {renaming ? (
        <NameInput
          initial={column.name}
          label="Column name"
          className="flex-1"
          onSubmit={(name) => {
            setRenaming(false);
            onRename(name);
          }}
          onCancel={() => setRenaming(false)}
        />
      ) : (
        <span
          data-column-name=""
          className="min-w-0 truncate text-[13px] font-medium text-content"
        >
          {column.name}
        </span>
      )}
      <span
        data-column-count=""
        data-wip-over={wip.over ? "" : undefined}
        title={
          column.wipLimit == null
            ? undefined
            : wip.over
              ? `Over the WIP limit of ${column.wipLimit}`
              : `WIP limit ${column.wipLimit}`
        }
        className={`shrink-0 text-[12px] tabular-nums ${
          wip.over ? "font-medium text-red-400" : "text-content/45"
        }`}
      >
        {wip.text}
      </span>
      <div className="ml-auto flex items-center opacity-0 group-hover/column-header:opacity-100 focus-within:opacity-100 has-[[data-menu-open]]:opacity-100">
        <button
          type="button"
          aria-label={`Add card to ${column.name}`}
          title="Add card"
          className={ICON_BUTTON}
          {...noDrag}
          onClick={(event) => {
            event.stopPropagation();
            onAddCard();
          }}
        >
          <Plus className="size-3.5" strokeWidth={1.75} />
        </button>
        <MenuButton label={`Column actions for ${column.name}`}>
          {(close) => (
            <>
              <MenuItem
                onSelect={() => {
                  close();
                  setRenaming(true);
                }}
              >
                Rename
              </MenuItem>
              <MenuItem
                onSelect={() => {
                  close();
                  onCollapse();
                }}
              >
                Collapse column
              </MenuItem>
              <MenuSeparator />
              <MenuItem
                disabled={count < 2}
                onSelect={() => {
                  close();
                  onSort("priority");
                }}
              >
                Sort by priority
              </MenuItem>
              <MenuItem
                disabled={count < 2}
                onSelect={() => {
                  close();
                  onSort("due");
                }}
              >
                Sort by due date
              </MenuItem>
              {onMarkAllDone ? (
                <MenuItem
                  disabled={count === 0}
                  onSelect={() => {
                    close();
                    onMarkAllDone();
                  }}
                >
                  Mark all as done
                </MenuItem>
              ) : null}
              {onArchiveCompleted ? (
                <MenuItem
                  disabled={count === 0}
                  onSelect={() => {
                    close();
                    onArchiveCompleted();
                  }}
                >
                  Archive completed
                </MenuItem>
              ) : null}
              <MenuSeparator />
              <MenuLabel>Role</MenuLabel>
              {[...ROLES, null].map((role) => (
                <MenuItem
                  key={role ?? "none"}
                  checked={column.role === role}
                  disabled={role !== null && takenRoles.has(role)}
                  title={
                    role !== null && takenRoles.has(role)
                      ? "Another column has this role"
                      : undefined
                  }
                  onSelect={() => {
                    close();
                    if (column.role !== role) onSetRole(role);
                  }}
                >
                  <ColumnRoleIcon role={role} className="shrink-0" />
                  {role ? ROLE_LABEL[role] : "No role"}
                </MenuItem>
              ))}
              <MenuSeparator />
              <DeleteColumnItem
                columnId={column.id}
                held={heldCards}
                moveTargets={onMoveAndDelete ? moveTargets : []}
                onSelect={() => {
                  close();
                  onDelete();
                }}
                onMoveAndDelete={(targetId) => {
                  close();
                  onMoveAndDelete?.(targetId);
                }}
              />
            </>
          )}
        </MenuButton>
      </div>
    </>
  );
}

/**
 * "Delete column", allowed only when the column holds no cards, live or
 * archived. A column holding only live cards offers "Move cards to…" each
 * other column instead, as the settings page does. Mounted when the menu
 * opens, so the counts are read then.
 */
function DeleteColumnItem({
  columnId,
  held,
  moveTargets,
  onSelect,
  onMoveAndDelete,
}: {
  columnId: string;
  held: number;
  moveTargets: readonly { id: string; name: string }[];
  onSelect: () => void;
  onMoveAndDelete: (targetId: string) => void;
}) {
  const [counts, setCounts] = useState<
    { live: number; archived: number } | "failed" | null
  >(null);
  useEffect(() => {
    let live = true;
    columnCardCount(columnId)
      .then(
        (count) =>
          live &&
          setCounts({
            live: count.live,
            // Held deletes go before the column does, so they don't block it.
            archived: Math.max(0, count.archived - held),
          }),
      )
      .catch(() => live && setCounts("failed"));
    return () => {
      live = false;
    };
  }, [columnId, held]);
  const total =
    counts === null || counts === "failed"
      ? counts
      : counts.live + counts.archived;
  const reason =
    total === "failed"
      ? "Couldn't check the column's cards"
      : typeof total === "number" && total > 0
        ? `Move or delete its ${total} ${total === 1 ? "card" : "cards"} first`
        : undefined;
  const canMove =
    typeof counts === "object" &&
    counts !== null &&
    counts.live > 0 &&
    counts.archived === 0 &&
    moveTargets.length > 0;
  return (
    <>
      <MenuItem
        danger
        disabled={total !== 0}
        title={reason}
        onSelect={onSelect}
      >
        Delete column
      </MenuItem>
      {canMove ? (
        <>
          <MenuLabel>Move cards to…</MenuLabel>
          {moveTargets.map((target) => (
            <MenuItem
              key={target.id}
              danger
              title={`Move the cards to ${target.name}, then delete the column`}
              onSelect={() => onMoveAndDelete(target.id)}
            >
              {target.name}
            </MenuItem>
          ))}
        </>
      ) : null}
    </>
  );
}
