import type { Active, Over } from "@dnd-kit/core";
import { describe, expect, it } from "vitest";
import type { BoardCard, BoardColumn, BoardDetail } from "../../model/board";
import { kanbanAnnouncements } from "./announcements";
import { applyCardMove, columnBodyId } from "./kanbanMove";

function column(id: string, name: string, position: number): BoardColumn {
  return { id, boardId: "b1", name, position, role: null };
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

function board(): BoardDetail {
  return {
    board: { id: "b1", name: "Board", createdAt: 1, updatedAt: 1 },
    columns: [
      column("backlog", "Backlog", 0),
      column("progress", "In Progress", 1),
      column("review", "Review", 2),
      column("done", "Done", 3),
    ],
    cards: [
      card("c11", "backlog", 0, 11),
      card("c12", "backlog", 1, 12, "Fix login"),
      card("c13", "backlog", 2, 13),
      card("c14", "backlog", 3, 14),
      card("c21", "progress", 0, 21),
      card("c22", "progress", 1, 22),
    ],
  };
}

function data(current: Record<string, unknown>) {
  return { current };
}

const cardActive = (id: string) =>
  ({ id, data: data({ kind: "card", cardId: id }) }) as unknown as Active;
const columnActive = (id: string) =>
  ({ id, data: data({ kind: "column", columnId: id }) }) as unknown as Active;
const cardOver = (id: string) =>
  ({ id, data: data({ kind: "card", cardId: id }) }) as unknown as Over;
const columnOver = (id: string) =>
  ({ id, data: data({ kind: "column", columnId: id }) }) as unknown as Over;
const bodyOver = (id: string) =>
  ({
    id: columnBodyId(id),
    data: data({ kind: "column-body", columnId: id }),
  }) as unknown as Over;

function setup(current: BoardDetail, snapshot: BoardDetail | null = current) {
  return kanbanAnnouncements({
    current: () => current,
    snapshot: () => snapshot,
  });
}

describe("card announcements", () => {
  it("announces pick up with the card's number, title and place", () => {
    expect(setup(board()).onDragStart({ active: cardActive("c12") })).toBe(
      "Picked up card #12, Fix login. Backlog, position 2 of 4.",
    );
  });

  it("announces where the card would land while over another column", () => {
    const snapshot = board();
    expect(
      setup(snapshot).onDragOver({
        active: cardActive("c12"),
        over: cardOver("c21"),
      }),
    ).toBe("Card #12 is over In Progress, position 1 of 3.");
    // Same string once the board has already applied the cross-column preview.
    const preview = applyCardMove(snapshot, "c12", {
      columnId: "progress",
      index: 0,
    });
    expect(
      setup(preview, snapshot).onDragOver({
        active: cardActive("c12"),
        over: cardOver("c12"),
      }),
    ).toBe("Card #12 is over In Progress, position 1 of 3.");
  });

  it("announces an empty column body as position 1 of 1", () => {
    expect(
      setup(board()).onDragOver({
        active: cardActive("c12"),
        over: bodyOver("review"),
      }),
    ).toBe("Card #12 is over Review, position 1 of 1.");
  });

  it("announces the drop place", () => {
    const snapshot = board();
    const preview = applyCardMove(snapshot, "c12", {
      columnId: "progress",
      index: 0,
    });
    expect(
      setup(preview, snapshot).onDragEnd({
        active: cardActive("c12"),
        over: cardOver("c12"),
      }),
    ).toBe("Dropped card #12 in In Progress, position 1 of 3.");
  });

  it("announces cancel with the place from the drag-start snapshot", () => {
    const snapshot = board();
    const preview = applyCardMove(snapshot, "c12", {
      columnId: "progress",
      index: 0,
    });
    expect(
      setup(preview, snapshot).onDragCancel({
        active: cardActive("c12"),
        over: cardOver("c12"),
      }),
    ).toBe(
      "Moving card #12 was cancelled. It is back in Backlog, position 2 of 4.",
    );
  });

  it("announces leaving every column", () => {
    expect(
      setup(board()).onDragOver({ active: cardActive("c12"), over: null }),
    ).toBe("Card #12 is no longer over a column.");
  });

  it("stays silent for unknown cards", () => {
    expect(
      setup(board()).onDragStart({ active: cardActive("nope") }),
    ).toBeUndefined();
  });
});

describe("column announcements", () => {
  it("announces pick up, over, drop and cancel", () => {
    const a = setup(board());
    expect(a.onDragStart({ active: columnActive("backlog") })).toBe(
      "Picked up column Backlog, position 1 of 4.",
    );
    expect(
      a.onDragOver({
        active: columnActive("backlog"),
        over: columnOver("review"),
      }),
    ).toBe("Column Backlog is over position 3 of 4.");
    expect(
      a.onDragEnd({
        active: columnActive("backlog"),
        over: columnOver("review"),
      }),
    ).toBe("Dropped column Backlog at position 3 of 4.");
    expect(
      a.onDragCancel({
        active: columnActive("backlog"),
        over: columnOver("review"),
      }),
    ).toBe(
      "Moving column Backlog was cancelled. It is back at position 1 of 4.",
    );
  });

  it("announces a drop over itself as its current position", () => {
    expect(
      setup(board()).onDragEnd({
        active: columnActive("progress"),
        over: columnOver("progress"),
      }),
    ).toBe("Dropped column In Progress at position 2 of 4.");
  });
});
