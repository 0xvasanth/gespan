import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { autoMoveTarget, onLinkedWork } from "./autoMove";
import type { BoardCard, BoardColumn, ColumnRole } from "./board";

function column(
  id: string,
  position: number,
  role: ColumnRole | null,
  boardId = "b1",
): BoardColumn {
  return { id, boardId, name: id, position, role };
}

const todo = column("todo", 0, "todo");
const doing = column("doing", 1, "doing");
const review = column("review", 2, "review");
const done = column("done", 3, "done");
const columns = [todo, doing, review, done];

describe("autoMoveTarget", () => {
  it("moves started work from todo to doing", () => {
    expect(autoMoveTarget(columns, todo, "started")).toBe(doing);
  });

  it("never moves started work backward", () => {
    expect(autoMoveTarget(columns, review, "started")).toBeNull();
    expect(autoMoveTarget(columns, done, "started")).toBeNull();
  });

  it("stays put when started work is already in doing", () => {
    expect(autoMoveTarget(columns, doing, "started")).toBeNull();
  });

  it("moves completed work from doing to review", () => {
    expect(autoMoveTarget(columns, doing, "completed")).toBe(review);
  });

  it("moves completed work from todo straight to review", () => {
    expect(autoMoveTarget(columns, todo, "completed")).toBe(review);
  });

  it("falls back to done when there is no review column", () => {
    const board = [todo, doing, done];
    expect(autoMoveTarget(board, doing, "completed")).toBe(done);
  });

  it("does not move completed work out of done or review", () => {
    expect(autoMoveTarget(columns, done, "completed")).toBeNull();
    expect(autoMoveTarget(columns, review, "completed")).toBeNull();
  });

  it("never moves a failed run", () => {
    for (const current of columns) {
      expect(autoMoveTarget(columns, current, "failed")).toBeNull();
    }
  });

  it("stays put on started when the board has no doing column", () => {
    expect(autoMoveTarget([todo, review, done], todo, "started")).toBeNull();
  });

  it("stays put on completed when the board has neither review nor done", () => {
    expect(autoMoveTarget([todo, doing], doing, "completed")).toBeNull();
  });

  it("handles columns passed out of position order", () => {
    const shuffled = [done, review, todo, doing];
    expect(autoMoveTarget(shuffled, todo, "started")).toBe(doing);
    expect(autoMoveTarget(shuffled, doing, "completed")).toBe(review);
  });

  describe("with a user column between doing and review", () => {
    const qa = column("qa", 2, null);
    const board = [
      todo,
      doing,
      qa,
      column("review", 3, "review"),
      column("done", 4, "done"),
    ];

    it("moves completed work from the user column to review", () => {
      expect(autoMoveTarget(board, qa, "completed")?.id).toBe("review");
    });

    it("does not move started work backward to doing", () => {
      expect(autoMoveTarget(board, qa, "started")).toBeNull();
    });
  });

  it("treats a role-less column left of doing as before it", () => {
    const inbox = column("inbox", 0, null);
    const board = [inbox, column("doing", 1, "doing"), column("done", 2, "done")];
    expect(autoMoveTarget(board, inbox, "started")?.id).toBe("doing");
  });
});

function card(id: string, boardId: string, columnId: string): BoardCard {
  return {
    id,
    boardId,
    columnId,
    number: 1,
    title: id,
    body: "",
    position: 0,
    linkKind: "session",
    linkId: "s1",
    createdAt: 0,
    updatedAt: 0,
  };
}

function detail(boardId: string) {
  return {
    board: { id: boardId, name: boardId, createdAt: 0, updatedAt: 0 },
    columns: [
      column(`${boardId}-todo`, 0, "todo", boardId),
      column(`${boardId}-doing`, 1, "doing", boardId),
      column(`${boardId}-review`, 2, "review", boardId),
      column(`${boardId}-done`, 3, "done", boardId),
    ],
    cards: [],
  };
}

describe("onLinkedWork", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  function mockBackend(cards: BoardCard[]) {
    invoke.mockImplementation(async (command: string, args?: { boardId?: string }) => {
      if (command === "board_cards_for_link") return cards;
      if (command === "board_get") return detail(args!.boardId!);
      return undefined;
    });
  }

  it("moves each linked card on its own board as the system actor", async () => {
    mockBackend([card("c1", "b1", "b1-todo"), card("c2", "b2", "b2-todo")]);

    await onLinkedWork("session", "s1", "started");

    expect(invoke).toHaveBeenCalledWith("board_cards_for_link", {
      linkKind: "session",
      linkId: "s1",
    });
    expect(invoke).toHaveBeenCalledWith("board_card_move", {
      cardId: "c1",
      columnId: "b1-doing",
      index: Number.MAX_SAFE_INTEGER,
      actor: "system",
    });
    expect(invoke).toHaveBeenCalledWith("board_card_move", {
      cardId: "c2",
      columnId: "b2-doing",
      index: Number.MAX_SAFE_INTEGER,
      actor: "system",
    });
    expect(invoke).not.toHaveBeenCalledWith("board_card_record", expect.anything());
  });

  it("records the event instead of moving when the card must stay", async () => {
    mockBackend([card("c1", "b1", "b1-review")]);

    await onLinkedWork("session", "s1", "started");

    expect(invoke).toHaveBeenCalledWith("board_card_record", {
      cardId: "c1",
      kind: "started",
    });
    expect(invoke).not.toHaveBeenCalledWith("board_card_move", expect.anything());
  });

  it("records failed runs without moving the card", async () => {
    mockBackend([card("c1", "b1", "b1-doing")]);

    await onLinkedWork("pilot", "p1", "failed");

    expect(invoke).toHaveBeenCalledWith("board_card_record", {
      cardId: "c1",
      kind: "failed",
    });
    expect(invoke).not.toHaveBeenCalledWith("board_card_move", expect.anything());
  });

  it("loads each board once when several cards share it", async () => {
    mockBackend([card("c1", "b1", "b1-todo"), card("c2", "b1", "b1-todo")]);

    await onLinkedWork("session", "s1", "started");

    const loads = invoke.mock.calls.filter(([cmd]) => cmd === "board_get");
    expect(loads).toHaveLength(1);
  });

  it("does nothing for a link with no cards", async () => {
    mockBackend([]);

    await expect(onLinkedWork("flow-run", "r1", "completed")).resolves.toBeUndefined();

    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("keeps processing other cards when one fails, then reports the error", async () => {
    invoke.mockImplementation(async (command: string, args?: { boardId?: string; cardId?: string }) => {
      if (command === "board_cards_for_link") {
        return [card("c1", "b1", "b1-todo"), card("c2", "b2", "b2-todo")];
      }
      if (command === "board_get") return detail(args!.boardId!);
      if (command === "board_card_move" && args!.cardId === "c1") {
        throw new Error("boom");
      }
      return undefined;
    });

    await expect(onLinkedWork("session", "s1", "started")).rejects.toThrow("boom");

    expect(invoke).toHaveBeenCalledWith("board_card_move", {
      cardId: "c2",
      columnId: "b2-doing",
      index: Number.MAX_SAFE_INTEGER,
      actor: "system",
    });
  });
});
