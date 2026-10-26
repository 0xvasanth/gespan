// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { IS_MAC } from "../../../../platform/tauri/platform";
import type { BoardCard, BoardColumn, BoardSettings } from "../../model/board";
import { DEFAULT_DISPLAY } from "../../model/display";
import {
  LANE_HEADER_ESTIMATE,
  laneBandEstimate,
  laneCellKey,
  laneOrder,
  laneRows,
  type Lane,
} from "../../model/swimlanes";
import { CARD_ESTIMATE } from "../../model/windowing";
import { BoardView } from "../BoardView";
import {
  card,
  createTestBackend,
  extra,
  flush,
  type TestBackend,
} from "../testBackend";
import { LANE_TOP_OFFSET, Swimlanes } from "./Swimlanes";

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

const MAX_RENDERED = 60;
const MOD: MouseEventInit = IS_MAC ? { metaKey: true } : { ctrlKey: true };
const FULL_BAND = laneBandEstimate(4, CARD_ESTIMATE.comfortable);
const EMPTY_BAND = laneBandEstimate(0, CARD_ESTIMATE.comfortable);

const SETTINGS: BoardSettings = {
  runWith: true,
  writeBack: true,
  ticketStatusChanges: true,
  autoMoveJson: null,
  icon: null,
  color: null,
  defaultViewId: null,
  displayJson: JSON.stringify({ ...DEFAULT_DISPLAY, swimlane: "priority" }),
  priorityJson: null,
};

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
  // Lanes by priority: Urgent, High, Medium, Low, None.
  backend.handlers.board_settings = () => SETTINGS;
  backend.handlers.board_column_set_display = () => undefined;
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

/**
 * 1,000 cards over Backlog (even k) and In Progress (odd k): k0–k399 are
 * Urgent (200 a column, 50 bands), the rest have no priority (300 a column).
 */
function seedLanes() {
  backend.seedThousand(2);
  backend.extras = Object.fromEntries(
    backend.cards.map((c, i) => [c.id, extra({ priority: i < 400 ? 4 : 0 })]),
  );
}

const commands = (cmd: string) =>
  vi.mocked(invoke).mock.calls.filter(([name]) => name === cmd);

const scroller = () =>
  container.querySelector<HTMLElement>("[data-swimlanes]")!;
const laneEl = (name: string) =>
  container.querySelector<HTMLElement>(`section[aria-label="Lane ${name}"]`);
const cardEl = (id: string) =>
  container.querySelector<HTMLElement>(`[data-kanban-card="${id}"]`);
const focusedCard = () => {
  const el = document.activeElement as HTMLElement | null;
  const id = el?.getAttribute("data-kanban-card");
  return id ? `${el!.closest("section")!.getAttribute("aria-label")}/${id}` : null;
};
/** Mounted cards in column `id`, in every lane. */
const renderedIn = (id: string) =>
  [...container.querySelectorAll<HTMLElement>("[data-lane-band]")]
    .filter((band) => band.getAttribute("data-lane-column") === id)
    .flatMap((band) => [...band.querySelectorAll("[data-kanban-card]")]);
const liveRegion = () =>
  document.querySelector("[id^='DndLiveRegion']")?.textContent ?? "";
const toastText = () =>
  document.querySelector("[data-board-undo-toast]")?.textContent ?? "";
const undoButton = () =>
  [
    ...document.querySelectorAll<HTMLButtonElement>(
      "[data-board-undo-toast] button",
    ),
  ].find((button) => button.textContent === "Undo");

function scrollTo(top: number) {
  act(() => {
    scroller().scrollTop = top;
    scroller().dispatchEvent(new Event("scroll"));
  });
}

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

/** Focuses `id` and presses Space: the keyboard sensor binds on the next tick. */
async function lift(id: string) {
  const el = cardEl(id)!;
  act(() => el.focus());
  key(el, " ");
  await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
  expect(cardEl(id)!.hasAttribute("data-dragging")).toBe(true);
}

