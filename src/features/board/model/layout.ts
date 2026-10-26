import { DEFAULT_PRIORITY_SCHEME } from "./boardSettings";
import { type BoardCard, type BoardColumn, type BoardField, type BoardLabel, type CardExtras, type PriorityScheme } from "./board";

export type LayoutKind = "board" | "list" | "table";
export const LAYOUTS: readonly (readonly [LayoutKind, string])[] = [
  ["board", "Board"],
  ["list", "List"],
  ["table", "Table"],
];

export type GroupBy = "status" | "priority" | "label" | "project" | "due";
export const GROUP_BYS: readonly (readonly [GroupBy, string])[] = [
  ["status", "Status"],
  ["priority", "Priority"],
  ["label", "Label"],
  ["project", "Project"],
  ["due", "Due date"],
];

/** The smallest a table column can be. */
export const MIN_COLUMN = 60;
/** Every list and table row is this tall, which is what windowing relies on. */
export const ROW_HEIGHT = 32;

/** Built-in table columns as (id, label, default width). */
export const TABLE_COLUMNS: readonly (readonly [string, string, number])[] = [
  ["title", "Title", 320],
  ["status", "Status", 130],
  ["priority", "Priority", 110],
  ["due", "Due", 110],
  ["start", "Start", 110],
  ["labels", "Labels", 190],
  ["projects", "Projects", 150],
  ["estimate", "Estimate", 90],
  ["created", "Created", 110],
  ["updated", "Updated", 110],
];
export const DEFAULT_COLUMNS = ["title", "status", "priority", "due", "labels"];
export const FIELD_WIDTH = 140;

/** A view's `layout_json` as this build reads it. Unknown keys aren't here: `patchLayout` keeps them. */
export type LayoutState = {
  layout: LayoutKind;
  groupBy: GroupBy;
  collapsed: string[];
  /** Null until the user picks columns: the defaults show. */
  columns: string[] | null;
  widths: Record<string, number>;
};

