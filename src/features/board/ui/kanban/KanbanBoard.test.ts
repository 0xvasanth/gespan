// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BoardCard, BoardColumn, BoardDetail } from "../../model/board";
import { KanbanBoard, type KanbanBoardProps } from "./KanbanBoard";

function column(id: string, name: string, position: number): BoardColumn {
  return { id, boardId: "b1", name, position, role: null };
}

function card(
  id: string,
  columnId: string,
  position: number,
  number: number,
): BoardCard {
  return {
    id,
    boardId: "b1",
    columnId,
    number,
    title: `Card ${number}`,
    body: "",
    position,
    linkKind: null,
    linkId: null,
    createdAt: 1,
    updatedAt: 1,
  };
}

const detail: BoardDetail = {
  board: { id: "b1", name: "Board", createdAt: 1, updatedAt: 1 },
  columns: [
    column("progress", "In Progress", 1),
    column("backlog", "Backlog", 0),
    column("review", "Review", 2),
  ],
  cards: [
    card("c12", "backlog", 1, 12),
    card("c21", "progress", 0, 21),
    card("c11", "backlog", 0, 11),
  ],
};

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

function render(overrides: Partial<KanbanBoardProps> = {}) {
  const props: KanbanBoardProps = {
    detail,
    renderCard: (c) => createElement("span", { "data-card": c.id }, c.title),
    renderColumnHeader: (col, count) =>
      createElement("span", { "data-header": col.id }, `${col.name} ${count}`),
    renderColumnFooter: (col) =>
      createElement("span", { "data-footer": col.id }, "+ Add card"),
    onCardMove: vi.fn(),
    onColumnMove: vi.fn(),
    ...overrides,
  };
  act(() => root.render(createElement(KanbanBoard, props)));
  return props;
}

const attr = (selector: string, name: string) =>
  [...container.querySelectorAll(selector)].map((el) => el.getAttribute(name));

describe("KanbanBoard", () => {
  it("renders columns in position order with header counts and footers", () => {
    render();
    expect(attr("[data-header]", "data-header")).toEqual([
      "backlog",
      "progress",
      "review",
    ]);
    expect(
      [...container.querySelectorAll("[data-header]")].map(
        (el) => el.textContent,
      ),
    ).toEqual(["Backlog 2", "In Progress 1", "Review 0"]);
    expect(attr("[data-footer]", "data-footer")).toEqual([
      "backlog",
      "progress",
      "review",
    ]);
  });

  it("renders each column's cards in position order inside that column", () => {
    render();
    const columns = [...container.querySelectorAll("[data-kanban-column]")];
    expect(columns.map((el) => el.getAttribute("data-kanban-column"))).toEqual([
      "backlog",
      "progress",
      "review",
    ]);
    const cardsIn = (el: Element) =>
      [...el.querySelectorAll("[data-card]")].map((c) =>
        c.getAttribute("data-card"),
      );
    expect(columns.map(cardsIn)).toEqual([["c11", "c12"], ["c21"], []]);
  });

  it("renders the trailing slot after the last column", () => {
    render({ trailing: createElement("button", { "data-trailing": "" }, "+") });
    const scroller = container.querySelector("[data-kanban-board]")!;
    expect(scroller.lastElementChild?.hasAttribute("data-trailing")).toBe(true);
  });

  it("does not use native HTML drag and drop", () => {
    render();
    expect(container.querySelector("[draggable='true']")).toBeNull();
  });

  it("is not dragging or targeting any column at rest", () => {
    render();
    expect(container.querySelector("[data-dragging]")).toBeNull();
    expect(container.querySelector("[data-over]")).toBeNull();
  });
});
