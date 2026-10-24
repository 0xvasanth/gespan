import type { ExternalEditor } from "../../platform/tauri/fs";
import type { ExplorerMenuItem } from "../../features/files/ui/ExplorerMenu";
import { isLocalProject } from "../../features/projects/model/recents";
import { IS_MAC, IS_WIN } from "../../platform/tauri/platform";

export const REVEAL_LABEL = IS_MAC
  ? "Reveal in Finder"
  : IS_WIN
    ? "Reveal in File Explorer"
    : "Open Containing Folder";

export const LAST_EXTERNAL_EDITOR_KEY = "gespan.lastExternalEditor";

export const EDITOR_ITEM_PREFIX = "editor:";

export function loadLastExternalEditor(): string | null {
  try {
    return localStorage.getItem(LAST_EXTERNAL_EDITOR_KEY);
  } catch {
    return null;
  }
}

export function saveLastExternalEditor(id: string): void {
  try {
    localStorage.setItem(LAST_EXTERNAL_EDITOR_KEY, id);
  } catch {}
}

/** Only a folder on this computer can open in one of its editors. */
export function openInEditorFolder(
  cwd: string | undefined,
): string | undefined {
  return cwd && isLocalProject(cwd) ? cwd : undefined;
}

/**
 * The editors to offer, the last one used first and checked; the rest keep
 * the scan's order. `null` while the scan is still running. A failed scan
 * with nothing found earlier says so rather than claiming there are none.
 */
export function openInEditorMenuItems(
  editors: ExternalEditor[] | null,
  lastEditorId: string | null,
  scanFailed = false,
): ExplorerMenuItem[] {
  const tail: ExplorerMenuItem[] = [
    { kind: "sep" },
    { kind: "item", id: "reveal", label: REVEAL_LABEL },
    { kind: "item", id: "copy-path", label: "Copy path" },
  ];
  if (editors === null) {
    return [
      {
        kind: "item",
        id: "editor-loading",
        label: "Looking for editors…",
        disabled: true,
      },
      ...tail,
    ];
  }
  if (editors.length === 0) {
    return [
      scanFailed
        ? {
            kind: "item",
            id: "editor-scan-failed",
            label: "Couldn't look for editors",
            description: "Open this menu again to retry.",
            disabled: true,
          }
        : {
            kind: "item",
            id: "editor-none",
            label: "No supported editors found",
            disabled: true,
          },
      ...tail,
    ];
  }
  const last = editors.find((editor) => editor.id === lastEditorId);
  const ordered = last
    ? [last, ...editors.filter((editor) => editor !== last)]
    : editors;
  return [
    ...ordered.map((editor) => ({
      kind: "item" as const,
      id: `${EDITOR_ITEM_PREFIX}${editor.id}`,
      label: editor.name,
      checked: editor === last,
    })),
    ...tail,
  ];
}
