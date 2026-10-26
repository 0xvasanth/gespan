import { describe, expect, it } from "vitest";
import { visibleRange } from "./windowing";

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
