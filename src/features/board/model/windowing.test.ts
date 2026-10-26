import { describe, expect, it } from "vitest";
import { type Slot, visibleRange, windowSlots } from "./windowing";

describe("visibleRange", () => {
  it("renders the rows in view plus the overscan", () => {
    expect(visibleRange(0, 320, 32, 1000, 5)).toEqual({ start: 0, end: 15 });
    expect(visibleRange(3200, 320, 32, 1000, 5)).toEqual({ start: 95, end: 115 });
  });

  it("clamps at both ends and for short lists", () => {
    expect(visibleRange(32 * 999, 320, 32, 1000, 5)).toEqual({
      start: 994,
      end: 1000,
    });
    expect(visibleRange(0, 320, 32, 3, 5)).toEqual({ start: 0, end: 3 });
    expect(visibleRange(0, 320, 32, 0, 5)).toEqual({ start: 0, end: 0 });
    expect(visibleRange(-50, 320, 32, 100, 0)).toEqual({ start: 0, end: 10 });
  });

  it("bounds the rendered rows however long the list", () => {
    const { start, end } = visibleRange(10_000, 640, 32, 100_000, 6);
    expect(end - start).toBeLessThanOrEqual(640 / 32 + 2 * 6 + 1);
  });
});

describe("windowSlots", () => {
  const items = (slots: Slot[]) =>
    slots.flatMap((slot) => (slot.kind === "item" ? [slot.index] : []));
  /** Laid out with `gap` between slots, as the column body does. */
  const laidOut = (slots: Slot[], heights: number[], gap: number) =>
    slots.reduce(
      (sum, slot) =>
        sum + (slot.kind === "gap" ? slot.height : heights[slot.index]),
      0,
    ) +
    Math.max(0, slots.length - 1) * gap;

  it("renders the items in view plus the overscan, with gaps for the rest", () => {
    // 100 items of 50px + 10px gaps: item i starts at 60 * i.
    const heights = Array.from({ length: 100 }, () => 50);
    const slots = windowSlots(heights, 10, 600, 300, 2, []);
    // In view: items 10..14 (600..900); overscan 2 each side.
    expect(items(slots)).toEqual([8, 9, 10, 11, 12, 13, 14, 15, 16]);
    expect(slots[0]).toEqual({ kind: "gap", height: 8 * 60 - 10 });
    expect(slots.at(-1)).toEqual({ kind: "gap", height: 83 * 60 - 10 });
    // At the top there is no leading gap.
    const top = windowSlots(heights, 10, 0, 300, 2, []);
    expect(top[0]).toEqual({ kind: "item", index: 0 });
    expect(items(top)).toEqual([0, 1, 2, 3, 4, 5, 6]);
  });

  it("uses each item's own height to find the view", () => {
    // A tall item 0 (500px) pushes the view past the short ones.
    const heights = [500, ...Array.from({ length: 50 }, () => 20)];
    const slots = windowSlots(heights, 0, 520, 40, 0, []);
    expect(items(slots)).toEqual([2, 3]);
    expect(slots[0]).toEqual({ kind: "gap", height: 520 });
  });

  it("keeps a pinned item far below the view, between two gaps", () => {
    const heights = Array.from({ length: 1000 }, () => 50);
    const slots = windowSlots(heights, 10, 0, 300, 1, [900]);
    expect(items(slots)).toEqual([0, 1, 2, 3, 4, 5, 900]);
    const pinnedAt = slots.findIndex(
      (slot) => slot.kind === "item" && slot.index === 900,
    );
    // The gap before it spans items 6..899, so it keeps its offset.
    expect(slots[pinnedAt - 1]).toEqual({ kind: "gap", height: 894 * 60 - 10 });
    expect(slots[pinnedAt + 1]).toEqual({ kind: "gap", height: 99 * 60 - 10 });
    // A pinned item already in view, or out of range, adds nothing.
    expect(items(windowSlots(heights, 10, 0, 300, 1, [2, -1, 1000, 1.5]))).toEqual(
      [0, 1, 2, 3, 4, 5],
    );
  });

  it("gaps sum to the total height wherever the view is", () => {
    const heights = Array.from({ length: 300 }, (_, i) => 30 + ((i * 37) % 70));
    const gap = 6;
    const total =
      heights.reduce((sum, h) => sum + h, 0) + (heights.length - 1) * gap;
    for (const scrollTop of [0, 1234, 9_999, total - 200, total + 500]) {
      for (const pinned of [[], [0], [150, 299], [299]]) {
        const slots = windowSlots(heights, gap, scrollTop, 400, 4, pinned);
        expect(laidOut(slots, heights, gap)).toBe(total);
        // Items are in order and never repeated.
        const shown = items(slots);
        expect(shown).toEqual([...new Set(shown)].sort((a, b) => a - b));
        // No two gaps in a row.
        slots.forEach((slot, i) => {
          if (slot.kind === "gap") expect(slots[i + 1]?.kind).not.toBe("gap");
        });
      }
    }
  });

  it("clamps a view past the end to the last items", () => {
    const heights = Array.from({ length: 20 }, () => 50);
    expect(items(windowSlots(heights, 10, 99_999, 300, 1, []))).toEqual([18, 19]);
  });

  it("renders nothing for an empty list", () => {
    expect(windowSlots([], 6, 0, 600, 4, [])).toEqual([]);
    expect(windowSlots([], 6, 500, 600, 4, [3])).toEqual([]);
  });

  it("bounds the rendered items however long the list", () => {
    const heights = Array.from({ length: 100_000 }, () => 44);
    const slots = windowSlots(heights, 6, 2_000_000, 1400, 4, [5, 99_999]);
    expect(items(slots).length).toBeLessThanOrEqual(Math.ceil(1400 / 50) + 1 + 8 + 2);
  });
});
