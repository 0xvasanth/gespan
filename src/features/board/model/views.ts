import { DEFAULT_PRIORITY_SCHEME, offeredPriorities } from "./boardSettings";
import {
  type BoardCard,
  type BoardColumn,
  type CardLinkKind,
  type BoardField,
  type BoardLabel,
  type BoardView,
  type FilterOp,
  type FilterRow,
  type PriorityScheme,
  type SortKey,
  type SortSpec,
  type ViewSpec,
} from "./board";

/** The layout every view starts with. The whole string is stored and
 * restored as it is, so keys this build doesn't know survive a save. */
export const DEFAULT_LAYOUT = '{"layout":"board"}';

export const EMPTY_SPEC: ViewSpec = {
  query: "",
  rows: [],
  sort: { key: "manual", dir: "asc" },
};

/** The fields a filter row can be on, as (field, label). */
export const FILTER_FIELDS: readonly (readonly [string, string])[] = [
  ["status", "Status"],
  ["priority", "Priority"],
  ["labels", "Labels"],
  ["projects", "Projects"],
  ["due", "Due"],
  ["created", "Created"],
  ["updated", "Updated"],
  ["linked", "Linked work"],
];

const DUE_CHOICES: Choice[] = [
  ["overdue", "Overdue"],
  ["today", "Today"],
  ["next7", "Next 7 days"],
  ["later", "Later"],
  ["none", "No due date"],
];
const WITHIN_CHOICES: Choice[] = [
  ["24h", "Last 24 hours"],
  ["7d", "Last 7 days"],
  ["30d", "Last 30 days"],
];
const LINKED_CHOICES: Choice[] = [
  ["running", "Running"],
  ["failed", "Failed"],
  ["needs_review", "Needs review"],
  ["none", "No linked work"],
];

export const SORT_KEYS: readonly SortKey[] = [
  "manual",
  "priority",
  "due",
  "start",
  "title",
  "created",
  "updated",
];

export function sortLabel(key: SortKey): string {
  switch (key) {
    case "manual":
      return "Manual";
    case "priority":
      return "Priority";
    case "due":
      return "Due date";
    case "start":
      return "Start date";
    case "title":
      return "Title";
    case "created":
      return "Created";
    case "updated":
      return "Updated";
  }
}

/** A choice a row can pick: the stored value and its label. */
export type Choice = readonly [value: string, label: string];

/** What the choices of a row are read from. */
export type FilterSources = {
  columns: readonly Pick<BoardColumn, "id" | "name" | "position">[];
  labels: readonly Pick<BoardLabel, "id" | "name">[];
  projects: readonly { path: string; name: string }[];
  fields: readonly BoardField[];
  /** The board's priority names and hidden levels; the defaults when absent. */
  priorities?: PriorityScheme;
};

export const customFieldKey = (id: string) => `field:${id}`;

function fieldOf(sources: FilterSources, field: string): BoardField | null {
  const id = field.startsWith("field:") ? field.slice("field:".length) : null;
  return (id && sources.fields.find((found) => found.id === id)) || null;
}

function selectOptions(field: BoardField): string[] {
  try {
    const parsed: unknown = JSON.parse(field.optionsJson ?? "[]");
    return Array.isArray(parsed)
      ? parsed.filter((option): option is string => typeof option === "string")
      : [];
  } catch {
    return [];
  }
}

/** A held custom-field value as text: strings bare, dates as a day. */
function heldLabel(kind: BoardField["kind"], raw: string): string {
  try {
    const value: unknown = JSON.parse(raw);
    if (typeof value === "string") return value;
    if (typeof value === "number" && kind === "date") {
      return new Date(value).toLocaleDateString();
    }
    if (typeof value === "number" || typeof value === "boolean") {
      return String(value);
    }
  } catch {
    // Falls through to the raw text.
  }
  return raw;
}

/**
 * The values a row on `field` can pick. `held` is what the row already
 * holds: the only choices a free-form custom field (text, number, date) can
 * offer. Custom values are JSON-encoded as `board-store` reads them.
 */
export function fieldChoices(
  field: string,
  sources: FilterSources,
  held: readonly string[] = [],
): Choice[] {
  switch (field) {
    case "status":
      return [...sources.columns]
        .sort((a, b) => a.position - b.position)
        .map((column) => [column.id, column.name]);
    case "priority":
      // Urgent first, as the menu always listed them.
      {
        // A hidden level is no new choice, but a row that already holds one
        // keeps it, so a saved filter doesn't change what it matches.
        const scheme = sources.priorities ?? DEFAULT_PRIORITY_SCHEME;
        const levels = [0, 1, 2, 3, 4].filter(
          (level) =>
            offeredPriorities(scheme).includes(level) ||
            held.includes(String(level)),
        );
        return levels
          .map((level): Choice => [String(level), scheme.names[level]])
          .reverse();
      }
    case "labels":
      return sources.labels.map((label) => [label.id, label.name]);
    case "projects":
      return sources.projects.map((project) => [project.path, project.name]);
    case "due":
      return DUE_CHOICES;
    case "created":
    case "updated":
      return WITHIN_CHOICES;
    case "linked":
      return LINKED_CHOICES;
  }
  const custom = fieldOf(sources, field);
  if (!custom) return [];
  switch (custom.kind) {
    case "select":
      return selectOptions(custom).map((option) => [
        JSON.stringify(option),
        option,
      ]);
    case "checkbox":
      return [
        ["true", "Yes"],
        ["false", "No"],
      ];
    default:
      return held.map((raw) => [raw, heldLabel(custom.kind, raw)]);
  }
}

