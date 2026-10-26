import type { Board, BoardSettings, BoardView } from "../../model/board";
import { BOARD_ICONS } from "../../model/boardSettings";
import {
  CommitInput,
  FIELD,
  SettingsSection,
  Swatches,
} from "./settingsControls";

/** Name, icon, colour and default view. Each choice writes only its own setting. */
export function GeneralSection({
  board,
  settings,
  views,
  onRename,
  onPatch,
  onError,
}: {
  board: Board;
  settings: BoardSettings | null;
  views: readonly BoardView[];
  onRename: (name: string) => void;
  onPatch: (patch: Partial<BoardSettings>, what: string) => void;
  onError: (message: string) => void;
}) {
  const ordered = [...views].sort((a, b) => a.position - b.position);
  // A default that was deleted reads as "All cards".
  const defaultView = ordered.some(
    (view) => view.id === settings?.defaultViewId,
  )
    ? (settings?.defaultViewId ?? "")
    : "";
  return (
    <SettingsSection title="General">
      <label className="flex items-center gap-3 text-[12px] text-content/70">
        <span className="w-28 shrink-0">Name</span>
        <CommitInput
          label="Board name"
          value={board.name}
          className="w-64"
          onCommit={(text) => {
            const name = text.trim();
            if (!name) {
              onError("A board needs a name");
              return false;
            }
            if (name !== board.name) onRename(name);
          }}
        />
      </label>
      <div className="flex items-center gap-3 text-[12px] text-content/70">
        <span className="w-28 shrink-0">Icon</span>
        <div
          role="radiogroup"
          aria-label="Icon"
          className="flex flex-wrap gap-1"
        >
          {BOARD_ICONS.map((icon) => (
            <button
              key={icon}
              type="button"
              role="radio"
              aria-checked={settings?.icon === icon}
              aria-label={icon}
              onClick={() => onPatch({ icon }, "set the icon")}
              className={`h-7 rounded-md border px-2 text-[12px] capitalize outline-none focus-visible:ring-2 focus-visible:ring-accent/60 ${
                settings?.icon === icon
                  ? "border-accent/60 bg-content/10 text-content"
                  : "border-content/15 text-content/70 hover:bg-content/10"
              }`}
            >
              {icon}
            </button>
          ))}
        </div>
      </div>
      <div className="flex items-center gap-3 text-[12px] text-content/70">
        <span className="w-28 shrink-0">Colour</span>
        <Swatches
          none
          label="Colour"
          value={settings?.color ?? null}
          onChange={(color) => onPatch({ color }, "set the colour")}
        />
      </div>
      <label className="flex items-center gap-3 text-[12px] text-content/70">
        <span className="w-28 shrink-0">Default view</span>
        <select
          aria-label="Default view"
          value={defaultView}
          onChange={(event) =>
            onPatch(
              { defaultViewId: event.target.value || null },
              "set the default view",
            )
          }
          className={`${FIELD} w-48`}
        >
          <option value="">All cards</option>
          {ordered.map((view) => (
            <option key={view.id} value={view.id}>
              {view.name}
            </option>
          ))}
        </select>
      </label>
    </SettingsSection>
  );
}
