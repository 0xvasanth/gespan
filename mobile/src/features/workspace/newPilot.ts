// The new-Pilot sheet is a route of its own, presented over the list or the
// drawer, so the Pilot it made comes back to them through here to be opened.
const listeners = new Set<(pilotId: string) => void>();

/** Hands the Pilot just made to whichever screen opens it. */
export function pilotCreated(pilotId: string) {
  listeners.forEach((listener) => listener(pilotId));
}

/** Calls `listener` with each Pilot made; returns the unsubscribe. */
export function onPilotCreated(listener: (pilotId: string) => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
