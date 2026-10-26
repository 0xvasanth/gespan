import { IS_MAC } from "../../../platform/tauri/platform";
import { keybindingPressed } from "../../settings/model/settings";

export const DICTATE_COMMAND = "Composer: Dictate";

/** Cmd+Shift+M on macOS, Ctrl+Shift+M elsewhere, unless rebound. */
export function isDictationShortcut(event: {
  code: string;
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}): boolean {
  const mod = IS_MAC
    ? event.metaKey && !event.ctrlKey
    : event.ctrlKey && !event.metaKey;
  return keybindingPressed(
    DICTATE_COMMAND,
    event,
    mod && event.shiftKey && !event.altKey && event.key.toLowerCase() === "m",
  );
}