/** A row's field as the user reads it; a deleted custom field says so. */
export function fieldTitle(field: string, sources: FilterSources): string {
  if (field.startsWith("field:")) {
    return fieldOf(sources, field)?.name ?? "Deleted field";
  }
  return FILTER_FIELDS.find(([known]) => known === field)?.[1] ?? field;
}

/** What a row says, for its chip: "Priority is Urgent, High". Values that no
 * longer exist are left out. */
export function chipText(row: FilterRow, sources: FilterSources): string {
  const choices = fieldChoices(row.field, sources, row.values);
  const names = row.values.flatMap((value) => {
    const found = choices.find(([known]) => known === value);
    return found ? [found[1]] : [];
  });
  const op = row.op === "is" ? "is" : "is not";
  return `${fieldTitle(row.field, sources)} ${op} ${
    names.length ? names.join(", ") : "…"
  }`;
}

/** Rows that filter something: those holding at least one value. */
export function activeRows(rows: readonly FilterRow[]): FilterRow[] {
  return rows.filter((row) => row.values.length > 0);
}

/**
 * `rows` without the values the board no longer has (a deleted label,
 * column, project, custom field or select option); a row left empty drops
 * out. Free-form custom values are kept: only the engine can tell.
 */
export function liveRows(
  rows: readonly FilterRow[],
  sources: FilterSources,
): FilterRow[] {
  return rows.flatMap((row) => {
    const free = fieldOf(sources, row.field)?.kind;
    const keepsHeld = free === "text" || free === "number" || free === "date";
    const known = new Set(
      fieldChoices(row.field, sources, row.values).map(([value]) => value),
    );
    // Numeric priorities, and the fixed lists, are known to the engine.
    const values = row.values.filter((value) => keepsHeld || known.has(value));
    return values.length ? [{ ...row, values }] : [];
  });
}

const isOp = (op: unknown): op is FilterOp => op === "is" || op === "is_not";

/**
 * A stored spec that no longer parses shows everything rather than failing.
 * Rows holding no values filter nothing, so they aren't part of the spec.
 */
export function parseSpec(json: string): ViewSpec {
  try {
    const raw: unknown = JSON.parse(json);
    if (!raw || typeof raw !== "object") return EMPTY_SPEC;
    const spec = raw as Partial<Record<keyof ViewSpec, unknown>>;
    const rows = Array.isArray(spec.rows)
      ? spec.rows.flatMap((row: unknown): FilterRow[] => {
          const candidate = row as Partial<FilterRow> | null;
          if (!candidate || typeof candidate.field !== "string") return [];
          return [
            {
              field: candidate.field,
              op: isOp(candidate.op) ? candidate.op : "is",
              values: Array.isArray(candidate.values)
                ? candidate.values.filter(
                    (value): value is string => typeof value === "string",
                  )
                : [],
            },
          ];
        })
      : [];
    const sort = spec.sort as Partial<SortSpec> | undefined;
    return {
      query: typeof spec.query === "string" ? spec.query.trim() : "",
      rows: activeRows(rows),
      sort: {
        key: SORT_KEYS.includes(sort?.key as SortKey)
          ? (sort!.key as SortKey)
          : "manual",
        dir: sort?.dir === "desc" ? "desc" : "asc",
      },
    };
  } catch {
    return EMPTY_SPEC;
  }
}

/** The spec a view saves and compares: trimmed search, rows that hold values. */
export function currentSpec(parts: {
  query: string;
  rows: readonly FilterRow[];
  sort: SortSpec;
}): ViewSpec {
  return {
    query: parts.query.trim(),
    rows: activeRows(parts.rows),
    sort: parts.sort,
  };
}

export function sameSpec(a: ViewSpec, b: ViewSpec): boolean {
  return JSON.stringify(normalised(a)) === JSON.stringify(normalised(b));
}

// One key order, so a stringify compare is exact.
function normalised(spec: ViewSpec): ViewSpec {
  return {
    query: spec.query,
    rows: spec.rows.map((row) => ({
      field: row.field,
      op: row.op,
      values: row.values,
    })),
    sort: { key: spec.sort.key, dir: spec.sort.dir },
  };
}

function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([a], [b]) => (a < b ? -1 : 1))
        .map(([key, inner]) => [key, canonical(inner)]),
    );
  }
  return value;
}

/** Whether two JSON texts say the same thing, whatever their key order. */
export function sameJson(a: string, b: string): boolean {
  try {
    return (
      JSON.stringify(canonical(JSON.parse(a))) ===
      JSON.stringify(canonical(JSON.parse(b)))
    );
  } catch {
    return a === b;
  }
}

/** The ids of cards whose linked session or Pilot is running now. */
export function runningCardIds(
  cards: readonly Pick<BoardCard, "id" | "linkKind" | "linkId">[],
  isRunning: (kind: CardLinkKind, id: string) => boolean,
): string[] {
  return cards
    .filter(
      (card) =>
        card.linkKind && card.linkId && isRunning(card.linkKind, card.linkId),
    )
    .map((card) => card.id)
    .sort();
}

/** The views the tab strip lists, in their stored order. */
export const orderedViews = (views: readonly BoardView[]): BoardView[] =>
  [...views].sort((a, b) => a.position - b.position);
