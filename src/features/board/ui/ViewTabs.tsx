import { useState } from "react";
import { Plus } from "../../../shared/ui/icons";
import type { BoardView } from "../model/board";
import { MenuButton, MenuItem, NameInput } from "./boardControls";

const TAB =
  "inline-flex h-6 shrink-0 items-center gap-1 rounded-md px-2 text-[12px] text-content/65 hover:bg-content/10 hover:text-content aria-selected:bg-content/10 aria-selected:text-content";

const SMALL =
  "h-6 shrink-0 rounded-md border border-content/15 px-2 text-[12px] text-content/80 hover:border-content/30 hover:bg-content/10 hover:text-content";

type Props = {
  /** The saved views, in tab order. "All cards" is not one of them. */
  views: readonly BoardView[];
  /** The saved view showing; null is "All cards". */
  activeId: string | null;
  /** The showing saved view has unsaved changes. */
  dirty: boolean;
  onSelect: (id: string | null) => void;
  onSaveAs: (name: string) => void;
  onSaveChanges: () => void;
  onRevert: () => void;
  onRename: (id: string, name: string) => void;
  onDuplicate: (id: string) => void;
  onDelete: (id: string) => void;
};

/**
 * The tab strip above the columns: the built-in "All cards", then the saved
 * views. A saved view with unsaved changes shows "•" and Save / Revert.
 */
export function ViewTabs({
  views,
  activeId,
  dirty,
  onSelect,
  onSaveAs,
  onSaveChanges,
  onRevert,
  onRename,
  onDuplicate,
  onDelete,
}: Props) {
  const [naming, setNaming] = useState<"save" | { rename: string } | null>(
    null,
  );
  return (
    <div
      data-view-tabs=""
      className="flex h-9 shrink-0 items-center gap-1 overflow-x-auto border-b border-stroke px-4"
    >
      <div
        role="tablist"
        aria-label="Views"
        className="flex items-center gap-1"
      >
        <button
          type="button"
          role="tab"
          aria-selected={activeId === null}
          data-view-tab="all"
          className={TAB}
          onClick={() => onSelect(null)}
        >
          All cards
        </button>
        {views.map((view) => {
          const selected = view.id === activeId;
          if (
            typeof naming === "object" &&
            naming &&
            naming.rename === view.id
          ) {
            return (
              <NameInput
                key={view.id}
                initial={view.name}
                label="View name"
                className="w-36"
                onSubmit={(name) => {
                  setNaming(null);
                  onRename(view.id, name);
                }}
                onCancel={() => setNaming(null)}
              />
            );
          }
          return (
            <span key={view.id} className="inline-flex items-center">
              <button
                type="button"
                role="tab"
                aria-selected={selected}
                data-view-tab={view.id}
                className={TAB}
                onClick={() => onSelect(view.id)}
              >
                {view.name}
                {selected && dirty ? (
                  <span aria-label="Unsaved changes" data-view-dot="">
                    •
                  </span>
                ) : null}
              </button>
              <MenuButton label={`View options for ${view.name}`}>
                {(close) => (
                  <>
                    <MenuItem
                      onSelect={() => {
                        close();
                        setNaming({ rename: view.id });
                      }}
                    >
                      Rename
                    </MenuItem>
                    <MenuItem
                      onSelect={() => {
                        close();
                        onDuplicate(view.id);
                      }}
                    >
                      Duplicate
                    </MenuItem>
                    <MenuItem
                      danger
                      onSelect={() => {
                        close();
                        onDelete(view.id);
                      }}
                    >
                      Delete
                    </MenuItem>
                  </>
                )}
              </MenuButton>
            </span>
          );
        })}
      </div>
      {activeId && dirty ? (
        <>
          <button type="button" className={SMALL} onClick={onSaveChanges}>
            Save
          </button>
          <button type="button" className={SMALL} onClick={onRevert}>
            Revert
          </button>
        </>
      ) : null}
      <div className="flex-1" />
      {naming === "save" ? (
        <NameInput
          label="View name"
          placeholder="View name"
          className="w-40"
          onSubmit={(name) => {
            setNaming(null);
            onSaveAs(name);
          }}
          onCancel={() => setNaming(null)}
        />
      ) : (
        <button
          type="button"
          className={`${SMALL} inline-flex items-center gap-1`}
          onClick={() => setNaming("save")}
        >
          <Plus className="size-3" strokeWidth={1.75} />
          Save view
        </button>
      )}
    </div>
  );
}
