import { useEffect, useRef } from "react";
import {
  updateBoardSettings,
  type Board,
  type BoardCard,
  type BoardColumn,
  type BoardLabel,
  type BoardSettings as Settings,
  type BoardView,
  type PriorityScheme,
} from "../../model/board";
import { errorText } from "../boardFormat";
import { ColumnsSection } from "./ColumnsSection";
import { GeneralSection } from "./GeneralSection";
import { LabelsSection } from "./LabelsSection";
import { PrioritySection } from "./PrioritySection";

/**
 * A board's settings page: General, Columns, Labels and Priority. It shows
 * what the board last read; every write goes to the store, which tells the
 * board to read again (`gespan:board-changed`).
 */
export function BoardSettings({
  board,
  settings,
  scheme,
  columns,
  cards,
  labels,
  views,
  onClose,
  onRename,
  onError,
  onSettings,
  beforeColumnDelete,
}: {
  board: Board;
  settings: Settings | null;
  scheme: PriorityScheme;
  columns: readonly BoardColumn[];
  cards: readonly BoardCard[];
  labels: readonly BoardLabel[];
  views: readonly BoardView[];
  onClose: () => void;
  onRename: (name: string) => void;
  onError: (message: string) => void;
  /** The settings a write left, so the page doesn't wait for the board to read again. */
  onSettings: (settings: Settings) => void;
  beforeColumnDelete?: () => Promise<void>;
}) {
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    root.current?.focus();
  }, []);

  /** A patch changes only the settings it names, so the other app's survive. */
  const patch = (changes: Partial<Settings>, what: string) =>
    updateBoardSettings(board.id, changes).then(onSettings, (cause) =>
      onError(`Couldn't ${what}: ${errorText(cause)}`),
    );

  return (
    <div
      ref={root}
      tabIndex={-1}
      role="region"
      aria-label="Board settings"
      data-board-settings=""
      className="flex min-h-0 flex-1 flex-col overflow-y-auto outline-none"
      onKeyDown={(event) => {
        if (event.key === "Escape" && !event.defaultPrevented) {
          event.preventDefault();
          onClose();
        }
      }}
    >
      <div className="mx-auto flex w-full max-w-2xl flex-col divide-y divide-stroke px-4 pb-10 pt-4">
        <div className="flex items-center gap-3 pb-3">
          <button
            type="button"
            aria-label="Back to board"
            onClick={onClose}
            className="inline-flex h-7 items-center rounded-md border border-content/15 px-2.5 text-[12px] text-content/80 hover:bg-content/10 hover:text-content"
          >
            ← Board
          </button>
          <h1 className="truncate text-[15px] font-medium text-content">
            {board.name} settings
          </h1>
        </div>
        <GeneralSection
          board={board}
          settings={settings}
          views={views}
          onRename={onRename}
          onPatch={patch}
          onError={onError}
        />
        <ColumnsSection
          boardId={board.id}
          columns={columns}
          cards={cards}
          onError={onError}
          beforeDelete={beforeColumnDelete}
        />
        <LabelsSection boardId={board.id} labels={labels} onError={onError} />
        <PrioritySection
          scheme={scheme}
          stored={!!settings?.priorityJson}
          onPatch={patch}
          onError={onError}
        />
      </div>
    </div>
  );
}
