// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BoardView } from "./BoardView";
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
