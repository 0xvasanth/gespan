import { invoke } from "@tauri-apps/api/core";
import { useSyncExternalStore } from "react";
import type { Session } from "../../sessions/model/session";

/** Shared with the native app, which mirrors `gespan.keepAwake` into its preferences. */
export const KEEP_AWAKE_KEY = "gespan.keepAwake";
export const KEEP_AWAKE_CHANGE_EVENT = "gespan:keepawakechange";

/** When Gespan stops the idle timer from dimming and locking the screen. */
export type KeepAwakeMode = "off" | "agents_running" | "app_open";
export const KEEP_AWAKE_DEFAULT: KeepAwakeMode = "off";
export const KEEP_AWAKE_MODES: readonly KeepAwakeMode[] = [
  "off",
  "agents_running",
  "app_open",
];

function isMode(value: unknown): value is KeepAwakeMode {
  return KEEP_AWAKE_MODES.includes(value as KeepAwakeMode);
}

export function loadKeepAwake(): KeepAwakeMode {
  try {
    const raw = localStorage.getItem(KEEP_AWAKE_KEY);
    return isMode(raw) ? raw : KEEP_AWAKE_DEFAULT;
  } catch {
    return KEEP_AWAKE_DEFAULT;
  }
}

export function saveKeepAwake(mode: KeepAwakeMode): void {
  try {
    localStorage.setItem(KEEP_AWAKE_KEY, mode);
  } catch {
    // Storage may be unavailable; the event still updates this window.
  }
  if (typeof window === "undefined") return;
  window.dispatchEvent(
    new CustomEvent<KeepAwakeMode>(KEEP_AWAKE_CHANGE_EVENT, { detail: mode }),
  );
}

/** Calls back when this window or another one changes the mode. */
export function subscribeKeepAwake(onChange: () => void): () => void {
  if (typeof window === "undefined") return () => {};
  const onStorage = (event: StorageEvent) => {
    if (event.key === KEEP_AWAKE_KEY || event.key === null) onChange();
  };
  window.addEventListener(KEEP_AWAKE_CHANGE_EVENT, onChange);
  window.addEventListener("storage", onStorage);
  return () => {
    window.removeEventListener(KEEP_AWAKE_CHANGE_EVENT, onChange);
    window.removeEventListener("storage", onStorage);
  };
}

export function useKeepAwake(): KeepAwakeMode {
  return useSyncExternalStore(
    subscribeKeepAwake,
    loadKeepAwake,
    () => KEEP_AWAKE_DEFAULT,
  );
}

/** Any agent still working: a live turn, or work it left running in the background. */
export function hasWorkingAgent(sessions: readonly Session[]): boolean {
  return sessions.some(
    (session) =>
      !session.worktreeRemoved &&
      (!!session.busy || (session.backgroundTasks?.length ?? 0) > 0),
  );
}

let last: string | null = null;

/**
 * Report this window's state to Rust, which holds one OS assertion for every
 * window. Only transitions are sent, so this is cheap to call on each update.
 */
export function syncKeepAwake(
  sessions: readonly Session[],
  mode: KeepAwakeMode = loadKeepAwake(),
): void {
  const running = hasWorkingAgent(sessions);
  const key = `${mode}:${running}`;
  if (key === last) return;
  last = key;
  void invoke("set_keep_awake", { mode, running }).catch(() => {
    // Let the next call retry instead of assuming Rust holds this state.
    last = null;
  });
}

/** Test seam: forget the last reported state. */
export function resetKeepAwakeSync(): void {
  last = null;
}
