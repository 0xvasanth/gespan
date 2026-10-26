import { HARNESSES, type HarnessId } from "./session";
import {
  hasProbedHarnessAvailability,
  isHarnessAvailable,
} from "../../../integrations/harness/core/availabilityState";

/** Shown on the disabled model picker and send button. Same copy as Native. */
export const NO_CODING_AGENT_HINT =
  "No coding agent detected. Set one up in Settings → Agent CLIs.";

/** Window event the workspace answers by opening Settings → Agent CLIs. */
export const OPEN_AGENT_CLIS_EVENT = "gespan:open-agent-clis";
/**
 * The same request from another webview (the quick composer, a floating
 * Pilot chat), sent to every window as a Tauri event.
 */
export const OPEN_AGENT_CLIS_REQUEST_EVENT = "open_agent_clis_settings";

/**
 * True once detection has run and found no agent CLI. Before the first probe
 * nothing is known, so nothing is disabled.
 */
export function noCodingAgentDetected(
  available: (id: HarnessId) => boolean = isHarnessAvailable,
  probed: boolean = hasProbedHarnessAvailability(),
): boolean {
  return probed && !HARNESSES.some((id) => available(id));
}

export function openAgentClisSettings(): void {
  window.dispatchEvent(new Event(OPEN_AGENT_CLIS_EVENT));
}

/**
 * Phone sends need a coding agent on this computer, unless the session runs
 * on a host. Throws the settings hint, as Native does.
 */
export function assertPhoneSendAllowed(remote: boolean): void {
  if (!remote && noCodingAgentDetected()) throw new Error(NO_CODING_AGENT_HINT);
}

/**
 * Which window answers an "Open Settings" request from the quick composer or
 * a floating Pilot chat: the focused workspace window, else a visible one,
 * else the first, as quick launches pick. Every window hears the request, so
 * each asks whether it is the one.
 */
export function agentClisSettingsWindow(
  windows: readonly { label: string; focused: boolean; visible: boolean }[],
): string | null {
  const workspace = windows
    .filter(
      ({ label }) =>
        label !== "quick-composer" &&
        label !== "quick-composer-git" &&
        label !== "pilot-chat" &&
        !label.startsWith("pilot-chat-"),
    )
    .sort((a, b) => (a.label < b.label ? -1 : a.label > b.label ? 1 : 0));
  return (
    (
      workspace.find((window) => window.focused) ??
      workspace.find((window) => window.visible) ??
      workspace[0]
    )?.label ?? null
  );
}

type SettingsWindowHandle = {
  label: string;
  isFocused(): Promise<boolean>;
  isVisible(): Promise<boolean>;
};

/**
 * Answers an "Open Settings" request in `self` if it is the elected window:
 * shows it (close-to-dock may have hidden it), restores and focuses it, then
 * opens Settings there. Returns whether this window answered.
 */
export async function answerAgentClisSettingsRequest(
  self: SettingsWindowHandle & {
    show(): Promise<void>;
    unminimize(): Promise<void>;
    setFocus(): Promise<void>;
  },
  windows: readonly SettingsWindowHandle[],
  open: () => void,
): Promise<boolean> {
  const states = await Promise.all(
    windows.map(async (win) => ({
      label: win.label,
      focused: await win.isFocused().catch(() => false),
      visible: await win.isVisible().catch(() => false),
    })),
  );
  if (agentClisSettingsWindow(states) !== self.label) return false;
  await self
    .show()
    .then(() => self.unminimize())
    .then(() => self.setFocus())
    .catch(() => {});
  open();
  return true;
}
