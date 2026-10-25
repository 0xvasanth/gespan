import { useState } from "react";
import { copyText } from "../../../platform/tauri/clipboard";
import { ChevronLeft } from "../../../shared/ui/icons";
import type { Board, BoardCard, BoardLabel } from "../model/board";
import { MenuItem, MenuSeparator } from "./boardControls";
import { LabelPicker, toggled } from "./CardDrawerParts";
import { PriorityItems } from "./CardPropertyMenus";
import { errorText } from "./boardFormat";

/** What the More menu does; the board supplies these. */
export type MoreActions = {
  open: () => void;
  setStatus: (columnId: string, cardCount: number) => void;
  setPriority: (priority: number) => void;
  setLabels: (labelIds: string[]) => void;
  createLabel: (name: string) => void;
  moveToBoard: (boardId: string) => void;
  duplicate: () => void;
  archive: () => void;
  remove: () => void;
  onError: (message: string) => void;
};

const COPY_LABEL_TITLE = 40;

/** The link "Copy link" writes; routing it inside the app comes later. */
export function cardLink(card: Pick<BoardCard, "boardId" | "id">): string {
  return `gespan://board/${card.boardId}/card/${card.id}`;
}

/** What "Copy #N title" puts on the clipboard. */
export function cardReference(
  card: Pick<BoardCard, "number" | "title">,
): string {
  return `#${card.number} ${card.title}`;
}

/** The menu row for the reference, with a long title cut short. */
export function copyLabel(number: number, title: string): string {
  const trimmed = title.trim();
  const chars = [...trimmed];
  const shown =
    chars.length > COPY_LABEL_TITLE
      ? `${chars.slice(0, COPY_LABEL_TITLE).join("").trimEnd()}…`
      : trimmed;
  return `Copy #${number} ${shown}`;
}

type View = "main" | "status" | "priority" | "labels" | "boards";

/**
 * The More menu: Open, the quick pickers, Move to board…, the two copies,
 * Duplicate, Archive and Delete. Status…, Priority…, Labels… and Move to
 * board… swap this menu's content for the matching list.
 */
export function CardMoreItems({
  card,
  columns,
  labels,
  labelIds,
  priority,
  boards,
  actions,
  close,
}: {
  card: BoardCard;
  columns: readonly { id: string; name: string; cardCount: number }[];
  labels: readonly BoardLabel[];
  labelIds: readonly string[];
  priority: number;
  boards: readonly Board[];
  actions: MoreActions;
  close: () => void;
}) {
  const [view, setView] = useState<View>("main");
  const done = (run: () => void) => () => {
    close();
    run();
  };
  const copy = (text: string, what: string) => () => {
    close();
    copyText(text).catch((cause) =>
      actions.onError(
        `Couldn't copy the ${what} of card #${card.number}: ${errorText(cause)}`,
      ),
    );
  };
  if (view !== "main") {
    const others = boards.filter((board) => board.id !== card.boardId);
    return (
      <>
        <MenuItem onSelect={() => setView("main")}>
          <ChevronLeft className="size-3.5 shrink-0" strokeWidth={1.75} />
          Back
        </MenuItem>
        <MenuSeparator />
        {view === "status"
          ? columns.map((column) => (
              <MenuItem
                key={column.id}
                checked={column.id === card.columnId}
                onSelect={() => {
                  close();
                  if (column.id !== card.columnId) {
                    actions.setStatus(column.id, column.cardCount);
                  }
                }}
              >
                {column.name}
              </MenuItem>
            ))
          : view === "priority"
            ? (
                <PriorityItems
                  priority={priority}
                  onChoose={actions.setPriority}
                  close={close}
                />
              )
            : view === "labels"
              ? (
                  <LabelPicker
                    labels={labels}
                    selected={labelIds}
                    onToggle={(id) => actions.setLabels(toggled(labelIds, id))}
                    onCreate={actions.createLabel}
                    close={close}
                  />
                )
              : others.length
                ? others.map((board) => (
                    <MenuItem
                      key={board.id}
                      onSelect={done(() => actions.moveToBoard(board.id))}
                    >
                      <span className="min-w-0 truncate">{board.name}</span>
                    </MenuItem>
                  ))
                : (
                    <MenuItem disabled onSelect={() => undefined}>
                      No other boards
                    </MenuItem>
                  )}
      </>
    );
  }
  return (
    <>
      <MenuItem onSelect={done(actions.open)}>Open</MenuItem>
      <MenuItem onSelect={() => setView("status")}>Status…</MenuItem>
      <MenuItem onSelect={() => setView("priority")}>Priority…</MenuItem>
      <MenuItem onSelect={() => setView("labels")}>Labels…</MenuItem>
      <MenuItem onSelect={() => setView("boards")}>Move to board…</MenuItem>
      <MenuItem onSelect={copy(cardLink(card), "link")}>Copy link</MenuItem>
      <MenuItem onSelect={copy(cardReference(card), "reference")}>
        {copyLabel(card.number, card.title)}
      </MenuItem>
      <MenuItem onSelect={done(actions.duplicate)}>Duplicate</MenuItem>
      <MenuItem onSelect={done(actions.archive)}>Archive</MenuItem>
      <MenuItem danger onSelect={done(actions.remove)}>
        Delete
      </MenuItem>
    </>
  );
}
