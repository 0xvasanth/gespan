import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  archivedLabel,
  createUndo,
  deletedLabel,
  movedLabel,
  runUndo,
  sortedLabel,
  undoFailure,
  UNDO_WINDOW_MS,
  type UndoApi,
} from "./undo";

describe("toast copy", () => {
  it("names one card by number and more by count", () => {
    expect(movedLabel([4], "Done")).toBe("Moved #4 to Done");
    expect(movedLabel([4, 5], "Done")).toBe("Moved 2 cards to Done");
    expect(archivedLabel([7])).toBe("Archived #7");
    expect(archivedLabel([7, 8, 9])).toBe("Archived 3 cards");
    expect(deletedLabel(12)).toBe("Deleted #12");
    expect(sortedLabel("Backlog")).toBe("Sorted Backlog");
  });

  it("reads store errors as what was lost", () => {
    expect(undoFailure("Card not found")).toBe(
      "Couldn't undo: the card was deleted",
    );
    expect(undoFailure("Column not on this board")).toBe(
      "Couldn't undo: the column was deleted",
    );
    expect(undoFailure("Column not found")).toBe(
      "Couldn't undo: the column was deleted",
    );
    expect(undoFailure(new Error("disk full"))).toBe(
      "Couldn't undo: disk full",
    );
  });
});

describe("createUndo", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const move = {
    kind: "move",
    cardId: "c1",
    columnId: "backlog",
    index: 0,
  } as const;
  const del = {
    kind: "delete",
    cardId: "c9",
    columnId: "backlog",
    index: 1,
  } as const;

  it("shows one toast for 8 seconds, then closes it", () => {
    const undo = createUndo({ onCommitDelete: vi.fn() });
    const seen = vi.fn();
    undo.subscribe(seen);
    undo.push("Moved #1 to Done", move);
    expect(undo.current()).toMatchObject({
      label: "Moved #1 to Done",
      action: move,
    });
    expect(seen).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(UNDO_WINDOW_MS - 100);
    expect(undo.current()).not.toBeNull();
    vi.advanceTimersByTime(200);
    expect(undo.current()).toBeNull();
    expect(seen).toHaveBeenCalledTimes(2);
    expect(UNDO_WINDOW_MS).toBe(8000);
  });

  it("undo hands back the inverse once and closes the toast", () => {
    const onCommitDelete = vi.fn();
    const undo = createUndo({ onCommitDelete });
    undo.push("Deleted #9", del);
    expect(undo.undo()).toEqual(del);
    expect(undo.current()).toBeNull();
    expect(undo.undo()).toBeNull();
    // The undone delete's timer deletes nothing.
    vi.advanceTimersByTime(UNDO_WINDOW_MS * 2);
    expect(onCommitDelete).not.toHaveBeenCalled();
  });

  it("holds a delete until the toast expires", () => {
    const onCommitDelete = vi.fn();
    const undo = createUndo({ onCommitDelete });
    undo.push("Deleted #9", del);
    expect(undo.pendingDelete()).toBe("c9");
    vi.advanceTimersByTime(UNDO_WINDOW_MS - 1);
    expect(onCommitDelete).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(onCommitDelete).toHaveBeenCalledExactlyOnceWith("c9");
    expect(undo.pendingDelete()).toBeNull();
  });

  it("a new action replaces the toast and commits the pending delete at once", () => {
    const onCommitDelete = vi.fn();
    const undo = createUndo({ onCommitDelete });
    undo.push("Deleted #9", del);
    undo.push("Moved #1 to Done", move);
    expect(onCommitDelete).toHaveBeenCalledExactlyOnceWith("c9");
    expect(undo.current()?.label).toBe("Moved #1 to Done");
    expect(undo.pendingDelete()).toBeNull();
    // The replaced toast's timer neither closes the new one early nor commits again.
    vi.advanceTimersByTime(UNDO_WINDOW_MS - 100);
    expect(undo.current()?.label).toBe("Moved #1 to Done");
    vi.advanceTimersByTime(UNDO_WINDOW_MS);
    expect(onCommitDelete).toHaveBeenCalledTimes(1);
  });

  it("commitPending carries out the held delete now and waits for it", async () => {
    let finish!: () => void;
    const onCommitDelete = vi.fn(
      () => new Promise<void>((resolve) => (finish = resolve)),
    );
    const undo = createUndo({ onCommitDelete });
    undo.push("Deleted #9", del);
    let done = false;
    const committed = undo.commitPending().then(() => (done = true));
    expect(onCommitDelete).toHaveBeenCalledExactlyOnceWith("c9");
    expect(undo.current()).toBeNull();
    await Promise.resolve();
    expect(done).toBe(false);
    finish();
    await committed;
    expect(done).toBe(true);
    // Nothing is pending any more.
    await undo.commitPending();
    expect(onCommitDelete).toHaveBeenCalledTimes(1);
  });

  it("commitPending closes a plain toast without committing anything", async () => {
    const onCommitDelete = vi.fn();
    const undo = createUndo({ onCommitDelete });
    undo.push("Moved #1 to Done", move);
    await undo.commitPending();
    expect(undo.current()).toBeNull();
    expect(onCommitDelete).not.toHaveBeenCalled();
  });

  it("quitting_with_a_pending_delete_leaves_the_card_archived", () => {
    const onCommitDelete = vi.fn();
    const undo = createUndo({ onCommitDelete });
    undo.push("Deleted #9", del);
    undo.dispose();
    vi.advanceTimersByTime(UNDO_WINDOW_MS * 2);
    expect(onCommitDelete).not.toHaveBeenCalled();
  });

  it("a message toast has no inverse and expires like any toast", () => {
    const undo = createUndo({ onCommitDelete: vi.fn() });
    undo.notify("Couldn't undo: the card was deleted");
    expect(undo.current()).toMatchObject({
      label: "Couldn't undo: the card was deleted",
      action: null,
    });
    expect(undo.undo()).toBeNull();
    expect(undo.current()).not.toBeNull();
    vi.advanceTimersByTime(UNDO_WINDOW_MS);
    expect(undo.current()).toBeNull();
  });
});

