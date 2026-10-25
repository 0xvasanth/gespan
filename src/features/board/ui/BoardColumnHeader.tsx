import { useState } from "react";
import { Plus } from "../../../shared/ui/icons";
import type { BoardColumn, ColumnRole } from "../model/board";
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
}: {
  column: BoardColumn;
  /** Every card in the column. */
  count: number;
  /** Cards a search or filter leaves showing; set only while one is active. */
  shown?: number;
  /** Roles held by the board's other columns. */
  takenRoles: ReadonlySet<ColumnRole>;
  onAddCard: () => void;
  onRename: (name: string) => void;
  onSetRole: (role: ColumnRole | null) => void;
  onDelete: () => void;
}) {
  const [renaming, setRenaming] = useState(false);
  const plural = count === 1 ? "card" : "cards";
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
        className="shrink-0 text-[12px] tabular-nums text-content/45"
      >
        {shown === undefined ? count : `${shown}/${count}`}
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
              <MenuItem
                danger
                disabled={count > 0}
                title={count > 0 ? `Column has ${count} ${plural}` : undefined}
                onSelect={() => {
                  close();
                  onDelete();
                }}
              >
                Delete column
              </MenuItem>
            </>
          )}
        </MenuButton>
      </div>
    </>
  );
}
