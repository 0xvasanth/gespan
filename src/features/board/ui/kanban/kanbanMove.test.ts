import { describe, expect, it } from "vitest";
import type {
  BoardCard,
  BoardColumn,
  BoardDetail,
  ColumnRole,
} from "../../model/board";
import {
  applyCardMove,
  applyColumnMove,
  columnBodyId,
  columnCards,
  finalCardTarget,
  resolveCardDrop,
  resolveColumnDrop,
  sortedColumns,
  toDropRef,
} from "./kanbanMove";

function column(
  id: string,
  name: string,
  position: number,
  role: ColumnRole | null,
): BoardColumn {
  return { id, boardId: "b1", name, position, role };
}

function card(
  id: string,
  columnId: string,
  position: number,
  number: number,
  title = `Card ${number}`,
): BoardCard {
  return {
    id,
    boardId: "b1",
    columnId,
    number,
    title,
    body: "",
    position,
    linkKind: null,
    linkId: null,
    createdAt: 1,
    updatedAt: 1,
  };
}

// Arrays are deliberately out of position order so the code has to sort.
function board(): BoardDetail {
  return {
    board: { id: "b1", name: "Board", createdAt: 1, updatedAt: 1 },
    columns: [
      column("progress", "In Progress", 1, "doing"),
      column("backlog", "Backlog", 0, "todo"),
      column("done", "Done", 3, "done"),
      column("review", "Review", 2, "review"),
    ],
    cards: [
      card("c13", "backlog", 2, 13),
      card("c21", "progress", 0, 21),
      card("c11", "backlog", 0, 11),
      card("c41", "done", 0, 41),
      card("c14", "backlog", 3, 14),
      card("c22", "progress", 1, 22),
      card("c12", "backlog", 1, 12, "Fix login"),
    ],
  };
}

function order(detail: BoardDetail, columnId: string) {
  return columnCards(detail, columnId).map((c) => [c.id, c.position]);
}

function expectContiguous(detail: BoardDetail) {
  for (const col of detail.columns) {
    const positions = columnCards(detail, col.id).map((c) => c.position);
    expect(positions).toEqual(positions.map((_, i) => i));
  }
}

describe("resolveCardDrop", () => {
  it("over a card in another column gives that card's index", () => {
    expect(resolveCardDrop(board(), "c12", "c21", "card")).toEqual({
      columnId: "progress",
      index: 0,
    });
    expect(resolveCardDrop(board(), "c12", "c22", "card")).toEqual({
      columnId: "progress",
      index: 1,
    });
  });

  it("over an empty column's body gives index 0", () => {
    expect(resolveCardDrop(board(), "c12", "review", "column")).toEqual({
      columnId: "review",
      index: 0,
    });
  });

  it("over a non-empty column's body gives the end", () => {
    expect(resolveCardDrop(board(), "c12", "progress", "column")).toEqual({
      columnId: "progress",
      index: 2,
    });
  });

  it("over its own column's body moves it to the end of that column", () => {
    expect(resolveCardDrop(board(), "c12", "backlog", "column")).toEqual({
      columnId: "backlog",
      index: 3,
    });
  });

  it("over its own column's body returns null when it is already last", () => {
    expect(resolveCardDrop(board(), "c14", "backlog", "column")).toBeNull();
  });

  it("over itself returns null", () => {
    expect(resolveCardDrop(board(), "c12", "c12", "card")).toBeNull();
  });

  it("over its current neighbour in the same column gives the neighbour's index", () => {
    expect(resolveCardDrop(board(), "c12", "c13", "card")).toEqual({
      columnId: "backlog",
      index: 2,
    });
    expect(resolveCardDrop(board(), "c12", "c11", "card")).toEqual({
      columnId: "backlog",
      index: 0,
    });
  });

  it("returns null for unknown cards or targets", () => {
    expect(resolveCardDrop(board(), "nope", "c21", "card")).toBeNull();
    expect(resolveCardDrop(board(), "c12", "nope", "card")).toBeNull();
    expect(resolveCardDrop(board(), "c12", "nope", "column")).toBeNull();
  });
});

describe("applyCardMove", () => {
  it("moves within a column from 0 to 2 of 4 and renumbers", () => {
    const next = applyCardMove(board(), "c11", {
      columnId: "backlog",
      index: 2,
    });
    expect(order(next, "backlog")).toEqual([
      ["c12", 0],
      ["c13", 1],
      ["c11", 2],
      ["c14", 3],
    ]);
    expectContiguous(next);
  });

  it("moves across columns, renumbering both and keeping the other cards' order", () => {
    const next = applyCardMove(board(), "c12", {
      columnId: "progress",
      index: 1,
    });
    expect(order(next, "backlog")).toEqual([
      ["c11", 0],
      ["c13", 1],
      ["c14", 2],
    ]);
    expect(order(next, "progress")).toEqual([
      ["c21", 0],
      ["c12", 1],
      ["c22", 2],
    ]);
    expect(next.cards.find((c) => c.id === "c12")?.columnId).toBe("progress");
    expect(order(next, "done")).toEqual([["c41", 0]]);
    expectContiguous(next);
  });

  it("moves into an empty column", () => {
    const next = applyCardMove(board(), "c12", {
      columnId: "review",
      index: 0,
    });
    expect(order(next, "review")).toEqual([["c12", 0]]);
    expectContiguous(next);
  });

  it("clamps an index past the end", () => {
    const across = applyCardMove(board(), "c12", {
      columnId: "progress",
      index: 99,
    });
    expect(order(across, "progress")).toEqual([
      ["c21", 0],
      ["c22", 1],
      ["c12", 2],
    ]);
    const within = applyCardMove(board(), "c11", {
      columnId: "backlog",
      index: 99,
    });
    expect(order(within, "backlog").map(([id]) => id)).toEqual([
      "c12",
      "c13",
      "c14",
      "c11",
    ]);
    expectContiguous(within);
  });

  it("clamps a negative index to the top", () => {
    const next = applyCardMove(board(), "c14", {
      columnId: "backlog",
      index: -3,
    });
    expect(order(next, "backlog").map(([id]) => id)).toEqual([
      "c14",
      "c11",
      "c12",
      "c13",
    ]);
  });

  it("does not mutate its input", () => {
    const before = board();
    const snapshot = structuredClone(before);
    applyCardMove(before, "c12", { columnId: "progress", index: 0 });
    expect(before).toEqual(snapshot);
  });

  it("returns the same detail for an unknown card or column", () => {
    const detail = board();
    expect(
      applyCardMove(detail, "nope", { columnId: "progress", index: 0 }),
    ).toBe(detail);
    expect(applyCardMove(detail, "c12", { columnId: "nope", index: 0 })).toBe(
      detail,
    );
  });
});

