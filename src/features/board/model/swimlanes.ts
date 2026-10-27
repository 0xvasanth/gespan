import type {
  BoardCard,
  BoardLabel,
  CardExtras,
  PriorityScheme,
} from "./board";
import type { LaneBy } from "./display";
import { DEFAULT_PRIORITY_SCHEME, offeredPriorities } from "./boardSettings";
import { fieldOptions } from "./cardProps";

export type Lane = { key: string; label: string; cardIds: string[] };

/** What a drop into a lane sets on the card besides its column. */
export type LaneValue =
  | { kind: "priority"; priority: number }
  | { kind: "field"; fieldId: string; option: string | null };

const NONE = "none";

function selectedOption(raw: string | undefined): string | null {
  if (raw === undefined) return null;
  try {
    const value: unknown = JSON.parse(raw);
    return typeof value === "string" ? value : null;
  } catch {
    return null;
  }
}

/**
 * Rows of the board: one lane per value of `by`. Priority lanes list the
 * offered levels (Urgent first), plus a hidden level while a card holds it.
 * A card with several labels or projects sits in each of their lanes.
 */
export function buildLanes(input: {
  by: LaneBy;
  cards: readonly BoardCard[];
  extras: Readonly<Record<string, CardExtras | undefined>>;
  labels: readonly Pick<BoardLabel, "id" | "name">[];
  projects: readonly { path: string; name: string }[];
  /** Raw field values by card id (JSON), for a select-field lane. */
  fieldValues: Readonly<Record<string, Record<string, string> | undefined>>;
  priorities?: PriorityScheme;
}): Lane[] {
  const { by, cards, extras, labels, projects, fieldValues } = input;
  const scheme = input.priorities ?? DEFAULT_PRIORITY_SCHEME;
  let heads: [string, string][];
  let laneKeys: (card: BoardCard) => string[];
  let hideEmpty: (key: string) => boolean = (key) => key === NONE;

  switch (by.kind) {
    case "priority": {
      const held = new Set(cards.map((card) => extras[card.id]?.priority ?? 0));
      const offered = new Set(offeredPriorities(scheme));
      heads = [4, 3, 2, 1, 0]
        .filter((level) => offered.has(level) || held.has(level))
        .map((level) => [`p${level}`, scheme.names[level]]);
      laneKeys = (card) => [`p${extras[card.id]?.priority ?? 0}`];
      hideEmpty = () => false;
      break;
    }
    case "label":
      heads = [
        ...labels.map((label): [string, string] => [label.id, label.name]),
        [NONE, "No label"],
      ];
      laneKeys = (card) => {
        const ids = (extras[card.id]?.labels ?? []).filter((id) =>
          labels.some((label) => label.id === id),
        );
        return ids.length ? ids : [NONE];
      };
      break;
    case "project": {
      const known = new Set(projects.map((project) => project.path));
      const stray = [
        ...new Set(cards.flatMap((card) => extras[card.id]?.projects ?? [])),
      ].filter((path) => !known.has(path));
      heads = [
        ...projects.map((project): [string, string] => [
          project.path,
          project.name,
        ]),
        ...stray.map((path): [string, string] => [path, path]),
        [NONE, "No project"],
      ];
      laneKeys = (card) => {
        const paths = extras[card.id]?.projects ?? [];
        return paths.length ? paths : [NONE];
      };
      break;
    }
    case "field": {
      const options = fieldOptions(by.field.optionsJson);
      const valueOf = (card: BoardCard) =>
        selectedOption(fieldValues[card.id]?.[by.field.id]);
      const stray = [
        ...new Set(cards.flatMap((card) => valueOf(card) ?? [])),
      ].filter((option) => !options.includes(option));
      heads = [
        ...[...options, ...stray].map((option): [string, string] => [
          `opt:${option}`,
          option,
        ]),
        [NONE, `No ${by.field.name}`],
      ];
      laneKeys = (card) => {
        const option = valueOf(card);
        return [option === null ? NONE : `opt:${option}`];
      };
      break;
    }
  }

  const byKey = new Map<string, string[]>(heads.map(([key]) => [key, []]));
  for (const card of cards) {
    for (const key of laneKeys(card)) byKey.get(key)?.push(card.id);
  }
  return heads
    .filter(([key]) => !(hideEmpty(key) && !byKey.get(key)?.length))
    .map(([key, label]) => ({ key, label, cardIds: byKey.get(key) ?? [] }));
}

/**
 * What dropping into lane `key` sets besides the column: the priority of a
 * priority lane, the option of a select-field lane; label and project lanes
 * set nothing.
 */
export function laneValue(by: LaneBy, key: string): LaneValue | null {
  if (by.kind === "priority") {
    const level = Number(key.slice(1));
    return Number.isInteger(level) ? { kind: "priority", priority: level } : null;
  }
  if (by.kind === "field") {
    return {
      kind: "field",
      fieldId: by.field.id,
      option: key === NONE ? null : key.slice("opt:".length),
    };
  }
  return null;
}

