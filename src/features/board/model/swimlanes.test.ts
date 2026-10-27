import { describe, expect, it } from "vitest";
import type { BoardCard, BoardField, CardExtras } from "./board";
import { buildLanes, laneCellKey, laneRefocus, laneValue } from "./swimlanes";

const card = (id: string): BoardCard => ({
  id,
  boardId: "b",
  columnId: "c",
  number: 1,
  title: id,
  body: "",
  position: 0,
  linkKind: null,
  linkId: null,
  createdAt: 1,
  updatedAt: 1,
});
const extra = (patch: Partial<CardExtras>): CardExtras => ({
  priority: 0,
  dueAt: null,
  projects: [],
  labels: [],
  subtasksDone: 0,
  subtasksTotal: 0,
  comments: 0,
  ...patch,
});
const select: BoardField = {
  id: "f",
  boardId: "b",
  name: "Severity",
  kind: "select",
  optionsJson: '["S1","S2"]',
  position: 0,
};
const base = {
  cards: [card("a"), card("b"), card("c")],
  labels: [
    { id: "l1", name: "Bug" },
    { id: "l2", name: "Docs" },
  ],
  projects: [{ path: "/p", name: "p" }],
  fieldValues: {},
};

describe("buildLanes", () => {
  it("priority lanes run Urgent to None, each holding its cards", () => {
    const lanes = buildLanes({
      ...base,
      by: { kind: "priority" },
      extras: { a: extra({ priority: 4 }), b: extra({ priority: 2 }) },
    });
    expect(lanes.map((l) => [l.label, l.cardIds])).toEqual([
      ["Urgent", ["a"]],
      ["High", []],
      ["Medium", ["b"]],
      ["Low", []],
      ["None", ["c"]],
    ]);
  });

  it("a hidden priority lane appears only while a card holds it", () => {
    const priorities = {
      names: ["None", "Low", "Medium", "High", "Urgent"] as [string, string, string, string, string],
      hidden: [1, 2],
    };
    const lanes = buildLanes({
      ...base,
      by: { kind: "priority" },
      priorities,
      extras: { b: extra({ priority: 2 }) },
    });
    expect(lanes.map((l) => l.label)).toEqual(["Urgent", "High", "Medium", "None"]);
  });

  it("label lanes put a multi-label card in each and unlabelled ones under No label", () => {
    const lanes = buildLanes({
      ...base,
      by: { kind: "label" },
      extras: { a: extra({ labels: ["l1", "l2"] }), b: extra({ labels: ["l2"] }) },
    });
    expect(lanes.map((l) => [l.key, l.cardIds])).toEqual([
      ["l1", ["a"]],
      ["l2", ["a", "b"]],
      ["none", ["c"]],
    ]);
  });

  it("drops an empty No-value lane and keeps a stray value", () => {
    const lanes = buildLanes({
      ...base,
      by: { kind: "field", field: select },
      extras: {},
      fieldValues: { a: { f: '"S1"' }, b: { f: '"Old"' }, c: { f: '"S2"' } },
    });
    expect(lanes.map((l) => l.label)).toEqual(["S1", "S2", "Old"]);
  });

  it("project lanes list a project that is no longer known", () => {
    const lanes = buildLanes({
      ...base,
      by: { kind: "project" },
      extras: { a: extra({ projects: ["/gone"] }) },
    });
    expect(lanes.map((l) => l.key)).toEqual(["/p", "/gone", "none"]);
  });
});

describe("laneValue", () => {
  it("sets a priority for a priority lane and an option for a select lane", () => {
    expect(laneValue({ kind: "priority" }, "p3")).toEqual({ kind: "priority", priority: 3 });
    const by = { kind: "field", field: select } as const;
    expect(laneValue(by, "opt:S2")).toEqual({ kind: "field", fieldId: "f", option: "S2" });
    expect(laneValue(by, "none")).toEqual({ kind: "field", fieldId: "f", option: null });
  });

  it("sets nothing for label and project lanes", () => {
    expect(laneValue({ kind: "label" }, "l1")).toBeNull();
    expect(laneValue({ kind: "project" }, "/p")).toBeNull();
  });
});

describe("laneRefocus", () => {
  const cells = (entries: [string, string, string[]][]) =>
    new Map(entries.map(([lane, column, ids]) => [laneCellKey(lane, column), ids]));
  const before = cells([
    ["bug", "todo", ["a", "b", "c", "d", "e"]],
    ["bug", "done", ["z"]],
    ["ui", "todo", ["u"]],
  ]);
  const lanes = ["bug", "ui"];
  const columns = ["todo", "done"];

  it("keeps the card when its lane still shows it, in another band or column", () => {
    const moved = cells([["bug", "todo", ["b", "c", "d", "e"]], ["bug", "done", ["z"]]]);
    expect(laneRefocus({ lane: "bug", cardId: "e" }, before, moved, lanes, columns)).toEqual({
      lane: "bug",
      cardId: "e",
    });
    const across = cells([["bug", "done", ["z", "c"]]]);
    expect(laneRefocus({ lane: "bug", cardId: "c" }, before, across, lanes, columns)).toEqual({
      lane: "bug",
      cardId: "c",
    });
  });

  it("goes to the next card still shown in its cell, then the previous", () => {
    const noC = cells([["bug", "todo", ["a", "b", "d", "e"]]]);
    expect(laneRefocus({ lane: "bug", cardId: "c" }, before, noC, lanes, columns)).toEqual({
      lane: "bug",
      cardId: "d",
    });
    const noE = cells([["bug", "todo", ["a", "b", "c", "d"]]]);
    expect(laneRefocus({ lane: "bug", cardId: "e" }, before, noE, lanes, columns)).toEqual({
      lane: "bug",
      cardId: "d",
    });
    // Neighbours further out when the nearest ones went too.
    const far = cells([["bug", "todo", ["a"]]]);
    expect(laneRefocus({ lane: "bug", cardId: "c" }, before, far, lanes, columns)).toEqual({
      lane: "bug",
      cardId: "a",
    });
  });

  it("falls back to the cell's first card, then to the lane header", () => {
    const fresh = cells([["bug", "todo", ["new"]]]);
    expect(laneRefocus({ lane: "bug", cardId: "c" }, before, fresh, lanes, columns)).toEqual({
      lane: "bug",
      cardId: "new",
    });
    const empty = cells([["bug", "done", ["z"]]]);
    expect(laneRefocus({ lane: "bug", cardId: "c" }, before, empty, lanes, columns)).toEqual({
      lane: "bug",
      cardId: null,
    });
  });

  it("returns null when the lane is gone", () => {
    expect(laneRefocus({ lane: "bug", cardId: "c" }, before, new Map(), ["ui"], columns)).toBeNull();
  });
});
