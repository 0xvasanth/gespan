import type { BoardCard, BoardLabel, CardExtras } from "./board";

export type BoardFilter = {
  projects: Set<string>;
  labels: Set<string>;
  priorities: Set<number>;
};

export const EMPTY_FILTER: BoardFilter = {
  projects: new Set(),
  labels: new Set(),
  priorities: new Set(),
};

/** How many filter values are set; the badge on the Filter button. */
export function filterCount(filter: BoardFilter): number {
  return filter.projects.size + filter.labels.size + filter.priorities.size;
}

const anyIn = <T>(wanted: ReadonlySet<T>, have: readonly T[]) =>
  have.some((value) => wanted.has(value));

/**
 * Whether a card shows. `#N` is an exact card number, anything else a
 * case-insensitive title substring. Each filter matches any of its values;
 * the search and the three filters all have to hold.
 */
export function matchesCard(
  card: Pick<BoardCard, "number" | "title">,
  extras: CardExtras | undefined,
  query: string,
  filter: BoardFilter,
): boolean {
  const q = query.trim().toLowerCase();
  if (q.startsWith("#")) {
    if (String(card.number) !== q.slice(1)) return false;
  } else if (q && !card.title.toLowerCase().includes(q)) {
    return false;
  }
  if (filter.projects.size && !anyIn(filter.projects, extras?.projects ?? [])) {
    return false;
  }
  if (filter.labels.size && !anyIn(filter.labels, extras?.labels ?? [])) {
    return false;
  }
  if (filter.priorities.size && !filter.priorities.has(extras?.priority ?? 0)) {
    return false;
  }
  return true;
}

function keep<T>(set: Set<T>, known: (value: T) => boolean): Set<T> {
  const kept = new Set([...set].filter(known));
  return kept.size === set.size ? set : kept;
}

/**
 * Drops filter values whose label or project is gone, so a deleted label
 * can't leave a filter that matches nothing. Returns `filter` itself when
 * nothing was dropped.
 */
export function pruneFilter(
  filter: BoardFilter,
  labels: readonly Pick<BoardLabel, "id">[],
  projects: readonly { path: string }[],
): BoardFilter {
  const labelIds = new Set(labels.map((label) => label.id));
  const paths = new Set(projects.map((project) => project.path));
  const next = {
    projects: keep(filter.projects, (path) => paths.has(path)),
    labels: keep(filter.labels, (id) => labelIds.has(id)),
    priorities: filter.priorities,
  };
  return next.projects === filter.projects && next.labels === filter.labels
    ? filter
    : next;
}

/**
 * A drop index among the visible cards, as an index among all of a column's
 * cards: before the visible card it points at, or after the last visible
 * one. `all` and `visible` exclude the moved card; `visible` is in the same
 * order as `all`.
 */
export function fullIndex(
  all: readonly string[],
  visible: readonly string[],
  index: number,
): number {
  const at = visible[index];
  if (at !== undefined) return all.indexOf(at);
  const last = visible[visible.length - 1];
  return last === undefined ? all.length : all.indexOf(last) + 1;
}
