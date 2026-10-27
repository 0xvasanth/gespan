import { expect, it } from "vitest";
import type { BoardCard, CardExtras } from "./board";
import { sortedCardIds } from "./columnActions";

const card = (id: string, position: number): BoardCard => ({
  id,
  boardId: "b",
  columnId: "col",
  number: position + 1,
  title: id,
  body: "",
  position,
  linkKind: null,
  linkId: null,
  createdAt: 1,
  updatedAt: 1,
});
const extra = (priority: number, dueAt: number | null): CardExtras => ({
  priority,
  dueAt,
  projects: [],
  labels: [],
  subtasksDone: 0,
  subtasksTotal: 0,
  comments: 0,
});

const cards = ["a", "b", "c", "d", "e"].map(card);

it("sorts by priority, Urgent first, ties in current order", () => {
  const extras = {
    a: extra(1, null),
    b: extra(4, null),
    c: extra(1, null),
    // d has no extras at all: priority None
    e: extra(4, null),
  };
  expect(sortedCardIds(cards, extras, "priority")).toEqual([
    "b",
    "e",
    "a",
    "c",
    "d",
  ]);
});

it("sorts by due date, soonest first, undated last in current order", () => {
  const extras = {
    a: extra(0, null),
    b: extra(0, 300),
    c: extra(0, 100),
    d: extra(0, null),
    e: extra(0, 100),
  };
  expect(sortedCardIds(cards, extras, "due")).toEqual([
    "c",
    "e",
    "b",
    "a",
    "d",
  ]);
});

it("keeps the order when nothing differs", () => {
  expect(sortedCardIds(cards, {}, "priority")).toEqual([
    "a",
    "b",
    "c",
    "d",
    "e",
  ]);
  expect(sortedCardIds([], {}, "due")).toEqual([]);
});