/** Plays moves and priority writes into the fake store, as the real one does. */
function applyWritesInStore() {
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
    order(moving.columnId).forEach((c, i) =>
      placed.set(c.id, { ...c, position: i }),
    );
    backend.cards = backend.cards.map((c) => placed.get(c.id) ?? c);
    return undefined;
  };
  backend.handlers.board_card_set_priority = (args) => {
    const id = String(args.cardId);
    backend.extras = {
      ...backend.extras,
      [id]: { ...(backend.extras[id] ?? extra()), priority: Number(args.priority) },
    };
    return undefined;
  };
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
  await act(() => new Promise((resolve) => setTimeout(resolve, 200)));
  await flush();
  await flush();
}

describe("windowed swimlanes", () => {
  it("lane_order_matches_the_native_rows_for_the_same_board", () => {
    // The fixed board both apps test (Native: board/tests/lanes_scale.rs):
    // lanes Urgent, Medium, None; columns Todo, Done. Urgent holds 5 Todo
    // cards and 2 Done cards, Medium none, None 8 Done cards.
    const lanes: Lane[] = [
      { key: "p4", label: "Urgent", cardIds: ["u1", "u2", "u3", "u4", "u5", "u6", "u7"] },
      { key: "p2", label: "Medium", cardIds: [] },
      {
        key: "p0",
        label: "None",
        cardIds: ["n1", "n2", "n3", "n4", "n5", "n6", "n7", "n8"],
      },
    ];
    const cells = new Map<string, readonly string[]>([
      [laneCellKey("p4", "todo"), ["u1", "u2", "u3", "u4", "u5"]],
      [laneCellKey("p4", "done"), ["u6", "u7"]],
      [laneCellKey("p0", "done"), ["n1", "n2", "n3", "n4", "n5", "n6", "n7", "n8"]],
    ]);
    expect(laneRows(lanes, cells)).toEqual([
      { kind: "header", lane: "p4" },
      { kind: "band", lane: "p4", band: 0 },
      // A fifth card opens a second band.
      { kind: "band", lane: "p4", band: 1 },
      // No cards: the header and one empty band.
      { kind: "header", lane: "p2" },
      { kind: "band", lane: "p2", band: 0 },
      { kind: "header", lane: "p0" },
      { kind: "band", lane: "p0", band: 0 },
      // Exactly 8 cards fill two bands, no third.
      { kind: "band", lane: "p0", band: 1 },
    ]);
    // Lane by lane, then column by column, then by position.
    expect(laneOrder(lanes, cells, ["todo", "done"])).toEqual([
      "u1", "u2", "u3", "u4", "u5", "u6", "u7",
      "n1", "n2", "n3", "n4", "n5", "n6", "n7", "n8",
    ]);
    // A card in two lanes counts once, at its first place.
    const twice = new Map(cells).set(laneCellKey("p2", "todo"), ["u6", "m1"]);
    expect(laneOrder(lanes, twice, ["todo", "done"]).slice(5, 9)).toEqual([
      "u6", "u7", "m1", "n1",
    ]);
  });

  it("swimlanes_with_1000_cards_render_at_most_60_per_column", async () => {
    seedLanes();
    await renderBoard();
    expect(container.querySelector("[data-kanban-board]")).toBeNull();
    for (const id of ["backlog", "progress"]) {
      expect(renderedIn(id).length).toBeGreaterThan(0);
      expect(renderedIn(id).length).toBeLessThanOrEqual(MAX_RENDERED);
    }
    expect(cardEl("k0")).not.toBeNull();
    // Every lane header stays mounted, with the lane's full count.
    for (const [name, count] of [
      ["Urgent", "400"],
      ["High", "0"],
      ["Medium", "0"],
      ["Low", "0"],
      ["None", "600"],
    ]) {
      expect(laneEl(name)?.querySelector("[data-lane-count]")?.textContent).toBe(
        count,
      );
    }
    // Deep in the None lane: still bounded, and the top cards are gone.
    scrollTo(LANE_TOP_OFFSET + 5 * LANE_HEADER_ESTIMATE + 50 * FULL_BAND + 3 * EMPTY_BAND + 40 * FULL_BAND);
    for (const id of ["backlog", "progress"]) {
      expect(renderedIn(id).length).toBeGreaterThan(0);
      expect(renderedIn(id).length).toBeLessThanOrEqual(MAX_RENDERED);
    }
    expect(cardEl("k0")).toBeNull();
    // None band 40 holds cells [160, 164): k720.. in Backlog.
    expect(cardEl("k720")).not.toBeNull();
    expect(cardEl("k720")!.getAttribute("aria-description")).toBe(
      "161 of 300 in None, Backlog",
    );
    // The spacers keep the scroller its full height.
    const gaps = [...scroller().querySelectorAll<HTMLElement>("[data-window-gap]")];
    expect(gaps.length).toBeGreaterThan(0);
  });

  it("a_lane_with_no_cards_shows_its_header_and_an_empty_band", async () => {
    seedLanes();
    await renderBoard();
    // Scrolled to it, below the Urgent lane's 50 bands.
    scrollTo(LANE_TOP_OFFSET + LANE_HEADER_ESTIMATE + 49 * FULL_BAND);
    const high = laneEl("High")!;
    expect(high.querySelectorAll("[data-lane-row]")).toHaveLength(2);
    expect(high.querySelector("[data-lane-header]")).not.toBeNull();
    // One band, with an empty drop area for each of the four columns.
    const bands = [...high.querySelectorAll("[data-lane-band]")];
    expect(bands.map((band) => band.getAttribute("data-lane-band"))).toEqual([
      "p3|backlog|0",
      "p3|progress|0",
      "p3|review|0",
      "p3|done|0",
    ]);
    expect(high.querySelectorAll("[data-kanban-card]")).toHaveLength(0);
    // And a header drop cell per column, for keyboard drags.
    expect(high.querySelectorAll("[data-lane-cell]")).toHaveLength(4);
  });

  it("keyboard_drag_down_reaches_a_lane_whose_cards_are_not_mounted", async () => {
    seedLanes();
    applyWritesInStore();
    await renderBoard();
    expect(cardEl("k401")).toBeNull();
    await lift("k0");
    expect(liveRegion()).toBe(
      "Card #1, Card 1 is over lane Urgent, column Backlog, position 1 of 200.",
    );
    // Urgent → High → Medium → Low → None: four lanes down, past 50 bands.
    for (let i = 0; i < 4; i++) key(document, "ArrowDown");
    await flush();
    expect(liveRegion()).toBe(
      "Card #1, Card 1 is over lane None, column Backlog, position 301 of 301.",
    );
    // Scrolled down to the None lane's header.
    expect(scroller().scrollTop).toBeGreaterThan(48 * FULL_BAND);
    key(document, "ArrowRight");
    await flush();
    expect(liveRegion()).toBe(
      "Card #1, Card 1 is over lane None, column In Progress, position 301 of 301.",
    );
    expect(cardEl("k0")!.hasAttribute("data-dragging")).toBe(true);
    key(document, " ");
    await flush();
    expect(liveRegion()).toBe(
      "Dropped card #1 in lane None, column In Progress, position 301 of 301.",
    );
    // Appended after every In Progress card, and the lane's priority set.
    expect(commands("board_card_move").map(([, args]) => args)).toMatchObject([
      { cardId: "k0", columnId: "progress", index: 500 },
    ]);
    expect(commands("board_card_set_priority").map(([, args]) => args)).toEqual([
      { cardId: "k0", priority: 0 },
    ]);
    // A drop that also set the priority has no Undo (C5-o).
    expect(toastText()).toContain("Moved #1 to In Progress");
    expect(undoButton()).toBeUndefined();
    await reload();
    expect(backend.cards.find((c) => c.id === "k0")).toMatchObject({
      columnId: "progress",
      position: 500,
    });
    expect(backend.extras.k0.priority).toBe(0);
    expect(laneEl("None")!.querySelector("[data-lane-count]")?.textContent).toBe("601");
    expect(laneEl("Urgent")!.querySelector("[data-lane-count]")?.textContent).toBe("399");
  });

  it("shift_range_in_swimlanes_follows_lane_major_order", async () => {
    seedLanes();
    await renderBoard();
    const face = (id: string) =>
      container.querySelector(`[data-board-card="${id}"]`)!;
    await click(face("k2"), MOD);
    // Down to the top of the None lane: k2 is far above and unmounted.
    scrollTo(LANE_TOP_OFFSET + 4 * LANE_HEADER_ESTIMATE + 50 * FULL_BAND + 3 * EMPTY_BAND);
    expect(cardEl("k2")).toBeNull();
    await click(face("k402"), { shiftKey: true });
    // Lane-major: k2..k398 of Urgent/Backlog (199), all of Urgent/In
    // Progress (200), then k400 and k402 of None/Backlog. Column-major would
    // have been k2..k402 of Backlog alone (201).
    expect(
      container.querySelector("[data-selection-count]")?.textContent,
    ).toBe("401 selected");
    expect(cardEl("k402")!.getAttribute("aria-pressed")).toBe("true");
    expect(cardEl("k400")!.getAttribute("aria-pressed")).toBe("true");
    expect(cardEl("k404")!.getAttribute("aria-pressed")).toBe("false");
    // None/In Progress comes after None/Backlog in the order: not in range.
    expect(cardEl("k401")!.getAttribute("aria-pressed")).toBe("false");
  });

  it("collapsed_columns_draw_as_strips_in_swimlanes_and_take_drops", async () => {
    // c1 High/Backlog, c2 Low/Backlog, c3 None/In Progress, c4 Medium/Done.
    backend.columns = backend.columns.map((c) =>
      c.id === "progress" ? { ...c, collapsed: true } : c,
    );
    await renderBoard();
    const head = container.querySelector<HTMLElement>(
      '[data-lane-column-head="progress"]',
    )!;
    expect(head.hasAttribute("data-column-collapsed")).toBe(true);
    expect(head.className).toContain("w-9");
    expect(head.querySelector("[data-column-count]")?.textContent).toBe("1");
    // Every lane draws the column as a strip, and the strip mounts no cards.
    for (const lane of ["p4", "p3", "p2", "p1", "p0"]) {
      const cell = container.querySelector(`[data-lane-cell="${lane}|progress"]`)!;
      expect(cell.hasAttribute("data-column-collapsed")).toBe(true);
      expect(cell.className).toContain("w-9");
    }
    expect(cardEl("c3")).toBeNull();
    expect(laneEl("None")!.querySelector("[data-lane-count]")?.textContent).toBe("1");

    // A keyboard drag onto the strip lands at the end of the column.
    await lift("c1");
    key(document, "ArrowRight");
    await flush();
    expect(liveRegion()).toBe(
      "Card #1, Fix login is over lane High, column In Progress, position 1 of 1.",
    );
    key(document, " ");
    await flush();
    expect(commands("board_card_move").map(([, args]) => args)).toMatchObject([
      { cardId: "c1", columnId: "progress", index: 1 },
    ]);
    // Same lane: the priority stays, so the move keeps its Undo.
    expect(commands("board_card_set_priority")).toHaveLength(0);
    expect(undoButton()).toBeDefined();

    // The strip's button expands the column.
    await click(head.querySelector('button[aria-label="Expand In Progress"]')!);
    expect(commands("board_column_set_display").map(([, args]) => args)).toMatchObject([
      { columnId: "progress", collapsed: false },
    ]);
  });

  it("arrow_keys_move_focus_across_lanes_and_columns_and_mount_unmounted_cards", async () => {
    // Urgent: 8 Backlog cards. High: 400 Review cards (100 tall bands) and
    // nothing in Backlog. None: 8 Backlog cards and 3 Done cards.
    const cards: BoardCard[] = [];
    const extras: TestBackend["extras"] = {};
    const add = (id: string, columnId: string, position: number, priority: number) => {
      cards.push(card(id, columnId, position, cards.length + 1, `Card ${id}`));
      extras[id] = extra({ priority });
    };
    for (let i = 0; i < 8; i++) add(`u${i}`, "backlog", i, 4);
    for (let i = 0; i < 400; i++) add(`h${i}`, "review", i, 3);
    for (let i = 0; i < 8; i++) add(`n${i}`, "backlog", 8 + i, 0);
    for (let i = 0; i < 3; i++) add(`d${i}`, "done", i, 0);
    backend.cards = cards;
    backend.extras = extras;
    await renderBoard();
    expect(cardEl("n0")).toBeNull();

    act(() => cardEl("u6")!.focus());
    key(cardEl("u6")!, "ArrowDown");
    expect(focusedCard()).toBe("Lane Urgent/u7");
    // Past the last Urgent card, ↓ crosses the High lane (no Backlog cards,
    // 100 bands tall) into None, scrolling its first card in and mounting it.
    key(cardEl("u7")!, "ArrowDown");
    expect(focusedCard()).toBe("Lane None/n0");
    expect(scroller().scrollTop).toBeGreaterThan(90 * FULL_BAND);
    expect(cardEl("u0")).toBeNull();
    expect(cardEl("n0")!.getAttribute("aria-description")).toBe(
      "1 of 8 in None, Backlog",
    );
    for (const id of ["backlog", "review"]) {
      expect(renderedIn(id).length).toBeLessThanOrEqual(MAX_RENDERED);
    }
    // ↑ goes back to the last Urgent card, far above again.
    key(cardEl("n0")!, "ArrowUp");
    expect(focusedCard()).toBe("Lane Urgent/u7");
    key(cardEl("u7")!, "ArrowDown");
    // ←/→ stay in the lane: n5 → the nearest column with None cards (Done),
    // at the same index clamped to its last card, and back.
    for (let i = 0; i < 5; i++) {
      const at = document.activeElement as HTMLElement;
      key(at, "ArrowDown");
    }
    expect(focusedCard()).toBe("Lane None/n5");
    key(cardEl("n5")!, "ArrowRight");
    expect(focusedCard()).toBe("Lane None/d2");
    key(cardEl("d2")!, "ArrowLeft");
    expect(focusedCard()).toBe("Lane None/n2");
    // Nothing to the left of Backlog.
    key(cardEl("n2")!, "ArrowLeft");
    expect(focusedCard()).toBe("Lane None/n2");
  });

  it("focused_lane_card_stays_mounted_when_scrolled_away", async () => {
    seedLanes();
    await renderBoard();
    act(() => cardEl("k4")!.focus());
    scrollTo(LANE_TOP_OFFSET + 5 * LANE_HEADER_ESTIMATE + 50 * FULL_BAND + 3 * EMPTY_BAND + 40 * FULL_BAND);
    // k4's band stays (k0–k6); the next band went with the scroll.
    expect(cardEl("k8")).toBeNull();
    expect(focusedCard()).toBe("Lane Urgent/k4");
    key(cardEl("k4")!, "ArrowDown");
    expect(focusedCard()).toBe("Lane Urgent/k6");
    expect(scroller().scrollTop).toBeLessThan(LANE_TOP_OFFSET + 4 * FULL_BAND);
  });
});

