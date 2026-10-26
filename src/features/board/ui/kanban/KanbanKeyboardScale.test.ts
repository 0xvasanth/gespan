// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { IS_MAC } from "../../../../platform/tauri/platform";
import type { BoardCard } from "../../model/board";
import { ROW_HEIGHT } from "../../model/layout";
import { CARD_ESTIMATE, CARD_GAP } from "../../model/windowing";
import { BoardView } from "../BoardView";
import { createTestBackend, flush, type TestBackend } from "../testBackend";

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
const MOD: MouseEventInit = IS_MAC ? { metaKey: true } : { ctrlKey: true };

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
const focusedCard = () =>
  (document.activeElement as HTMLElement | null)?.getAttribute(
    "data-kanban-card",
  ) ?? null;
const liveRegion = () =>
  document.querySelector("[id^='DndLiveRegion']")?.textContent ?? "";
const searchBox = () =>
  container.querySelector<HTMLInputElement>('input[aria-label="Search cards"]')!;

function scrollEl(el: HTMLElement, top: number) {
  act(() => {
    el.scrollTop = top;
    el.dispatchEvent(new Event("scroll"));
  });
}
const scrollColumn = (id: string, top: number) => scrollEl(bodyOf(id), top);

function key(target: EventTarget, k: string, init: KeyboardEventInit = {}) {
  const code = k === " " ? "Space" : k;
  act(() => {
    target.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: k,
        code,
        bubbles: true,
        cancelable: true,
        ...init,
      }),
    );
  });
}

async function click(el: Element, init: MouseEventInit = {}) {
  act(() => {
    el.dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true, ...init }),
    );
  });
  await flush();
}

async function pick(menuLabel: string, itemText: string) {
  const trigger = container.querySelector<HTMLButtonElement>(
    `button[aria-label="${menuLabel}"]`,
  )!;
  if (trigger.getAttribute("aria-expanded") !== "true") {
    act(() => trigger.click());
  }
  const item = [
    ...document.querySelectorAll<HTMLElement>('[role^="menuitem"]'),
  ].find((el) => el.textContent?.trim() === itemText)!;
  expect(item, itemText).toBeDefined();
  await click(item);
}

/** Focuses `id` and presses Space: the keyboard sensor binds on the next tick. */
async function lift(id: string) {
  const el = cardEl(id)!;
  act(() => el.focus());
  key(el, " ");
  await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
  expect(cardEl(id)!.hasAttribute("data-dragging")).toBe(true);
}

/** Plays `board_card_move` into the fake store, as the real one orders it. */
function applyMovesInStore() {
  backend.handlers.board_card_move = (args) => {
    const { cardId, columnId, index } = args as {
      cardId: string;
      columnId: string;
      index: number;
    };
    const moving = backend.cards.find((c) => c.id === cardId)!;
    const order = (col: string) =>
      backend.cards
        .filter((c) => c.columnId === col && c.id !== cardId)
        .sort((a, b) => a.position - b.position);
    const target = order(columnId);
    target.splice(index, 0, { ...moving, columnId });
    const placed = new Map<string, BoardCard>();
    target.forEach((c, i) => placed.set(c.id, { ...c, position: i }));
    if (moving.columnId !== columnId) {
      order(moving.columnId).forEach((c, i) =>
        placed.set(c.id, { ...c, position: i }),
      );
    }
    backend.cards = backend.cards.map((c) => placed.get(c.id) ?? c);
    return undefined;
  };
}

const storeOrder = (columnId: string) =>
  backend.cards
    .filter((c) => c.columnId === columnId)
    .sort((a, b) => a.position - b.position)
    .map((c) => c.id);

