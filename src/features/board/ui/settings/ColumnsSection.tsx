import { useState } from "react";
import {
  columnCardCount,
  createColumn,
  deleteColumn,
  moveCards,
  moveColumn,
  setColumnDisplay,
  updateColumn,
  type BoardCard,
  type BoardColumn,
  type ColumnRole,
} from "../../model/board";
import { parseWipLimit } from "../../model/boardSettings";
import { errorText, ROLE_LABEL, ROLES } from "../boardFormat";
import {
  CommitInput,
  FIELD,
  MoveButtons,
  SETTINGS_BUTTON,
  SettingsSection,
  Swatches,
} from "./settingsControls";

/** Add, rename, recolour, set the role and WIP limit of, reorder and delete columns. */
export function ColumnsSection({
  boardId,
  columns,
  cards,
  onError,
  beforeDelete,
}: {
  boardId: string;
  columns: readonly BoardColumn[];
  /** The board's live cards. */
  cards: readonly BoardCard[];
  onError: (message: string) => void;
  /** Runs before a column goes, e.g. to carry out a held card delete. */
  beforeDelete?: () => Promise<void>;
}) {
  const ordered = [...columns].sort((a, b) => a.position - b.position);
  const [deleting, setDeleting] = useState<string | null>(null);
  const run = (what: string, work: () => Promise<unknown>) =>
    work().then(
      () => true,
      (cause) => {
        onError(`Couldn't ${what}: ${errorText(cause)}`);
        return false;
      },
    );

  const requestDelete = async (column: BoardColumn) => {
    setDeleting(null);
    let counts: { live: number; archived: number };
    try {
      counts = await columnCardCount(column.id);
    } catch (cause) {
      onError(`Couldn't check the column's cards: ${errorText(cause)}`);
      return;
    }
    if (counts.live > 0) {
      if (ordered.length < 2) {
        onError(
          "Couldn't delete the column: add another column to move its cards to",
        );
        return;
      }
      setDeleting(column.id);
      return;
    }
    // Empty of live cards: the store still refuses while archived ones remain.
    await run("delete the column", async () => {
      await beforeDelete?.();
      await deleteColumn(column.id);
    });
  };

  const moveAndDelete = async (column: BoardColumn, targetId: string) => {
    const moving = cards
      .filter((card) => card.columnId === column.id)
      .sort((a, b) => a.position - b.position)
      .map((card) => card.id);
    const end = cards.filter((card) => card.columnId === targetId).length;
    const done = await run("delete the column", async () => {
      await beforeDelete?.();
      if (moving.length) await moveCards(moving, targetId, end, "user");
      await deleteColumn(column.id);
    });
    if (done) setDeleting(null);
  };

  return (
    <SettingsSection
      title="Columns"
      hint="A WIP limit warns when a column holds more cards than it; it never blocks a move."
    >
      <ul className="flex flex-col gap-2">
        {ordered.map((column, index) => (
          <li
            key={column.id}
            data-settings-column={column.id}
            className="flex flex-col gap-2 rounded-lg border border-stroke p-2.5"
          >
            <div className="flex flex-wrap items-center gap-2">
              <CommitInput
                label="Column name"
                value={column.name}
                className="w-44"
                onCommit={(text) => {
                  const name = text.trim();
                  if (!name) {
                    onError("A column needs a name");
                    return false;
                  }
                  if (name === column.name) return;
                  void run("rename the column", () =>
                    updateColumn(column.id, name, column.role),
                  );
                }}
              />
              <select
                aria-label="Role"
                value={column.role ?? ""}
                onChange={(event) =>
                  void run("set the column's role", () =>
                    updateColumn(
                      column.id,
                      column.name,
                      (event.target.value || null) as ColumnRole | null,
                    ),
                  )
                }
                className={`${FIELD} w-32`}
              >
                <option value="">No role</option>
                {ROLES.map((role) => (
                  <option
                    key={role}
                    value={role}
                    disabled={ordered.some(
                      (other) => other.id !== column.id && other.role === role,
                    )}
                  >
                    {ROLE_LABEL[role]}
                  </option>
                ))}
              </select>
              <label className="flex items-center gap-1.5 text-[12px] text-content/60">
                WIP
                <CommitInput
                  label="WIP limit"
                  value={column.wipLimit == null ? "" : String(column.wipLimit)}
                  placeholder="None"
                  inputMode="numeric"
                  className="w-16"
                  onCommit={(text) => {
                    const limit = parseWipLimit(text);
                    if (limit === "invalid") {
                      onError(
                        "A WIP limit is a whole number of 1 or more, or empty for none",
                      );
                      return false;
                    }
                    void run("set the WIP limit", () =>
                      setColumnDisplay(
                        column.id,
                        column.color,
                        limit,
                        column.collapsed,
                      ),
                    );
                  }}
                />
              </label>
              <div className="ml-auto flex items-center gap-1">
                <MoveButtons
                  index={index}
                  count={ordered.length}
                  onMove={(to) =>
                    void run("move the column", () => moveColumn(column.id, to))
                  }
                />
                <button
                  type="button"
                  className={`${SETTINGS_BUTTON} text-red-400`}
                  onClick={() => void requestDelete(column)}
                >
                  Delete column
                </button>
              </div>
            </div>
            <Swatches
              none
              label="Column colour"
              value={column.color}
              onChange={(color) =>
                void run("recolour the column", () =>
                  setColumnDisplay(
                    column.id,
                    color,
                    column.wipLimit ?? null,
                    column.collapsed,
                  ),
                )
              }
            />
            {deleting === column.id ? (
              <MoveCardsPanel
                column={column}
                targets={ordered.filter((other) => other.id !== column.id)}
                count={
                  cards.filter((card) => card.columnId === column.id).length
                }
                onConfirm={(targetId) => void moveAndDelete(column, targetId)}
                onCancel={() => setDeleting(null)}
              />
            ) : null}
          </li>
        ))}
      </ul>
      <div className="flex items-center gap-2">
        <CommitInput
          label="New column"
          value=""
          placeholder="New column name"
          className="w-56"
          onCommit={(text) => {
            const name = text.trim();
            if (!name) return false;
            void run("add the column", () => createColumn(boardId, name));
          }}
        />
        <span className="text-[12px] text-content/45">Press Enter to add</span>
      </div>
    </SettingsSection>
  );
}

function MoveCardsPanel({
  column,
  targets,
  count,
  onConfirm,
  onCancel,
}: {
  column: BoardColumn;
  targets: readonly BoardColumn[];
  count: number;
  onConfirm: (targetId: string) => void;
  onCancel: () => void;
}) {
  const [target, setTarget] = useState(targets[0]?.id ?? "");
  return (
    <div
      data-move-cards=""
      className="flex flex-wrap items-center gap-2 rounded-md bg-content/5 p-2 text-[12px] text-content/80"
    >
      <span>
        “{column.name}” holds {count} {count === 1 ? "card" : "cards"}. Move
        cards to…
      </span>
      <select
        aria-label="Move cards to…"
        value={target}
        onChange={(event) => setTarget(event.target.value)}
        className={`${FIELD} w-40`}
      >
        {targets.map((other) => (
          <option key={other.id} value={other.id}>
            {other.name}
          </option>
        ))}
      </select>
      <button
        type="button"
        className={SETTINGS_BUTTON}
        disabled={!target}
        onClick={() => onConfirm(target)}
      >
        Move and delete
      </button>
      <button type="button" className={SETTINGS_BUTTON} onClick={onCancel}>
        Cancel
      </button>
    </div>
  );
}
