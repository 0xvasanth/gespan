import { describe, expect, it } from "vitest";
import { nextCardFocus, nextLaneFocus } from "./cardFocus";

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

// Lanes top to bottom, each with the board's columns left to right. "x" is a
// card with two labels, so it sits in two lanes.
const lanes = [
  {
    key: "bug",
    columns: [
      { id: "todo", cardIds: ["b0", "b1", "x"] },
      { id: "doing", cardIds: [] },
      { id: "done", cardIds: ["d0"] },
    ],
  },
  {
    key: "docs",
    columns: [
      { id: "todo", cardIds: [] },
      { id: "doing", cardIds: ["e0"] },
      { id: "done", cardIds: [] },
    ],
  },
  {
    key: "ui",
    columns: [
      { id: "todo", cardIds: ["x", "u1"] },
      { id: "doing", cardIds: ["u2"] },
      { id: "done", cardIds: ["u3", "u4"] },
    ],
  },
];
const at = (lane: string, cardId: string) => ({ lane, cardId });

describe("nextLaneFocus", () => {
  it("moves up and down inside a cell", () => {
    expect(nextLaneFocus(lanes, at("bug", "b0"), "ArrowDown")).toEqual(at("bug", "b1"));
    expect(nextLaneFocus(lanes, at("bug", "b1"), "ArrowUp")).toEqual(at("bug", "b0"));
  });

  it("crosses into the next lane with cards in the same column, skipping empty cells", () => {
    // Bug/todo ends at x; Docs has no todo cards; Ui/todo starts with x again.
    expect(nextLaneFocus(lanes, at("bug", "x"), "ArrowDown")).toEqual(at("ui", "x"));
    expect(nextLaneFocus(lanes, at("ui", "x"), "ArrowUp")).toEqual(at("bug", "x"));
    expect(nextLaneFocus(lanes, at("docs", "e0"), "ArrowDown")).toEqual(at("ui", "u2"));
    expect(nextLaneFocus(lanes, at("ui", "u2"), "ArrowUp")).toEqual(at("docs", "e0"));
    // ArrowUp into a lane lands on that cell's last card.
    expect(nextLaneFocus(lanes, at("ui", "u3"), "ArrowUp")).toEqual(at("bug", "d0"));
  });

  it("stops at the top of the first lane and the bottom of the last", () => {
    expect(nextLaneFocus(lanes, at("bug", "b0"), "ArrowUp")).toBeNull();
    expect(nextLaneFocus(lanes, at("ui", "u4"), "ArrowDown")).toBeNull();
    expect(nextLaneFocus(lanes, at("docs", "e0"), "ArrowUp")).toBeNull();
  });

  it("moves sideways inside the lane: same index in the nearest column with cards, clamped", () => {
    expect(nextLaneFocus(lanes, at("bug", "b1"), "ArrowRight")).toEqual(at("bug", "d0"));
    expect(nextLaneFocus(lanes, at("bug", "d0"), "ArrowLeft")).toEqual(at("bug", "b0"));
    expect(nextLaneFocus(lanes, at("ui", "u1"), "ArrowRight")).toEqual(at("ui", "u2"));
    expect(nextLaneFocus(lanes, at("ui", "u2"), "ArrowRight")).toEqual(at("ui", "u3"));
    // Never into another lane, even when this lane has nothing that way.
    expect(nextLaneFocus(lanes, at("docs", "e0"), "ArrowRight")).toBeNull();
    expect(nextLaneFocus(lanes, at("docs", "e0"), "ArrowLeft")).toBeNull();
  });

  it("follows the instance in the lane it was focused in, for a card in two lanes", () => {
    expect(nextLaneFocus(lanes, at("ui", "x"), "ArrowDown")).toEqual(at("ui", "u1"));
    expect(nextLaneFocus(lanes, at("bug", "x"), "ArrowUp")).toEqual(at("bug", "b1"));
  });

  it("returns null for a card or lane that isn't on the board", () => {
    expect(nextLaneFocus(lanes, at("bug", "u1"), "ArrowDown")).toBeNull();
    expect(nextLaneFocus(lanes, at("gone", "b0"), "ArrowDown")).toBeNull();
  });

  it("counts from the data however many cards a lane holds", () => {
    const long = [
      { key: "a", columns: [{ id: "c", cardIds: Array.from({ length: 700 }, (_, i) => `k${i}`) }] },
      { key: "b", columns: [{ id: "c", cardIds: Array.from({ length: 300 }, (_, i) => `m${i}`) }] },
    ];
    expect(nextLaneFocus(long, at("a", "k699"), "ArrowDown")).toEqual(at("b", "m0"));
    expect(nextLaneFocus(long, at("b", "m0"), "ArrowUp")).toEqual(at("a", "k699"));
  });
});