describe("keyboard focus, drags and announcements at scale", () => {
  it("arrow_down_past_the_window_scrolls_and_focuses_the_next_card", async () => {
    backend.seedThousand(1);
    await renderBoard();
    const shown = rendered("backlog");
    const last = shown.at(-1)!;
    const next = `k${Number(last.slice(1)) + 1}`;
    expect(cardEl(next)).toBeNull();
    act(() => cardEl(last)!.focus());
    key(cardEl(last)!, "ArrowDown");
    expect(focusedCard()).toBe(next);
    // Scrolled so the new card is in view, and the window followed.
    expect(bodyOf("backlog").scrollTop).toBeGreaterThan(0);
    expect(rendered("backlog").length).toBeLessThanOrEqual(MAX_RENDERED);
    expect(cardEl(next)!.getAttribute("aria-description")).toBe(
      `${Number(next.slice(1)) + 1} of 1000 in Backlog`,
    );
    // ArrowUp comes back one card, ArrowRight has no other column to go to.
    key(cardEl(next)!, "ArrowUp");
    expect(focusedCard()).toBe(last);
    key(cardEl(last)!, "ArrowRight");
    expect(focusedCard()).toBe(last);
  });

  it("arrow_left_and_right_move_to_the_same_index_in_the_next_column_with_cards", async () => {
    backend.seedThousand(4);
    backend.columns = backend.columns.map((c) =>
      c.id === "progress" ? { ...c, collapsed: true } : c,
    );
    await renderBoard();
    // 250 cards a column: k(4i) in Backlog, k(4i+2) in Review.
    scrollColumn("backlog", 200 * STEP);
    act(() => cardEl("k800")!.focus());
    // The collapsed In Progress strip has nothing to focus: skipped.
    key(cardEl("k800")!, "ArrowRight");
    expect(focusedCard()).toBe("k802");
    expect(cardEl("k802")!.getAttribute("aria-description")).toBe(
      "201 of 250 in Review",
    );
    key(cardEl("k802")!, "ArrowLeft");
    expect(focusedCard()).toBe("k800");
  });

  it("focus_survives_scrolling_the_focused_card_away_and_arrow_resumes_after_it", async () => {
    backend.seedThousand(1);
    await renderBoard();
    act(() => cardEl("k5")!.focus());
    // A wheel scroll far away: k5 stays mounted and keeps focus.
    scrollColumn("backlog", 500 * STEP);
    expect(rendered("backlog")).toContain("k500");
    expect(focusedCard()).toBe("k5");
    // And back again.
    scrollColumn("backlog", 300 * STEP);
    expect(focusedCard()).toBe("k5");
    key(cardEl("k5")!, "ArrowDown");
    expect(focusedCard()).toBe("k6");
    expect(bodyOf("backlog").scrollTop).toBeLessThanOrEqual(6 * STEP);
    expect(rendered("backlog")).not.toContain("k300");
    expect(rendered("backlog").length).toBeLessThanOrEqual(MAX_RENDERED);
  });

  it("lift_move_50_down_and_drop_lands_at_index_50", async () => {
    backend.seedThousand(1);
    applyMovesInStore();
    await renderBoard();
    await lift("k0");
    for (let i = 0; i < 50; i++) key(document, "ArrowDown");
    await flush();
    // The column scrolled to the card the drop line is next to.
    expect(rendered("backlog")).toContain("k50");
    expect(bodyOf("backlog").scrollTop).toBeGreaterThan(30 * STEP);
    expect(rendered("backlog").length).toBeLessThanOrEqual(MAX_RENDERED);
    key(document, " ");
    await flush();
    expect(commands("board_card_move").map(([, args]) => args)).toMatchObject([
      { cardId: "k0", columnId: "backlog", index: 50 },
    ]);
    await reload();
    const order = storeOrder("backlog");
    expect(order.indexOf("k0")).toBe(50);
    expect(order.slice(48, 52)).toEqual(["k49", "k50", "k0", "k51"]);
    expect(cardEl("k0")!.getAttribute("aria-description")).toBe(
      "51 of 1000 in Backlog",
    );
  });

  it("lift_move_up_from_deep_in_the_column_lands_at_the_real_index", async () => {
    backend.seedThousand(1);
    applyMovesInStore();
    await renderBoard();
    scrollColumn("backlog", 700 * STEP);
    await lift("k700");
    for (let i = 0; i < 60; i++) key(document, "ArrowUp");
    await flush();
    expect(liveRegion()).toBe("Card #701 is over Backlog, position 641 of 1000.");
    key(document, " ");
    await flush();
    expect(commands("board_card_move").map(([, args]) => args)).toMatchObject([
      { cardId: "k700", columnId: "backlog", index: 640 },
    ]);
  });

  it("keyboard_drag_announces_position_of_the_full_count", async () => {
    backend.seedThousand(1);
    await renderBoard();
    await lift("k0");
    expect(liveRegion()).toBe(
      "Picked up card #1, Card 1. Backlog, position 1 of 1000.",
    );
    for (let i = 0; i < 50; i++) key(document, "ArrowDown");
    await flush();
    expect(liveRegion()).toBe("Card #1 is over Backlog, position 51 of 1000.");
    key(document, "Escape");
    await flush();
    expect(liveRegion()).toBe(
      "Moving card #1 was cancelled. It is back in Backlog, position 1 of 1000.",
    );
    expect(commands("board_card_move")).toHaveLength(0);
    await lift("k0");
    for (let i = 0; i < 50; i++) key(document, "ArrowDown");
    await flush();
    key(document, " ");
    await flush();
    expect(liveRegion()).toBe("Dropped card #1 in Backlog, position 51 of 1000.");
  });

  it("tab_from_search_reaches_the_first_mounted_card", async () => {
    backend.seedThousand(1);
    await renderBoard();
    scrollColumn("backlog", 500 * STEP);
    // Tab order is document order over elements with tabIndex >= 0.
    const order = [
      ...container.querySelectorAll<HTMLElement>(
        "button, input, [tabindex]",
      ),
    ].filter(
      (el) =>
        el.tabIndex >= 0 &&
        !(el as HTMLButtonElement).disabled &&
        !el.closest("[inert]"),
    );
    const from = order.indexOf(searchBox());
    expect(from).toBeGreaterThanOrEqual(0);
    const after = order.slice(from + 1);
    const firstCard = after.find((el) => el.hasAttribute("data-kanban-card"));
    expect(firstCard?.getAttribute("data-kanban-card")).toBe(rendered("backlog")[0]);
    expect(rendered("backlog")[0]).toBe("k496");
    // The column header is a tab stop before it.
    const header = columnEl("backlog").querySelector<HTMLElement>(
      '[aria-roledescription="column"]',
    )!;
    expect(after.indexOf(header)).toBeGreaterThanOrEqual(0);
    expect(after.indexOf(header)).toBeLessThan(after.indexOf(firstCard!));
    // Every mounted card is a tab stop.
    for (const id of rendered("backlog")) {
      expect(cardEl(id)!.tabIndex, id).toBe(0);
    }
  });

  it("shift_range_from_an_unmounted_anchor_selects_the_display_slice", async () => {
    backend.seedThousand(1);
    await renderBoard();
    const face = (id: string) =>
      container.querySelector(`[data-board-card="${id}"]`)!;
    await click(face("k2"), MOD);
    scrollColumn("backlog", 600 * STEP);
    expect(cardEl("k2")).toBeNull();
    await click(face("k600"), { shiftKey: true });
    expect(
      container.querySelector("[data-selection-count]")?.textContent,
    ).toBe("599 selected");
    expect(cardEl("k600")!.getAttribute("aria-pressed")).toBe("true");
    expect(cardEl("k598")!.getAttribute("aria-pressed")).toBe("true");
    expect(cardEl("k601")!.getAttribute("aria-pressed")).toBe("false");
  });

  it("arrow_moves_keep_the_lift_in_an_active_window", async () => {
    backend.seedThousand(1);
    await renderBoard();
    await lift("k3");
    for (const k of ["ArrowDown", "ArrowDown", "ArrowUp"]) {
      key(document.activeElement ?? document, k);
      await flush();
      expect(cardEl("k3")!.hasAttribute("data-dragging")).toBe(true);
    }
    // Focus stayed on the lifted card; the arrows moved the drop line.
    expect(focusedCard()).toBe("k3");
    expect(liveRegion()).toBe("Card #4 is over Backlog, position 5 of 1000.");
    key(document, "Escape");
    await flush();
    expect(cardEl("k3")!.hasAttribute("data-dragging")).toBe(false);
  });

  it("arrow_keys_do_nothing_while_typing_or_while_a_card_is_lifted", async () => {
    backend.seedThousand(1);
    await renderBoard();
    // Typing: the search box keeps its arrows.
    act(() => searchBox().focus());
    key(searchBox(), "ArrowDown");
    expect(document.activeElement).toBe(searchBox());
    // A control inside a card keeps its arrows too.
    const inner = cardEl("k1")!.querySelector<HTMLElement>("button");
    if (inner) {
      act(() => inner.focus());
      key(inner, "ArrowDown");
      expect(document.activeElement).toBe(inner);
    }
    // Lifted: dnd-kit owns the arrows, focus never moves to k4.
    await lift("k3");
    key(cardEl("k3")!, "ArrowDown");
    await flush();
    expect(focusedCard()).toBe("k3");
    expect(cardEl("k3")!.hasAttribute("data-dragging")).toBe(true);
    key(document, "Escape");
    await flush();
    // Modified arrows are left to the system.
    act(() => cardEl("k3")!.focus());
    key(cardEl("k3")!, "ArrowDown", { altKey: true });
    key(cardEl("k3")!, "ArrowDown", { shiftKey: true });
    expect(focusedCard()).toBe("k3");
    key(cardEl("k3")!, "ArrowDown");
    expect(focusedCard()).toBe("k4");
  });

  it("click_elsewhere_in_an_active_window_cancels_the_lift", async () => {
    backend.seedThousand(1);
    await renderBoard();
    await lift("k3");
    key(document, "ArrowDown");
    await flush();
    act(() => {
      searchBox().dispatchEvent(
        new PointerEvent("pointerdown", { bubbles: true, cancelable: true }),
      );
    });
    await flush();
    expect(cardEl("k3")!.hasAttribute("data-dragging")).toBe(false);
    expect(liveRegion()).toBe(
      "Moving card #4 was cancelled. It is back in Backlog, position 4 of 1000.",
    );
    // The cancelled lift no longer owns Space.
    key(document, " ");
    await flush();
    expect(commands("board_card_move")).toHaveLength(0);
  });

  it("space_drop_never_opens_the_drawer", async () => {
    backend.seedThousand(1);
    await renderBoard();
    for (const drop of [" ", "Enter"]) {
      await lift("k3");
      key(document, "ArrowDown");
      await flush();
      key(cardEl("k3")!, drop);
      await flush();
      expect(cardEl("k3")!.hasAttribute("data-dragging"), drop).toBe(false);
      expect(container.querySelector("[data-card-drawer]"), drop).toBeNull();
    }
    expect(commands("board_card_move")).toHaveLength(2);
  });

  it("shortcuts_act_on_a_card_focused_by_arrow_keys", async () => {
    backend.seedThousand(1);
    await renderBoard();
    const last = rendered("backlog").at(-1)!;
    act(() => cardEl(last)!.focus());
    key(cardEl(last)!, "ArrowDown");
    const next = focusedCard()!;
    expect(next).not.toBe(last);
    key(cardEl(next)!, "e");
    await flush();
    const drawer = container.querySelector("[data-card-drawer]");
    expect(drawer).not.toBeNull();
    expect(drawer!.textContent).toContain(`Card #${Number(next.slice(1)) + 1}`);
  });

  it("aria_pressed_is_absent_without_a_selection", async () => {
    backend.seedThousand(1);
    await renderBoard();
    for (const id of rendered("backlog")) {
      expect(cardEl(id)!.hasAttribute("aria-pressed"), id).toBe(false);
    }
    await click(container.querySelector('[data-board-card="k1"]')!, MOD);
    expect(cardEl("k1")!.getAttribute("aria-pressed")).toBe("true");
    expect(cardEl("k2")!.getAttribute("aria-pressed")).toBe("false");
    // Cleared with Escape: the attribute goes away again.
    act(() => cardEl("k1")!.focus());
    key(cardEl("k1")!, "Escape");
    await flush();
    expect(cardEl("k1")!.hasAttribute("aria-pressed")).toBe(false);
    expect(cardEl("k2")!.hasAttribute("aria-pressed")).toBe(false);
  });

  it("focused_list_and_table_rows_stay_mounted_when_scrolled_away", async () => {
    backend.seedThousand(1);
    await renderBoard();

    await pick("Layout", "List");
    const list = container.querySelector<HTMLElement>("[data-board-list]")!;
    const listRow = (id: string) =>
      list.querySelector<HTMLElement>(`[data-card-id="${id}"]`);
    act(() => listRow("k3")!.focus());
    scrollEl(list, 600 * ROW_HEIGHT);
    expect(listRow("k600")).not.toBeNull();
    expect(listRow("k3")).not.toBeNull();
    expect(document.activeElement).toBe(listRow("k3"));
    // Kept at its own place: under the group header, the fourth card row.
    expect(parseFloat(listRow("k3")!.style.top)).toBe(4 * ROW_HEIGHT);
    expect(list.querySelectorAll("[data-card-id]").length).toBeLessThan(80);
    // Focus elsewhere: the row is windowed out again.
    act(() => listRow("k3")!.blur());
    scrollEl(list, 600 * ROW_HEIGHT + 1);
    expect(listRow("k3")).toBeNull();

    await pick("Layout", "Table");
    const table = container.querySelector<HTMLElement>("[data-board-table]")!;
    const tableRow = (id: string) =>
      table.querySelector<HTMLElement>(`[data-table-row="${id}"]`);
    act(() => tableRow("k2")!.focus());
    scrollEl(table, 700 * ROW_HEIGHT);
    expect(tableRow("k700")).not.toBeNull();
    expect(tableRow("k2")).not.toBeNull();
    expect(document.activeElement).toBe(tableRow("k2"));
    expect(tableRow("k2")!.getAttribute("aria-rowindex")).toBe("4");
    expect(parseFloat(tableRow("k2")!.style.top)).toBe(2 * ROW_HEIGHT);
    expect(table.querySelectorAll("[data-table-row]").length).toBeLessThan(80);
    act(() => tableRow("k2")!.blur());
    scrollEl(table, 700 * ROW_HEIGHT + 1);
    expect(tableRow("k2")).toBeNull();
  });
});
