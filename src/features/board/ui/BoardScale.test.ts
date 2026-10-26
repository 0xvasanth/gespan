// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { IS_MAC } from "../../../platform/tauri/platform";
import type { CardLinkRow, ViewSpec } from "../model/board";
import { CARD_ESTIMATE, CARD_GAP } from "../model/windowing";
import { BoardView, SEARCH_DEBOUNCE_MS } from "./BoardView";
import type { BoardLinkTarget } from "./boardFormat";
import {
  createTestBackend,
  DAY,
  extra,
  flush,
  type TestBackend,
} from "./testBackend";

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
vi.mock("../../../app/shell/WindowControls", () => ({
  WindowControls: () => null,
}));

// Budgets (happy-dom; about 5-10x slower than WebKit). Ceilings the
// unwindowed board fails; never loosen them here.
const OPEN_BUDGET_MS = 2_000;
const FILTER_BUDGET_MS = 750;
const RUNS = 3;
const MAX_PER_COLUMN = 60;
const BULK_DELETE_BUDGET_MS = 100;

let backend: TestBackend;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const stored = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => stored.set(key, value),
    removeItem: (key: string) => stored.delete(key),
    clear: () => stored.clear(),
  });
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const props = (): ComponentProps<typeof BoardView> => ({
  onClose: vi.fn(),
  linkTargets: [],
  onOpenLinkedWork: vi.fn(() => Promise.resolve()),
  lookupLinkTargets: vi.fn(async () => []),
  projects: [],
});

/** Settles promises until `done` holds (bounded, so a failure can't hang). */
async function until(done: () => boolean) {
  for (let i = 0; i < 50 && !done(); i += 1) await flush();
  expect(done()).toBe(true);
}

