import type { BoardFieldKind, CardLinkRow, CardRelation } from "./board";

/** Most minutes the store accepts. */
const MAX_ESTIMATE = 6_000_000;

export type Parsed<T extends object> =
  ({ ok: true } & T) | { ok: false; error: string };

/** An estimate typed as `90`, `90m`, `1h`, `1h 30m` or `1.5h`, in minutes. */
export function parseEstimate(input: string): Parsed<{ minutes: number }> {
  const text = input.trim().toLowerCase();
  const bad = {
    ok: false as const,
    error: `“${text}” is not a time like 90m or 1h 30m`,
  };
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
        const value = Number(number);
        if (
          !number ||
          !Number.isFinite(value) ||
          /^\.|\.$|\..*\./.test(number)
        ) {
          return bad;
        }
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
  if (kind === "text") return { ok: true, value: JSON.stringify(text) };
  if (kind === "number") {
    const number = Number(text);
    if (!Number.isFinite(number)) {
      return { ok: false, error: `“${text}” is not a number` };
    }
    return { ok: true, value: JSON.stringify(number) };
  }
  return { ok: false, error: `Can't type a ${kind} value` };
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
  if ((kind === "text" || kind === "select") && typeof value === "string") {
    return value;
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

/** The card's links but the primary one, which the Linked work row shows. */
export function visibleLinks(links: readonly CardLinkRow[]): CardLinkRow[] {
  return links.filter((link) => link.role !== "primary");
}
