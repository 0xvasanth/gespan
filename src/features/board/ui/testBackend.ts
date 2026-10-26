// A fake Tauri backend for board view tests: the commands `BoardView` calls,
// over cards, extras and archived ids that a test can change to play the
// other app. The test file keeps its own `vi.mock` calls (they are hoisted
// per file) and hands in the board-changed listeners its `listen` mock keeps.
import { act } from "react";
import { vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { SEARCH_DEBOUNCE_MS } from "./BoardView";
import type {
  BoardCard,
  BoardColumn,
  BoardLabel,
  CardExtras,
  ColumnRole,
  ViewSpec,
} from "../model/board";

export const DAY = 86_400_000;
export const BOARD = { id: "b1", name: "Agents", createdAt: 1, updatedAt: 1 };
export const OTHER = { id: "b2", name: "Releases", createdAt: 2, updatedAt: 2 };

export const column = (
  id: string,
  name: string,
  position: number,
  role: ColumnRole | null,
): BoardColumn => ({
  id,
  boardId: "b1",
  name,
  position,
  role,
  color: null,
  wipLimit: null,
  collapsed: false,
});

export const COLUMNS = [
  column("backlog", "Backlog", 0, "todo"),
  column("progress", "In Progress", 1, "doing"),
  column("review", "Review", 2, "review"),
  column("done", "Done", 3, "done"),
];

export const card = (
  id: string,
  columnId: string,
  position: number,
  number: number,
  title: string,
): BoardCard => ({
  id,
  boardId: "b1",
  columnId,
  number,
  title,
  body: "",
  position,
  linkKind: null,
  linkId: null,
  createdAt: 1,
  updatedAt: Date.now(),
});

export const extra = (over: Partial<CardExtras> = {}): CardExtras => ({
  priority: 0,
  dueAt: null,
  projects: [],
  labels: [],
  subtasksDone: 0,
  subtasksTotal: 0,
  comments: 0,
  ...over,
});

export const LABELS: BoardLabel[] = [
  { id: "l1", boardId: "b1", name: "Bug", color: "e5484d", position: 0 },
  { id: "l2", boardId: "b1", name: "UI", color: "3e63dd", position: 1 },
];

export type Handler = (args: Record<string, unknown>) => unknown;
export type BoardListener = (event: { payload: unknown }) => void;

export type TestBackend = {
  /** Command name to handler; tests replace entries to change one answer. */
  handlers: Record<string, Handler>;
  /** The backend's cards, archived ones included. */
  cards: BoardCard[];
  extras: Record<string, CardExtras>;
  archived: Set<string>;
  savedViews: Record<string, unknown>[];
  /** The columns `board_get` returns for board b1. */
  columns: BoardColumn[];
  /** Cards that are not archived. */
  live(): BoardCard[];
  /** The four-card board most tests start from. */
  seed(): void;
  /**
   * 1,000 cards `k0`…`k999` titled "Card 1"…"Card 1000", dealt round-robin
   * over the first `columns` columns (one column gets all of them with 1).
   */
  seedThousand(columns?: number): void;
  /** Plays a write from the other app: every board listener hears it. */
  emitBoardChanged(boardId?: string): void;
};

/**
 * Builds the backend and routes the mocked `invoke` to it. `listeners` is
 * the set the test's `listen` mock adds board-changed handlers to.
 */
export function createTestBackend(
  listeners: Set<BoardListener>,
): TestBackend {
  const backend: TestBackend = {
    handlers: {},
    cards: [],
    extras: {},
    archived: new Set(),
    savedViews: [],
    columns: COLUMNS,
    live: () => backend.cards.filter((c) => !backend.archived.has(c.id)),
    seed() {
      backend.cards = [
        card("c1", "backlog", 0, 1, "Fix login"),
        card("c2", "backlog", 1, 2, "Write docs"),
        card("c3", "progress", 0, 3, "Ship board"),
        card("c4", "done", 0, 4, "Deploy"),
      ];
      backend.extras = {
        c1: extra({ priority: 3, labels: ["l1"], dueAt: Date.now() + 3 * DAY }),
        c2: extra({ priority: 1, labels: ["l1", "l2"] }),
        c4: extra({ priority: 2 }),
      };
      backend.archived = new Set();
    },
    seedThousand(columns = 4) {
      const used = backend.columns.slice(0, Math.max(1, columns));
      backend.cards = Array.from({ length: 1000 }, (_, i) =>
        card(
          `k${i}`,
          used[i % used.length].id,
          Math.floor(i / used.length),
          i + 1,
          `Card ${i + 1}`,
        ),
      );
      backend.extras = {};
      backend.archived = new Set();
    },
    emitBoardChanged(boardId = "b1") {
      for (const listener of [...listeners]) listener({ payload: { boardId } });
    },
  };

  /** Stands in for `board_query`: title text, status and label rows, priority and title sorts. */
  function fakeEngine(args: Record<string, unknown>): string[] {
    const spec = args.spec as ViewSpec;
    const { extras } = backend;
    const order = new Map(backend.columns.map((c) => [c.id, c.position]));
    const manual = [...backend.live()].sort(
      (a, b) =>
        (order.get(a.columnId) ?? 0) - (order.get(b.columnId) ?? 0) ||
        a.position - b.position,
    );
    const q = spec.query.trim().toLowerCase();
    const kept = manual.filter(
      (c) =>
        (!q || c.title.toLowerCase().includes(q)) &&
        spec.rows.every((row) => {
          const held =
            row.field === "status"
              ? [c.columnId]
              : row.field === "labels"
                ? (extras[c.id]?.labels ?? [])
                : [String(extras[c.id]?.priority ?? 0)];
          const hit = held.some((v) => row.values.includes(v));
          return row.op === "is" ? hit : !hit;
        }),
    );
    const { key, dir } = spec.sort;
    if (key === "manual") return kept.map((c) => c.id);
    const value = (c: BoardCard) =>
      key === "title" ? c.title.toLowerCase() : (extras[c.id]?.priority ?? 0);
    return kept
      .map((c, index) => ({ c, index, v: value(c) }))
      .sort((a, b) =>
        a.v === b.v
          ? a.index - b.index
          : (a.v < b.v ? -1 : 1) * (dir === "desc" ? -1 : 1),
      )
      .map(({ c }) => c.id);
  }

  backend.handlers = {
    board_query: fakeEngine,
    board_views: () => backend.savedViews,
    board_list: () => [BOARD, OTHER],
    board_get: (args) =>
      args.boardId === "b2"
        ? {
            board: OTHER,
            columns: [{ ...column("t2", "Todo", 0, "todo"), boardId: "b2" }],
            cards: [],
          }
        : { board: BOARD, columns: backend.columns, cards: backend.live() },
    board_activity: () => [],
    board_links: () => [],
    board_blocked: () => ({}),
    board_latest_run_events: () => ({}),
    board_card_extras: () => backend.extras,
    board_labels: () => LABELS,
    board_subtasks: () => [],
    board_comments: () => [],
    board_card_meta: () => ({
      priority: 0,
      dueAt: null,
      startAt: null,
      estimateMinutes: null,
      archivedAt: null,
      completedAt: null,
    }),
    board_fields: () => [],
    board_card_field_values: () => [],
    board_card_relations: () => [],
    board_card_links: () => [],
    board_card_index: () => [],
    board_card_archive: (args) => {
      backend.archived.add(String(args.cardId));
    },
    board_cards_archive: (args) => {
      for (const id of args.cardIds as string[]) backend.archived.add(id);
    },
    board_card_restore: (args) => {
      backend.archived.delete(String(args.cardId));
    },
    board_card_delete_archived: (args) => {
      const id = String(args.cardId);
      if (backend.archived.has(id)) {
        backend.cards = backend.cards.filter((c) => c.id !== id);
      }
    },
    board_archived_cards: () =>
      backend.cards.filter((c) => backend.archived.has(c.id)),
    board_cards_set_priority: () => undefined,
    board_cards_set_label: () => undefined,
    board_cards_move: () => undefined,
    board_card_update: () => undefined,
    board_card_set_priority: () => undefined,
    board_card_set_due: () => undefined,
    board_card_set_labels: () => undefined,
    board_card_move: () => undefined,
    board_view_create: (args) => {
      const view = {
        id: `v${backend.savedViews.length + 1}`,
        boardId: "b1",
        name: String(args.name),
        position: backend.savedViews.length,
        specJson: String(args.specJson),
        layoutJson: String(args.layoutJson),
        createdAt: 1,
        updatedAt: 1,
      };
      backend.savedViews = [...backend.savedViews, view];
      return view;
    },
    board_view_update: (args) => {
      backend.savedViews = backend.savedViews.map((view) =>
        view.id === args.viewId
          ? {
              ...view,
              layoutJson: (args.layoutJson as string | null) ?? view.layoutJson,
              specJson: (args.specJson as string | null) ?? view.specJson,
            }
          : view,
      );
    },
  };

  backend.seed();
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(async (cmd, args) => {
    const handler = backend.handlers[cmd];
    if (!handler) throw new Error(`unexpected command ${cmd}`);
    return handler((args ?? {}) as Record<string, unknown>);
  });
  return backend;
}

/**
 * Waits out the search box's debounce, so the typed search has been asked
 * for: by the clock, or by advancing fake timers when a test uses them.
 */
export async function waitForSearch() {
  await act(async () => {
    if (vi.isFakeTimers()) vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS);
    else await new Promise((resolve) => setTimeout(resolve, SEARCH_DEBOUNCE_MS));
  });
}

/** Lets pending promises (invoke answers, effects) settle inside `act`. */
export async function flush() {
  await act(async () => {
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
}
