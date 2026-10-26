import { describe, expect, it } from "vitest";
import type { BoardCard, BoardColumn, CardExtras } from "./board";
import {
  buildGroups,
  dueBucket,
  parseLayout,
  patchLayout,
  tableColumns,
} from "./layout";

const col = (id: string, position: number, role: BoardColumn["role"] = null) =>
  ({ id, name: id.toUpperCase(), position, role }) as BoardColumn;
const card = (id: string, columnId: string) =>
  ({ id, columnId, number: 1, title: id }) as BoardCard;
const extras = (over: Partial<CardExtras>): CardExtras => ({
  priority: 0,
  dueAt: null,
  projects: [],
  labels: [],
  subtasksDone: 0,
  subtasksTotal: 0,
  comments: 0,
  ...over,
});

describe("layout json", () => {
  it("reads defaults from anything unreadable", () => {
    for (const bad of ["", "nope", "[]", '{"layout":"cube"}']) {
      expect(parseLayout(bad)).toEqual({
        layout: "board",
        groupBy: "status",
        collapsed: [],
        columns: null,
        widths: {},
      });
    }
  });

  it("keeps keys it does not know, and drops emptied ones", () => {
    const stored =
      '{"layout":"table","columns":["title","due"],"widths":{"title":220},"collapsed":["a"],"future":{"x":1}}';
    const next = JSON.parse(patchLayout(stored, { collapsed: [] }));
    expect(next.future).toEqual({ x: 1 });
    expect(next.collapsed).toBeUndefined();
    expect(next.widths).toEqual({ title: 220 });
    expect(next.layout).toBe("table");
  });

  it("clamps stored widths to the 60 px minimum", () => {
    expect(parseLayout('{"widths":{"title":10,"due":200}}').widths).toEqual({
      title: 60,
      due: 200,
    });
  });

  it("puts Title first, drops a deleted custom field and applies widths", () => {
    const state = parseLayout(
      '{"layout":"table","columns":["due","field:gone","field:f1"],"widths":{"due":75}}',
    );
    const cols = tableColumns(state, [{ id: "f1", name: "Sprint" }]);
    expect(cols.map((c) => c.id)).toEqual(["title", "due", "field:f1"]);
    expect(cols[1]!.width).toBe(75);
  });
});

describe("dueBucket", () => {
  const now = new Date(2026, 9, 26, 23, 30).getTime();
  const at = (days: number) => new Date(2026, 9, 26 + days, 0, 5).getTime();
  it("uses local days, even just before midnight", () => {
    expect(dueBucket(at(0), false, now)).toBe("today");
    expect(dueBucket(at(-1), false, now)).toBe("overdue");
    expect(dueBucket(at(1), false, now)).toBe("next7");
    expect(dueBucket(at(7), false, now)).toBe("next7");
    expect(dueBucket(at(8), false, now)).toBe("later");
    expect(dueBucket(null, false, now)).toBe("none");
  });
  it("never calls a done card overdue", () => {
    expect(dueBucket(at(-3), true, now)).toBe("earlier");
  });
});

describe("buildGroups", () => {
  const columns = [col("todo", 0), col("done", 1, "done")];
  const cards = [card("a", "todo"), card("b", "todo"), card("c", "done")];
  const base = { cards, columns, labels: [], projects: [] };

  it("groups by status in column order and leaves empty groups out", () => {
    const groups = buildGroups({ ...base, by: "status", extras: {} });
    expect(groups.map((g) => [g.key, g.cardIds])).toEqual([
      ["todo", ["a", "b"]],
      ["done", ["c"]],
    ]);
  });

  it("groups by priority, urgent first, none last", () => {
    const groups = buildGroups({
      ...base,
      by: "priority",
      extras: { a: extras({ priority: 4 }), c: extras({ priority: 1 }) },
    });
    expect(groups.map((g) => [g.label, g.cardIds])).toEqual([
      ["Urgent", ["a"]],
      ["Low", ["c"]],
      ["None", ["b"]],
    ]);
  });

  it("puts a card with two labels in both groups, and unlabelled ones under No label", () => {
    const groups = buildGroups({
      ...base,
      by: "label",
      labels: [
        { id: "l1", name: "Bug" },
        { id: "l2", name: "UI" },
      ],
      extras: {
        a: extras({ labels: ["l1", "l2"] }),
        b: extras({ labels: ["deleted"] }),
      },
    });
    expect(groups.map((g) => [g.label, g.cardIds])).toEqual([
      ["Bug", ["a"]],
      ["UI", ["a"]],
      ["No label", ["b", "c"]],
    ]);
  });

  it("groups by project with No project, and keeps a project the list no longer names", () => {
    const groups = buildGroups({
      ...base,
      by: "project",
      projects: [{ path: "/p", name: "P" }],
      extras: {
        a: extras({ projects: ["/p"] }),
        b: extras({ projects: ["/gone"] }),
      },
    });
    expect(groups.map((g) => [g.label, g.cardIds])).toEqual([
      ["P", ["a"]],
      ["/gone", ["b"]],
      ["No project", ["c"]],
    ]);
  });

  it("groups by due bucket with No due date", () => {
    const now = new Date(2026, 9, 26, 12).getTime();
    const day = (n: number) => new Date(2026, 9, 26 + n, 9).getTime();
    const groups = buildGroups({
      ...base,
      by: "due",
      now,
      extras: {
        a: extras({ dueAt: day(-1) }),
        b: extras({ dueAt: day(0) }),
        c: extras({ dueAt: day(-5) }),
      },
    });
    // c is done: earlier, not overdue.
    expect(groups.map((g) => [g.label, g.cardIds])).toEqual([
      ["Overdue", ["a"]],
      ["Today", ["b"]],
      ["Earlier", ["c"]],
    ]);
  });
});