/** A row of the windowed swimlanes: a lane's header, or one band of it. */
export type LaneRow =
  | { kind: "header"; lane: string }
  | { kind: "band"; lane: string; band: number };

/** Cards per cell in one band: band `b` holds cards [4b, 4b + 4) of each cell. */
export const LANE_BAND = 4;

/** First-guess height of a lane header row (px); measured once mounted. */
export const LANE_HEADER_ESTIMATE = 36;
/** First-guess height of an empty band (its cells' drop areas). */
const EMPTY_BAND_ESTIMATE = 40;

/** First-guess height of a band whose tallest cell holds `cards` cards. */
export function laneBandEstimate(cards: number, cardEstimate: number): number {
  // Cards stack with a 6px gap, and a 6px pad closes each band.
  return cards > 0 ? cards * (cardEstimate + 6) : EMPTY_BAND_ESTIMATE;
}

/** The key of the cell where lane `lane` meets column `column`. */
export function laneCellKey(lane: string, column: string): string {
  return `${lane}\u0000${column}`;
}

/** Each lane's cards split by column and ordered by position (empty cells left out). */
export function laneCells(
  lanes: readonly Lane[],
  cards: readonly BoardCard[],
): Map<string, string[]> {
  const byId = new Map(cards.map((card) => [card.id, card]));
  const out = new Map<string, BoardCard[]>();
  for (const lane of lanes) {
    for (const id of lane.cardIds) {
      const card = byId.get(id);
      if (!card) continue;
      const key = laneCellKey(lane.key, card.columnId);
      const cell = out.get(key);
      if (cell) cell.push(card);
      else out.set(key, [card]);
    }
  }
  return new Map(
    [...out].map(([key, cell]) => [
      key,
      cell.sort((a, b) => a.position - b.position).map((card) => card.id),
    ]),
  );
}

/**
 * The rows of the lanes: each lane's header, then as many bands as its
 * longest cell needs (at least one, so an empty lane still takes drops).
 * `cells` holds the cards each cell shows; a collapsed column shows none.
 */
export function laneRows(
  lanes: readonly Lane[],
  cells: ReadonlyMap<string, readonly string[]>,
): LaneRow[] {
  const longest = new Map<string, number>();
  for (const [key, ids] of cells) {
    const lane = key.slice(0, key.indexOf("\u0000"));
    longest.set(lane, Math.max(longest.get(lane) ?? 0, ids.length));
  }
  const rows: LaneRow[] = [];
  for (const lane of lanes) {
    rows.push({ kind: "header", lane: lane.key });
    const bands = Math.max(1, Math.ceil((longest.get(lane.key) ?? 0) / LANE_BAND));
    for (let band = 0; band < bands; band += 1) {
      rows.push({ kind: "band", lane: lane.key, band });
    }
  }
  return rows;
}

/**
 * The lanes' cards in the order they read on screen: lane by lane, column
 * by column (`columnIds`, the shown ones in order), then by position. A card
 * in two lanes counts once, at its first place.
 */
export function laneOrder(
  lanes: readonly Lane[],
  cells: ReadonlyMap<string, readonly string[]>,
  columnIds: readonly string[],
): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const lane of lanes) {
    for (const column of columnIds) {
      for (const id of cells.get(laneCellKey(lane.key, column)) ?? []) {
        if (seen.has(id)) continue;
        seen.add(id);
        out.push(id);
      }
    }
  }
  return out;
}

/** Where focus goes in the lanes: a card in a lane, or the lane's header. */
export type LaneRefocus = { lane: string; cardId: string | null };

/**
 * Where focus goes when the focused card `from` lost its element across a
 * change of the lanes' cells (`before` → `after`): the same card if its lane
 * still shows it (a reload that moved it to another band or column), else
 * the nearest card that took its place in its old cell (next, then
 * previous, widening), else that cell's first card, else the lane's header.
 * Null when the lane itself is gone.
 */
export function laneRefocus(
  from: { lane: string; cardId: string },
  before: ReadonlyMap<string, readonly string[]>,
  after: ReadonlyMap<string, readonly string[]>,
  laneKeys: readonly string[],
  columnIds: readonly string[],
): LaneRefocus | null {
  if (!laneKeys.includes(from.lane)) return null;
  for (const column of columnIds) {
    if (after.get(laneCellKey(from.lane, column))?.includes(from.cardId)) {
      return { lane: from.lane, cardId: from.cardId };
    }
  }
  for (const column of columnIds) {
    const ids = before.get(laneCellKey(from.lane, column)) ?? [];
    const at = ids.indexOf(from.cardId);
    if (at < 0) continue;
    const now = after.get(laneCellKey(from.lane, column)) ?? [];
    const shown = new Set(now);
    for (let d = 1; d < ids.length; d += 1) {
      for (const id of [ids[at + d], ids[at - d]]) {
        if (id !== undefined && shown.has(id)) return { lane: from.lane, cardId: id };
      }
    }
    if (now.length) return { lane: from.lane, cardId: now[0] };
    break;
  }
  return { lane: from.lane, cardId: null };
}