describe("swimlane focus across reloads and filters", () => {
  it("a_reload_that_moves_the_focused_card_across_a_band_edge_keeps_focus_on_it", async () => {
    seedLanes();
    await renderBoard();
    // Urgent/Backlog: k0, k2, k4, k6 in band 0; k8 opens band 1.
    act(() => cardEl("k8")!.focus());
    expect(focusedCard()).toBe("Lane Urgent/k8");
    // The other app moves k0 to Review: k8 is now index 3, in band 0.
    backend.cards = backend.cards.map((c) =>
      c.id === "k0" ? { ...c, columnId: "review", position: 0 } : c,
    );
    await reload();
    expect(cardEl("k8")!.getAttribute("aria-description")).toBe(
      "4 of 199 in Urgent, Backlog",
    );
    expect(focusedCard()).toBe("Lane Urgent/k8");
    key(cardEl("k8")!, "ArrowDown");
    expect(focusedCard()).toBe("Lane Urgent/k10");
  });

  it("a_focused_lane_card_hidden_by_a_filter_moves_focus_to_the_nearest_card_in_its_cell", async () => {
    // Medium/Backlog holds m0, m1, m2; High/Backlog holds h0 alone.
    const add = (id: string, position: number, priority: number) => {
      backend.cards.push(card(id, "backlog", position, backend.cards.length + 1, `Card ${id}`));
      backend.extras[id] = extra({ priority });
    };
    backend.cards = [];
    backend.extras = {};
    add("m0", 0, 2);
    add("m1", 1, 2);
    add("m2", 2, 2);
    add("h0", 3, 3);
    await renderBoard();
    await search("card");
    const hide = async (id: string) => {
      backend.cards = backend.cards.map((c) =>
        c.id === id ? { ...c, title: "Hidden by the search" } : c,
      );
      await reload();
    };
    act(() => cardEl("m1")!.focus());
    await hide("m1");
    expect(cardEl("m1")).toBeNull();
    // The card that took its place.
    expect(focusedCard()).toBe("Lane Medium/m2");
    await hide("m2");
    // The last card of the cell: the one before it.
    expect(focusedCard()).toBe("Lane Medium/m0");
    // The only card of its cell: the lane's header.
    act(() => cardEl("h0")!.focus());
    await hide("h0");
    expect(document.activeElement?.getAttribute("data-lane-label")).toBe("p3");
    // Focus somewhere else is left alone.
    act(() => cardEl("m0")!.focus());
    const box = container.querySelector<HTMLInputElement>(
      'input[aria-label="Search cards"]',
    )!;
    act(() => box.focus());
    await hide("m0");
    expect(document.activeElement).toBe(box);
  });

  it("lane_rows_start_where_the_measured_column_header_ends", async () => {
    seedLanes();
    await renderBoard();
    // Lay the scroller out: 640px tall, with the first lane 80px below its
    // top (not the default 52), as a taller column header would put it.
    Object.defineProperty(scroller(), "clientHeight", { value: 640, configurable: true });
    const rect = (top: number) =>
      ({ x: 0, y: top, left: 0, top, width: 0, height: 0, right: 0, bottom: top, toJSON() {} }) as DOMRect;
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(
      function (this: Element) {
        if (this.matches("section[data-lane]") && this === scroller().querySelector("section[data-lane]")) {
          return rect(80 - scroller().scrollTop);
        }
        return rect(0);
      },
    );
    act(() => cardEl("k4")!.focus());
    scrollTo(30 * FULL_BAND);
    // k6 is band 0 (row 1, 36px down): revealing it scrolls to its top,
    // counted from where the rows really start.
    key(cardEl("k4")!, "ArrowDown");
    expect(focusedCard()).toBe("Lane Urgent/k6");
    expect(scroller().scrollTop).toBe(80 + LANE_HEADER_ESTIMATE);
  });
});

