import type { BoardFieldKind, CardLinkRow, CardRelation } from "./board";

/** Most minutes the store accepts. */
const MAX_ESTIMATE = 6_000_000;

export type Parsed<T extends object> =
  ({ ok: true } & T) | { ok: false; error: string };

/** The one hint for an estimate that isn't a time, the same in both apps. */
export const ESTIMATE_HELP = "Use minutes or hours, like 90, 90m, 1h 30m or 1.5h";

/** A number before an `h` or `m`: `1`, `1.5`, `.5`, never `1.` or `1.2.3`. */
const UNIT_NUMBER = /^(\d+(\.\d+)?|\.\d+)$/;

/**
 * An estimate typed as `90`, `90m`, `1h`, `1h 30m`, `1.5h` or `.5h`, in
 * minutes rounded to the minute. Hex, exponents, signs, a bare decimal and a
 * unit with no number are refused with `ESTIMATE_HELP`.
 */
export function parseEstimate(input: string): Parsed<{ minutes: number }> {
  const text = input.trim().toLowerCase();
  const bad = { ok: false as const, error: ESTIMATE_HELP };
  if (!text) return bad;
  let total = 0;
  if (/^\d+$/.test(text)) {
    total = Number(text);
  } else {
    let number = "";
    let seenHours = false;
    let seenMinutes = false;
    for (const c of text) {
      if ((c >= "0" && c <= "9") || c === ".") {
        number += c;
      } else if (c === "h" || c === "m") {
        if (!UNIT_NUMBER.test(number)) return bad;
        const value = Number(number);
        number = "";
        if (c === "h") {
          if (seenHours) return bad;
          seenHours = true;
          total += value * 60;
        } else {
          if (seenMinutes) return bad;
          seenMinutes = true;
          total += value;
        }
      } else if (/\s/.test(c)) {
        if (number) return bad;
      } else {
        return bad;
      }
    }
    if (number) return bad;
  }
  const minutes = Math.round(total);
  if (!Number.isFinite(minutes) || minutes < 1) {
    return { ok: false, error: "An estimate must be at least 1 minute" };
  }
  if (minutes > MAX_ESTIMATE) {
    return {
      ok: false,
      error: `An estimate can be at most ${MAX_ESTIMATE} minutes`,
    };
  }
  return { ok: true, minutes };
}

/** `90` shows as `1h 30m`, `45` as `45m`, `120` as `2h`. */
export function formatEstimate(minutes: number): string {
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours === 0) return `${rest}m`;
  if (rest === 0) return `${hours}h`;
  return `${hours}h ${rest}m`;
}

/** The date quick picks of a start or due menu: a name and days from today. */
export const DATE_PICKS = [
  ["Today", 0],
  ["Tomorrow", 1],
  ["Next week", 7],
  ["In 2 weeks", 14],
] as const;

/** Local midnight, `days` after today. */
export function datePickValue(days: number, now = Date.now()): number {
  const today = new Date(now);
  return new Date(
    today.getFullYear(),
    today.getMonth(),
    today.getDate() + days,
  ).getTime();
}

export type RelationChoice = "blocks" | "blocked-by" | "relates";

export const RELATION_CHOICES: readonly [RelationChoice, string][] = [
  ["blocks", "Blocks"],
  ["blocked-by", "Blocked by"],
  ["relates", "Relates to"],
];

/** The stored row a choice writes: "Blocked by" is the other card blocking this one. */
export function relationWrite(
  choice: RelationChoice,
  cardId: string,
  otherId: string,
): { cardId: string; relatedCardId: string; kind: "blocks" | "relates" } {
  switch (choice) {
    case "blocks":
      return { cardId, relatedCardId: otherId, kind: "blocks" };
    case "blocked-by":
      return { cardId: otherId, relatedCardId: cardId, kind: "blocks" };
    case "relates":
      return { cardId, relatedCardId: otherId, kind: "relates" };
  }
}

export type RelationRow = { otherId: string; relation: CardRelation };

/** A card's relations by how they read from it. Duplicates are not shown. */
export function groupRelations(
  cardId: string,
  relations: readonly CardRelation[],
): Record<RelationChoice, RelationRow[]> {
  const groups: Record<RelationChoice, RelationRow[]> = {
    blocks: [],
    "blocked-by": [],
    relates: [],
  };
  for (const relation of relations) {
    const mine = relation.cardId === cardId;
    const otherId = mine ? relation.relatedCardId : relation.cardId;
    if (relation.kind === "blocks") {
      groups[mine ? "blocks" : "blocked-by"].push({ otherId, relation });
    } else if (relation.kind === "relates") {
      groups.relates.push({ otherId, relation });
    }
  }
  return groups;
}

/** Cards on the board matching `#N` or title text, never the card itself. */
export function relationCandidates<
  T extends { id: string; number: number; title: string },
