import type { Active } from "@dnd-kit/core";
import { describe, expect, it } from "vitest";
import type { BoardCard, BoardColumn, BoardDetail } from "../../model/board";
import { kanbanAnnouncements } from "./announcements";
import { applyCardMove, columnCards, sortedColumns } from "./kanbanMove";
import {
  createSession,
  sessionCancel,
  sessionDetailChanged,
  sessionEnd,
  sessionOver,
  sessionStart,
  shownBoard,
} from "./kanbanSession";

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

/** v1 with an agent auto-move applied, as a refetch would deliver it. */
function refetched(v1: BoardDetail): BoardDetail {
  return structuredClone(
    applyCardMove(v1, "c21", { columnId: "review", index: 0 }),
  );
}

const ids = (detail: BoardDetail, columnId: string) =>
  columnCards(detail, columnId).map((c) => c.id);

const overCard = (id: string) => ({ overId: id, overKind: "card" as const });

describe("drag session", () => {
  it("previews a cross-column move during the drag", () => {
    let s = sessionStart(createSession(board()), { kind: "card", id: "c12" });
    s = sessionOver(s, overCard("c21"));
    expect(ids(shownBoard(s), "progress")).toEqual(["c12", "c21", "c22"]);
    // Same-column hovering is left to SortableContext.
    const same = sessionOver(
      sessionStart(createSession(board()), { kind: "card", id: "c12" }),
      overCard("c13"),
    );
    expect(same.local).toBeNull();
  });

  it("cancel restores the drag-start board", () => {
    const v1 = board();
    let s = sessionStart(createSession(v1), { kind: "card", id: "c12" });
    s = sessionCancel(sessionOver(s, overCard("c21")));
    expect(s.drag).toBeNull();
    expect(shownBoard(s)).toBe(v1);
  });

  it("cancel after detail changed mid-drag shows the new detail, not the snapshot", () => {
    const v1 = board();
    const v2 = refetched(v1);
    let s = sessionStart(createSession(v1), { kind: "card", id: "c12" });
    s = sessionOver(s, overCard("c22"));
    s = sessionDetailChanged(s, v2);
    // The preview survives the refresh while dragging...
    expect(ids(shownBoard(s), "progress")).toContain("c12");
    s = sessionCancel(s);
    // ...but cancelling shows the fresh board.
    expect(shownBoard(s)).toBe(v2);
  });

  it("same-place drop after detail changed mid-drag shows the new detail", () => {
    const v1 = board();
    const v2 = refetched(v1);
    let s = sessionStart(createSession(v1), { kind: "card", id: "c12" });
    s = sessionDetailChanged(s, v2);
    const { session, commit } = sessionEnd(s, overCard("c12"));
    expect(commit).toBeNull();
    expect(shownBoard(session)).toBe(v2);
  });

  it("same-place column drop after detail changed mid-drag shows the new detail", () => {
    const v1 = board();
    const v2 = refetched(v1);
    let s = sessionStart(createSession(v1), { kind: "column", id: "backlog" });
    s = sessionDetailChanged(s, v2);
    const { session, commit } = sessionEnd(s, {
      overId: "backlog",
      overKind: "column",
    });
    expect(commit).toBeNull();
    expect(shownBoard(session)).toBe(v2);
  });

  it("cancel keeps a pending optimistic move when detail did not change", () => {
    const v1 = board();
    let s = sessionStart(createSession(v1), { kind: "card", id: "c12" });
    s = sessionEnd(sessionOver(s, overCard("c21")), overCard("c12")).session;
    const optimistic = shownBoard(s);
    s = sessionCancel(sessionStart(s, { kind: "card", id: "c11" }));
    expect(shownBoard(s)).toBe(optimistic);
  });

  it("commits a cross-column drop once and shows it optimistically", () => {
    let s = sessionStart(createSession(board()), { kind: "card", id: "c12" });
    s = sessionOver(s, overCard("c21"));
    const { session, commit } = sessionEnd(s, overCard("c12"));
    expect(commit).toEqual({
      kind: "card",
      cardId: "c12",
      to: { columnId: "progress", index: 0 },
    });
    expect(session.drag).toBeNull();
    expect(ids(shownBoard(session), "progress")).toEqual(["c12", "c21", "c22"]);
  });

  it("drops a cross-column preview where it was shown when the drop is over the card itself", () => {
    // A=[X], B=[B0,B1]: hovering B0 previews X before it.
    let s = sessionStart(createSession(board()), { kind: "card", id: "c12" });
    s = sessionOver(s, overCard("c21"));
    expect(ids(shownBoard(s), "progress")).toEqual(["c12", "c21", "c22"]);
    // After the preview, collision detection reports the card itself.
    s = sessionOver(s, overCard("c12"));
    const { session, commit } = sessionEnd(s, overCard("c12"));
    expect(commit).toEqual({
      kind: "card",
      cardId: "c12",
      to: { columnId: "progress", index: 0 },
    });
    expect(ids(shownBoard(session), "progress")).toEqual(["c12", "c21", "c22"]);
  });

  it("does not shift a cross-column preview by one when dropped over the neighbour that made it", () => {
    let s = sessionStart(createSession(board()), { kind: "card", id: "c12" });
    s = sessionOver(s, overCard("c21"));
    // dnd-kit can still report the hovered neighbour at drop time.
    s = sessionOver(s, overCard("c21"));
    const { session, commit } = sessionEnd(s, overCard("c21"));
    expect(commit).toMatchObject({ to: { columnId: "progress", index: 0 } });
    expect(ids(shownBoard(session), "progress")).toEqual(["c12", "c21", "c22"]);
  });

  it("sorts within the new column once the drag moves on from the card that made the preview", () => {
    let s = sessionStart(createSession(board()), { kind: "card", id: "c12" });
    s = sessionOver(s, overCard("c21"));
    s = sessionOver(s, overCard("c12"));
    // Back over the neighbour after that: a within-column sort, which
    // dnd-kit shows as the neighbour sliding above the card.
    s = sessionOver(s, overCard("c21"));
    expect(sessionEnd(s, overCard("c21")).commit).toMatchObject({
      to: { columnId: "progress", index: 1 },
    });
    // Further down the column works the same way.
    let t = sessionStart(createSession(board()), { kind: "card", id: "c12" });
    t = sessionOver(t, overCard("c21"));
    t = sessionOver(t, overCard("c22"));
    expect(sessionEnd(t, overCard("c22")).commit).toMatchObject({
      to: { columnId: "progress", index: 2 },
    });
  });

  it("previews a move back into the source column after a cross-column preview", () => {
    let s = sessionStart(createSession(board()), { kind: "card", id: "c12" });
    s = sessionOver(s, overCard("c21"));
    s = sessionOver(s, overCard("c13"));
    expect(ids(shownBoard(s), "progress")).toEqual(["c21", "c22"]);
    const { commit } = sessionEnd(s, overCard("c13"));
    // Inserted before c13: where it started, so nothing to commit.
    expect(commit).toBeNull();
  });

  it("applies a drop made during a mid-drag refresh on top of the new detail", () => {
    const v1 = board();
    const v2 = refetched(v1);
    let s = sessionStart(createSession(v1), { kind: "card", id: "c12" });
    s = sessionOver(s, overCard("c22"));
    s = sessionDetailChanged(s, v2);
    const { session, commit } = sessionEnd(s, overCard("c12"));
    expect(commit?.kind).toBe("card");
    // The agent's auto-move (c21 → Review) is not undone by the optimistic copy.
    expect(ids(shownBoard(session), "review")).toEqual(["c21"]);
    // Index 1 applied to the fresh column, as the backend will apply it.
    expect(commit).toMatchObject({ to: { columnId: "progress", index: 1 } });
    expect(ids(shownBoard(session), "progress")).toEqual(["c22", "c12"]);
  });

  it("commits a column move", () => {
    const s = sessionStart(createSession(board()), {
      kind: "column",
      id: "backlog",
    });
    const { session, commit } = sessionEnd(s, {
      overId: "review",
      overKind: "column",
    });
    expect(commit).toEqual({ kind: "column", columnId: "backlog", index: 2 });
    expect(sortedColumns(shownBoard(session)).map((c) => c.id)).toEqual([
      "progress",
      "review",
      "backlog",
    ]);
  });

  it("a fresh detail at rest replaces the optimistic copy", () => {
    let s = sessionStart(createSession(board()), { kind: "card", id: "c12" });
    s = sessionEnd(sessionOver(s, overCard("c21")), overCard("c12")).session;
    const v2 = board();
    s = sessionDetailChanged(s, v2);
    expect(shownBoard(s)).toBe(v2);
  });

  it("keeps the drag-start snapshot for announcements after cancel", () => {
    const v1 = board();
    let s = sessionStart(createSession(v1), { kind: "card", id: "c12" });
    s = sessionOver(s, overCard("c21"));
    // dnd-kit announces after our onDragCancel ran, while the preview is
    // still what is rendered.
    const preview = shownBoard(s);
    s = sessionCancel(s);
    expect(s.lastSnapshot).toBe(v1);
    const announce = kanbanAnnouncements({
      current: () => preview,
      snapshot: () => s.lastSnapshot,
    });
    const active = {
      id: "c12",
      data: { current: { kind: "card", cardId: "c12" } },
    } as unknown as Active;
    expect(announce.onDragCancel({ active, over: null })).toBe(
      "Moving card #12 was cancelled. It is back in Backlog, position 2 of 4.",
    );
  });

  it("replaces the announcement snapshot on the next drag start", () => {
    let s = sessionStart(createSession(board()), { kind: "card", id: "c12" });
    s = sessionEnd(sessionOver(s, overCard("c21")), overCard("c12")).session;
    const shown = shownBoard(s);
    s = sessionStart(s, { kind: "card", id: "c11" });
    expect(s.lastSnapshot).toBe(shown);
  });
});
