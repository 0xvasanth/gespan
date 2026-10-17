import { useState } from "react";

/** The active tab plus the two before it switch back instantly. */
export const WARM_TAB_COUNT = 3;
export const WARM_MONO_COUNT = 1;

/**
 * The last `count` distinct ids that were current, most recent first. Hidden
 * surfaces outside this set release their transcripts to the pool's budget.
 */
export function useRecentIds(
  current: string | null | undefined,
  count: number,
): readonly string[] {
  const [recent, setRecent] = useState<readonly string[]>(() =>
    current ? [current] : [],
  );
  if (current && recent[0] !== current) {
    const next = [current, ...recent.filter((id) => id !== current)].slice(
      0,
      count,
    );
    setRecent(next);
    return next;
  }
  return recent;
}
