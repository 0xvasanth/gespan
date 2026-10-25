import { describe, expect, it } from "vitest";
import type { BoardLabel, CardExtras } from "./board";
import {
  EMPTY_FILTER,
  filterCount,
  fullIndex,
  matchesCard,
  pruneFilter,
  type BoardFilter,
} from "./boardFilter";

const card = (number: number, title: string) => ({
  id: `c${number}`,
  number,
  title,
});

const extras = (over: Partial<CardExtras> = {}): CardExtras => ({
  priority: 0,
  dueAt: null,
  projects: [],
  labels: [],
  subtasksDone: 0,
  subtasksTotal: 0,
  comments: 0,
  ...over,
});

const filter = (over: Partial<BoardFilter> = {}): BoardFilter => ({
  projects: new Set(),
  labels: new Set(),
  priorities: new Set(),
  ...over,
});

describe("matchesCard", () => {
  it("hash_number_is_exact", () => {
    expect(matchesCard(card(1, "A"), undefined, "#1", EMPTY_FILTER)).toBe(true);
    expect(matchesCard(card(12, "B"), undefined, "#1", EMPTY_FILTER)).toBe(
      false,
    );
    expect(matchesCard(card(12, "B"), undefined, "#12", EMPTY_FILTER)).toBe(
      true,
    );
    // A number is not a title search.
    expect(
      matchesCard(card(3, "Fix #1 bug"), undefined, "#1", EMPTY_FILTER),
    ).toBe(false);
  });

  it("title_substring_is_case_insensitive", () => {
    const c = card(1, "Fix Login redirect");
    expect(matchesCard(c, undefined, "login", EMPTY_FILTER)).toBe(true);
    expect(matchesCard(c, undefined, "  LOGIN ", EMPTY_FILTER)).toBe(true);
    expect(matchesCard(c, undefined, "logout", EMPTY_FILTER)).toBe(false);
    expect(matchesCard(c, undefined, "", EMPTY_FILTER)).toBe(true);
  });

  it("filters_and_together_and_any_value_matches", () => {
    const c = card(1, "A");
    const e = extras({ priority: 3, labels: ["l1", "l2"], projects: ["/p"] });
    // Any value of one filter is enough.
    expect(
      matchesCard(c, e, "", filter({ labels: new Set(["x", "l2"]) })),
    ).toBe(true);
    expect(matchesCard(c, e, "", filter({ priorities: new Set([1, 3]) }))).toBe(
      true,
    );
    // Filters AND together.
    expect(
      matchesCard(
        c,
        e,
        "",
        filter({ labels: new Set(["l1"]), priorities: new Set([2]) }),
      ),
    ).toBe(false);
    expect(
      matchesCard(
        c,
        e,
        "",
        filter({
          labels: new Set(["l1"]),
          priorities: new Set([3]),
          projects: new Set(["/p"]),
        }),
      ),
    ).toBe(true);
    // The search ANDs with the filters.
    expect(matchesCard(c, e, "zzz", filter({ labels: new Set(["l1"]) }))).toBe(
      false,
    );
  });

  it("a card without extras has no labels or projects and priority none", () => {
    expect(
      matchesCard(
        card(1, "A"),
        undefined,
        "",
        filter({ labels: new Set(["l1"]) }),
      ),
    ).toBe(false);
    expect(
      matchesCard(
        card(1, "A"),
        undefined,
        "",
        filter({ projects: new Set(["/p"]) }),
      ),
    ).toBe(false);
    expect(
      matchesCard(
        card(1, "A"),
        undefined,
        "",
        filter({ priorities: new Set([0]) }),
      ),
    ).toBe(true);
    expect(
      matchesCard(
        card(1, "A"),
        undefined,
        "",
        filter({ priorities: new Set([2]) }),
      ),
    ).toBe(false);
  });
});

describe("pruneFilter", () => {
  const label = (id: string): BoardLabel => ({
    id,
    boardId: "b1",
    name: id,
    color: "ff0000",
    position: 0,
  });

  it("prune_drops_deleted_labels", () => {
    const f = filter({
      labels: new Set(["l1", "gone"]),
      projects: new Set(["/a", "/removed"]),
      priorities: new Set([2]),
    });
    const pruned = pruneFilter(f, [label("l1")], [{ path: "/a", name: "a" }]);
    expect([...pruned.labels]).toEqual(["l1"]);
    expect([...pruned.projects]).toEqual(["/a"]);
    expect([...pruned.priorities]).toEqual([2]);
  });

  it("returns the same filter when nothing was dropped", () => {
    const f = filter({ labels: new Set(["l1"]) });
    expect(pruneFilter(f, [label("l1")], [])).toBe(f);
  });

  it("an all-gone filter ends up empty, not hiding everything", () => {
    const f = filter({ labels: new Set(["gone"]) });
    const pruned = pruneFilter(f, [], []);
    expect(filterCount(pruned)).toBe(0);
    expect(matchesCard(card(1, "A"), undefined, "", pruned)).toBe(true);
  });
});

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
