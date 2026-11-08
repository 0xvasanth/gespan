import Storage from "expo-sqlite/kv-store";
import { useSyncExternalStore } from "react";

import { ACCENTS } from "@/ui/theme";

/** An accent picked for one Pilot's chat, or "theme" for the app's own. */
export type PilotAccent = keyof typeof ACCENTS | "theme";

// Kept on this phone, per desktop and Pilot, as reactions are.
const KEY = "gespan.pilotAccents";
let accents: Readonly<Record<string, PilotAccent>> = load();
const listeners = new Set<() => void>();

function load() {
  try {
    return JSON.parse(Storage.getItemSync(KEY) ?? "{}") as Record<string, PilotAccent>;
  } catch {
    return {};
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Draws this Pilot's chat in `accent`, or back in the app's own for "theme". */
export function setPilotAccent(pilotId: string, accent: PilotAccent) {
  const { [pilotId]: _, ...rest } = accents;
  accents = accent === "theme" ? rest : { ...rest, [pilotId]: accent };
  void Storage.setItem(KEY, JSON.stringify(accents));
  listeners.forEach((listener) => listener());
}

/** The accent picked for this Pilot's chat. */
export function usePilotAccent(pilotId: string | undefined): PilotAccent {
  return useSyncExternalStore(subscribe, () => (pilotId ? accents[pilotId] : undefined) ?? "theme");
}
