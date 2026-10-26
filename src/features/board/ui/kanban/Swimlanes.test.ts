// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BoardCard, BoardColumn } from "../../model/board";
import type { Lane } from "../../model/swimlanes";
import { Swimlanes } from "./Swimlanes";

const column = (id: string, name: string, position: number): BoardColumn => ({
  id,
  boardId: "b",
  name,
  position,
  role: null,
  color: null,
  wipLimit: null,
  collapsed: false,
});
const card = (id: string, columnId: string, number: number): BoardCard => ({
  id,
  boardId: "b",
  columnId,
  number,
  title: `Card ${number}`,
  body: "",
  position: number,
  linkKind: null,
  linkId: null,
  createdAt: 1,
  updatedAt: 1,
});

const columns = [column("backlog", "Backlog", 0), column("review", "Review", 1)];
const cards = [card("c1", "backlog", 1), card("c2", "backlog", 2)];
const lanes: Lane[] = [
  { key: "l1", label: "Bug", cardIds: ["c1"] },
  { key: "l2", label: "Docs", cardIds: ["c2"] },
];

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  document.body.innerHTML = "";
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

// happy-dom has no layout: cells sit in a grid, columns 300px apart, lanes 200px apart.
function stubLayout() {
  const rect = (x: number, y: number, w: number, h: number) =>
    ({ x, y, left: x, top: y, width: w, height: h, right: x + w, bottom: y + h, toJSON() {} }) as DOMRect;
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(
    function (this: Element) {
      const cell = this.closest("[data-lane-cell]");
      if (!cell) return rect(6, 6, 268, 60);
      const [lane, col] = cell.getAttribute("data-lane-cell")!.split("|");
      const x = columns.findIndex((c) => c.id === col) * 300;
      const y = lanes.findIndex((l) => l.key === lane) * 200;
      if (this === cell) return rect(x, y, 280, 180);
      return rect(x + 6, y + 6, 268, 60);
    },
  );
}

function render(over: Partial<Parameters<typeof Swimlanes>[0]> = {}) {
  const props = {
    columns,
    cards,
    lanes,
    renderCard: (c: BoardCard) => createElement("span", null, c.title),
    renderColumnHeader: (c: BoardColumn) => createElement("span", null, c.name),
    onDrop: vi.fn(),
    onCardOpen: vi.fn(),
    ...over,
  };
  act(() => root.render(createElement(Swimlanes, props)));
  return props;
}

const key = (target: EventTarget, code: string, k: string) =>
  act(() => {
    target.dispatchEvent(
      new KeyboardEvent("keydown", { code, key: k, bubbles: true, cancelable: true }),
    );
  });
const tick = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));
const cardEl = (id: string) =>
  container.querySelector<HTMLElement>(`[data-kanban-card='${id}']`)!;
const liveRegion = () =>
  document.querySelector("[id^='DndLiveRegion']")?.textContent ?? "";

describe("swimlane keyboard", () => {
  it("names each card like a column card does", () => {
    render();
    expect(cardEl("c1").getAttribute("aria-label")).toBe("Card #1, Card 1");
    expect(cardEl("c1").getAttribute("aria-roledescription")).toBe("card");
  });

  it("Enter opens a focused card instead of picking it up; Space picks it up", async () => {
    stubLayout();
    const props = render();
    const el = cardEl("c1");
    act(() => el.focus());
    key(el, "Enter", "Enter");
    await tick();
    expect(props.onCardOpen).toHaveBeenCalledTimes(1);
    expect(props.onCardOpen).toHaveBeenCalledWith(expect.objectContaining({ id: "c1" }));
    expect(container.querySelector("[data-dragging]")).toBeNull();
    key(el, "Space", " ");
    await tick();
    expect(container.querySelector("[data-dragging]")).not.toBeNull();
    expect(props.onCardOpen).toHaveBeenCalledTimes(1);
    // Enter while carrying drops it; it doesn't open it.
    key(el, "Enter", "Enter");
    await tick();
    expect(container.querySelector("[data-dragging]")).toBeNull();
    expect(props.onCardOpen).toHaveBeenCalledTimes(1);
  });

  it("leaves Enter on a control inside the card to that control", async () => {
    const props = render({
      renderCard: (c) =>
        createElement("button", { type: "button", "data-inner": c.id }, "…"),
    });
    const inner = container.querySelector<HTMLElement>("[data-inner='c1']")!;
    act(() => inner.focus());
    key(inner, "Enter", "Enter");
    await tick();
    expect(props.onCardOpen).not.toHaveBeenCalled();
    expect(container.querySelector("[data-dragging]")).toBeNull();
  });

  it("lifts with Space, moves cell to cell with the arrows, drops with Space, and says where", async () => {
    stubLayout();
    const props = render();
    const el = cardEl("c1");
    act(() => el.focus());
    key(el, "Space", " ");
    await tick();
    // Lifting announces the pick-up, then at once the cell it is over.
    expect(liveRegion()).toBe("Card #1, Card 1 is over lane Bug, column Backlog.");

    key(el, "ArrowRight", "ArrowRight");
    await tick();
    expect(liveRegion()).toBe("Card #1, Card 1 is over lane Bug, column Review.");

    key(el, "ArrowDown", "ArrowDown");
    await tick();
    expect(liveRegion()).toBe("Card #1, Card 1 is over lane Docs, column Review.");

    key(document, "Space", " ");
    await tick();
    expect(props.onDrop).toHaveBeenCalledTimes(1);
    expect(props.onDrop).toHaveBeenCalledWith("c1", "l2", "review");
    expect(liveRegion()).toBe("Dropped card #1 in lane Docs, column Review.");
    // No internal ids in what is spoken.
    expect(liveRegion()).not.toMatch(/cell:|\|/);
  });

  it("Escape cancels and says where the card is back", async () => {
    stubLayout();
    const props = render();
    const el = cardEl("c1");
    act(() => el.focus());
    key(el, "Space", " ");
    await tick();
    key(el, "ArrowRight", "ArrowRight");
    await tick();
    key(document, "Escape", "Escape");
    await tick();
    expect(props.onDrop).not.toHaveBeenCalled();
    expect(liveRegion()).toBe(
      "Moving card #1 was cancelled. It is back in lane Bug, column Backlog.",
    );
  });

  it("an arrow with no cell beyond it keeps the card where it is", async () => {
    stubLayout();
    const props = render();
    const el = cardEl("c1");
    act(() => el.focus());
    key(el, "Space", " ");
    await tick();
    key(el, "ArrowLeft", "ArrowLeft");
    await tick();
    key(document, "Space", " ");
    await tick();
    expect(props.onDrop).toHaveBeenCalledWith("c1", "l1", "backlog");
  });

  it("explains the keys to screen readers", () => {
    render();
    expect(document.body.textContent).toContain(
      "arrow keys to move it between lanes and columns",
    );
  });
});
