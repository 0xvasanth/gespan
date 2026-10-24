import { useCallback, useEffect, useRef, useState } from "react";
import { AppWindow } from "../../shared/ui/icons";
import {
  basename,
  listExternalEditors,
  openInExternalEditor,
  revealPath,
  type ExternalEditor,
} from "../../platform/tauri/fs";
import { copyText } from "../../platform/tauri/clipboard";
import { ExplorerMenu } from "../../features/files/ui/ExplorerMenu";
import {
  EDITOR_ITEM_PREFIX,
  loadLastExternalEditor,
  openInEditorMenuItems,
  saveLastExternalEditor,
} from "./openInEditor";

const MENU_WIDTH = 232;

/**
 * The title bar's corner button: opens the focused session's folder (its
 * worktree when it has one) in an installed editor or IDE.
 */
export function OpenInEditorButton({ cwd }: { cwd: string }) {
  const buttonRef = useRef<HTMLButtonElement>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [editors, setEditors] = useState<ExternalEditor[] | null>(null);
  const [lastEditorId, setLastEditorId] = useState(loadLastExternalEditor);
  const [error, setError] = useState<string | null>(null);
  const scan = useRef(0);

  // Scan again on every open: an editor installed since launch shows up
  // without a restart. The previous list stays up while the scan runs.
  const refreshEditors = useCallback(() => {
    const id = ++scan.current;
    void listExternalEditors()
      .then((installed) => {
        if (scan.current === id)
          setEditors(Array.isArray(installed) ? installed : []);
      })
      .catch(() => {
        if (scan.current === id) setEditors((current) => current ?? []);
      });
  }, []);

  useEffect(() => {
    refreshEditors();
    return () => {
      scan.current += 1;
    };
  }, [refreshEditors]);

  useEffect(() => {
    setMenu(null);
    setError(null);
  }, [cwd]);

  const close = () => {
    setMenu(null);
    setError(null);
    buttonRef.current?.focus();
  };

  const toggle = () => {
    if (menu) {
      close();
      return;
    }
    const rect = buttonRef.current?.getBoundingClientRect();
    if (!rect) return;
    setError(null);
    setLastEditorId(loadLastExternalEditor());
    refreshEditors();
    setMenu({
      x: Math.max(8, rect.right - MENU_WIDTH),
      y: rect.bottom + 4,
    });
  };

  const fail = (cause: unknown) =>
    setError(cause instanceof Error ? cause.message : String(cause));

  const onPick = (id: string) => {
    if (id.startsWith(EDITOR_ITEM_PREFIX)) {
      const editorId = id.slice(EDITOR_ITEM_PREFIX.length);
      if (!editors?.some((editor) => editor.id === editorId)) return;
      void openInExternalEditor(editorId, cwd)
        .then(() => {
          saveLastExternalEditor(editorId);
          setLastEditorId(editorId);
          close();
        })
        .catch(fail);
      return;
    }
    if (id === "reveal") {
      void revealPath(cwd).then(close).catch(fail);
    } else if (id === "copy-path") {
      void copyText(cwd).then(close).catch(fail);
    }
  };

  const name = basename(cwd) || cwd;
  const label = `Open ${name} in editor`;
  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        title={label}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={menu != null}
        data-open-in-editor
        data-tauri-drag-region="false"
        // The open menu treats this press as outside and closes; the click
        // after it would reopen it.
        onPointerDown={(event) => {
          if (menu) event.stopPropagation();
        }}
        onClick={toggle}
        className={`grid size-6 place-items-center rounded-md ${
          menu
            ? "bg-content/10 text-content"
            : "text-content/50 hover:bg-content/10 hover:text-content"
        }`}
      >
        <AppWindow className="size-3.5" strokeWidth={1.75} />
      </button>
      {menu ? (
        <ExplorerMenu
          x={menu.x}
          y={menu.y}
          width={MENU_WIDTH}
          ariaLabel={`Open ${name} in`}
          header={
            <div className="px-2 pt-1.5 pb-1">
              <p className="truncate text-[11px] font-medium text-content/50">
                Open in
              </p>
              <p
                title={cwd}
                className="mt-0.5 truncate text-[12px] text-content/80"
              >
                {cwd}
              </p>
              {error ? (
                <p role="alert" className="mt-1 text-[11px] text-red-300">
                  {error}
                </p>
              ) : null}
            </div>
          }
          items={openInEditorMenuItems(editors, lastEditorId)}
          onPick={onPick}
          onClose={close}
        />
      ) : null}
    </>
  );
}
