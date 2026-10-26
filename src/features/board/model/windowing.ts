import { useCallback, useEffect, useRef, useState } from "react";

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
