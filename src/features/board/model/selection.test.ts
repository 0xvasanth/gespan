import { describe, expect, it } from "vitest";
import {
  EMPTY_SELECTION,
  pruneSelection,
  rangeSelect,
  toggleSelect,
} from "./selection";

const ORDER = ["a", "b", "c", "d", "e"];

describe("selection", () => {
  it("toggles a card on and off and anchors on the last click", () => {
    const on = toggleSelect(EMPTY_SELECTION, "b");
    expect(on).toEqual({ ids: ["b"], anchor: "b" });
    const two = toggleSelect(on, "d");
    expect(two.ids).toEqual(["b", "d"]);
    expect(toggleSelect(two, "b")).toEqual({ ids: ["d"], anchor: "b" });
  });

  it("selects the range from the anchor in either direction, replacing the selection", () => {
    const start = toggleSelect(EMPTY_SELECTION, "b");
    expect(rangeSelect(start, "d", ORDER).ids).toEqual(["b", "c", "d"]);
    expect(rangeSelect(start, "a", ORDER).ids).toEqual(["a", "b"]);
    // A second range from the same anchor replaces the first.
    const first = rangeSelect(start, "e", ORDER);
    expect(rangeSelect(first, "c", ORDER).ids).toEqual(["b", "c"]);
  });

  it("follows the order it is given, so a new sort changes the range", () => {
    const start = toggleSelect(EMPTY_SELECTION, "a");
    expect(rangeSelect(start, "c", ORDER).ids).toEqual(["a", "b", "c"]);
    expect(rangeSelect(start, "c", ["c", "e", "a", "b", "d"]).ids).toEqual([
      "c",
      "e",
      "a",
    ]);
  });

  it("selects just the clicked card with no anchor, or one that left the order", () => {
    expect(rangeSelect(EMPTY_SELECTION, "c", ORDER)).toEqual({
      ids: ["c"],
      anchor: "c",
    });
    const gone = { ids: ["x"], anchor: "x" };
    expect(rangeSelect(gone, "c", ORDER)).toEqual({ ids: ["c"], anchor: "c" });
  });

  it("prunes hidden cards but keeps the anchor of a card that still exists", () => {
    const sel = { ids: ["a", "b", "c"], anchor: "a" };
    const exists = new Set(ORDER);
    const pruned = pruneSelection(sel, new Set(["b", "c"]), exists);
    expect(pruned).toEqual({ ids: ["b", "c"], anchor: "a" });
    expect(pruneSelection(sel, new Set(["b", "c"]), new Set(["b", "c"]))).toEqual(
      { ids: ["b", "c"], anchor: null },
    );
  });

  it("returns the same object when nothing needs pruning", () => {
    const sel = { ids: ["a"], anchor: "a" };
    expect(pruneSelection(sel, new Set(ORDER), new Set(ORDER))).toBe(sel);
  });
});
