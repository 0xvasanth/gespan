import { useRef } from "react";

/** The active tab plus the two before it switch back instantly. */
export const WARM_TAB_COUNT = 3;
export const WARM_MONO_COUNT = 1;

/**
 * Ids that were current once but are no longer among the last `keep`. Hidden
 * surfaces in this set release their transcripts to the pool's budget. One
 * never shown stays mounted: it renders no turns, and mounting its transcript
 * during the first visit would slow that switch.
 *
 * Derived during render without a state update, which would render the whole
 * workspace twice on every switch.
 */
export function useReleasedIds(
  current: string | null | undefined,
  keep: number,
): ReadonlySet<string> {
  const state = useRef({
    recent: [] as readonly string[],
    released: new Set<string>() as ReadonlySet<string>,
  });
  const { recent, released } = state.current;
  if (current && recent[0] !== current) {
    const next = [current, ...recent.filter((id) => id !== current)];
    const nextReleased = new Set(released);
    nextReleased.delete(current);
    for (const id of next.slice(keep)) nextReleased.add(id);
    state.current = { recent: next.slice(0, keep), released: nextReleased };
  }
  return state.current.released;
}
