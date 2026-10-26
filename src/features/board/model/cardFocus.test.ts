import { describe, expect, it } from "vitest";
import { nextCardFocus } from "./cardFocus";

const board = [
  { id: "backlog", cardIds: ["a0", "a1", "a2", "a3"] },
  { id: "empty", cardIds: [] },
  { id: "progress", cardIds: ["b0", "b1"] },
  { id: "done", cardIds: ["c0"] },
];

describe("nextCardFocus", () => {
  it("moves to the previous and next card in the column", () => {
    expect(nextCardFocus(board, "a1", "ArrowDown")).toBe("a2");
    expect(nextCardFocus(board, "a1", "ArrowUp")).toBe("a0");
  });

  it("stops at either end of the column", () => {
    expect(nextCardFocus(board, "a0", "ArrowUp")).toBeNull();
    expect(nextCardFocus(board, "a3", "ArrowDown")).toBeNull();
    expect(nextCardFocus(board, "c0", "ArrowDown")).toBeNull();
  });

  it("moves sideways to the same index, skipping empty columns", () => {
    expect(nextCardFocus(board, "a1", "ArrowRight")).toBe("b1");
    expect(nextCardFocus(board, "b0", "ArrowLeft")).toBe("a0");
    expect(nextCardFocus(board, "b1", "ArrowLeft")).toBe("a1");
  });

  it("clamps the index to the last card of a shorter column", () => {
    expect(nextCardFocus(board, "a3", "ArrowRight")).toBe("b1");
    expect(nextCardFocus(board, "b1", "ArrowRight")).toBe("c0");
  });

  it("returns null past the first and last columns", () => {
    expect(nextCardFocus(board, "a2", "ArrowLeft")).toBeNull();
    expect(nextCardFocus(board, "c0", "ArrowRight")).toBeNull();
  });

  it("returns null for a card that isn't on the board", () => {
    expect(nextCardFocus(board, "zz", "ArrowDown")).toBeNull();
    expect(nextCardFocus([], "a0", "ArrowRight")).toBeNull();
  });

  it("counts from the data however far down the card is", () => {
    const long = [
      { id: "x", cardIds: Array.from({ length: 1000 }, (_, i) => `k${i}`) },
      { id: "y", cardIds: Array.from({ length: 600 }, (_, i) => `m${i}`) },
    ];
    expect(nextCardFocus(long, "k500", "ArrowDown")).toBe("k501");
    expect(nextCardFocus(long, "k900", "ArrowRight")).toBe("m599");
    expect(nextCardFocus(long, "m42", "ArrowLeft")).toBe("k42");
  });
});
