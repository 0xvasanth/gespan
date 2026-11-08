import Storage from "expo-sqlite/kv-store";
import { useEffect, useMemo, useSyncExternalStore } from "react";

import type { Pilot } from "./types";

// What the user last saw of each pilot, per desktop: its activity then, so a
// reply that came after reads as new. Kept on the phone, as the desktop sends
// no count of its own.
type Seen = Readonly<Record<string, string>>;

const storageKey = (desktop: string) => `gespan.pilotSeen.${desktop}`;
let loadedFor: string | undefined;
let seen: Seen = {};
const listeners = new Set<() => void>();

function load(desktop: string | undefined) {
  if (!desktop || desktop === loadedFor) return;
  loadedFor = desktop;
  try {
    seen = JSON.parse(Storage.getItemSync(storageKey(desktop)) ?? "{}") as Seen;
  } catch {
    seen = {};
  }
}

function save(next: Seen) {
  seen = next;
  if (loadedFor) void Storage.setItem(storageKey(loadedFor), JSON.stringify(next));
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** A pilot's last reply, as far as the phone can tell; null when it can't,
 *  as when the desktop sends no message for a conversation it hasn't open. */
function activity(pilot: Pilot) {
  return pilot.lastMessage || pilot.lastActiveAt
    ? `${pilot.lastActiveAt}\n${pilot.lastMessage}`
    : null;
}

/** Marks the pilot's latest as seen, as its chat shows it. */
export function markPilotSeen(desktop: string | undefined, pilot: Pilot) {
  load(desktop);
  const now = activity(pilot);
  if (now !== null && seen[pilot.id] !== now) save({ ...seen, [pilot.id]: now });
}

/** Marks the pilot's latest as not seen, so it reads as new until opened again. */
export function markPilotUnread(desktop: string | undefined, pilot: Pilot) {
  load(desktop);
  if (seen[pilot.id] !== "") save({ ...seen, [pilot.id]: "" });
}

/**
 * The pilots with a reply the user hasn't seen. One seen for the first time
 * counts as seen, so a new phone doesn't open to every pilot marked; one still
 * working has nothing finished to show yet.
 */
export function useUnreadPilots(desktop: string | undefined, pilots: Pilot[]): Set<string> {
  load(desktop);
  const current = useSyncExternalStore(subscribe, () => seen);
  useEffect(() => {
    const fresh = pilots.filter((pilot) => !(pilot.id in seen) && activity(pilot) !== null);
    if (fresh.length)
      save({ ...seen, ...Object.fromEntries(fresh.map((pilot) => [pilot.id, activity(pilot)!])) });
  }, [pilots]);
  return useMemo(
    () =>
      new Set(
        pilots
          .filter((pilot) => {
            if (pilot.status === "working") return false;
            if (pilot.unread > 0) return true;
            const now = activity(pilot);
            return now !== null && pilot.id in current && current[pilot.id] !== now;
          })
          .map((pilot) => pilot.id),
      ),
    [pilots, current],
  );
}
