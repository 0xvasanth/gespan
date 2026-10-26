import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

/** The rows to render for a scroll position: [start, end) with `overscan` spare rows each side. */
export function visibleRange(
  scrollTop: number,
  viewport: number,
  rowHeight: number,
  count: number,
  overscan: number,
): { start: number; end: number } {
  if (count <= 0) return { start: 0, end: 0 };
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const last = Math.ceil((Math.max(0, scrollTop) + viewport) / rowHeight);
  return {
    start: Math.min(count, Math.max(0, first - overscan)),
    end: Math.min(count, Math.max(0, last + overscan)),
  };
}

/** Used until the scroller reports a height (it has none before layout). */
const FALLBACK_VIEWPORT = 640;

/**
 * Fixed-row-height windowing for a scroll container: attach `ref` and
 * `onScroll` to it, render rows `start..end` positioned at `row * rowHeight`
 * inside a block `totalHeight` tall.
 */
export function useWindowedRows({
  count,
  rowHeight,
  overscan = 6,
  topOffset = 0,
}: {
  count: number;
  rowHeight: number;
  overscan?: number;
  /** Height of what sits above the first row inside the scroller (a sticky header). */
  topOffset?: number;
}) {
  const element = useRef<HTMLDivElement | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(FALLBACK_VIEWPORT);

  const measure = useCallback(() => {
    const node = element.current;
    if (!node) return;
    setViewport(node.clientHeight || FALLBACK_VIEWPORT);
    setScrollTop(node.scrollTop);
  }, []);

  const ref = useCallback(
    (node: HTMLDivElement | null) => {
      element.current = node;
      if (node) measure();
    },
    [measure],
  );

  useEffect(() => {
    const node = element.current;
    if (!node || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [measure]);

  return {
    ref,
    onScroll: measure,
    totalHeight: count * rowHeight,
    ...visibleRange(scrollTop - topOffset, viewport, rowHeight, count, overscan),
  };
}

/** One row of a variable-height window: a spacer for skipped items, or an item. */
export type Slot =
  | { kind: "gap"; height: number }
  | { kind: "item"; index: number };

/** First-guess card heights (px), replaced per card once it is measured. */
export const CARD_ESTIMATE = { comfortable: 76, compact: 44 } as const;
/** The space between two cards (the column body's `gap-1.5`). */
export const CARD_GAP = 6;
/** Cards rendered beyond each edge of the view. */
export const CARD_OVERSCAN = 4;

/** `offsets[i]` is the top of item i; `offsets[n]` is the total plus one trailing gap. */
function itemOffsets(heights: readonly number[], gap: number): number[] {
  const offsets = new Array<number>(heights.length + 1);
  offsets[0] = 0;
  for (let i = 0; i < heights.length; i += 1) {
    offsets[i + 1] = offsets[i] + heights[i] + gap;
  }
  return offsets;
}

/** The index of the item under `y`: the first whose bottom is below it, clamped. */
function itemAt(offsets: readonly number[], gap: number, y: number): number {
  const count = offsets.length - 1;
  let lo = 0;
  let hi = count - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (offsets[mid + 1] - gap > y) hi = mid;
    else lo = mid + 1;
  }
  return lo;
}

/** The last item that starts above `y` (0 when none does). */
function lastStartingBefore(offsets: readonly number[], y: number): number {
  let lo = 0;
  let hi = offsets.length - 2;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (offsets[mid] < y) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

/**
 * The slots to render for a scroll position in a list of variable-height
 * items laid out with `gap` between them: the items in view plus `overscan`
 * each side, and every `pinned` index wherever it is, in order. Each run of
 * skipped items becomes one gap whose height is the run's heights and the
 * gaps inside it, so with `gap` between slots the list keeps its full height
 * and every item keeps its offset.
 */
export function windowSlots(
  heights: readonly number[],
  gap: number,
  scrollTop: number,
  viewport: number,
  overscan: number,
  pinned: readonly number[],
): Slot[] {
  const count = heights.length;
  if (count === 0) return [];
  const offsets = itemOffsets(heights, gap);
  const top = Math.max(0, scrollTop);
  const first = itemAt(offsets, gap, top);
  const last = Math.max(first, lastStartingBefore(offsets, top + viewport));
  const start = Math.max(0, first - overscan);
  const end = Math.min(count, last + 1 + overscan);
  const shown = new Set<number>();
  for (let i = start; i < end; i += 1) shown.add(i);
  for (const index of pinned) {
    if (Number.isInteger(index) && index >= 0 && index < count) shown.add(index);
  }
  const slots: Slot[] = [];
  let next = 0;
  const skip = (to: number) => {
    if (to > next) {
      slots.push({
        kind: "gap",
        height: offsets[to] - offsets[next] - gap,
      });
    }
  };
  for (const index of [...shown].sort((a, b) => a - b)) {
    skip(index);
    slots.push({ kind: "item", index });
    next = index + 1;
  }
  skip(count);
  return slots;
}

type Measured = { estimate: number; height: number };
type Anchor = { id: string; offset: number };

/**
 * Variable-height windowing for one scroll container of keyed items (a board
 * column). Heights start at `estimate` and are replaced per id once an item
 * is measured; a new `estimate` (a density change) drops every measurement.
 * Once scrolled, the item at the top of the view is the anchor: when ids or
 * heights change above it (a reload, a card added above, a re-measure), the
 * scroll moves with it so it stays where it was. `pinned` ids are always
 * rendered (focus, a drag, an open drawer).
 */
export function useMeasuredWindow({
  ids,
  estimate,
  gap,
  overscan,
  pinned,
}: {
  ids: readonly string[];
  estimate: number;
  gap: number;
  overscan: number;
  pinned: readonly string[];
}) {
  const element = useRef<HTMLElement | null>(null);
  const measured = useRef(new Map<string, Measured>());
  const nodes = useRef(new Map<string, HTMLElement>());
  const measureRefs = useRef(new Map<string, (node: HTMLElement | null) => void>());
  const anchor = useRef<Anchor | null>(null);
  const [version, setVersion] = useState(0);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(FALLBACK_VIEWPORT);

  const indexById = useMemo(
    () => new Map(ids.map((id, index) => [id, index])),
    [ids],
  );
  const heights = useMemo(() => {
    void version; // measurements live in a ref; `version` marks a change
    return ids.map((id) => {
      const entry = measured.current.get(id);
      return entry && entry.estimate === estimate ? entry.height : estimate;
    });
  }, [ids, estimate, version]);
  const offsets = useMemo(() => itemOffsets(heights, gap), [heights, gap]);
  const pinnedIndexes = useMemo(
    () => pinned.flatMap((id) => indexById.get(id) ?? []),
    [pinned, indexById],
  );
  const slots = useMemo(
    () =>
      windowSlots(heights, gap, scrollTop, viewport, overscan, pinnedIndexes),
    [heights, gap, scrollTop, viewport, overscan, pinnedIndexes],
  );

  const isDragging = useCallback(
    (id: string) => !!nodes.current.get(id)?.hasAttribute("data-dragging"),
    [],
  );

  const latest = useRef({ ids, offsets });
  latest.current = { ids, offsets };

  /** The item at `top` and how far into it the view starts. */
  const anchorAt = useCallback(
    (top: number): Anchor | null => {
      const { ids: list, offsets: at } = latest.current;
      if (top <= 0 || list.length === 0) return null;
      let index = itemAt(at, gap, top);
      // The dragged card moves with the pointer; anchoring to it would
      // scroll the column after it.
      while (index + 1 < list.length && isDragging(list[index])) index += 1;
      return { id: list[index], offset: top - at[index] };
    },
    [gap, isDragging],
  );

  const sync = useCallback(() => {
    const node = element.current;
    if (!node) return;
    setViewport(node.clientHeight || FALLBACK_VIEWPORT);
    setScrollTop(node.scrollTop);
    anchor.current = anchorAt(node.scrollTop);
  }, [anchorAt]);

  // The scroller can come and go (a column collapsed, then expanded), so the
  // observer follows the node the ref is given, never the first one.
  const observer = useRef<ResizeObserver | null>(null);
  const ref = useCallback(
    (node: HTMLElement | null) => {
      observer.current?.disconnect();
      observer.current = null;
      element.current = node;
      if (!node) return;
      sync();
      if (typeof ResizeObserver === "undefined") return;
      observer.current = new ResizeObserver(() => {
        if (element.current === node) {
          setViewport(node.clientHeight || FALLBACK_VIEWPORT);
        }
      });
      observer.current.observe(node);
    },
    [sync],
  );
  useEffect(
    () => () => {
      observer.current?.disconnect();
      observer.current = null;
    },
    [],
  );

  useLayoutEffect(() => {
    // Re-measure what is mounted: a density toggle, new labels or a longer
    // title change a card's height after it was first measured. The dragged
    // card's slot is measured as the line it shows, so the spacers match.
    let changed = false;
    for (const [id, node] of nodes.current) {
      const height = node.getBoundingClientRect().height;
      // Zero is "not laid out" (hidden, or no layout engine), not a height.
      if (!(height > 0)) continue;
      const entry = measured.current.get(id);
      if (
        !entry ||
        entry.estimate !== estimate ||
        Math.abs(entry.height - height) > 0.5
      ) {
        measured.current.set(id, { estimate, height });
        changed = true;
      }
    }
    if (changed) {
      setVersion((v) => v + 1);
      return;
    }
    // Keep the anchor where it was in the view.
    const node = element.current;
    const held = anchor.current;
    if (!node || !held) return;
    const index = indexById.get(held.id);
    if (index === undefined || isDragging(held.id)) {
      // The top card is gone (or lifted): stay at this offset, under the
      // card now there.
      anchor.current = anchorAt(node.scrollTop);
      return;
    }
    const wanted = offsets[index] + held.offset;
    if (Math.abs(node.scrollTop - wanted) > 0.5) {
      node.scrollTop = wanted;
      setScrollTop(node.scrollTop);
    }
  });

  const measureRef = useCallback((id: string) => {
    let callback = measureRefs.current.get(id);
    if (!callback) {
      callback = (node: HTMLElement | null) => {
        if (node) {
          nodes.current.set(id, node);
          return;
        }
        // A card unmounted mid-drag was measured as its drop line, not its
        // height: forget that so it comes back at its estimate.
        if (nodes.current.get(id)?.hasAttribute("data-dragging")) {
          measured.current.delete(id);
        }
        nodes.current.delete(id);
      };
      measureRefs.current.set(id, callback);
    }
    return callback;
  }, []);

  /** Scrolls the least distance that shows item `index` whole. */
  const scrollToIndex = useCallback(
    (index: number) => {
      const node = element.current;
      const { offsets: at } = latest.current;
      if (!node || index < 0 || index >= at.length - 1) return;
      const top = at[index];
      const bottom = at[index + 1] - gap;
      const view = node.clientHeight || FALLBACK_VIEWPORT;
      let next = node.scrollTop;
      if (top < next) next = top;
      else if (bottom > next + view) next = bottom - view;
      else return;
      node.scrollTop = next;
      sync();
    },
    [gap, sync],
  );

  return { ref, onScroll: sync, slots, measureRef, scrollToIndex };
}
