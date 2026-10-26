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
