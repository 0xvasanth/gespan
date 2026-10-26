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
