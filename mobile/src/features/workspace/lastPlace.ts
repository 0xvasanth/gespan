import Storage from "expo-sqlite/kv-store";

/** Where the user last was: a project or a pilot, opened again at launch. */
export type Place = { kind: "project" | "pilot"; id: string };

// Per desktop, since each has its own projects and pilots.
const storageKey = (desktop: string) => `gespan.lastPlace.${desktop}`;
/** The last place written, so revisiting it writes nothing. */
const written = new Map<string, string>();

export function readLastPlace(desktop: string | undefined): Place | null {
  if (!desktop) return null;
  try {
    const raw = Storage.getItemSync(storageKey(desktop));
    const place = raw ? (JSON.parse(raw) as Partial<Place>) : null;
    return place &&
      (place.kind === "project" || place.kind === "pilot") &&
      typeof place.id === "string"
      ? { kind: place.kind, id: place.id }
      : null;
  } catch {
    return null;
  }
}

export function rememberPlace(desktop: string | undefined, place: Place) {
  if (!desktop) return;
  const value = JSON.stringify(place);
  if (written.get(desktop) === value) return;
  written.set(desktop, value);
  void Storage.setItem(storageKey(desktop), value);
}
