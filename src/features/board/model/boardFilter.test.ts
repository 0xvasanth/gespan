import { describe, expect, it } from "vitest";
import { fullIndex } from "./boardFilter";

describe("fullIndex", () => {
  const all = ["a", "b", "c", "d", "e"];
  const visible = ["a", "c", "e"];

  it("lands before the visible card it points at", () => {
    expect(fullIndex(all, visible, 0)).toBe(0);
    expect(fullIndex(all, visible, 1)).toBe(2);
    expect(fullIndex(all, visible, 2)).toBe(4);
  });

  it("lands right after the last visible card, ahead of trailing hidden ones", () => {
    expect(fullIndex(["a", "b", "c", "d"], ["a", "c"], 2)).toBe(3);
  });

  it("goes to the end when nothing in the column is visible", () => {
    expect(fullIndex(["x", "y"], [], 0)).toBe(2);
    expect(fullIndex([], [], 0)).toBe(0);
  });
});
