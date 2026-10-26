// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import type { BoardSettings } from "../../model/board";
import { CARD_ESTIMATE, CARD_GAP } from "../../model/windowing";
import { BoardView } from "../BoardView";
import {
  card,
  COLUMNS,
  createTestBackend,
  flush,
  type TestBackend,
} from "../testBackend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const events = vi.hoisted(() => ({
  listeners: new Set<(event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    (_name: string, handler: (event: { payload: unknown }) => void) => {
      events.listeners.add(handler);
      return Promise.resolve(() => events.listeners.delete(handler));
    },
  ),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));
vi.mock("../../../../app/shell/WindowControls", () => ({
  WindowControls: () => null,
}));

// Mounting 1,000 cards is the subject here; give each test room.
vi.setConfig({ testTimeout: 15_000 });

/** A comfortable card's slot before it is measured: its estimate and the gap. */
const STEP = CARD_ESTIMATE.comfortable + CARD_GAP;
const MAX_RENDERED = 60;

let backend: TestBackend;
let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const stored = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => stored.set(key, value),
    removeItem: (key: string) => stored.delete(key),
    clear: () => stored.clear(),
  });
  events.listeners.clear();
  backend = createTestBackend(events.listeners);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

async function renderBoard() {
  const props: ComponentProps<typeof BoardView> = {
    onClose: vi.fn(),
    linkTargets: [],
    onOpenLinkedWork: vi.fn(() => Promise.resolve()),
    lookupLinkTargets: vi.fn(async () => []),
    projects: [],
  };
  await act(async () => {
    root.render(createElement(BoardView, props));
  });
  await flush();
}

async function reload() {
  await act(async () => backend.emitBoardChanged());
  await flush();
  await flush();
}

const commands = (cmd: string) =>
  vi.mocked(invoke).mock.calls.filter(([name]) => name === cmd);

const columnEl = (id: string) =>
  container.querySelector<HTMLElement>(`[data-kanban-column="${id}"]`)!;
const bodyOf = (id: string) =>
  columnEl(id).querySelector<HTMLElement>(".overflow-y-auto")!;
const rendered = (id: string) =>
  [...columnEl(id).querySelectorAll<HTMLElement>("[data-kanban-card]")].map(
    (el) => el.dataset.kanbanCard!,
  );
const cardEl = (id: string) =>
  container.querySelector<HTMLElement>(`[data-kanban-card="${id}"]`);

function scrollColumn(id: string, top: number) {
  const body = bodyOf(id);
  act(() => {
    body.scrollTop = top;
    body.dispatchEvent(new Event("scroll"));
  });
}

async function pick(menuLabel: string, itemText: string) {
  const trigger = container.querySelector<HTMLButtonElement>(
    `button[aria-label="${menuLabel}"]`,
  )!;
  expect(trigger, menuLabel).not.toBeNull();
  if (trigger.getAttribute("aria-expanded") !== "true") {
    act(() => trigger.click());
  }
  const item = [
    ...document.querySelectorAll<HTMLElement>('[role^="menuitem"]'),
  ].find((el) => el.textContent?.trim() === itemText)!;
  expect(item, itemText).toBeDefined();
  act(() => {
    item.dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true }),
    );
  });
  await flush();
}

async function search(value: string) {
  const input = container.querySelector<HTMLInputElement>(
    'input[aria-label="Search cards"]',
  )!;
  act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await flush();
  await flush();
}

/** The 1,000 cards in Backlog, plus one card to drag in from In Progress. */
function thousandInBacklog() {
  backend.seedThousand(1);
  backend.cards = [
    ...backend.cards,
    card("src", "progress", 0, 1001, "Incoming"),
  ];
}

// happy-dom has no layout. This lays a column body out the way the browser
// would: 640px tall at y = 40, children stacked in DOM order with the 6px
// gap and padding, spacers at their style height, cards at `cardHeight`.
// Columns sit 300px apart.
const BODY_TOP = 40;
const BODY_HEIGHT = 640;
const PAD = 6;
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