// A drop with the pointer, on the component alone. happy-dom has no layout:
// this stacks rows 400px apart (by their row index) and columns 300px apart.
describe("pointer drops in lane bands", () => {
  const column = (id: string, name: string, position: number): BoardColumn => ({
    id,
    boardId: "b1",
    name,
    position,
    role: null,
    color: null,
    wipLimit: null,
    collapsed: false,
  });
  const columns = [column("todo", "Todo", 0), column("done", "Done", 1)];
  const cards = [
    ...Array.from({ length: 9 }, (_, i) => card(`a${i}`, "todo", i, i + 1, `A ${i}`)),
    card("d0", "done", 0, 20, "D 0"),
    card("b0", "todo", 9, 30, "B 0"),
  ];
  const lanes: Lane[] = [
    {
      key: "A",
      label: "Alpha",
      cardIds: [...Array.from({ length: 9 }, (_, i) => `a${i}`), "d0"],
    },
    { key: "B", label: "Beta", cardIds: ["b0"] },
  ];
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

  function stubGrid(cols: readonly BoardColumn[] = columns) {
    const overlay = { rect: rect(0, 0, 0, 0) };
    const layout = function (this: Element): DOMRect {
      const row = this.closest("[data-lane-row]");
      if (!row) return overlay.rect;
      const y = Number(row.getAttribute("data-lane-row")) * 400;
      const segment = this.closest("[data-lane-band], [data-lane-cell]");
      // A row itself measures small, so the window keeps every row mounted.
      if (!segment) return rect(0, y, 1000, 60);
      const x =
        cols.findIndex((c) => c.id === segment.getAttribute("data-lane-column")) *
        300;
      const slot = this.closest("[data-kanban-card]");
      if (!slot) return rect(x, y, 280, 400);
      const at = [...segment.querySelectorAll("[data-kanban-card]")].indexOf(slot);
      return rect(x + 6, y + 6 + at * 70, 268, 60);
    };
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(layout);
    return { rectOf: (el: Element) => layout.call(el), overlay };
  }

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

  async function dragOnto(
    layout: ReturnType<typeof stubGrid>,
    from: string,
    band: string,
  ) {
    const source = cardEl(from)!;
    const start = layout.rectOf(source);
    layout.overlay.rect = start;
    const a = center(start);
    pointer("pointerdown", source, a.x, a.y);
    pointer("pointermove", document, a.x + 10, a.y + 10);
    await flush();
    const target = container.querySelector(`[data-lane-band="${band}"]`)!;
    const b = center(layout.rectOf(target));
    pointer("pointermove", document, b.x, b.y);
    await flush();
    pointer("pointerup", document, b.x, b.y);
    await flush();
    await act(() => new Promise((resolve) => setTimeout(resolve, 60)));
  }

  it("drop_on_any_band_of_a_cell_appends_to_that_cell", async () => {
    const onDrop = vi.fn();
    const layout = stubGrid();
    act(() =>
      root.render(
        createElement(Swimlanes, {
          columns,
          cards,
          lanes,
          renderCard: (c: BoardCard) => createElement("span", null, c.title),
          renderColumnHeader: (c: BoardColumn) => createElement("span", null, c.name),
          onDrop,
          // Compact cards keep both lanes in the first view.
          density: "compact",
        }),
      ),
    );
    // Alpha has three bands (9 Todo cards); its Done cell holds one card,
    // in band 0, so bands 1 and 2 of that cell are empty drop areas.
    expect(
      [...container.querySelectorAll("[data-lane-band]")].map((el) =>
        el.getAttribute("data-lane-band"),
      ),
    ).toEqual([
      "A|todo|0",
      "A|done|0",
      "A|todo|1",
      "A|done|1",
      "A|todo|2",
      "A|done|2",
      "B|todo|0",
      "B|done|0",
    ]);
    await dragOnto(layout, "b0", "A|done|2");
    expect(onDrop).toHaveBeenLastCalledWith("b0", "A", "done");
    await dragOnto(layout, "b0", "A|done|0");
    expect(onDrop).toHaveBeenLastCalledWith("b0", "A", "done");
    // A band of the card's own lane in another column: that cell.
    await dragOnto(layout, "a8", "A|done|1");
    expect(onDrop).toHaveBeenLastCalledWith("a8", "A", "done");
    expect(onDrop).toHaveBeenCalledTimes(3);
  });

  it("pointer_drop_on_a_collapsed_column_strip_reports_that_column", async () => {
    const withStrip = [...columns, { ...column("later", "Later", 2), collapsed: true }];
    const onDrop = vi.fn();
    const layout = stubGrid(withStrip);
    act(() =>
      root.render(
        createElement(Swimlanes, {
          columns: withStrip,
          cards: [...cards, card("l0", "later", 0, 40, "L 0")],
          lanes: [lanes[0], { ...lanes[1], cardIds: ["b0", "l0"] }],
          renderCard: (c: BoardCard) => createElement("span", null, c.title),
          renderColumnHeader: (c: BoardColumn) => createElement("span", null, c.name),
          onDrop,
          density: "compact",
        }),
      ),
    );
    // The strip mounts none of its cards, in any band.
    expect(cardEl("l0")).toBeNull();
    const strip = container.querySelector('[data-lane-band="A|later|1"]')!;
    expect(strip.hasAttribute("data-column-collapsed")).toBe(true);
    expect(strip.className).toContain("w-9");
    await dragOnto(layout, "b0", "A|later|1");
    expect(onDrop).toHaveBeenLastCalledWith("b0", "A", "later");
    // From a lane card onto its own lane's strip.
    await dragOnto(layout, "a3", "A|later|0");
    expect(onDrop).toHaveBeenLastCalledWith("a3", "A", "later");
    expect(onDrop).toHaveBeenCalledTimes(2);
  });
});
