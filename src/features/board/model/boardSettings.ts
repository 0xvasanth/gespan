import type { PriorityScheme } from "./board";

/** Board icons, as the settings page offers them and the store accepts them. */
export const BOARD_ICONS = [
  "board",
  "bug",
  "rocket",
  "check",
  "star",
  "flag",
  "bolt",
  "book",
] as const;

export const DEFAULT_PRIORITY_SCHEME: PriorityScheme = {
  names: ["None", "Low", "Medium", "High", "Urgent"],
  hidden: [],
};

export const PRIORITY_NAME_MAX = 20;

/**
 * The scheme a board stores. `null`, bad JSON and bad parts fall back to the
 * defaults one by one, so a half-written scheme never blanks a picker.
 */
export function parsePriorityScheme(json: string | null): PriorityScheme {
  if (!json) return DEFAULT_PRIORITY_SCHEME;
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return DEFAULT_PRIORITY_SCHEME;
  }
  if (!raw || typeof raw !== "object") return DEFAULT_PRIORITY_SCHEME;
  const { names, hidden } = raw as { names?: unknown; hidden?: unknown };
  const fallback = DEFAULT_PRIORITY_SCHEME.names;
  const list = Array.isArray(names) ? names : [];
  return {
    names: fallback.map((name, level) => {
      const given = list[level];
      return typeof given === "string" && validPriorityName(given)
        ? given
        : name;
    }) as PriorityScheme["names"],
    // Level 0 can't be hidden.
    hidden: Array.isArray(hidden)
      ? [
          ...new Set(
            hidden.filter(
              (level): level is number =>
                Number.isInteger(level) && level >= 1 && level <= 4,
            ),
          ),
        ].sort()
      : [],
  };
}

export function serializePriorityScheme(scheme: PriorityScheme): string {
  return JSON.stringify({ names: scheme.names, hidden: scheme.hidden });
}

export function validPriorityName(name: string): boolean {
  const length = [...name.trim()].length;
  return length >= 1 && length <= PRIORITY_NAME_MAX;
}

export function priorityName(scheme: PriorityScheme, level: number): string {
  return scheme.names[level] ?? scheme.names[0];
}

/** The levels a picker or a filter offers: every one not hidden. */
export function offeredPriorities(scheme: PriorityScheme): number[] {
  return [0, 1, 2, 3, 4].filter((level) => !scheme.hidden.includes(level));
}

/** The scheme with `level` shown or hidden; level 0 stays shown. */
export function withHidden(
  scheme: PriorityScheme,
  level: number,
  hidden: boolean,
): PriorityScheme {
  const rest = scheme.hidden.filter((other) => other !== level);
  return {
    ...scheme,
    hidden: hidden && level > 0 ? [...rest, level].sort() : rest,
  };
}

export function withName(
  scheme: PriorityScheme,
  level: number,
  name: string,
): PriorityScheme {
  const names = [...scheme.names] as PriorityScheme["names"];
  names[level] = name.trim();
  return { ...scheme, names };
}

/**
 * A column's count. Unfiltered it is `{total}/{limit}` (or the total with no
 * limit); filtered it is `{shown}/{total}`, and the limit still compares to
 * the total. `over` is total > limit.
 */
export function wipCount(input: {
  total: number;
  shown?: number;
  limit: number | null;
}): { text: string; over: boolean } {
  const { total, shown, limit } = input;
  const over = limit !== null && total > limit;
  if (shown !== undefined) return { text: `${shown}/${total}`, over };
  return { text: limit === null ? String(total) : `${total}/${limit}`, over };
}

export function overLimitMessage(total: number, limit: number): string {
  return `Over the WIP limit (${total}/${limit})`;
}

/** A WIP limit as typed: empty clears it, otherwise a whole number of 1 or more. */
export function parseWipLimit(text: string): number | null | "invalid" {
  const trimmed = text.trim();
  if (!trimmed) return null;
  if (!/^\d+$/.test(trimmed)) return "invalid";
  const value = Number(trimmed);
  return value >= 1 ? value : "invalid";
}

/** Whether `name` is taken by another label, ignoring case. */
export function labelNameTaken(
  labels: readonly { id: string; name: string }[],
  name: string,
  exceptId?: string,
): boolean {
  const wanted = name.trim().toLowerCase();
  return labels.some(
    (label) =>
      label.id !== exceptId && label.name.trim().toLowerCase() === wanted,
  );
}