function record(json: string): Record<string, unknown> {
  try {
    const raw: unknown = JSON.parse(json);
    return raw && typeof raw === "object" && !Array.isArray(raw)
      ? (raw as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

const strings = (value: unknown): string[] | null =>
  Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : null;

/** Reads a stored layout; anything unreadable falls back to the board layout. */
export function parseLayout(json: string): LayoutState {
  const raw = record(json);
  const widths: Record<string, number> = {};
  if (raw.widths && typeof raw.widths === "object") {
    for (const [id, width] of Object.entries(raw.widths)) {
      if (typeof width === "number" && Number.isFinite(width)) {
        widths[id] = Math.max(MIN_COLUMN, Math.round(width));
      }
    }
  }
  return {
    layout: LAYOUTS.some(([kind]) => kind === raw.layout)
      ? (raw.layout as LayoutKind)
      : "board",
    groupBy: GROUP_BYS.some(([by]) => by === raw.groupBy)
      ? (raw.groupBy as GroupBy)
      : "status",
    collapsed: strings(raw.collapsed) ?? [],
    columns: strings(raw.columns),
    widths,
  };
}

/** `json` with `patch` applied; keys this build doesn't know are kept as they were. */
export function patchLayout(
  json: string,
  patch: Partial<LayoutState>,
): string {
  const next: Record<string, unknown> = { ...record(json), ...patch };
  if (!next.layout) next.layout = "board";
  if (Array.isArray(next.collapsed) && !next.collapsed.length) {
    delete next.collapsed;
  }
  if (
    next.widths &&
    typeof next.widths === "object" &&
    !Object.keys(next.widths).length
  ) {
    delete next.widths;
  }
  if (next.columns === null) delete next.columns;
  return JSON.stringify(next);
}

export type TableColumn = { id: string; label: string; width: number };

/**
 * The columns a table shows: Title always leads, the rest as chosen (or the
 * defaults); a custom field deleted since the view was saved is dropped.
 */
export function tableColumns(
  state: LayoutState,
  fields: readonly Pick<BoardField, "id" | "name">[],
): TableColumn[] {
  const ids = [
    "title",
    ...(state.columns ?? DEFAULT_COLUMNS).filter((id) => id !== "title"),
  ];
  return ids.flatMap((id): TableColumn[] => {
    let label: string;
    let width: number;
    const built = TABLE_COLUMNS.find(([known]) => known === id);
    if (built) {
      [, label, width] = built;
    } else {
      const field = id.startsWith("field:")
        ? fields.find((found) => `field:${found.id}` === id)
        : undefined;
      if (!field) return [];
      label = field.name;
      width = FIELD_WIDTH;
    }
    return [{ id, label, width: Math.max(MIN_COLUMN, state.widths[id] ?? width) }];
  });
}

function localDay(ms: number): number {
  const date = new Date(ms);
  return Math.round(
    Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / 86_400_000,
  );
}

/** The due bucket of a card, by local days. A done card is never overdue: it is "earlier". */
export function dueBucket(
  dueAt: number | null,
  done: boolean,
  now = Date.now(),
): string {
  if (dueAt === null) return "none";
  const days = localDay(dueAt) - localDay(now);
  if (days < 0) return done ? "earlier" : "overdue";
  if (days === 0) return "today";
  return days <= 7 ? "next7" : "later";
}

const DUE_GROUPS: readonly (readonly [string, string])[] = [
  ["overdue", "Overdue"],
  ["today", "Today"],
  ["next7", "Next 7 days"],
  ["later", "Later"],
  ["earlier", "Earlier"],
  ["none", "No due date"],
];

/** `key` is `mode:value` (e.g. `label:none`), unique across group-by modes. */
export type Group = { key: string; label: string; cardIds: string[] };

/**
 * Groups `cards` (already in the order shown) for the list. A card with two
 * labels is in both label groups. Empty groups are left out. A label, project
 * or column the card holds but the board no longer lists is treated as
 * absent, so the card is never lost from the list.
 */
export function buildGroups(input: {
  by: GroupBy;
  cards: readonly BoardCard[];
  columns: readonly BoardColumn[];
  extras: Readonly<Record<string, CardExtras | undefined>>;
  labels: readonly Pick<BoardLabel, "id" | "name">[];
  projects: readonly { path: string; name: string }[];
  /** Names the priority groups; a hidden level keeps its group while a card holds it. */
  priorities?: PriorityScheme;
  now?: number;
}): Group[] {
  const { by, cards, columns, extras, labels, projects, now } = input;
  const priorities = input.priorities ?? DEFAULT_PRIORITY_SCHEME;
  const roles = new Map(columns.map((column) => [column.id, column.role]));
  let heads: [string, string][];
  switch (by) {
    case "priority":
      heads = priorities.names.map((name, p): [string, string] => [`p${p}`, name]).reverse();
      break;
    case "label":
      heads = [...labels.map((l): [string, string] => [l.id, l.name]), ["none", "No label"]];
      break;
    case "project": {
      const known = new Set(projects.map((project) => project.path));
      const stray = [
        ...new Set(cards.flatMap((card) => extras[card.id]?.projects ?? [])),
      ].filter((path) => !known.has(path));
      heads = [
        ...projects.map((p): [string, string] => [p.path, p.name]),
        ...stray.map((path): [string, string] => [path, path]),
        ["none", "No project"],
      ];
      break;
    }
    case "due":
      heads = DUE_GROUPS.map(([key, label]): [string, string] => [key, label]);
      break;
    default:
      heads = [
        ...[...columns]
          .sort((a, b) => a.position - b.position)
          .map((c): [string, string] => [c.id, c.name]),
        ["none", "No status"],
      ];
  }
  // Keys carry the mode, so what a view remembers as collapsed ("No label",
  // "No project") never matches a group of another mode.
  const groups = new Map<string, Group>(
    heads.map(([key, label]) => [
      `${by}:${key}`,
      { key: `${by}:${key}`, label, cardIds: [] },
    ]),
  );
  const put = (raw: readonly string[], card: BoardCard) => {
    const found = raw.map((key) => `${by}:${key}`).filter((key) => groups.has(key));
    // Nothing the board still lists: the card goes in the catch-all group.
    const none = `${by}:none`;
    const into = found.length ? found : groups.has(none) ? [none] : [];
    for (const key of into) groups.get(key)!.cardIds.push(card.id);
  };
  for (const card of cards) {
    const mine = extras[card.id];
    switch (by) {
      case "priority":
        put([`p${mine?.priority ?? 0}`], card);
        break;
      case "label":
        put(mine?.labels ?? [], card);
        break;
      case "project":
        put(mine?.projects ?? [], card);
        break;
      case "due":
        put([dueBucket(mine?.dueAt ?? null, roles.get(card.columnId) === "done", now)], card);
        break;
      default:
        put([card.columnId], card);
    }
  }
  return [...groups.values()].filter((group) => group.cardIds.length);
}