>(cards: readonly T[], selfId: string, query: string): T[] {
  const needle = query.trim().toLowerCase().replace(/^#/, "");
  return cards.filter(
    (card) =>
      card.id !== selfId &&
      (!needle ||
        String(card.number).startsWith(needle) ||
        card.title.toLowerCase().includes(needle)),
  );
}

/** A text or number field value as stored JSON; empty clears it. */
export function encodeField(
  kind: BoardFieldKind,
  input: string,
): Parsed<{ value: string | null }> {
  const text = input.trim();
  if (!text) return { ok: true, value: null };
  if (kind === "text") {
    if (text.length > FIELD_TEXT_MAX) {
      return { ok: false, error: `Text is at most ${FIELD_TEXT_MAX} characters` };
    }
    return { ok: true, value: JSON.stringify(text) };
  }
  if (kind === "number") {
    // Plain decimals only: `Number` would also take `0x10`, `1e3` and `0b11`.
    const number = FIELD_NUMBER.test(text) ? Number(text) : NaN;
    if (!Number.isFinite(number)) {
      return { ok: false, error: `“${text}” is not a number` };
    }
    return { ok: true, value: JSON.stringify(number) };
  }
  if (kind === "url") {
    if (!validFieldUrl(text)) {
      return {
        ok: false,
        error: `“${text}” is not a URL: it must start with http:// or https://`,
      };
    }
    return { ok: true, value: JSON.stringify(text) };
  }
  return { ok: false, error: `Can't type a ${kind} value` };
}

/** A typed number field value: an optional minus, then `42`, `2.5` or `.5`. */
const FIELD_NUMBER = /^-?(\d+(\.\d+)?|\.\d+)$/;

/** Text and URL values hold at most this many characters. */
export const FIELD_TEXT_MAX = 2000;

/** A URL field takes `http://` or `https://` text with a host, no spaces, at most 2,000 characters. */
export function validFieldUrl(text: string): boolean {
  return (
    text.length <= FIELD_TEXT_MAX &&
    /^https?:\/\/[^/\s]\S*$/i.test(text)
  );
}

/** The options a multi-select field value holds, from its stored JSON array. */
export function fieldPicks(raw: string | undefined): string[] {
  if (raw === undefined) return [];
  try {
    const value: unknown = JSON.parse(raw);
    return Array.isArray(value)
      ? value.filter((item): item is string => typeof item === "string")
      : [];
  } catch {
    return [];
  }
}

/** The stored value for a multi-select: distinct picks in option order; none clears it. */
export function encodePicks(
  options: readonly string[],
  picks: readonly string[],
): string | null {
  const chosen = options.filter((option) => picks.includes(option));
  return chosen.length ? JSON.stringify(chosen) : null;
}

/** The choices of a select field, from its `["a","b"]` options. */
export function fieldOptions(optionsJson: string | null): string[] {
  if (!optionsJson) return [];
  try {
    const parsed: unknown = JSON.parse(optionsJson);
    return Array.isArray(parsed) && parsed.every((o) => typeof o === "string")
      ? parsed
      : [];
  } catch {
    return [];
  }
}

const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

/** A stored field value as shown; null when empty or not fitting the kind. */
export function fieldText(kind: BoardFieldKind, raw: string): string | null {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return null;
  }
  if (
    (kind === "text" || kind === "select" || kind === "url") &&
    typeof value === "string"
  ) {
    return value;
  }
  if (kind === "multi-select" && Array.isArray(value)) {
    const picks = value.filter((item): item is string => typeof item === "string");
    return picks.length ? picks.join(", ") : null;
  }
  if (kind === "number" && typeof value === "number") return String(value);
  if (kind === "date" && typeof value === "number") {
    const day = new Date(value);
    if (Number.isNaN(day.getTime())) return null;
    return `${MONTHS[day.getMonth()]} ${day.getDate()}, ${day.getFullYear()}`;
  }
  if (kind === "checkbox" && typeof value === "boolean") {
    return value ? "Yes" : "No";
  }
  return null;
}

/** Whether a checkbox field's stored value is on. */
export function fieldChecked(raw: string | undefined): boolean {
  return raw === "true";
}

/** A PR link is an http or https URL with a host and no spaces. */
export function validPrUrl(text: string): boolean {
  return /^https?:\/\/[^/\s]\S*$/i.test(text) && !/\s/.test(text);
}

/**
 * The card's links but the primary one, which the Linked work row shows. An
 * extra link that is also the primary (`primary` is the card's
 * `linkKind`/`linkId`) is left out too, so it shows once.
 */
export function visibleLinks(
  links: readonly CardLinkRow[],
  primary: { kind: string | null; id: string | null } = { kind: null, id: null },
): CardLinkRow[] {
  return links.filter(
    (link) =>
      link.role !== "primary" &&
      !(link.kind === primary.kind && link.reference === primary.id),
  );
}
