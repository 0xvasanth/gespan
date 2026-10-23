import { CHAT_BACKGROUND_OPACITY_DEFAULT } from "../../settings/model/appearance";
import { clearProjectChatBackground } from "../../projects/model/chatBackground";
import {
  clearProjectChatBackgroundSetting,
  loadProjectChatBackgroundSettings,
  type ProjectChatBackgroundSettings,
} from "../../projects/model/projectChatBackground";

/**
 * A Pilot's background is stored like a project's, under a key no project
 * folder has, so the project background dialog can edit it as is.
 */
export function pilotBackgroundKey(pilotId: string): string {
  return `pilot:${pilotId}`;
}

/**
 * How a Pilot's chat shows its image: always the Haze, dimmed the same on an
 * empty chat as in a running one. Only the image is the user's choice.
 */
export function pilotChatBackground(
  pilotId: string,
): ProjectChatBackgroundSettings | null {
  const stored = loadProjectChatBackgroundSettings(pilotBackgroundKey(pilotId));
  if (!stored) return null;
  return {
    path: stored.path,
    emptyOpacity: CHAT_BACKGROUND_OPACITY_DEFAULT,
    sessionOpacity: CHAT_BACKGROUND_OPACITY_DEFAULT,
    scope: "all",
    effect: "gradient-blur",
  };
}

/** Forgets the image along with its Pilot. */
export function removePilotBackground(pilotId: string): void {
  const key = pilotBackgroundKey(pilotId);
  if (!loadProjectChatBackgroundSettings(key)) return;
  clearProjectChatBackgroundSetting(key);
  void clearProjectChatBackground(key).catch(() => {});
}