describe("applyColumnMove", () => {
  it("moves a column and keeps positions contiguous", () => {
    const next = applyColumnMove(board(), "backlog", 2);
    expect(sortedColumns(next).map((c) => [c.id, c.position])).toEqual([
      ["progress", 0],
      ["review", 1],
      ["backlog", 2],
      ["done", 3],
    ]);
  });

  it("moves a column left and clamps out-of-range indexes", () => {
    expect(
      sortedColumns(applyColumnMove(board(), "done", 0)).map((c) => c.id),
    ).toEqual(["done", "backlog", "progress", "review"]);
    expect(
      sortedColumns(applyColumnMove(board(), "backlog", 99)).map((c) => [
        c.id,
        c.position,
      ]),
    ).toEqual([
      ["progress", 0],
      ["review", 1],
      ["done", 2],
      ["backlog", 3],
    ]);
  });

  it("returns the same detail for an unknown column", () => {
    const detail = board();
    expect(applyColumnMove(detail, "nope", 1)).toBe(detail);
  });
});

describe("resolveColumnDrop", () => {
  it("gives the index of the column it is over", () => {
    expect(resolveColumnDrop(board(), "backlog", "review")).toBe(2);
    expect(resolveColumnDrop(board(), "done", "progress")).toBe(1);
  });

  it("returns null over itself or an unknown column", () => {
    expect(resolveColumnDrop(board(), "backlog", "backlog")).toBeNull();
    expect(resolveColumnDrop(board(), "backlog", "nope")).toBeNull();
  });
});

describe("finalCardTarget", () => {
  it("commits a cross-column preview even when the card ends over itself", () => {
    const snapshot = board();
    const preview = applyCardMove(snapshot, "c12", {
      columnId: "progress",
      index: 0,
    });
    expect(
      finalCardTarget(snapshot, preview, "c12", {
        overId: "c12",
        overKind: "card",
      }),
    ).toEqual({ columnId: "progress", index: 0 });
  });

  it("applies the final over target on top of the preview", () => {
    const snapshot = board();
    const preview = applyCardMove(snapshot, "c12", {
      columnId: "progress",
      index: 0,
    });
    expect(
      finalCardTarget(snapshot, preview, "c12", {
        overId: "c22",
        overKind: "card",
      }),
    ).toEqual({ columnId: "progress", index: 2 });
  });

  it("returns null when the card lands where it started", () => {
    const snapshot = board();
    expect(
      finalCardTarget(snapshot, snapshot, "c12", {
        overId: "c12",
        overKind: "card",
      }),
    ).toBeNull();
    // Dragged out and back into its own slot.
    const preview = applyCardMove(snapshot, "c12", {
      columnId: "progress",
      index: 0,
    });
    expect(
      finalCardTarget(snapshot, preview, "c12", {
        overId: "c13",
        overKind: "card",
      }),
    ).toBeNull();
  });

  it("keeps the preview position when there is nothing under the pointer", () => {
    const snapshot = board();
    const preview = applyCardMove(snapshot, "c12", {
      columnId: "review",
      index: 0,
    });
    expect(finalCardTarget(snapshot, preview, "c12", null)).toEqual({
      columnId: "review",
      index: 0,
    });
    expect(finalCardTarget(snapshot, snapshot, "c12", null)).toBeNull();
  });
});

describe("toDropRef", () => {
  const ref = (current: unknown) => ({ current });

  it("maps a card droppable to a card target", () => {
    expect(
      toDropRef({ id: "c21", data: ref({ kind: "card", cardId: "c21" }) }),
    ).toEqual({ overId: "c21", overKind: "card" });
  });

  it("maps a column or its body to the column id", () => {
    expect(
      toDropRef({
        id: "review",
        data: ref({ kind: "column", columnId: "review" }),
      }),
    ).toEqual({ overId: "review", overKind: "column" });
    expect(
      toDropRef({
        id: columnBodyId("review"),
        data: ref({ kind: "column-body", columnId: "review" }),
      }),
    ).toEqual({ overId: "review", overKind: "column" });
  });

  it("returns null for nothing or unknown droppables", () => {
    expect(toDropRef(null)).toBeNull();
    expect(toDropRef({ id: "x", data: ref(undefined) })).toBeNull();
    expect(toDropRef({ id: "x", data: ref({ kind: "other" }) })).toBeNull();
  });
});