function stubLayout(cardHeight: (el: Element) => number) {
  /** Where the dragged card's overlay sits (it is portalled out of the column). */
  const overlay = { rect: rect(0, 0, 0, 0) };
  const heightOf = (el: Element) =>
    el.hasAttribute("data-kanban-card")
      ? el.hasAttribute("data-dragging")
        ? 2
        : cardHeight(el)
      : parseFloat((el as HTMLElement).style.height) || 0;
  const layout = function (this: Element): DOMRect {
    const section = this.closest("[data-kanban-column]");
    if (!section) return overlay.rect;
    const x =
      [...container.querySelectorAll("[data-kanban-column]")].indexOf(section) *
      300;
    if (this === section) return rect(x, 0, 280, BODY_TOP + BODY_HEIGHT + 40);
    const body = section.querySelector<HTMLElement>(".overflow-y-auto");
    const slot = this.closest("[data-kanban-card]");
    if (body && slot && slot.parentElement) {
      let y = BODY_TOP + PAD - body.scrollTop;
      for (const sibling of slot.parentElement.children) {
        if (sibling === slot) break;
        y += heightOf(sibling) + CARD_GAP;
      }
      return rect(x + PAD, y, 268, heightOf(slot));
    }
    if (body && (this === body || body.contains(this))) {
      return rect(x, BODY_TOP, 280, BODY_HEIGHT);
    }
    return rect(x, 0, 280, 36);
  };
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(
    layout,
  );
  return {
    rectOf: (el: Element) => layout.call(el),
    overlay,
  };
}

/** Comfortable cards are 90px tall once laid out, compact ones 40px. */
const byDensity = (el: Element) =>
  el.querySelector('[data-density="compact"]') ? 40 : 90;

function pointer(type: string, target: EventTarget, x: number, y: number) {
  act(() => {
    target.dispatchEvent(
      new PointerEvent(type, {
        bubbles: true,
        cancelable: true,
        clientX: x,
        clientY: y,
        isPrimary: true,
        button: 0,
        pointerId: 1,
      }),
    );
  });
}

const center = (r: DOMRect) => ({ x: r.left + r.width / 2, y: r.top + r.height / 2 });

/** Drags card `from` by the pointer and drops it over the middle of card `over`. */
async function dragOnto(
  layout: ReturnType<typeof stubLayout>,
  from: string,
  over: string,
) {
  const source = cardEl(from)!;
  const start = layout.rectOf(source);
  layout.overlay.rect = start;
  const a = center(start);
  pointer("pointerdown", source, a.x, a.y);
  // Past the 4px activation distance, then onto the target.
  pointer("pointermove", document, a.x + 10, a.y + 10);
  await flush();
  const b = center(layout.rectOf(cardEl(over)!));
  pointer("pointermove", document, b.x, b.y);
  await flush();
  pointer("pointerup", document, b.x, b.y);
  await flush();
  await flush();
}

const lastMove = () =>
  commands("board_card_move").at(-1)?.[1] as
    | { cardId: string; columnId: string; index: number }
    | undefined;