/** One open and one search on a fresh 1,000-card board; times in ms. */
async function measure(): Promise<{ open: number; filter: number }> {
  events.listeners.clear();
  backend = createTestBackend(events.listeners);
  backend.seedThousand(4);
  // Real faces: every card has a priority, labels, a due date and subtasks.
  backend.extras = Object.fromEntries(
    backend.cards.map((c, i) => [
      c.id,
      extra({
        priority: i % 5,
        labels: i % 3 ? ["l1"] : ["l1", "l2"],
        dueAt: Date.now() + (i % 30) * DAY,
        subtasksDone: i % 4,
        subtasksTotal: 4,
        comments: i % 7,
      }),
    ]),
  );
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const painted = () =>
    [...container.querySelectorAll("[data-kanban-card]")].map(
      (el) => el.getAttribute("data-kanban-card")!,
    );
  try {
    const openStart = performance.now();
    await act(async () => root.render(createElement(BoardView, props())));
    await until(() => painted().includes("k0"));
    const open = performance.now() - openStart;
    // The scale test guards windowing too: 250 cards per column, at most 60
    // of them mounted in each expanded column.
    const columns = [...container.querySelectorAll("[data-kanban-column]")];
    expect(columns).toHaveLength(4);
    for (const column of columns) {
      const mounted = column.querySelectorAll("[data-kanban-card]").length;
      expect(mounted).toBeGreaterThan(0);
      expect(mounted).toBeLessThanOrEqual(MAX_PER_COLUMN);
    }

    // "card 99" matches #99 and #990…#999 (k98, k989…k998).
    const expected = ["k98", ...Array.from({ length: 10 }, (_, i) => `k${989 + i}`)];
    const input = container.querySelector<HTMLInputElement>(
      'input[aria-label="Search cards"]',
    )!;
    const filterStart = performance.now();
    act(() => {
      Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!.call(input, "card 99");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    // The search waits for typing to stop; the wait counts toward the budget.
    await act(
      () => new Promise((resolve) => setTimeout(resolve, SEARCH_DEBOUNCE_MS)),
    );
    await until(
      () => painted().sort().join() === [...expected].sort().join(),
    );
    const filter = performance.now() - filterStart;
    return { open, filter };
  } finally {
    act(() => root.unmount());
    container.remove();
  }
}

describe("board at 1,000 cards", () => {
  it(
    "tauri_open_and_filter_meet_the_budget",
    async () => {
      const runs: { open: number; filter: number }[] = [];
      for (let i = 0; i < RUNS; i += 1) runs.push(await measure());
      // The best run: the first pays for module and JIT warm-up.
      const open = Math.min(...runs.map((r) => r.open));
      const filter = Math.min(...runs.map((r) => r.filter));
      console.info(
        `board scale: open ${open.toFixed(0)} ms (budget ${OPEN_BUDGET_MS}), ` +
          `filter ${filter.toFixed(0)} ms (budget ${FILTER_BUDGET_MS}); ` +
          `runs ${runs.map((r) => `${r.open.toFixed(0)}/${r.filter.toFixed(0)}`).join(", ")}`,
      );
      expect(open).toBeLessThanOrEqual(OPEN_BUDGET_MS);
      expect(filter).toBeLessThanOrEqual(FILTER_BUDGET_MS);
    },
    15_000,
  );
});

// Linear work per render: lookups by card, debounced search, one read of the
// failed state, Set-based bulk paths.
describe("board work at 1,000 cards", () => {
  let container: HTMLDivElement;
  let root: ReturnType<typeof createRoot>;

  beforeEach(() => {
    events.listeners.clear();
    backend = createTestBackend(events.listeners);
    backend.seedThousand(4);
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(() => {
    vi.useRealTimers();
    act(() => root.unmount());
    container.remove();
  });

  async function mount(over: Partial<ComponentProps<typeof BoardView>> = {}) {
    await act(async () =>
      root.render(createElement(BoardView, { ...props(), ...over })),
    );
    await until(() => !!container.querySelector('[data-kanban-card="k0"]'));
  }

  const commands = (cmd: string) =>
    vi.mocked(invoke).mock.calls.filter(([name]) => name === cmd);
  const searchBox = () =>
    container.querySelector<HTMLInputElement>('input[aria-label="Search cards"]')!;
  function type(value: string) {
    act(() => {
      Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!.call(searchBox(), value);
      searchBox().dispatchEvent(new Event("input", { bubbles: true }));
    });
  }
  const sessionTarget = (
    id: string,
    status: BoardLinkTarget["status"],
  ): BoardLinkTarget => ({ kind: "session", id, label: `session ${id}`, status });

  it("failed_chip_shows_for_a_card_whose_failure_is_older_than_200_activity_rows", async () => {
    backend.cards[0] = { ...backend.cards[0], linkKind: "session", linkId: "s0" };
    backend.handlers.board_links = () => [
      { cardId: "k0", kind: "session", reference: "s0", role: "primary", createdAt: 1 },
      { cardId: "k0", kind: "session", reference: "s1", role: null, createdAt: 1 },
    ];
    // The newest 200 activity rows are all other cards' runs: k0's failure is
    // further back than any activity page reaches.
    backend.handlers.board_activity = () =>
      Array.from({ length: 200 }, (_, i) => ({
        id: `a${i}`,
        boardId: "b1",
        cardId: `k${i + 1}`,
        kind: "completed",
        actor: "system",
        fromColumn: null,
        toColumn: null,
        detail: null,
        createdAt: 10_000 - i,
      }));
    backend.handlers.board_latest_run_events = () => ({
      k0: "failed",
      k1: "completed",
    });
    await mount({
      linkTargets: [sessionTarget("s0", "completed"), sessionTarget("s1", "completed")],
    });
    const face = () =>
      container.querySelector<HTMLElement>('[data-board-card="k0"]')!;
    await until(
      () =>
        face().querySelector("[data-link-chip] [aria-label]")?.getAttribute(
          "aria-label",
        ) === "failed",
    );
    expect(
      [...face().querySelectorAll("[data-link-state]")].map((el) =>
        el.getAttribute("data-link-state"),
      ),
    ).toEqual(["failed"]);
    // One board-wide read, no activity page, answers it.
    expect(commands("board_latest_run_events")).toEqual([
      ["board_latest_run_events", { boardId: "b1" }],
    ]);
    expect(commands("board_activity")).toHaveLength(0);
  });

  it("search_runs_one_query_after_typing_stops", async () => {
    await mount();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const before = commands("board_query").length;
    // Five keystrokes, each inside 150 ms of the last.
    for (const value of ["c", "ca", "car", "card", "card 99"]) {
      type(value);
      await act(async () => vi.advanceTimersByTime(40));
    }
    // 149 ms after the last keystroke: still waiting.
    await act(async () => vi.advanceTimersByTime(109));
    await flush();
    expect(commands("board_query").length - before).toBe(0);
    // The input shows what was typed at once.
    expect(searchBox().value).toBe("card 99");
    await act(async () => vi.advanceTimersByTime(1));
    await flush();
    const asked = commands("board_query").slice(before);
    expect(asked).toHaveLength(1);
    expect((asked[0][1] as { spec: ViewSpec }).spec.query).toBe("card 99");
    await until(
      () =>
        [...container.querySelectorAll("[data-kanban-card]")].length === 11,
    );
    // Nothing more runs later.
    await act(async () => vi.advanceTimersByTime(1_000));
    await flush();
    expect(commands("board_query").length - before).toBe(1);
  });

  it("search_drops_a_late_answer_for_an_older_query", async () => {
    await mount();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const engine = backend.handlers.board_query;
    let release: () => void = () => {};
    backend.handlers.board_query = (args) => {
      const ids = engine(args);
      if ((args.spec as ViewSpec).query !== "card 1") return ids;
      // The first query's answer arrives after the second one's.
      return new Promise((resolve) => {
        release = () => resolve(ids);
      });
    };
    type("card 1");
    await act(async () => vi.advanceTimersByTime(150));
    await flush();
    type("card 99");
    await act(async () => vi.advanceTimersByTime(150));
    await flush();
    await until(
      () => container.querySelectorAll("[data-kanban-card]").length === 11,
    );
    await act(async () => release());
    await flush();
    expect(container.querySelectorAll("[data-kanban-card]")).toHaveLength(11);
  });

  it("filter_rows_query_without_waiting", async () => {
    await mount();
    // A filter row from the builder: Status is In Progress.
    act(() =>
      container
        .querySelector<HTMLButtonElement>('button[aria-label="Filter"]')!
        .click(),
    );
    const add = [
      ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
    ].find((el) => el.textContent?.trim() === "Status")!;
    act(() => add.click());
    await flush();
    const pick = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Pick Status values"]',
    )!;
    if (pick.getAttribute("aria-expanded") !== "true") act(() => pick.click());
    const value = [
      ...document.querySelectorAll<HTMLElement>('[role="menuitemcheckbox"]'),
    ].find((el) => el.textContent?.trim() === "In Progress")!;
    // Timers stop here: only a debounce would hold the query back.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const before = commands("board_query").length;
    act(() => value.click());
    // No timer advanced: the query ran on the change itself.
    await flush();
    const asked = commands("board_query").slice(before);
    expect(asked).toHaveLength(1);
    expect((asked[0][1] as { spec: ViewSpec }).spec.rows).toEqual([
      expect.objectContaining({ field: "status", values: ["progress"] }),
    ]);
    await until(
      () =>
        [...container.querySelectorAll("[data-kanban-column]")].every(
          (col) =>
            col.getAttribute("data-kanban-column") === "progress" ||
            !col.querySelector("[data-kanban-card]"),
        ),
    );
  });

  it("rendering_1000_cards_with_300_links_calls_no_per_card_scan", async () => {
    // 300 links over the first 150 cards: a session and a PR each.
    const rows: CardLinkRow[] = Array.from({ length: 150 }, (_, i) => [
      { cardId: `k${i}`, kind: "session", reference: `s${i}`, role: null, createdAt: 1 },
      {
        cardId: `k${i}`,
        kind: "pr",
        reference: `https://github.com/acme/api/pull/${i + 1}`,
        role: null,
        createdAt: 1,
      },
    ]).flat();
    // Every pass over the links (a loop or an array method) counts once.
    const PASSES = new Set<PropertyKey>([
      Symbol.iterator,
      "flatMap",
      "map",
      "filter",
      "forEach",
      "some",
      "every",
      "find",
      "findIndex",
      "reduce",
      "includes",
      "indexOf",
    ]);
    let passes = 0;
    backend.handlers.board_links = () =>
      new Proxy([...rows], {
        get(target, key, receiver) {
          if (PASSES.has(key)) passes += 1;
          return Reflect.get(target, key, receiver);
        },
      });
    await mount({
      linkTargets: rows
        .filter((row) => row.kind === "session")
        .map((row) => sessionTarget(row.reference, "completed")),
    });
    await flush();
    // The chips are there: the PR and session of k0.
    const face = container.querySelector<HTMLElement>('[data-board-card="k0"]')!;
    expect(
      [...face.querySelectorAll("[data-link-chip-kind]")].map((el) =>
        el.getAttribute("data-link-chip-kind"),
      ),
    ).toEqual(["session", "pr"]);
    const mounted = container.querySelectorAll("[data-kanban-card]").length;
    expect(mounted).toBeGreaterThan(40);
    // A per-card scan passes once (or more) per mounted card; a board-wide
    // index passes a handful of times per new link list.
    expect(passes).toBeLessThanOrEqual(8);
    // Re-renders that bring no new links pass over them not at all.
    passes = 0;
    act(() =>
      container
        .querySelector<HTMLElement>('[data-board-card="k1"]')!
        .dispatchEvent(
          new MouseEvent("click", {
            bubbles: true,
            ...(IS_MAC ? { metaKey: true } : { ctrlKey: true }),
          }),
        ),
    );
    await flush();
    expect(container.querySelector("[data-selection-count]")?.textContent).toBe(
      "1 selected",
    );
    expect(passes).toBe(0);
  });

  it(
    "bulk_delete_of_1000_selected_cards_stays_under_100ms",
    async () => {
      backend.seedThousand(1);
      await mount();
      const STEP = CARD_ESTIMATE.comfortable + CARD_GAP;
      const face = (id: string) =>
        container.querySelector(`[data-board-card="${id}"]`)!;
      const click = async (el: Element, init: MouseEventInit) => {
        act(() => {
          el.dispatchEvent(
            new MouseEvent("click", { bubbles: true, cancelable: true, ...init }),
          );
        });
        await flush();
      };
      await click(face("k0"), IS_MAC ? { metaKey: true } : { ctrlKey: true });
      const body = container
        .querySelector('[data-kanban-column="backlog"]')!
        .querySelector<HTMLElement>(".overflow-y-auto")!;
      act(() => {
        body.scrollTop = 999 * STEP;
        body.dispatchEvent(new Event("scroll"));
      });
      await click(face("k999"), { shiftKey: true });
      expect(
        container.querySelector("[data-selection-count]")?.textContent,
      ).toBe("1000 selected");
      const remove = [
        ...container.querySelectorAll<HTMLButtonElement>(
          '[aria-label="Selected cards"] button',
        ),
      ].find((el) => el.textContent?.trim() === "Delete")!;
      const start = performance.now();
      act(() => remove.click());
      const took = performance.now() - start;
      console.info(
        `bulk delete of 1000: ${took.toFixed(0)} ms (budget ${BULK_DELETE_BUDGET_MS})`,
      );
      expect(container.querySelectorAll("[data-kanban-card]")).toHaveLength(0);
      expect(took).toBeLessThanOrEqual(BULK_DELETE_BUDGET_MS);
      await flush();
      // The held delete covers every card, at its place in the column.
      expect(commands("board_cards_archive")).toHaveLength(1);
      const archived = commands("board_cards_archive")[0][1] as {
        cardIds: string[];
      };
      expect(archived.cardIds).toHaveLength(1000);
      expect(
        document.querySelector("[data-board-undo-toast]")?.textContent,
      ).toContain("1000");
    },
    15_000,
  );
});
