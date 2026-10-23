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

  describe("keyboard drag", () => {
    afterEach(() => vi.restoreAllMocks());

    const liveRegion = () =>
      document.querySelector("[id^='DndLiveRegion']")?.textContent ?? "";

    function key(target: EventTarget, code: string, k: string) {
      act(() => {
        target.dispatchEvent(
          new KeyboardEvent("keydown", {
            code,
            key: k,
            bubbles: true,
            cancelable: true,
          }),
        );
      });
    }

    const flush = () =>
      act(() => new Promise((resolve) => setTimeout(resolve, 0)));

    // happy-dom has no layout: lay columns out 300px apart and stack cards.
    // `collapse` sizes the dragged card's slot as the 2px line it renders as.
    function stubLayout({ collapse = false } = {}) {
      const rect = (x: number, y: number, width: number, height: number) =>
        ({
          x,
          y,
          left: x,
          top: y,
          width,
          height,
          right: x + width,
          bottom: y + height,
          toJSON() {},
        }) as DOMRect;
      vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(
        function (this: Element) {
          const section = this.closest("[data-kanban-column]");
          // The drag overlay (portalled to body) starts over c12's slot.
          if (!section) return rect(6, 120, 268, 60);
          const sections = [
            ...container.querySelectorAll("[data-kanban-column]"),
          ];
          const x = sections.indexOf(section) * 300;
          if (this === section) return rect(x, 0, 280, 500);
          if (this.hasAttribute("data-kanban-card")) {
            const height = (el: Element) =>
              collapse && el.hasAttribute("data-dragging") ? 2 : 60;
            const siblings = [...this.parentElement!.children];
            const above = siblings.slice(0, siblings.indexOf(this));
            const y = above.reduce((top, el) => top + height(el) + 10, 50);
            return rect(x + 6, y, 268, height(this));
          }
          if (
            this.querySelector("[data-kanban-card]") ||
            this.matches(".overflow-y-auto")
          ) {
            return rect(x, 40, 280, 420);
          }
          return rect(x, 0, 280, 36);
        },
      );
    }

    // The keyboard sensor binds its keydown listener on the next tick.
    async function pickUp(cardId: string) {
      const el = container.querySelector<HTMLElement>(
        `[data-kanban-card='${cardId}']`,
      )!;
      act(() => el.focus());
      key(el, "Space", " ");
      await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
      return el;
    }

    it("announces cancel with the drag-start position", async () => {
      render();
      const el = await pickUp("c12");
      expect(container.querySelector("[data-dragging='']")).not.toBeNull();
      key(el, "Escape", "Escape");
      await flush();
      expect(liveRegion()).toBe(
        "Moving card #12 was cancelled. It is back in Backlog, position 2 of 2.",
      );
      expect(container.querySelector("[data-dragging]")).toBeNull();
    });

    it("shows a detail that arrived mid-drag after cancel", async () => {
      const props = render();
      const el = await pickUp("c12");
      const fresh: BoardDetail = {
        ...detail,
        cards: [
          ...detail.cards,
          {
            ...detail.cards[0],
            id: "c13",
            number: 13,
            title: "Card 13",
            position: 2,
          },
        ],
      };
      act(() =>
        root.render(createElement(KanbanBoard, { ...props, detail: fresh })),
      );
      key(el, "Escape", "Escape");
      await flush();
      const backlog = container.querySelector(
        "[data-kanban-column='backlog']",
      )!;
      expect(
        [...backlog.querySelectorAll("[data-card]")].map((c) =>
          c.getAttribute("data-card"),
        ),
      ).toEqual(["c11", "c12", "c13"]);
      expect(props.onCardMove).not.toHaveBeenCalled();
    });

    it("cancel after a cross-column preview announces the original column", async () => {
      stubLayout();
      const props = render();
      const el = await pickUp("c12");
      key(el, "ArrowRight", "ArrowRight");
      await flush();
      const progress = container.querySelector(
        "[data-kanban-column='progress']",
      )!;
      // The preview moved the card into In Progress.
      expect(progress.querySelector("[data-kanban-card='c12']")).not.toBeNull();
      // The card remounted in another column, so press Escape on the document.
      key(document, "Escape", "Escape");
      await flush();
      expect(liveRegion()).toBe(
        "Moving card #12 was cancelled. It is back in Backlog, position 2 of 2.",
      );
      const backlog = container.querySelector(
        "[data-kanban-column='backlog']",
      )!;
      expect(backlog.querySelector("[data-kanban-card='c12']")).not.toBeNull();
      expect(props.onCardMove).not.toHaveBeenCalled();
    });

    it("drops a keyboard cross-column move where the preview showed it", async () => {
      stubLayout({ collapse: true });
      const props = render();
      const el = await pickUp("c12");
      key(el, "ArrowRight", "ArrowRight");
      await flush();
      const progress = container.querySelector(
        "[data-kanban-column='progress']",
      )!;
      const shown = () =>
        [...progress.querySelectorAll("[data-kanban-card]")].map((c) =>
          c.getAttribute("data-kanban-card"),
        );
      // Previewed before the card it moved next to.
      expect(shown()).toEqual(["c12", "c21"]);
      key(document, "Space", " ");
      await flush();
      expect(props.onCardMove).toHaveBeenCalledTimes(1);
      expect(props.onCardMove).toHaveBeenCalledWith("c12", {
        columnId: "progress",
        index: 0,
      });
      expect(shown()).toEqual(["c12", "c21"]);
      expect(liveRegion()).toBe(
        "Dropped card #12 in In Progress, position 1 of 2.",
      );
    });

    it("shows a detail that arrived mid-drag after a same-place drop", async () => {
      stubLayout();
      const props = render();
      const el = await pickUp("c12");
      const fresh: BoardDetail = {
        ...detail,
        cards: detail.cards.filter((c) => c.id !== "c21"),
      };
      act(() =>
        root.render(createElement(KanbanBoard, { ...props, detail: fresh })),
      );
      key(el, "Space", " ");
      await flush();
      expect(liveRegion()).toBe(
        "Dropped card #12 in Backlog, position 2 of 2.",
      );
      expect(props.onCardMove).not.toHaveBeenCalled();
      expect(container.querySelector("[data-kanban-card='c21']")).toBeNull();
    });
  });
});