describe("windowed board columns", () => {
  it("a_1000_card_column_renders_at_most_60_cards", async () => {
    backend.seedThousand(1);
    await renderBoard();
    const shown = rendered("backlog");
    expect(shown.length).toBeGreaterThan(0);
    expect(shown.length).toBeLessThanOrEqual(MAX_RENDERED);
    expect(shown[0]).toBe("k0");
    // Counts and positions come from the data, not the mounted cards.
    expect(
      columnEl("backlog").querySelector("[data-column-count]")?.textContent,
    ).toBe("1000");
    expect(cardEl("k0")!.getAttribute("aria-description")).toBe(
      "1 of 1000 in Backlog",
    );
    // The spacer after the window keeps the column its full height.
    const spacers = [
      ...bodyOf("backlog").querySelectorAll<HTMLElement>("[data-window-gap]"),
    ];
    expect(spacers).toHaveLength(1);
    expect(parseFloat(spacers[0].style.height)).toBe(
      (1000 - shown.length) * STEP - CARD_GAP,
    );
  });

  it("every_expanded_column_is_bounded_and_collapsed_columns_render_none", async () => {
    backend.seedThousand(4);
    backend.columns = COLUMNS.map((c) =>
      c.id === "review" ? { ...c, collapsed: true } : c,
    );
    await renderBoard();
    for (const id of ["backlog", "progress", "done"]) {
      expect(rendered(id).length, id).toBeGreaterThan(0);
      expect(rendered(id).length, id).toBeLessThanOrEqual(MAX_RENDERED);
    }
    expect(rendered("review")).toEqual([]);
    expect(
      columnEl("review").querySelector("[data-column-count]")?.textContent,
    ).toBe("250");
    expect(
      container.querySelectorAll("[data-kanban-card]").length,
    ).toBeLessThanOrEqual(3 * MAX_RENDERED);
  });

  it("scrolling_a_column_renders_the_cards_at_that_offset", async () => {
    backend.seedThousand(1);
    await renderBoard();
    scrollColumn("backlog", 500 * STEP);
    const shown = rendered("backlog");
    expect(shown.length).toBeLessThanOrEqual(MAX_RENDERED);
    expect(shown).toContain("k500");
    expect(shown).not.toContain("k0");
    // Overscan: four cards above the first one in view.
    expect(shown[0]).toBe("k496");
    // The spacer above stands in for k0…k495, so k500 keeps its offset.
    const first = bodyOf("backlog").firstElementChild as HTMLElement;
    expect(first.hasAttribute("data-window-gap")).toBe(true);
    expect(parseFloat(first.style.height)).toBe(496 * STEP - CARD_GAP);
    expect(cardEl("k500")!.getAttribute("aria-description")).toBe(
      "501 of 1000 in Backlog",
    );
  });

  it("a_reload_with_unchanged_ids_keeps_the_scroll_position", async () => {
    backend.seedThousand(1);
    await renderBoard();
    scrollColumn("backlog", 500 * STEP + 20);
    const before = rendered("backlog");
    // The other app renames a card: same ids, new data.
    backend.cards = backend.cards.map((c) =>
      c.id === "k501" ? { ...c, title: "Renamed elsewhere" } : c,
    );
    expect(before).not.toContain("k0");
    await reload();
    expect(cardEl("k501")!.textContent).toContain("Renamed elsewhere");
    expect(bodyOf("backlog").scrollTop).toBe(500 * STEP + 20);
    expect(rendered("backlog")).toEqual(before);
  });

  it("a_card_added_above_keeps_the_top_card_in_view", async () => {
    backend.seedThousand(1);
    await renderBoard();
    // 20px into k500: k500 is the card at the top of the view.
    scrollColumn("backlog", 500 * STEP + 20);
    // The other app adds a card at the top of the column.
    backend.cards = [
      card("new", "backlog", 0, 2000, "Added elsewhere"),
      ...backend.cards.map((c) => ({ ...c, position: c.position + 1 })),
    ];
    await reload();
    expect(bodyOf("backlog").scrollTop).toBe(501 * STEP + 20);
    expect(rendered("backlog")).toContain("k500");
    expect(cardEl("k500")!.getAttribute("aria-description")).toBe(
      "502 of 1001 in Backlog",
    );
    // And it deletes two cards above: k500 still stays at the top.
    backend.cards = backend.cards.filter(
      (c) => c.id !== "new" && c.id !== "k3" && c.id !== "k7",
    );
    await reload();
    expect(bodyOf("backlog").scrollTop).toBe(498 * STEP + 20);
    expect(rendered("backlog")).toContain("k500");
    // The top card itself goes: the view stays put, never jumps to the top.
    backend.cards = backend.cards.filter((c) => c.id !== "k500");
    await reload();
    expect(bodyOf("backlog").scrollTop).toBe(498 * STEP + 20);
    expect(rendered("backlog")).toContain("k501");
    expect(rendered("backlog")).not.toContain("k0");
  });

  it("drop_into_a_scrolled_column_lands_between_the_hovered_cards", async () => {
    thousandInBacklog();
    await renderBoard();
    const layout = stubLayout(() => 90);
    scrollColumn("backlog", 500 * STEP);
    await flush();
    const shown = rendered("backlog");
    expect(shown).toContain("k503");
    expect(shown).not.toContain("k0");
    await dragOnto(layout, "src", "k503");
    // Index 503 counts every card in the column, mounted or not: it lands
    // between k502 and k503.
    expect(lastMove()).toMatchObject({
      cardId: "src",
      columnId: "backlog",
      index: 503,
    });
    const order = rendered("backlog");
    expect(order.slice(order.indexOf("src") - 1, order.indexOf("src") + 2)).toEqual(
      ["k502", "src", "k503"],
    );
    expect(cardEl("src")!.getAttribute("aria-description")).toBe(
      "504 of 1001 in Backlog",
    );
  });

  it("drop_into_a_filtered_windowed_column_keeps_hidden_cards_in_place", async () => {
    thousandInBacklog();
    // Even cards are "Alpha", odd ones "Beta"; the search hides the odd ones.
    backend.cards = backend.cards.map((c, i) =>
      c.id === "src"
        ? { ...c, title: "Alpha incoming" }
        : { ...c, title: `${i % 2 ? "Beta" : "Alpha"} ${c.number}` },
    );
    await renderBoard();
    await search("alpha");
    expect(
      columnEl("backlog").querySelector("[data-column-count]")?.textContent,
    ).toBe("500/1000");
    const layout = stubLayout(() => 90);
    // Visible index 250 is k500.
    scrollColumn("backlog", 250 * STEP);
    await flush();
    expect(rendered("backlog")).toContain("k504");
    expect(rendered("backlog")).not.toContain("k503");
    expect(rendered("backlog")).not.toContain("k0");
    await dragOnto(layout, "src", "k504");
    // Over k504 (visible index 252): in the full column it goes after the
    // hidden k503 and ahead of k504, so no hidden card moves.
    expect(lastMove()).toMatchObject({
      cardId: "src",
      columnId: "backlog",
      index: 504,
    });
  });

  it("sorted_windowed_column_appends_on_drop", async () => {
    thousandInBacklog();
    await renderBoard();
    await pick("Sort", "Title");
    const layout = stubLayout(() => 90);
    scrollColumn("backlog", 500 * STEP);
    await flush();
    const shown = rendered("backlog");
    expect(shown).not.toContain("k0");
    expect(shown.length).toBeLessThanOrEqual(MAX_RENDERED);
    const target = shown[8];
    expect(target).toBeDefined();
    await dragOnto(layout, "src", target);
    // A sorted column has no places to drop between: the card goes last.
    expect(lastMove()).toMatchObject({
      cardId: "src",
      columnId: "backlog",
      index: 1000,
    });
    const moves = commands("board_card_move").length;
    // A drop inside the sorted column moves nothing.
    scrollColumn("backlog", 300 * STEP);
    await flush();
    const inside = rendered("backlog");
    await dragOnto(layout, inside[6], inside[9]);
    expect(commands("board_card_move")).toHaveLength(moves);
  });

  it("density_toggle_remeasures_without_overlap", async () => {
    const settings = (density: "comfortable" | "compact"): BoardSettings => ({
      runWith: true,
      writeBack: true,
      ticketStatusChanges: true,
      autoMoveJson: null,
      icon: null,
      color: null,
      defaultViewId: null,
      displayJson: JSON.stringify({ density }),
      priorityJson: null,
    });
    let current = settings("comfortable");
    backend.handlers.board_settings = () => current;
    backend.seedThousand(1);
    await renderBoard();
    const layout = stubLayout(byDensity);
    scrollColumn("backlog", 500 * STEP);
    await flush();

    /** Mounted cards in DOM order, with their laid-out rects. */
    const laidOut = () =>
      [...bodyOf("backlog").querySelectorAll("[data-kanban-card]")].map((el) => ({
        id: el.getAttribute("data-kanban-card")!,
        r: layout.rectOf(el),
      }));
    function expectSound(height: number) {
      const cards = laidOut();
      expect(cards.length).toBeLessThanOrEqual(MAX_RENDERED);
      for (const { r } of cards) expect(r.height).toBe(height);
      // Nothing overlaps, and the spacing is exactly the gap.
      for (let i = 1; i < cards.length; i += 1) {
        expect(cards[i].r.top - cards[i - 1].r.bottom).toBe(CARD_GAP);
      }
      // No blank gap: the mounted cards cover the whole view.
      expect(cards[0].r.top).toBeLessThanOrEqual(BODY_TOP);
      expect(cards.at(-1)!.r.bottom).toBeGreaterThanOrEqual(
        BODY_TOP + BODY_HEIGHT,
      );
      // The card that was at the top is still there.
      const top = cards.find((c) => c.id === "k500")!;
      expect(top, "k500 mounted").toBeDefined();
      expect(top.r.top).toBe(BODY_TOP + PAD);
    }

    expectSound(90);
    current = settings("compact");
    await reload();
    expect(cardEl("k500")!.querySelector('[data-density="compact"]')).not.toBeNull();
    expectSound(40);
    current = settings("comfortable");
    await reload();
    expectSound(90);
  });

  it("the_dragged_card_stays_mounted_while_scrolled_away", async () => {
    backend.seedThousand(1);
    await renderBoard();
    const el = cardEl("k3")!;
    act(() => el.focus());
    act(() => {
      el.dispatchEvent(
        new KeyboardEvent("keydown", {
          code: "Space",
          key: " ",
          bubbles: true,
          cancelable: true,
        }),
      );
    });
    await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
    expect(cardEl("k3")!.hasAttribute("data-dragging")).toBe(true);
    scrollColumn("backlog", 500 * STEP);
    const shown = rendered("backlog");
    expect(shown).toContain("k500");
    expect(shown).toContain("k3");
    expect(shown).not.toContain("k2");
    expect(shown.length).toBeLessThanOrEqual(MAX_RENDERED);
    expect(cardEl("k3")!.hasAttribute("data-dragging")).toBe(true);
    // Kept in its place: a spacer on each side of it.
    const slot = cardEl("k3")!;
    expect(slot.previousElementSibling?.hasAttribute("data-window-gap")).toBe(true);
    expect(slot.nextElementSibling?.hasAttribute("data-window-gap")).toBe(true);
    act(() => {
      document.dispatchEvent(
        new KeyboardEvent("keydown", {
          code: "Escape",
          key: "Escape",
          bubbles: true,
          cancelable: true,
        }),
      );
    });
    await flush();
    expect(cardEl("k3")!.hasAttribute("data-dragging")).toBe(false);
    expect(commands("board_card_move")).toHaveLength(0);
  });

  it("the focused and open cards stay mounted while scrolled away", async () => {
    backend.seedThousand(1);
    await renderBoard();
    act(() => cardEl("k2")!.focus());
    scrollColumn("backlog", 500 * STEP);
    expect(rendered("backlog")).toContain("k2");
    expect(document.activeElement).toBe(cardEl("k2"));
    // Focus elsewhere: k2 is no longer kept.
    act(() => (document.activeElement as HTMLElement).blur());
    scrollColumn("backlog", 500 * STEP + 1);
    expect(rendered("backlog")).not.toContain("k2");

    // Open k501's drawer, then scroll back to the top.
    act(() => cardEl("k501")!.focus());
    act(() => {
      cardEl("k501")!.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Enter",
          bubbles: true,
          cancelable: true,
        }),
      );
    });
    await flush();
    expect(container.querySelector("[data-card-drawer]")).not.toBeNull();
    act(() => (document.activeElement as HTMLElement)?.blur?.());
    scrollColumn("backlog", 0);
    expect(rendered("backlog")).toContain("k0");
    expect(rendered("backlog")).toContain("k501");
    expect(rendered("backlog").length).toBeLessThanOrEqual(MAX_RENDERED);
  });
});