describe("runUndo", () => {
  type Call = [string, ...unknown[]];
  function api(
    fail: (call: Call) => unknown = () => undefined,
    snapshot: Awaited<ReturnType<UndoApi["snapshot"]>> = {
      live: [],
      archivedIds: [],
    },
  ) {
    const calls: Call[] = [];
    const record = async (call: Call) => {
      calls.push(call);
      const error = fail(call);
      if (error) throw error;
    };
    const value: UndoApi = {
      moveCard: (cardId, columnId, index) =>
        record(["moveCard", cardId, columnId, index]),
      moveCards: (ids, columnId, index) =>
        record(["moveCards", ids, columnId, index]),
      restoreCard: (cardId, columnId, index) =>
        record(["restoreCard", cardId, columnId, index]),
      snapshot: async () => snapshot,
    };
    return { calls, value };
  }

  it("moves a single card back to its old column and index", async () => {
    const { calls, value } = api();
    await runUndo(
      { kind: "move", cardId: "c1", columnId: "backlog", index: 2 },
      value,
    );
    expect(calls).toEqual([["moveCard", "c1", "backlog", 2]]);
  });

  it("restores a held delete at its column and index", async () => {
    const { calls, value } = api();
    await runUndo(
      { kind: "delete", cardId: "c1", columnId: "backlog", index: 1 },
      value,
    );
    expect(calls).toEqual([["restoreCard", "c1", "backlog", 1]]);
  });

  it("puts a bulk move back lowest index first per column, trying every card", async () => {
    const { calls, value } = api((call) =>
      call[1] === "b" ? "Card not found" : undefined,
    );
    await expect(
      runUndo(
        {
          kind: "moveMany",
          items: [
            ["c", "x", 2],
            ["z", "w", 0],
            ["a", "x", 0],
            ["b", "x", 1],
          ],
        },
        value,
      ),
    ).rejects.toBe("Card not found");
    expect(calls).toEqual([
      ["moveCard", "z", "w", 0],
      ["moveCard", "a", "x", 0],
      ["moveCard", "b", "x", 1],
      ["moveCard", "c", "x", 2],
    ]);
  });

  it("restores archived cards the same way", async () => {
    const { calls, value } = api();
    await runUndo(
      {
        kind: "archive",
        items: [
          ["d2", "done", 1],
          ["d1", "done", 0],
        ],
      },
      value,
    );
    expect(calls).toEqual([
      ["restoreCard", "d1", "done", 0],
      ["restoreCard", "d2", "done", 1],
    ]);
  });

  it("restores a sort's exact order with one write", async () => {
    const { calls, value } = api(undefined, {
      live: ["b", "d", "a", "c"].map((id) => ({ id, columnId: "backlog" })),
      archivedIds: [],
    });
    await runUndo(
      { kind: "sort", columnId: "backlog", order: ["a", "b", "c", "d"] },
      value,
    );
    expect(calls).toEqual([["moveCards", ["a", "b", "c", "d"], "backlog", 0]]);
  });

  it("a sort undo skips cards that left; a deleted one is reported", async () => {
    const moved = api(undefined, {
      live: [
        { id: "b", columnId: "backlog" },
        { id: "a", columnId: "done" },
        { id: "c", columnId: "backlog" },
      ],
      archivedIds: ["d"],
    });
    await runUndo(
      { kind: "sort", columnId: "backlog", order: ["a", "b", "c", "d"] },
      moved.value,
    );
    expect(moved.calls).toEqual([["moveCards", ["b", "c"], "backlog", 0]]);

    const deleted = api(undefined, {
      live: [{ id: "b", columnId: "backlog" }],
      archivedIds: [],
    });
    await expect(
      runUndo(
        { kind: "sort", columnId: "backlog", order: ["a", "b"] },
        deleted.value,
      ),
    ).rejects.toThrow("Card not found");
    expect(deleted.calls).toEqual([["moveCards", ["b"], "backlog", 0]]);
  });
});
