// @vitest-environment happy-dom
import {
  act,
  createElement,
  StrictMode,
  useSyncExternalStore,
  type ComponentProps,
} from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import type {
  BoardCard,
  BoardLabel,
  CardExtras,
  BoardColumn,
  BoardDetail,
  ColumnRole,
  ViewSpec,
} from "../model/board";
import type { BoardLinkTarget } from "./boardFormat";
import type { KanbanBoardProps } from "./kanban/KanbanBoard";
import { LAYER } from "../../../shared/lib/layers";
import { IS_MAC } from "../../../platform/tauri/platform";
import { BoardView } from "./BoardView";
import { waitForSearch } from "./testBackend";
import { Sidebar } from "../../../app/shell/Sidebar";
import {
  saveBoardCardOpen,
  loadBoardEnabled,
  saveBoardEnabled,
  subscribeBoardEnabled,
} from "../../settings/model/settings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
// Delivers `gespan:board-changed` the way the backend emits it.
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

// The real board, with its latest props kept so a test can drive a drop.
const kanban: { props: KanbanBoardProps | null } = { props: null };
vi.mock("./kanban/KanbanBoard", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./kanban/KanbanBoard")>();
  return {
    KanbanBoard: (props: KanbanBoardProps) => {
      kanban.props = props;
      return createElement(actual.KanbanBoard, props);
    },
  };
});

// Keep the Sidebar's native services out of the flag test.
vi.mock("../../source-control/hooks/useProjectDiffStats", () => ({
  useProjectDiffStats: vi.fn(() => null),
}));
vi.mock("../../source-control/hooks/useGitFileStatuses", () => ({
  useGitFileStatuses: () => ({ files: new Map(), dirs: new Map() }),
}));
vi.mock("../../../app/shell/SidebarUpdate", () => ({
  SidebarUpdateFooter: () => null,
}));
vi.mock("../../files/ui/FileTree", () => ({ FileTree: () => null }));
vi.mock("../../../app/shell/WindowControls", () => ({
  WindowControls: () => null,
}));

function column(
  id: string,
  name: string,
  position: number,
  role: ColumnRole | null,
): BoardColumn {
  return {
    id,
    boardId: "b1",
    name,
    position,
    role,
    color: null,
    wipLimit: null,
    collapsed: false,
  };
}

function card(
  id: string,
  columnId: string,
  position: number,
  number: number,
  title: string,
): BoardCard {
  return {
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
  };
}

const BOARD = { id: "b1", name: "Agents", createdAt: 1, updatedAt: 1 };

function fixture(): BoardDetail {
  return {
    board: BOARD,
    columns: [
      column("backlog", "Backlog", 0, "todo"),
      column("progress", "In Progress", 1, "doing"),
      column("review", "Review", 2, "review"),
      column("done", "Done", 3, "done"),
    ],
    cards: [
      card("c1", "backlog", 0, 1, "Fix login"),
      card("c2", "backlog", 1, 2, "Write docs"),
      card("c3", "progress", 0, 3, "Ship board"),
    ],
  };
}

type Handler = (args: Record<string, unknown>) => unknown;
let handlers: Record<string, Handler>;
// Custom-field values the stand-in engine reads: card id -> field id -> JSON.
let fieldValues: Record<string, Record<string, string>>;
// The board's saved views, as `board_views` lists them.
let savedViews: BoardViewRow[];

type BoardViewRow = {
  id: string;
  boardId: string;
  name: string;
  position: number;
  specJson: string;
  layoutJson: string;
  createdAt: number;
  updatedAt: number;
};

/**
 * Stands in for the Rust `board_query`: it matches the title and description
 * (or `#N`), the rows the tests use, and the sort, over what the fake board
 * serves. Only the shape matters here; the real rules are tested in Rust.
 * Differences from Rust: `start` is always missing (so it keeps manual
 * order), and `linked: none` means "not running" rather than "no links".
 * Priority 0 sorts as missing (last), as in Rust.
 */
function fakeEngine(args: Record<string, unknown>): string[] {
  const spec = args.spec as ViewSpec;
  const running = (args.running as string[]) ?? [];
  const board = handlers.board_get({}) as BoardDetail;
  const extras = (handlers.board_card_extras({}) ?? {}) as Record<
    string,
    Partial<CardExtras> | undefined
  >;
  const order = new Map(board.columns.map((col) => [col.id, col.position]));
  const manual = [...board.cards].sort(
    (a, b) =>
      (order.get(a.columnId) ?? 0) - (order.get(b.columnId) ?? 0) ||
      a.position - b.position,
  );
  const q = spec.query.trim().toLowerCase();
  const matchesRow = (c: BoardCard, row: ViewSpec["rows"][number]) => {
    const e = extras[c.id];
    const held: string[] =
      row.field === "status"
        ? [c.columnId]
        : row.field === "priority"
          ? [String(e?.priority ?? 0)]
          : row.field === "labels"
            ? (e?.labels ?? [])
            : row.field === "projects"
              ? (e?.projects ?? [])
              : row.field === "linked"
                ? [running.includes(c.id) ? "running" : "none"]
                : row.field.startsWith("field:")
                  ? [fieldValues[c.id]?.[row.field.slice(6)] ?? ""]
                  : [];
    const hit = held.some((value) => row.values.includes(value));
    return row.op === "is" ? hit : !hit;
  };
  const kept = manual.filter(
    (c) =>
      (!q ||
        (q.startsWith("#")
          ? String(c.number) === q.slice(1)
          : `${c.title}\n${c.body}`.toLowerCase().includes(q))) &&
      spec.rows.every((row) => matchesRow(c, row)),
  );
  const { key, dir } = spec.sort;
  if (key === "manual") return kept.map((c) => c.id);
  const value = (c: BoardCard): number | string | null => {
    const e = extras[c.id];
    return key === "priority"
      ? e?.priority || null
      : key === "due"
        ? (e?.dueAt ?? null)
        : key === "title"
          ? c.title.toLowerCase()
          : key === "created"
            ? c.createdAt
            : key === "updated"
              ? c.updatedAt
              : null;
  };
  return kept
    .map((c, index) => ({ c, index, v: value(c) }))
    .sort((a, b) => {
      if (a.v === null || b.v === null) {
        return a.v === b.v ? a.index - b.index : a.v === null ? 1 : -1;
      }
      if (a.v === b.v) return a.index - b.index;
      return (a.v < b.v ? -1 : 1) * (dir === "desc" ? -1 : 1);
    })
    .map(({ c }) => c.id);
}

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
  kanban.props = null;
  events.listeners.clear();
  fieldValues = {};
  savedViews = [];
  handlers = {
    board_query: fakeEngine,
    board_views: () => savedViews,
    board_list: () => [BOARD],
    board_get: () => fixture(),
    board_activity: () => [],
    board_card_extras: () => ({}),
    board_labels: () => [],
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
    board_card_create: (args) =>
      card("new", String(args.columnId), 9, 9, String(args.title)),
  };
  vi.mocked(invoke).mockReset();
  vi.mocked(ask).mockReset();
  vi.mocked(invoke).mockImplementation(async (cmd, args) => {
    const handler = handlers[cmd];
    if (!handler) throw new Error(`unexpected command ${cmd}`);
    return handler((args ?? {}) as Record<string, unknown>);
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function flush() {
  await act(async () => {
    for (let i = 0; i < 5; i += 1) await Promise.resolve();
  });
}

async function renderBoard(
  overrides: Partial<ComponentProps<typeof BoardView>> = {},
) {
  const props: ComponentProps<typeof BoardView> = {
    onClose: vi.fn(),
    linkTargets: [],
    onOpenLinkedWork: vi.fn(() => Promise.resolve()),
    lookupLinkTargets: vi.fn(async () => []),
    projects: [],
    ...overrides,
  };
  await act(async () => {
    root.render(createElement(BoardView, props));
  });
  await flush();
  return props;
}

const columnEl = (id: string) =>
  container.querySelector<HTMLElement>(`[data-kanban-column="${id}"]`)!;

const cardIdsIn = (id: string) =>
  [...columnEl(id).querySelectorAll("[data-kanban-card]")].map((el) =>
    el.getAttribute("data-kanban-card"),
  );

function emitBoardChanged(boardId: string) {
  for (const listener of [...events.listeners]) {
    listener({ payload: { boardId } });
  }
}

/** A board_get that never answers: nothing but local state can change the board. */
const hang = () => new Promise<never>(() => undefined);

const commands = (cmd: string) =>
  vi.mocked(invoke).mock.calls.filter(([name]) => name === cmd);

function typeInto(field: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    "value",
  )!.set!;
  act(() => {
    setter.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function press(target: HTMLElement, key: string, init: KeyboardEventInit = {}) {
  act(() => {
    target.dispatchEvent(
      new KeyboardEvent("keydown", {
        key,
        bubbles: true,
        cancelable: true,
        ...init,
      }),
    );
  });
}

async function search(value: string) {
  const input = container.querySelector<HTMLInputElement>(
    'input[aria-label="Search cards"]',
  )!;
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )!.set!;
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await waitForSearch();
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
  act(() => item.click());
  await flush();
}


/** Filters through the builder: adds a row on `field` if there is none, then picks `value`. */
async function filterBy(field: string, value: string) {
  const trigger = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Filter"]',
  )!;
  if (trigger.getAttribute("aria-expanded") !== "true") {
    act(() => trigger.click());
  }
  const toggle = () =>
    document.querySelector<HTMLButtonElement>(
      `button[aria-label="Pick ${field} values"]`,
    );
  if (!toggle()) {
    const add = [
      ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
    ].find((el) => el.textContent?.trim() === field);
    expect(add, `Add filter: ${field}`).toBeDefined();
    act(() => add!.click());
    await flush();
  }
  if (toggle()!.getAttribute("aria-expanded") !== "true") {
    act(() => toggle()!.click());
  }
  const item = [
    ...document.querySelectorAll<HTMLElement>('[role="menuitemcheckbox"]'),
  ].find((el) => el.textContent?.trim() === value);
  expect(item, value).toBeDefined();
  act(() => item!.click());
  await flush();
}


describe("BoardView", () => {
  it("shows the empty state with no boards", async () => {
    handlers.board_list = () => [];
    await renderBoard();
    expect(container.textContent).toContain("No boards yet");
    expect(
      [...container.querySelectorAll("button")].some(
        (button) => button.textContent === "Create board",
      ),
    ).toBe(true);
    expect(commands("board_get")).toHaveLength(0);
  });

  it("renders the columns with names, role icons and counts, and card numbers", async () => {
    await renderBoard();
    const columns = [...container.querySelectorAll("[data-kanban-column]")];
    expect(columns).toHaveLength(4);
    expect(
      columns.map((el) => el.querySelector("[data-column-name]")?.textContent),
    ).toEqual(["Backlog", "In Progress", "Review", "Done"]);
    expect(
      columns.map((el) =>
        el.querySelector("svg[data-role]")?.getAttribute("data-role"),
      ),
    ).toEqual(["todo", "doing", "review", "done"]);
    expect(
      columns.map((el) => el.querySelector("[data-column-count]")?.textContent),
    ).toEqual(["2", "1", "0", "0"]);
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2"]);
    const first = columnEl("backlog").querySelector("[data-kanban-card='c1']")!;
    expect(first.textContent).toContain("#1");
    expect(first.textContent).toContain("Fix login");
    // The board that opened is remembered for next time.
    expect(localStorage.getItem("gespan:open-board")).toBe("b1");
  });

  it("adds a card with Enter, ignores an empty title and closes on Esc", async () => {
    await renderBoard();
    const addButton = [
      ...columnEl("backlog").querySelectorAll<HTMLButtonElement>("button"),
    ].find((button) => button.textContent?.includes("Add card"))!;
    act(() => addButton.click());
    const field = columnEl("backlog").querySelector("textarea")!;
    expect(field).not.toBeNull();
    expect(columnEl("backlog").textContent).toContain(
      "Enter to add · Esc to cancel",
    );

    typeInto(field, "   ");
    press(field, "Enter");
    await flush();
    expect(commands("board_card_create")).toHaveLength(0);

    typeInto(field, "  Fix the build  ");
    press(field, "Enter");
    await flush();
    expect(commands("board_card_create")).toHaveLength(1);
    expect(commands("board_card_create")[0][1]).toMatchObject({
      columnId: "backlog",
      title: "Fix the build",
    });

    // Shift+Enter is a newline, not a submit.
    typeInto(field, "Two");
    press(field, "Enter", { shiftKey: true });
    await flush();
    expect(commands("board_card_create")).toHaveLength(1);

    press(columnEl("backlog").querySelector("textarea")!, "Escape");
    expect(columnEl("backlog").querySelector("textarea")).toBeNull();
  });

  it("opens a focused card's detail with Enter", async () => {
    await renderBoard();
    const wrapper = columnEl("backlog").querySelector<HTMLElement>(
      "[data-kanban-card='c1']",
    )!;
    // Enter on the card's own menu button opens the menu, not the card.
    const menu = wrapper.querySelector<HTMLButtonElement>(
      'button[aria-label="Card actions for #1"]',
    )!;
    act(() => menu.focus());
    press(menu, "Enter", { code: "Enter" });
    await flush();
    expect(document.querySelector("[data-card-drawer]")).toBeNull();

    act(() => wrapper.focus());
    press(wrapper, "Enter", { code: "Enter" });
    await flush();
    expect(wrapper.hasAttribute("data-dragging")).toBe(false);
    const dialog = document.querySelector("[data-card-drawer]");
    expect(dialog?.textContent).toContain("Card #1");
  });

  it("disables Delete column unless the column has no live or archived cards", async () => {
    const counts: Record<string, { live: number; archived: number }> = {
      backlog: { live: 2, archived: 0 },
      progress: { live: 0, archived: 1 },
      review: { live: 0, archived: 0 },
      done: { live: 0, archived: 3 },
    };
    handlers.board_column_card_count = (args) => counts[String(args.columnId)];
    await renderBoard();
    const deleteItem = async (columnId: string) => {
      const menuButton = columnEl(columnId).querySelector<HTMLButtonElement>(
        'button[aria-label^="Column actions"]',
      )!;
      act(() => menuButton.click());
      await flush();
      const item = [
        ...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
      ].find((el) => el.textContent?.startsWith("Delete"))!;
      return item;
    };
    const closeMenu = async (item: HTMLElement) => {
      press(item, "Escape");
      act(() => document.body.dispatchEvent(new PointerEvent("pointerdown")));
      await flush();
    };
    const live = await deleteItem("backlog");
    expect(live.disabled).toBe(true);
    expect(live.getAttribute("title")).toBe("Move or delete its 2 cards first");
    await closeMenu(live);

    // Archived cards count too, and one card is singular.
    const archivedOne = await deleteItem("progress");
    expect(archivedOne.disabled).toBe(true);
    expect(archivedOne.getAttribute("title")).toBe(
      "Move or delete its 1 card first",
    );
    await closeMenu(archivedOne);

    const archived = await deleteItem("done");
    expect(archived.disabled).toBe(true);
    expect(archived.getAttribute("title")).toBe(
      "Move or delete its 3 cards first",
    );
    await closeMenu(archived);

    const open = await deleteItem("review");
    expect(open.disabled).toBe(false);
    expect(open.getAttribute("title")).toBeNull();
  });

  it("keeps Delete column disabled when the card count can't be read", async () => {
    handlers.board_column_card_count = () => {
      throw new Error("boom");
    };
    await renderBoard();
    act(() =>
      columnEl("review")
        .querySelector<HTMLButtonElement>(
          'button[aria-label^="Column actions"]',
        )!
        .click(),
    );
    await flush();
    const item = [
      ...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
    ].find((el) => el.textContent?.startsWith("Delete"))!;
    expect(item.disabled).toBe(true);
  });

  it("moves a card optimistically and puts it back when the move fails", async () => {
    await renderBoard();
    // The refetch after the failure never answers, so only the restore can
    // put the card back.
    handlers.board_get = hang;
    let reject!: (error: unknown) => void;
    handlers.board_card_move = () =>
      new Promise((_, fail) => {
        reject = fail;
      });

    act(() => kanban.props!.onCardMove("c1", { columnId: "review", index: 0 }));
    expect(cardIdsIn("backlog")).toEqual(["c2"]);
    expect(cardIdsIn("review")).toEqual(["c1"]);
    expect(commands("board_card_move")).toHaveLength(1);
    expect(commands("board_card_move")[0][1]).toEqual({
      cardId: "c1",
      columnId: "review",
      index: 0,
      actor: "user",
    });

    await act(async () => reject("database is locked"));
    await flush();
    expect(commands("board_get")).toHaveLength(2);
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2"]);
    expect(cardIdsIn("review")).toEqual([]);
    expect(
      container.ownerDocument.querySelector('[role="alert"]')?.textContent,
    ).toContain("Couldn't move the card: database is locked");
  });

  it("puts a column back when its move fails", async () => {
    await renderBoard();
    handlers.board_get = hang;
    let reject!: (error: unknown) => void;
    handlers.board_column_move = () =>
      new Promise((_, fail) => {
        reject = fail;
      });
    const order = () =>
      [...container.querySelectorAll("[data-kanban-column]")].map((el) =>
        el.getAttribute("data-kanban-column"),
      );

    act(() => kanban.props!.onColumnMove("done", 0));
    expect(order()).toEqual(["done", "backlog", "progress", "review"]);

    await act(async () => reject("database is locked"));
    await flush();
    expect(commands("board_get")).toHaveLength(2);
    expect(order()).toEqual(["backlog", "progress", "review", "done"]);
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      "Couldn't move the column: database is locked",
    );
  });

  it("moves a column optimistically and calls board_column_move", async () => {
    await renderBoard();
    handlers.board_column_move = () => undefined;
    act(() => kanban.props!.onColumnMove("done", 0));
    expect(
      [...container.querySelectorAll("[data-kanban-column]")].map((el) =>
        el.getAttribute("data-kanban-column"),
      ),
    ).toEqual(["done", "backlog", "progress", "review"]);
    expect(commands("board_column_move")[0][1]).toEqual({
      columnId: "done",
      index: 0,
    });
  });
});

describe("BoardView board delete", () => {
  const OTHER = { id: "b2", name: "Releases", createdAt: 2, updatedAt: 2 };

  function otherFixture(): BoardDetail {
    return {
      board: OTHER,
      columns: [{ ...column("todo2", "Todo", 0, "todo"), boardId: "b2" }],
      cards: [],
    };
  }

  /** Boards as the backend holds them; delete emits board-changed first. */
  function backend(initial: (typeof BOARD)[]) {
    let boards = initial;
    handlers.board_list = () => boards;
    handlers.board_get = (args) => {
      const id = String(args.boardId);
      if (!boards.some((board) => board.id === id)) {
        throw "Board not found";
      }
      return id === "b1" ? fixture() : otherFixture();
    };
    handlers.board_delete = (args) => {
      boards = boards.filter((board) => board.id !== args.boardId);
      emitBoardChanged(String(args.boardId));
    };
  }

  async function deleteOpenBoard() {
    vi.mocked(ask).mockResolvedValue(true);
    const menu = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Board actions"]',
    )!;
    act(() => menu.click());
    const item = [
      ...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
    ].find((el) => el.textContent === "Delete board")!;
    await act(async () => item.click());
    await flush();
    await flush();
  }

  it("opens another board without an error when the open board is deleted", async () => {
    backend([BOARD, OTHER]);
    await renderBoard();
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2"]);

    await deleteOpenBoard();
    expect(commands("board_delete")[0][1]).toEqual({ boardId: "b1" });
    expect(document.querySelector('[role="alert"]')).toBeNull();
    expect(commands("board_list").length).toBeGreaterThan(1);
    expect(
      [...container.querySelectorAll("[data-kanban-column]")].map((el) =>
        el.getAttribute("data-kanban-column"),
      ),
    ).toEqual(["todo2"]);
    expect(localStorage.getItem("gespan:open-board")).toBe("b2");
  });

  it("shows the empty state without an error when the last board is deleted", async () => {
    backend([BOARD]);
    await renderBoard();
    await deleteOpenBoard();
    expect(document.querySelector('[role="alert"]')).toBeNull();
    expect(container.textContent).toContain("No boards yet");
    expect(localStorage.getItem("gespan:open-board")).toBeNull();
  });

  it("still reports a failed reload of a board that exists", async () => {
    backend([BOARD, OTHER]);
    await renderBoard();
    handlers.board_get = () => {
      throw "database is locked";
    };
    act(() => emitBoardChanged("b1"));
    await flush();
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      "Couldn't load the board: database is locked",
    );
    // The board stays open with what it last showed.
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2"]);
  });
});

describe("BoardView linked work", () => {
  function linkedFixture(): BoardDetail {
    const detail = fixture();
    detail.cards[0] = { ...detail.cards[0], linkKind: "session", linkId: "s1" };
    detail.cards[1] = {
      ...detail.cards[1],
      linkKind: "flow-run",
      linkId: "run-gone",
    };
    return detail;
  }

  it("shows a linked session's chip and opens it from the card", async () => {
    handlers.board_get = () => linkedFixture();
    const target = {
      kind: "session" as const,
      id: "s1",
      label: "Codex · Fix login",
      status: "running" as const,
    };
    const props = await renderBoard({ linkTargets: [target] });
    const linked = columnEl("backlog").querySelector(
      "[data-kanban-card='c1']",
    )!;
    const chip = linked.querySelector("[data-link-chip]")!;
    expect(chip.getAttribute("data-link-chip")).toBe("found");
    expect(chip.textContent).toContain("Codex · Fix login");
    expect(chip.querySelector('[aria-label="running"]')).not.toBeNull();

    const open = linked.querySelector<HTMLButtonElement>(
      'button[aria-label="Open linked work"]',
    )!;
    await act(async () => open.click());
    expect(props.onOpenLinkedWork).toHaveBeenCalledWith(target);
    // The button doesn't also open the card, and success shows no error.
    expect(document.querySelector("[data-card-drawer]")).toBeNull();
    expect(document.querySelector('[role="alert"]')).toBeNull();
  });

  it("shows the error toast when opening linked work fails", async () => {
    handlers.board_get = () => linkedFixture();
    const target = { kind: "session" as const, id: "s1", label: "Fix login" };
    const onOpenLinkedWork = vi.fn(() =>
      Promise.reject(new Error("This conversation is no longer available.")),
    );
    await renderBoard({ linkTargets: [target], onOpenLinkedWork });
    const alert = () => document.querySelector('[role="alert"]');

    // From the card's hover action.
    const linked = columnEl("backlog").querySelector(
      "[data-kanban-card='c1']",
    )!;
    await act(async () =>
      linked
        .querySelector<HTMLButtonElement>(
          'button[aria-label="Open linked work"]',
        )!
        .click(),
    );
    await flush();
    expect(onOpenLinkedWork).toHaveBeenCalledWith(target);
    expect(alert()?.textContent).toContain(
      "Couldn't open linked work: This conversation is no longer available.",
    );
    act(() => alert()!.querySelector<HTMLButtonElement>("button")!.click());
    expect(alert()).toBeNull();

    // From the card detail's button.
    act(() =>
      columnEl("backlog")
        .querySelector<HTMLElement>("[data-board-card='c1']")!
        .click(),
    );
    await flush();
    const detailButton = [
      ...document.querySelectorAll<HTMLButtonElement>(
        "[data-card-drawer] button",
      ),
    ].find((button) => button.textContent?.includes("Open linked work"))!;
    await act(async () => detailButton.click());
    await flush();
    expect(onOpenLinkedWork).toHaveBeenCalledTimes(2);
    expect(alert()?.textContent).toContain(
      "Couldn't open linked work: This conversation is no longer available.",
    );
  });

  it("marks a link whose work is gone, including Flow runs", async () => {
    handlers.board_get = () => linkedFixture();
    await renderBoard();
    for (const id of ["c1", "c2"]) {
      const el = columnEl("backlog").querySelector(
        `[data-kanban-card='${id}']`,
      )!;
      const chip = el.querySelector("[data-link-chip]")!;
      expect(chip.getAttribute("data-link-chip")).toBe("missing");
      expect(chip.textContent).toBe("Linked work no longer exists");
      expect(
        el.querySelector('button[aria-label="Open linked work"]'),
      ).toBeNull();
    }
  });

  it("looks up links the app hasn't loaded, and marks only deleted ones missing", async () => {
    handlers.board_get = () => {
      const detail = fixture();
      detail.cards[0] = {
        ...detail.cards[0],
        linkKind: "session",
        linkId: "s1",
      };
      detail.cards[1] = {
        ...detail.cards[1],
        linkKind: "session",
        linkId: "other-project",
      };
      detail.cards[2] = {
        ...detail.cards[2],
        linkKind: "session",
        linkId: "deleted",
      };
      return detail;
    };
    const loaded = { kind: "session" as const, id: "s1", label: "Fix login" };
    const stored = {
      kind: "session" as const,
      id: "other-project",
      label: "Write docs",
      detail: "docs-site",
    };
    let answer!: (targets: BoardLinkTarget[]) => void;
    const lookupLinkTargets = vi.fn(
      () =>
        new Promise<BoardLinkTarget[]>((resolve) => {
          answer = resolve;
        }),
    );
    const props = await renderBoard({
      linkTargets: [loaded],
      lookupLinkTargets,
    });
    const chip = (id: string) =>
      container
        .querySelector(`[data-kanban-card='${id}'] [data-link-chip]`)!
        .getAttribute("data-link-chip");

    // Only the links the app doesn't know are looked up, in one batch.
    expect(lookupLinkTargets).toHaveBeenCalledTimes(1);
    expect(lookupLinkTargets).toHaveBeenCalledWith([
      { kind: "session", id: "other-project" },
      { kind: "session", id: "deleted" },
    ]);
    // Until the lookup answers, nothing reads as gone.
    expect([chip("c1"), chip("c2"), chip("c3")]).toEqual([
      "found",
      "unconfirmed",
      "unconfirmed",
    ]);
    expect(container.textContent).not.toContain("Linked work no longer exists");

    await act(async () => answer([stored]));
    await flush();
    expect([chip("c1"), chip("c2"), chip("c3")]).toEqual([
      "found",
      "found",
      "missing",
    ]);
    const c2 = columnEl("backlog").querySelector("[data-kanban-card='c2']")!;
    expect(c2.textContent).toContain("Write docs");
    await act(async () =>
      c2
        .querySelector<HTMLButtonElement>(
          'button[aria-label="Open linked work"]',
        )!
        .click(),
    );
    expect(props.onOpenLinkedWork).toHaveBeenCalledWith(stored);
    // A board refresh doesn't ask again.
    act(() => emitBoardChanged("b1"));
    await flush();
    expect(lookupLinkTargets).toHaveBeenCalledTimes(1);
  });

  it("looks a link up again when the app drops a session it held", async () => {
    handlers.board_get = () => linkedFixture();
    const loaded = { kind: "session" as const, id: "s1", label: "Fix login" };
    const lookupLinkTargets = vi.fn(async () => [loaded]);
    const props = await renderBoard({ lookupLinkTargets });
    const chip = () =>
      container
        .querySelector("[data-kanban-card='c1'] [data-link-chip]")!
        .getAttribute("data-link-chip");
    expect(chip()).toBe("found");
    // The app loads the session's project, then the session is deleted.
    await act(async () =>
      root.render(
        createElement(BoardView, { ...props, linkTargets: [loaded] }),
      ),
    );
    lookupLinkTargets.mockImplementation(async () => []);
    await act(async () =>
      root.render(createElement(BoardView, { ...props, linkTargets: [] })),
    );
    await flush();
    expect(chip()).toBe("missing");
  });

  it("doesn't call linked work gone when the lookup fails", async () => {
    handlers.board_get = () => linkedFixture();
    await renderBoard({
      lookupLinkTargets: vi.fn(() => Promise.reject("session store is locked")),
    });
    const chips = [
      ...columnEl("backlog").querySelectorAll("[data-link-chip]"),
    ].map((el) => el.getAttribute("data-link-chip"));
    expect(chips).toEqual(["unconfirmed", "unconfirmed"]);
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      "Couldn't check linked work: session store is locked",
    );
  });

  it("opens the card detail with its activity timeline", async () => {
    handlers.board_activity = () => [
      {
        id: "a1",
        boardId: "b1",
        cardId: "c3",
        kind: "moved",
        actor: "system",
        fromColumn: "Backlog",
        toColumn: "In Progress",
        detail: null,
        createdAt: Date.now() - 2 * 60_000,
      },
    ];
    await renderBoard();
    const cardEl = columnEl("progress").querySelector<HTMLElement>(
      "[data-board-card='c3']",
    )!;
    act(() => cardEl.click());
    await flush();
    const dialog = document.querySelector("[data-card-drawer]")!;
    expect(dialog.textContent).toContain("Card #3");
    // The drawer asks for this card's own activity.
    expect(
      commands("board_activity").map(([, args]) => args),
    ).toContainEqual(expect.objectContaining({ boardId: "b1", cardId: "c3" }));
    await act(async () =>
      [...dialog.querySelectorAll<HTMLButtonElement>('[role="tab"]')]
        .find((tab) => tab.textContent === "Activity")!
        .click(),
    );
    expect(dialog.querySelector("[data-card-activity]")?.textContent).toBe(
      "Moved from Backlog to In Progress · System · 2m ago",
    );
  });
});

describe("BoardView card drawer", () => {
  const openCardEl = (id: string) =>
    columnEl("backlog")
      .closest("[data-app-board]")!
      .querySelector<HTMLElement>(`[data-board-card='${id}']`)!;
  const drawerEl = () =>
    document.querySelector<HTMLElement>("[data-card-drawer]");

  it("docks beside the board, which stays on screen, and swaps with another card", async () => {
    await renderBoard();
    act(() => openCardEl("c1").click());
    await flush();
    expect(drawerEl()?.getAttribute("aria-label")).toBe("Card #1");
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    // In the same layout row as the board, not on top of it.
    expect(drawerEl()!.parentElement!.contains(columnEl("backlog"))).toBe(true);
    expect(drawerEl()!.contains(columnEl("backlog"))).toBe(false);
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2"]);

    act(() => openCardEl("c2").click());
    await flush();
    expect(document.querySelectorAll("[data-card-drawer]")).toHaveLength(1);
    expect(drawerEl()?.getAttribute("aria-label")).toBe("Card #2");
  });

  it("keeps a title being typed when the board refreshes under it", async () => {
    await renderBoard();
    act(() => openCardEl("c1").click());
    await flush();
    const title = () =>
      drawerEl()!.querySelector<HTMLInputElement>('input[aria-label="Title"]')!;
    act(() => title().focus());
    act(() => {
      Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!.call(title(), "My draft");
      title().dispatchEvent(new Event("input", { bubbles: true }));
    });
    // Native renames the same card; the board-changed refresh brings it in.
    handlers.board_get = () => {
      const next = fixture();
      next.cards[0] = { ...next.cards[0], title: "Renamed elsewhere" };
      return next;
    };
    emitBoardChanged("b1");
    await flush();
    expect(columnEl("backlog").textContent).toContain("Renamed elsewhere");
    expect(title().value).toBe("My draft");
    expect(commands("board_card_update")).toHaveLength(0);
    act(() => title().blur());
    await flush();
    expect(commands("board_card_update")).toEqual([
      ["board_card_update", { cardId: "c1", title: "My draft", body: "" }],
    ]);
  });
});

describe("Board sidebar entry", () => {
  let props: ComponentProps<typeof Sidebar>;

  function Harness() {
    const enabled = useSyncExternalStore(
      subscribeBoardEnabled,
      loadBoardEnabled,
    );
    return createElement(Sidebar, {
      ...props,
      onOpenBoard: enabled ? props.onOpenBoard : undefined,
    });
  }

  const boardEntry = () =>
    [...container.querySelectorAll('nav[aria-label="Views"] button')].find(
      (button) => button.textContent?.trim().startsWith("Board"),
    );

  it("shows the Board entry by default and hides it while the setting is off", async () => {
    props = {
      cwd: "/workspace/project",
      open: true,
      inset: true,
      sessions: [],
      busySessionIds: new Set(),
      approvalSessionIds: new Set(),
      status: "idle",
      pending: false,
      tab: "sessions",
      filesSearchOpen: false,
      onSelectSession: vi.fn(),
      onOpenFile: vi.fn(),
      onTabChange: vi.fn(),
      onFilesSearchOpenChange: vi.fn(),
      onOpenAutomations: vi.fn(),
      onOpenBoard: vi.fn(),
    };
    await act(async () => root.render(createElement(Harness)));
    // On by default: the entry is there without touching Settings.
    expect(loadBoardEnabled()).toBe(true);
    const entry = boardEntry();
    expect(entry).toBeDefined();
    act(() => (entry as HTMLButtonElement).click());
    expect(props.onOpenBoard).toHaveBeenCalledTimes(1);

    await act(async () => saveBoardEnabled(false));
    expect(boardEntry()).toBeUndefined();
    // Its neighbour is still there, so the list itself rendered.
    expect(
      [...container.querySelectorAll('nav[aria-label="Views"] button')].some(
        (button) => button.textContent?.includes("Automations"),
      ),
    ).toBe(true);

    await act(async () => saveBoardEnabled(true));
    expect(boardEntry()).toBeDefined();
  });
});

describe("BoardView card face, search and filters", () => {
  const extras = (over: Partial<CardExtras> = {}): CardExtras => ({
    priority: 0,
    dueAt: null,
    projects: [],
    labels: [],
    subtasksDone: 0,
    subtasksTotal: 0,
    comments: 0,
    ...over,
  });
  const label = (id: string, name: string): BoardLabel => ({
    id,
    boardId: "b1",
    name,
    color: "e5484d",
    position: 0,
  });
  const tomorrow = () => {
    const now = new Date();
    return new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate() + 1,
    ).getTime();
  };

  /** Five cards in one column: titles tell search apart, #2 and #4 are low priority. */
  function five() {
    const titles = [
      "Alpha one",
      "Beta two",
      "Alpha three",
      "Beta four",
      "Alpha five",
    ];
    handlers.board_get = () => ({
      board: BOARD,
      columns: [
        column("backlog", "Backlog", 0, "todo"),
        column("done", "Done", 1, "done"),
      ],
      cards: titles.map((title, i) =>
        card(`c${i + 1}`, "backlog", i, i + 1, title),
      ),
    });
    handlers.board_card_extras = () => ({
      c2: extras({ priority: 1 }),
      c4: extras({ priority: 1 }),
      c1: extras({ priority: 3, labels: ["l1"] }),
    });
    handlers.board_labels = () => [label("l1", "Bug")];
  }

  const countOf = (id: string) =>
    columnEl(id).querySelector("[data-column-count]")?.textContent;

  it("card_face_shows_priority_due_labels_and_subtask_progress", async () => {
    handlers.board_card_extras = () => ({
      c1: extras({
        priority: 3,
        dueAt: tomorrow(),
        labels: ["l1"],
        subtasksDone: 1,
        subtasksTotal: 3,
      }),
    });
    handlers.board_labels = () => [label("l1", "Bug")];
    await renderBoard();
    const face = columnEl("backlog").querySelector("[data-board-card='c1']")!;
    expect(face.textContent).toContain("High");
    expect(face.querySelector("[data-card-due]")?.textContent).toBe("Tomorrow");
    expect(face.textContent).toContain("Bug");
    expect(face.textContent).toContain("1/3");
    // A card with no extras shows none of it.
    const plain = columnEl("backlog").querySelector("[data-board-card='c2']")!;
    expect(plain.querySelector("[data-card-due]")).toBeNull();
    expect(plain.textContent).not.toContain("High");
  });

  it("colours an overdue due date apart from a later one", async () => {
    const day = 86_400_000;
    handlers.board_card_extras = () => ({
      c1: extras({ dueAt: Date.now() - 3 * day }),
      c2: extras({ dueAt: Date.now() + 20 * day }),
    });
    await renderBoard();
    const tone = (id: string) =>
      columnEl("backlog")
        .querySelector(`[data-board-card='${id}'] [data-card-due]`)
        ?.getAttribute("data-card-due");
    expect(tone("c1")).toBe("overdue");
    expect(tone("c2")).toBe("later");
  });

  it("search_and_filter_show_shown_over_total_counts", async () => {
    five();
    await renderBoard();
    expect(countOf("backlog")).toBe("5");
    await search("alpha");
    expect(countOf("backlog")).toBe("3/5");
    expect(countOf("done")).toBe("0/0");
    expect(cardIdsIn("backlog")).toEqual(["c1", "c3", "c5"]);
    await search("#2");
    expect(cardIdsIn("backlog")).toEqual(["c2"]);
    await search("");
    expect(countOf("backlog")).toBe("5");

    await filterBy("Priority", "Low");
    expect(countOf("backlog")).toBe("2/5");
    expect(cardIdsIn("backlog")).toEqual(["c2", "c4"]);
    expect(container.querySelector("[data-filter-count]")?.textContent).toBe(
      "1",
    );
    // Search and filter narrow together.
    await search("beta two");
    expect(cardIdsIn("backlog")).toEqual(["c2"]);
    await search("");
    await pick("Filter", "Clear all");
    expect(countOf("backlog")).toBe("5");
    expect(container.querySelector("[data-filter-count]")).toBeNull();
  });

  it("drop_while_filtered_lands_between_the_right_cards", async () => {
    five();
    handlers.board_card_move = () => undefined;
    await renderBoard();
    await search("alpha"); // shows #1, #3, #5; hides #2 and #4
    expect(cardIdsIn("backlog")).toEqual(["c1", "c3", "c5"]);

    // #5 dropped after visible #3 (visible index 2 once #5 is lifted).
    act(() =>
      kanban.props!.onCardMove("c5", { columnId: "backlog", index: 2 }),
    );
    await flush();
    expect(commands("board_card_move").at(-1)![1]).toMatchObject({
      cardId: "c5",
      columnId: "backlog",
      index: 3,
    });
    await act(async () => emitBoardChanged("b1"));
    await flush();

    // #3 dropped before visible #1: the very top.
    act(() =>
      kanban.props!.onCardMove("c3", { columnId: "backlog", index: 0 }),
    );
    await flush();
    expect(commands("board_card_move").at(-1)![1]).toMatchObject({
      cardId: "c3",
      index: 0,
    });
    await act(async () => emitBoardChanged("b1"));
    await flush();

    // #1 dropped before visible #5: ahead of #5, after the hidden #4.
    act(() =>
      kanban.props!.onCardMove("c1", { columnId: "backlog", index: 1 }),
    );
    await flush();
    expect(commands("board_card_move").at(-1)![1]).toMatchObject({
      cardId: "c1",
      index: 3,
    });

    // Into a column whose cards are all hidden: its end.
    act(() => kanban.props!.onCardMove("c1", { columnId: "done", index: 0 }));
    await flush();
    expect(commands("board_card_move").at(-1)![1]).toMatchObject({
      columnId: "done",
      index: 0,
    });
  });

  it("drops into a column with hidden cards after the last visible one", async () => {
    five();
    handlers.board_get = () => ({
      board: BOARD,
      columns: [
        column("backlog", "Backlog", 0, "todo"),
        column("done", "Done", 1, "done"),
      ],
      cards: [
        card("c1", "backlog", 0, 1, "Alpha one"),
        card("d1", "done", 0, 6, "Alpha six"),
        card("d2", "done", 1, 7, "Beta seven"),
      ],
    });
    handlers.board_card_move = () => undefined;
    await renderBoard();
    await search("alpha");
    act(() => kanban.props!.onCardMove("c1", { columnId: "done", index: 1 }));
    await flush();
    // Visible: d1 only, so index 1 is after d1, ahead of hidden d2.
    expect(commands("board_card_move").at(-1)![1]).toMatchObject({
      columnId: "done",
      index: 1,
    });
  });

  it("filter options are checkboxes that report whether they are on", async () => {
    five();
    await renderBoard();
    await filterBy("Priority", "Low");
    await filterBy("Labels", "Bug");
    const items = [
      ...document.querySelectorAll<HTMLElement>(
        '[role="menu"] [role^="menuitem"]',
      ),
    ];
    const option = (name: string) =>
      items.find((el) => el.textContent?.trim() === name)!;
    for (const name of ["None", "Low", "High", "Bug"]) {
      expect(option(name).getAttribute("role")).toBe("menuitemcheckbox");
    }
    expect(option("Low").getAttribute("aria-checked")).toBe("true");
    expect(option("Bug").getAttribute("aria-checked")).toBe("true");
    expect(option("High").getAttribute("aria-checked")).toBe("false");
    // Clear all is an action, not a toggle.
    expect(option("Clear all").getAttribute("role")).toBe("menuitem");
  });

  it("keyboard_drop_while_filtered_lands_between_the_right_cards", async () => {
    five();
    handlers.board_card_move = () => undefined;
    await renderBoard();
    await search("alpha"); // shows #1, #3, #5; hides #2 and #4
    // happy-dom has no layout: columns 300px apart, cards stacked.
    const rect = (x: number, y: number, w: number, h: number) =>
      ({
        x,
        y,
        left: x,
        top: y,
        width: w,
        height: h,
        right: x + w,
        bottom: y + h,
        toJSON() {},
      }) as DOMRect;
    const spy = vi
      .spyOn(Element.prototype, "getBoundingClientRect")
      .mockImplementation(function (this: Element) {
        const section = this.closest("[data-kanban-column]");
        if (!section) return rect(6, 190, 268, 60); // over #5
        const x =
          [...container.querySelectorAll("[data-kanban-column]")].indexOf(
            section,
          ) * 300;
        if (this === section) return rect(x, 0, 280, 500);
        if (this.hasAttribute("data-kanban-card")) {
          const above = [...this.parentElement!.children].indexOf(this);
          return rect(x + 6, 50 + above * 70, 268, 60);
        }
        if (
          this.querySelector("[data-kanban-card]") ||
          this.matches(".overflow-y-auto")
        ) {
          return rect(x, 40, 280, 420);
        }
        return rect(x, 0, 280, 36);
      });
    try {
      const key = (target: EventTarget, code: string) =>
        act(() => {
          target.dispatchEvent(
            new KeyboardEvent("keydown", {
              code,
              key: code === "Space" ? " " : code,
              bubbles: true,
              cancelable: true,
            }),
          );
        });
      const tick = () =>
        act(() => new Promise((resolve) => setTimeout(resolve, 0)));
      const el = container.querySelector<HTMLElement>(
        "[data-kanban-card='c5']",
      )!;
      act(() => el.focus());
      key(el, "Space");
      await tick();
      key(el, "ArrowUp");
      await tick();
      key(document, "Space");
      await tick();
      await flush();
      // Over visible #3 means before #3: after the hidden #2, so full index 2.
      expect(commands("board_card_move")).toHaveLength(1);
      expect(commands("board_card_move")[0][1]).toMatchObject({
        cardId: "c5",
        columnId: "backlog",
        index: 2,
      });
    } finally {
      spy.mockRestore();
    }
  });

  it("a_deleted_label_in_the_filter_is_dropped", async () => {
    five();
    await renderBoard();
    await filterBy("Labels", "Bug");
    expect(cardIdsIn("backlog")).toEqual(["c1"]);

    // Another window deletes the label: the filter lets go instead of hiding every card.
    handlers.board_labels = () => [];
    await act(async () => emitBoardChanged("b1"));
    await flush();
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2", "c3", "c4", "c5"]);
    expect(countOf("backlog")).toBe("5");
    expect(container.querySelector("[data-filter-count]")).toBeNull();
  });

  it("filters by project from the projects prop", async () => {
    five();
    handlers.board_card_extras = () => ({
      c1: extras({ projects: ["/work/api"] }),
      c3: extras({ projects: ["/work/web"] }),
    });
    await renderBoard({
      projects: [
        { path: "/work/api", name: "api" },
        { path: "/work/web", name: "web" },
      ],
    });
    await filterBy("Projects", "api");
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    await filterBy("Projects", "web");
    expect(cardIdsIn("backlog")).toEqual(["c1", "c3"]);
  });

  it("keeps the board when extras fail to load", async () => {
    handlers.board_card_extras = () => {
      throw new Error("boom");
    };
    await renderBoard();
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2"]);
  });

  // ---- quick edits and the column menu ----
  async function columnAction(columnId: string, itemText: string) {
    act(() =>
      columnEl(columnId)
        .querySelector<HTMLButtonElement>(
          'button[aria-label^="Column actions"]',
        )!
        .click(),
    );
    await flush();
    const item = [
      ...document.querySelectorAll<HTMLElement>('[role^="menuitem"]'),
    ].find((el) => el.textContent?.trim() === itemText);
    return item;
  }
  async function runColumnAction(columnId: string, itemText: string) {
    const item = await columnAction(columnId, itemText);
    expect(item, itemText).toBeDefined();
    act(() => item!.click());
    await flush();
  }
  const menuTexts = () =>
    [...document.querySelectorAll('[role^="menuitem"]')].map((el) =>
      el.textContent?.trim(),
    );

  it("collapses a column with its current colour and WIP limit", async () => {
    five();
    const base = (handlers.board_get as () => BoardDetail)();
    handlers.board_get = () => ({
      ...base,
      columns: base.columns.map((c) =>
        c.id === "backlog" ? { ...c, color: "a371f7", wipLimit: 5 } : c,
      ),
    });
    handlers.board_column_set_display = () => undefined;
    await renderBoard();
    await runColumnAction("backlog", "Collapse column");
    expect(commands("board_column_set_display")).toEqual([
      [
        "board_column_set_display",
        { columnId: "backlog", color: "a371f7", wipLimit: 5, collapsed: true },
      ],
    ]);
  });

  it("shows a collapsed column as a strip and expands it on click", async () => {
    five();
    const base = (handlers.board_get as () => BoardDetail)();
    handlers.board_get = () => ({
      ...base,
      columns: base.columns.map((c) =>
        c.id === "backlog"
          ? { ...c, color: "a371f7", wipLimit: 5, collapsed: true }
          : c,
      ),
    });
    handlers.board_column_set_display = () => undefined;
    await renderBoard();
    expect(columnEl("backlog").hasAttribute("data-column-collapsed")).toBe(
      true,
    );
    expect(cardIdsIn("backlog")).toEqual([]);
    act(() =>
      columnEl("backlog").querySelector<HTMLButtonElement>("button")!.click(),
    );
    await flush();
    expect(commands("board_column_set_display")[0][1]).toEqual({
      columnId: "backlog",
      color: "a371f7",
      wipLimit: 5,
      collapsed: false,
    });
  });

  it("drops a card onto a collapsed column at its end", async () => {
    handlers.board_get = () => ({
      board: BOARD,
      columns: [
        column("backlog", "Backlog", 0, "todo"),
        { ...column("progress", "In Progress", 1, "doing"), collapsed: true },
      ],
      cards: [
        card("c1", "backlog", 0, 1, "One"),
        card("c2", "progress", 0, 2, "Two"),
        card("c3", "progress", 1, 3, "Three"),
      ],
    });
    handlers.board_card_move = () => undefined;
    await renderBoard();
    act(() =>
      kanban.props!.onCardMove("c1", { columnId: "progress", index: 2 }),
    );
    await flush();
    expect(commands("board_card_move")[0][1]).toMatchObject({
      cardId: "c1",
      columnId: "progress",
      index: 2,
    });
  });

  it("sorts the whole column by priority while a search hides some cards", async () => {
    five();
    handlers.board_cards_move = () => undefined;
    await renderBoard();
    await search("alpha");
    expect(cardIdsIn("backlog")).toEqual(["c1", "c3", "c5"]);
    await runColumnAction("backlog", "Sort by priority");
    // c1 is High, c2 and c4 Low (keeping their order), c3 and c5 None.
    expect(commands("board_cards_move")).toEqual([
      [
        "board_cards_move",
        {
          cardIds: ["c1", "c2", "c4", "c3", "c5"],
          columnId: "backlog",
          index: 0,
          actor: "user",
        },
      ],
    ]);
  });

  it("sorts by due date with the soonest first and undated last", async () => {
    five();
    handlers.board_card_extras = () => ({
      c3: extras({ dueAt: 2_000 }),
      c5: extras({ dueAt: 1_000 }),
    });
    handlers.board_cards_move = () => undefined;
    await renderBoard();
    await runColumnAction("backlog", "Sort by due date");
    expect(commands("board_cards_move")[0][1]).toMatchObject({
      cardIds: ["c5", "c3", "c1", "c2", "c4"],
      columnId: "backlog",
      index: 0,
    });
  });

  it("doesn't write when a sort would change nothing", async () => {
    five();
    handlers.board_cards_move = () => undefined;
    await renderBoard();
    // No card has a due date: the order is already the sorted one.
    await runColumnAction("backlog", "Sort by due date");
    expect(commands("board_cards_move")).toHaveLength(0);
  });

  it("marks every card of a column done, including hidden ones, at the end of Done", async () => {
    five();
    const base = (handlers.board_get as () => BoardDetail)();
    handlers.board_get = () => ({
      ...base,
      cards: [...base.cards, card("d1", "done", 0, 9, "Shipped")],
    });
    handlers.board_cards_move = () => undefined;
    await renderBoard();
    await search("alpha");
    await runColumnAction("backlog", "Mark all as done");
    expect(commands("board_cards_move")).toEqual([
      [
        "board_cards_move",
        {
          cardIds: ["c1", "c2", "c3", "c4", "c5"],
          columnId: "done",
          index: 1,
          actor: "user",
        },
      ],
    ]);
  });

  it("hides Mark all as done in the done column and when no column is done", async () => {
    five();
    await renderBoard();
    expect(await columnAction("done", "Mark all as done")).toBeUndefined();
    expect(menuTexts()).toContain("Archive completed");
    press(document.querySelector<HTMLElement>('[role="menu"]')!, "Escape");
    act(() => document.body.dispatchEvent(new PointerEvent("pointerdown")));
    await flush();
    expect(await columnAction("backlog", "Mark all as done")).toBeDefined();
    expect(menuTexts()).not.toContain("Archive completed");

    act(() => root.unmount());
    root = createRoot(container);
    handlers.board_get = () => ({
      board: BOARD,
      columns: [column("backlog", "Backlog", 0, "todo")],
      cards: [card("c1", "backlog", 0, 1, "One")],
    });
    await renderBoard();
    expect(await columnAction("backlog", "Mark all as done")).toBeUndefined();
  });

  it("archives the completed cards of the done column only", async () => {
    five();
    const base = (handlers.board_get as () => BoardDetail)();
    handlers.board_get = () => ({
      ...base,
      cards: [
        ...base.cards,
        card("d1", "done", 0, 8, "Shipped"),
        card("d2", "done", 1, 9, "Released"),
      ],
    });
    handlers.board_cards_archive = () => undefined;
    await renderBoard();
    await runColumnAction("done", "Archive completed");
    expect(commands("board_cards_archive")).toEqual([
      ["board_cards_archive", { cardIds: ["d1", "d2"] }],
    ]);
  });

  describe("face quick edits", () => {
    const face = (id: string) =>
      columnEl("backlog").querySelector<HTMLElement>(
        `[data-board-card='${id}']`,
      )!;
    const trigger = (id: string, name: string) =>
      face(id).querySelector<HTMLButtonElement>(
        `button[aria-label^="${name}"]`,
      )!;
    async function chooseOnFace(id: string, name: string, itemText: string) {
      act(() => trigger(id, name).click());
      const item = [
        ...document.querySelectorAll<HTMLElement>('[role^="menuitem"]'),
      ].find((el) => el.textContent?.trim() === itemText)!;
      expect(item, itemText).toBeDefined();
      act(() => item.click());
      await flush();
    }

    it("sets the priority from the face without opening the card", async () => {
      five();
      handlers.board_card_set_priority = () => undefined;
      await renderBoard();
      await chooseOnFace("c1", "Priority of #1", "Urgent");
      expect(commands("board_card_set_priority")).toEqual([
        ["board_card_set_priority", { cardId: "c1", priority: 4 }],
      ]);
      expect(document.querySelector("[data-card-drawer]")).toBeNull();
    });

    it("sets the priority of an unprioritised card from its hover target", async () => {
      five();
      handlers.board_card_set_priority = () => undefined;
      await renderBoard();
      expect(
        face("c3").querySelector("[data-card-priority-target]"),
      ).not.toBeNull();
      expect(
        face("c1").querySelector("[data-card-priority-target]"),
      ).toBeNull();
      await chooseOnFace("c3", "Priority of #3", "Medium");
      expect(commands("board_card_set_priority")[0][1]).toEqual({
        cardId: "c3",
        priority: 2,
      });
      expect(document.querySelector("[data-card-drawer]")).toBeNull();
    });

    it("sets and clears the due date from the face", async () => {
      five();
      handlers.board_card_extras = () => ({
        c1: extras({ dueAt: tomorrow() }),
      });
      handlers.board_card_set_due = () => undefined;
      await renderBoard();
      await chooseOnFace("c1", "Due date of #1", "Clear");
      expect(commands("board_card_set_due")).toEqual([
        ["board_card_set_due", { cardId: "c1", dueAt: null }],
      ]);
      await chooseOnFace("c1", "Due date of #1", "Tomorrow");
      expect(commands("board_card_set_due")[1][1]).toEqual({
        cardId: "c1",
        dueAt: tomorrow(),
      });
      expect(document.querySelector("[data-card-drawer]")).toBeNull();
    });

    it("toggles a label from the face, keeping the card's other labels", async () => {
      five();
      handlers.board_labels = () => [label("l1", "Bug"), label("l2", "Docs")];
      handlers.board_card_extras = () => ({ c1: extras({ labels: ["l1"] }) });
      handlers.board_card_set_labels = () => undefined;
      await renderBoard();
      await chooseOnFace("c1", "Labels of #1", "Docs");
      expect(commands("board_card_set_labels")).toEqual([
        ["board_card_set_labels", { cardId: "c1", labelIds: ["l1", "l2"] }],
      ]);
      expect(document.querySelector("[data-card-drawer]")).toBeNull();
    });
  });
});

describe("BoardView undo toasts, held deletes and the Archive view", () => {
  const OTHER = { id: "b2", name: "Releases", createdAt: 2, updatedAt: 2 };

  /**
   * A board as the backend holds it: live positions stay 0..n-1, archived
   * cards keep their column, and every write emits board-changed.
   */
  function backend(detail: BoardDetail = fixture()) {
    const columns = detail.columns.map((c) => ({ ...c }));
    const cards = detail.cards.map((c) => ({ ...c }));
    const archived = new Map<string, number>();
    let clock = 0;
    const changed = () =>
      void Promise.resolve().then(() => emitBoardChanged("b1"));
    const live = (columnId: string) =>
      cards
        .filter((c) => c.columnId === columnId && !archived.has(c.id))
        .sort((a, b) => a.position - b.position);
    const renumber = (columnId: string, ids: string[]) =>
      ids.forEach((id, index) => {
        cards.find((c) => c.id === id)!.position = index;
      });
    const find = (id: unknown) => {
      const found = cards.find((c) => c.id === id);
      if (!found) throw "Card not found";
      return found;
    };
    const column = (id: unknown) => {
      if (!columns.some((c) => c.id === id)) throw "Column not found";
      return String(id);
    };
    const place = (id: string, columnId: string, index: number) => {
      const moving = find(id);
      const from = moving.columnId;
      moving.columnId = columnId;
      archived.delete(id);
      const ids = live(columnId)
        .map((c) => c.id)
        .filter((other) => other !== id);
      ids.splice(Math.min(index, ids.length), 0, id);
      renumber(columnId, ids);
      if (from !== columnId)
        renumber(
          from,
          live(from).map((c) => c.id),
        );
    };
    const archive = (id: string) => {
      const card = find(id);
      archived.set(id, ++clock);
      renumber(
        card.columnId,
        live(card.columnId).map((c) => c.id),
      );
    };
    const remove = (id: string) => {
      const index = cards.findIndex((c) => c.id === id);
      if (index < 0) throw "Card not found";
      const [gone] = cards.splice(index, 1);
      archived.delete(id);
      renumber(
        gone.columnId,
        live(gone.columnId).map((c) => c.id),
      );
    };
    handlers.board_list = () => [BOARD, OTHER];
    handlers.board_get = (args) =>
      args.boardId === "b2"
        ? {
            board: OTHER,
            columns: [{ ...column_("todo2"), boardId: "b2" }],
            cards: [],
          }
        : {
            board: BOARD,
            columns: columns.map((c) => ({ ...c })),
            cards: cards
              .filter((c) => !archived.has(c.id))
              .map((c) => ({ ...c })),
          };
    handlers.board_card_move = (a) => {
      find(a.cardId);
      place(String(a.cardId), column(a.columnId), Number(a.index));
      changed();
    };
    handlers.board_cards_move = (a) => {
      const ids = a.cardIds as string[];
      ids.forEach(find);
      const target = column(a.columnId);
      ids.forEach((id, offset) => place(id, target, Number(a.index) + offset));
      changed();
    };
    handlers.board_card_archive = (a) => {
      archive(String(a.cardId));
      changed();
    };
    handlers.board_cards_archive = (a) => {
      (a.cardIds as string[]).forEach(archive);
      changed();
    };
    handlers.board_card_restore = (a) => {
      find(a.cardId);
      place(String(a.cardId), column(a.columnId), Number(a.index));
      changed();
    };
    handlers.board_card_unarchive = (a) => {
      const card = find(a.cardId);
      place(card.id, card.columnId, Number.MAX_SAFE_INTEGER);
      changed();
    };
    handlers.board_archived_cards = () =>
      [...archived.entries()]
        .sort((a, b) => b[1] - a[1])
        .map(([id]) => ({ ...find(id) }));
    handlers.board_card_delete = (a) => {
      remove(String(a.cardId));
      changed();
    };
    handlers.board_card_delete_archived = (a) => {
      if (!archived.has(String(a.cardId))) return;
      remove(String(a.cardId));
      changed();
    };
    handlers.board_column_card_count = (a) => ({
      live: live(String(a.columnId)).length,
      archived: cards.filter(
        (c) => c.columnId === a.columnId && archived.has(c.id),
      ).length,
    });
    handlers.board_column_delete = (a) => {
      const index = columns.findIndex((c) => c.id === a.columnId);
      columns.splice(index, 1);
      changed();
    };
    return {
      /** Live card ids of a column, in order, as the backend holds them. */
      ids: (columnId: string) => live(columnId).map((c) => c.id),
      archivedIds: () => [...archived.keys()],
      exists: (id: string) => cards.some((c) => c.id === id),
      /** The other app or an agent changing the board. */
      deleteElsewhere: remove,
      archiveElsewhere: archive,
      unarchiveElsewhere: (id: string) => place(id, find(id).columnId, 99),
      deleteColumnElsewhere: (id: string) => {
        columns.splice(
          columns.findIndex((c) => c.id === id),
          1,
        );
        for (const c of cards.filter((c) => c.columnId === id)) remove(c.id);
      },
    };
  }
  const column_ = (id: string) => column(id, "Todo", 0, "todo");

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  });
  afterEach(() => vi.useRealTimers());

  const toastEl = () =>
    document.querySelector<HTMLElement>("[data-board-undo-toast]");
  const toastText = () => toastEl()?.querySelector("span")?.textContent ?? null;
  const undoButton = () =>
    [...(toastEl()?.querySelectorAll("button") ?? [])].find(
      (b) => b.textContent === "Undo",
    );
  async function clickUndo() {
    expect(undoButton(), "Undo button").toBeDefined();
    await act(async () => undoButton()!.click());
    await flush();
    await flush();
  }
  async function advance(ms: number) {
    await act(async () => {
      vi.advanceTimersByTime(ms);
    });
    await flush();
  }
  async function drop(cardId: string, columnId: string, index: number) {
    act(() => kanban.props!.onCardMove(cardId, { columnId, index }));
    await flush();
    await flush();
  }
  async function deleteFromMenu(number: number) {
    const menu = container.querySelector<HTMLButtonElement>(
      `button[aria-label="Card actions for #${number}"]`,
    )!;
    act(() => menu.click());
    const item = [
      ...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
    ].find((el) => el.textContent === "Delete")!;
    await act(async () => item.click());
    await flush();
    await flush();
  }
  async function deleteFromDrawer(id: string) {
    act(() =>
      container
        .querySelector<HTMLElement>(`[data-board-card='${id}']`)!
        .click(),
    );
    await flush();
    const button = [
      ...document.querySelectorAll<HTMLButtonElement>(
        "[data-card-drawer] button",
      ),
    ].find((b) => b.textContent?.includes("Delete card"))!;
    await act(async () => button.click());
    await flush();
    await flush();
  }
  async function columnAction(columnId: string, itemText: string) {
    act(() =>
      columnEl(columnId)
        .querySelector<HTMLButtonElement>(
          'button[aria-label^="Column actions"]',
        )!
        .click(),
    );
    await flush();
    const item = [
      ...document.querySelectorAll<HTMLButtonElement>('[role^="menuitem"]'),
    ].find((el) => el.textContent?.trim() === itemText);
    expect(item, itemText).toBeDefined();
    return item!;
  }
  async function runColumnAction(columnId: string, itemText: string) {
    const item = await columnAction(columnId, itemText);
    act(() => item.click());
    await flush();
    await flush();
  }
  async function toggleArchive() {
    const button = [...container.querySelectorAll("button")].find(
      (b) => b.textContent === "Archive",
    )!;
    act(() => button.click());
    await flush();
    await flush();
  }
  const archiveEl = () =>
    document.querySelector<HTMLElement>("[data-board-archive]");
  const listed = () =>
    [...(archiveEl()?.querySelectorAll("[data-archived-card]") ?? [])].map(
      (el) => el.getAttribute("data-archived-card"),
    );

  it("a drop shows 'Moved #N to {column}' for 8 seconds and Undo moves it back", async () => {
    const db = backend();
    await renderBoard();
    await drop("c1", "review", 0);
    expect(db.ids("review")).toEqual(["c1"]);
    expect(toastText()).toBe("Moved #1 to Review");
    expect(toastEl()!.getAttribute("role")).toBe("status");
    expect((toastEl()!.parentElement as HTMLElement).style.zIndex).toBe(
      String(LAYER.toast),
    );

    await clickUndo();
    expect(commands("board_card_move").at(-1)![1]).toEqual({
      cardId: "c1",
      columnId: "backlog",
      index: 0,
      actor: "user",
    });
    expect(db.ids("backlog")).toEqual(["c1", "c2"]);
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2"]);
    expect(toastEl()).toBeNull();

    // A reorder inside the column (the keyboard sensor's drop) is undoable too.
    await drop("c2", "backlog", 0);
    expect(toastText()).toBe("Moved #2 to Backlog");
    await clickUndo();
    expect(db.ids("backlog")).toEqual(["c1", "c2"]);

    // Left alone, the toast goes after 8 seconds.
    await drop("c3", "done", 0);
    await advance(7_900);
    expect(toastText()).toBe("Moved #3 to Done");
    await advance(200);
    expect(toastEl()).toBeNull();
    expect(db.ids("done")).toEqual(["c3"]);
  });

  it("a drop onto the same place shows no toast", async () => {
    backend();
    await renderBoard();
    await drop("c2", "backlog", 1);
    expect(toastEl()).toBeNull();
  });

  it("mark all done undo puts every card back at its old index", async () => {
    const base = fixture();
    const db = backend({
      ...base,
      cards: [...base.cards, card("x", "done", 0, 9, "Shipped")],
    });
    await renderBoard();
    await runColumnAction("backlog", "Mark all as done");
    expect(db.ids("done")).toEqual(["x", "c1", "c2"]);
    expect(toastText()).toBe("Moved 2 cards to Done");
    await clickUndo();
    // One card at a time, lowest index first, so each lands where it was.
    expect(
      commands("board_card_move").map(([, a]) => [
        (a as { cardId: string }).cardId,
        (a as { index: number }).index,
      ]),
    ).toEqual([
      ["c1", 0],
      ["c2", 1],
    ]);
    expect(db.ids("backlog")).toEqual(["c1", "c2"]);
    expect(db.ids("done")).toEqual(["x"]);
  });

  it("sort undo restores the exact previous order of the whole column", async () => {
    const db = backend();
    handlers.board_card_extras = () => ({
      c2: {
        priority: 4,
        dueAt: null,
        projects: [],
        labels: [],
        subtasksDone: 0,
        subtasksTotal: 0,
      },
    });
    await renderBoard();
    await runColumnAction("backlog", "Sort by priority");
    expect(db.ids("backlog")).toEqual(["c2", "c1"]);
    expect(toastText()).toBe("Sorted Backlog");
    await clickUndo();
    expect(commands("board_cards_move").at(-1)![1]).toEqual({
      cardIds: ["c1", "c2"],
      columnId: "backlog",
      index: 0,
      actor: "user",
    });
    expect(db.ids("backlog")).toEqual(["c1", "c2"]);
  });

  it("delete from the card menu never asks, hides the card and deletes it when the toast expires", async () => {
    const db = backend();
    await renderBoard();
    await deleteFromMenu(2);
    expect(ask).not.toHaveBeenCalled();
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    expect(toastText()).toBe("Deleted #2");
    // Held: archived, not deleted, while the toast shows.
    expect(db.archivedIds()).toEqual(["c2"]);
    await advance(7_999);
    expect(commands("board_card_delete_archived")).toHaveLength(0);
    expect(db.exists("c2")).toBe(true);
    await advance(1);
    expect(commands("board_card_delete_archived")).toEqual([
      ["board_card_delete_archived", { cardId: "c2" }],
    ]);
    expect(db.exists("c2")).toBe(false);
    expect(commands("board_card_delete")).toHaveLength(0);
    expect(toastEl()).toBeNull();
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
  });

  function rightClick(id: string) {
    act(() => {
      container
        .querySelector<HTMLElement>(`[data-board-card='${id}']`)!
        .dispatchEvent(
          new MouseEvent("contextmenu", {
            bubbles: true,
            cancelable: true,
            clientX: 40,
            clientY: 60,
          }),
        );
    });
  }
  const moreItem = (text: string) =>
    [...document.querySelectorAll<HTMLButtonElement>('[role="menu"] button')].find(
      (el) => el.textContent === text,
    )!;

  it("right-click on a card opens the More menu at the pointer, in order", async () => {
    backend();
    await renderBoard();
    rightClick("c2");
    const menu = document.querySelector<HTMLElement>(
      '[role="menu"][aria-label="More actions for #2"]',
    )!;
    expect(menu).not.toBeNull();
    expect(
      [...menu.querySelectorAll("button")].map((el) => el.textContent),
    ).toEqual([
      "Open",
      "Status…",
      "Priority…",
      "Labels…",
      "Move to board…",
      "Copy link",
      "Copy #2 Write docs",
      "Duplicate",
      "Archive",
      "Delete",
    ]);
    // A right-click is not a click: the card did not open.
    expect(document.querySelector("[data-card-drawer]")).toBeNull();
  });

  it("the card's … menu has More… and it opens the same menu", async () => {
    backend();
    await renderBoard();
    act(() =>
      container
        .querySelector<HTMLButtonElement>(
          'button[aria-label="Card actions for #2"]',
        )!
        .click(),
    );
    await act(async () => moreItem("More…").click());
    expect(moreItem("Copy #2 Write docs")).toBeDefined();
    expect(moreItem("Duplicate")).toBeDefined();
  });

  it("Archive from the More menu shows an Archived toast and Undo restores it", async () => {
    const db = backend();
    await renderBoard();
    rightClick("c2");
    await act(async () => moreItem("Archive").click());
    await flush();
    await flush();
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    expect(db.archivedIds()).toEqual(["c2"]);
    expect(toastText()).toBe("Archived #2");
    await clickUndo();
    expect(commands("board_card_restore")).toEqual([
      ["board_card_restore", { cardId: "c2", columnId: "backlog", index: 1 }],
    ]);
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2"]);
  });

  it("Delete from the More menu is held: toast, then deleted when it expires", async () => {
    const db = backend();
    await renderBoard();
    rightClick("c2");
    await act(async () => moreItem("Delete").click());
    await flush();
    await flush();
    expect(ask).not.toHaveBeenCalled();
    expect(toastText()).toBe("Deleted #2");
    expect(db.archivedIds()).toEqual(["c2"]);
    expect(commands("board_card_delete_archived")).toHaveLength(0);
    await advance(8_000);
    expect(db.exists("c2")).toBe(false);
  });

  it("Duplicate and Move to board run the store commands and say so", async () => {
    backend();
    handlers.board_list = () => [
      BOARD,
      { id: "b2", name: "Side", createdAt: 1, updatedAt: 1 },
    ];
    handlers.board_card_duplicate = (a) => ({
      ...card("c9", "backlog", 1, 9, "Write docs (copy)"),
      id: String(a.cardId) + "-copy",
    });
    handlers.board_card_move_to_board = () => ({
      ...card("c2", "side-todo", 0, 5, "Write docs"),
      boardId: "b2",
    });
    await renderBoard();
    rightClick("c2");
    await act(async () => moreItem("Duplicate").click());
    await flush();
    expect(commands("board_card_duplicate")).toEqual([
      ["board_card_duplicate", { cardId: "c2" }],
    ]);
    expect(toastText()).toBe("Duplicated #2 as #9");
    rightClick("c1");
    await act(async () => moreItem("Move to board…").click());
    expect(
      [...document.querySelectorAll('[role="menu"] button')].map(
        (el) => el.textContent,
      ),
    ).toEqual(["Back", "Side"]);
    await act(async () => moreItem("Side").click());
    await flush();
    expect(commands("board_card_move_to_board")).toEqual([
      ["board_card_move_to_board", { cardId: "c1", boardId: "b2" }],
    ]);
    expect(toastText()).toBe("Moved #1 to Side as #5");
  });

  it("undo of a delete from the drawer restores the card at its index", async () => {
    const db = backend();
    await renderBoard();
    await deleteFromDrawer("c1");
    expect(ask).not.toHaveBeenCalled();
    expect(document.querySelector("[data-card-drawer]")).toBeNull();
    expect(cardIdsIn("backlog")).toEqual(["c2"]);
    expect(toastText()).toBe("Deleted #1");
    await clickUndo();
    expect(commands("board_card_restore")).toEqual([
      ["board_card_restore", { cardId: "c1", columnId: "backlog", index: 0 }],
    ]);
    expect(db.ids("backlog")).toEqual(["c1", "c2"]);
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2"]);
    // The undone delete's timer deletes nothing.
    await advance(10_000);
    expect(commands("board_card_delete_archived")).toHaveLength(0);
    expect(db.exists("c1")).toBe(true);
  });

  it("a new undoable action replaces the toast and commits the pending delete", async () => {
    const db = backend();
    await renderBoard();
    await deleteFromMenu(1);
    expect(db.exists("c1")).toBe(true);
    await drop("c3", "done", 0);
    expect(commands("board_card_delete_archived")).toEqual([
      ["board_card_delete_archived", { cardId: "c1" }],
    ]);
    expect(db.exists("c1")).toBe(false);
    expect(toastText()).toBe("Moved #3 to Done");
    // Undo now only undoes the move.
    await clickUndo();
    expect(db.ids("progress")).toEqual(["c3"]);
    expect(db.exists("c1")).toBe(false);
    expect(commands("board_card_restore")).toHaveLength(0);
  });

  it("a held delete restored elsewhere is never deleted", async () => {
    const db = backend();
    await renderBoard();
    await deleteFromMenu(1);
    db.unarchiveElsewhere("c1");
    await advance(8_000);
    // The commit is the atomic store op: it leaves a live card alone.
    expect(commands("board_card_delete_archived")).toHaveLength(1);
    expect(db.exists("c1")).toBe(true);
    expect(document.querySelector('[role="alert"]')).toBeNull();
  });

  it("switching boards commits a pending delete", async () => {
    const db = backend();
    await renderBoard();
    await deleteFromMenu(1);
    act(() =>
      container
        .querySelector<HTMLButtonElement>('button[aria-label^="Board:"]')!
        .click(),
    );
    const option = [
      ...document.querySelectorAll<HTMLButtonElement>('[role="option"]'),
    ].find((el) => el.textContent?.includes("Releases"))!;
    await act(async () => option.click());
    await flush();
    await flush();
    expect(commands("board_card_delete_archived")).toEqual([
      ["board_card_delete_archived", { cardId: "c1" }],
    ]);
    expect(db.exists("c1")).toBe(false);
    expect(toastEl()).toBeNull();
  });

  it("a delete that finishes after a board switch is carried out without a toast", async () => {
    const db = backend();
    const archive = handlers.board_card_archive;
    let finish!: () => void;
    handlers.board_card_archive = (a) =>
      new Promise<void>((resolve) => {
        finish = () => {
          archive(a);
          resolve();
        };
      });
    await renderBoard();
    await deleteFromMenu(1);
    act(() =>
      container
        .querySelector<HTMLButtonElement>('button[aria-label^="Board:"]')!
        .click(),
    );
    const option = [
      ...document.querySelectorAll<HTMLButtonElement>('[role="option"]'),
    ].find((el) => el.textContent?.includes("Releases"))!;
    await act(async () => option.click());
    await flush();
    await act(async () => finish());
    await flush();
    expect(toastEl()).toBeNull();
    expect(commands("board_card_delete_archived")).toEqual([
      ["board_card_delete_archived", { cardId: "c1" }],
    ]);
    expect(db.exists("c1")).toBe(false);
  });

  it("quitting_with_a_pending_delete_leaves_the_card_archived", async () => {
    const db = backend();
    await renderBoard();
    await deleteFromMenu(1);
    act(() => root.unmount());
    root = createRoot(container);
    await advance(10_000);
    expect(commands("board_card_delete_archived")).toHaveLength(0);
    expect(commands("board_card_delete")).toHaveLength(0);
    expect(db.exists("c1")).toBe(true);
    expect(db.archivedIds()).toEqual(["c1"]);
  });

  it("an archive that finishes after unmount shows no toast and deletes nothing", async () => {
    const db = backend();
    const archive = handlers.board_card_archive;
    let finish!: () => void;
    handlers.board_card_archive = (a) =>
      new Promise<void>((resolve) => {
        finish = () => {
          archive(a);
          resolve();
        };
      });
    await renderBoard();
    await deleteFromMenu(1);
    act(() => root.unmount());
    root = createRoot(container);
    await act(async () => finish());
    await flush();
    await advance(10_000);
    expect(commands("board_card_delete_archived")).toHaveLength(0);
    expect(db.archivedIds()).toEqual(["c1"]);
  });

  it("a failed undo never cuts short a newer delete's window", async () => {
    const db = backend();
    await renderBoard();
    await drop("c1", "done", 0);
    // The undo's move is held, then fails, after card #2 was deleted.
    let fail!: () => void;
    handlers.board_card_move = () =>
      new Promise<void>((_, reject) => {
        fail = () => reject("Card not found");
      });
    act(() => undoButton()!.click());
    await flush();
    await deleteFromMenu(2);
    expect(toastText()).toBe("Deleted #2");
    await act(async () => fail());
    await flush();
    await flush();
    // The delete's toast stays, Undo and all; the failure goes to the error toast.
    expect(toastText()).toBe("Deleted #2");
    expect(undoButton()).toBeDefined();
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      "Couldn't undo: the card was deleted",
    );
    expect(commands("board_card_delete_archived")).toHaveLength(0);
    await advance(7_900);
    expect(commands("board_card_delete_archived")).toHaveLength(0);
    expect(db.exists("c2")).toBe(true);
    await advance(200);
    expect(commands("board_card_delete_archived")).toEqual([
      ["board_card_delete_archived", { cardId: "c2" }],
    ]);
  });

  it("a drop back onto the same place leaves a held delete held", async () => {
    const db = backend();
    await renderBoard();
    await deleteFromMenu(1);
    await drop("c2", "backlog", 0);
    expect(commands("board_card_delete_archived")).toHaveLength(0);
    expect(toastText()).toBe("Deleted #1");
    expect(db.archivedIds()).toEqual(["c1"]);
  });

  it("undo after the card was deleted elsewhere says so; a bulk undo restores what it can", async () => {
    const db = backend();
    await renderBoard();
    await drop("c1", "done", 0);
    db.deleteElsewhere("c1");
    await clickUndo();
    expect(toastText()).toBe("Couldn't undo: the card was deleted");
    expect(undoButton()).toBeUndefined();
    expect(db.ids("backlog")).toEqual(["c2"]);

    await drop("c2", "progress", 1);
    await runColumnAction("progress", "Mark all as done");
    expect(toastText()).toBe("Moved 2 cards to Done");
    db.deleteElsewhere("c3");
    await clickUndo();
    expect(toastText()).toBe("Couldn't undo: the card was deleted");
    expect(db.ids("progress")).toEqual(["c2"]);
    expect(db.ids("done")).toEqual([]);
    // The message goes like any toast.
    await advance(8_000);
    expect(toastEl()).toBeNull();
  });

  it("undo after the column was deleted elsewhere says so", async () => {
    const db = backend();
    await renderBoard();
    await drop("c3", "done", 0);
    db.deleteColumnElsewhere("progress");
    await clickUndo();
    expect(toastText()).toBe("Couldn't undo: the column was deleted");
    expect(db.ids("done")).toEqual(["c3"]);
  });

  it("archive completed shows 'Archived N cards' and undo restores them in place", async () => {
    const base = fixture();
    const db = backend({
      ...base,
      cards: [
        ...base.cards,
        card("d1", "done", 0, 8, "Shipped"),
        card("d2", "done", 1, 9, "Released"),
      ],
    });
    await renderBoard();
    await runColumnAction("done", "Archive completed");
    expect(toastText()).toBe("Archived 2 cards");
    expect(db.ids("done")).toEqual([]);
    await clickUndo();
    expect(commands("board_card_restore").map(([, a]) => a)).toEqual([
      { cardId: "d1", columnId: "done", index: 0 },
      { cardId: "d2", columnId: "done", index: 1 },
    ]);
    expect(db.ids("done")).toEqual(["d1", "d2"]);
    expect(db.archivedIds()).toEqual([]);
  });

  it("the Archive view lists newest first, restores, and deletes forever after asking", async () => {
    const db = backend();
    db.archiveElsewhere("c2");
    db.archiveElsewhere("c1");
    await renderBoard();
    await toggleArchive();
    expect(archiveEl()!.textContent).toContain("Archive");
    expect(listed()).toEqual(["c1", "c2"]);

    // Restore puts it back at the end of its column.
    const row = (id: string) =>
      archiveEl()!.querySelector<HTMLElement>(`[data-archived-card='${id}']`)!;
    const rowButton = (id: string, text: string) =>
      [...row(id).querySelectorAll("button")].find(
        (b) => b.textContent === text,
      )!;
    await act(async () => rowButton("c1", "Restore").click());
    await flush();
    await flush();
    expect(commands("board_card_unarchive")).toEqual([
      ["board_card_unarchive", { cardId: "c1" }],
    ]);
    expect(listed()).toEqual(["c2"]);

    // Delete forever asks first; Cancel keeps it.
    vi.mocked(ask).mockResolvedValue(false);
    await act(async () => rowButton("c2", "Delete forever").click());
    await flush();
    expect(ask).toHaveBeenCalledTimes(1);
    expect(vi.mocked(ask).mock.calls[0][0]).toContain("#2");
    expect(commands("board_card_delete")).toHaveLength(0);
    vi.mocked(ask).mockResolvedValue(true);
    await act(async () => rowButton("c2", "Delete forever").click());
    await flush();
    await flush();
    expect(commands("board_card_delete_archived")).toEqual([
      ["board_card_delete_archived", { cardId: "c2" }],
    ]);
    expect(db.exists("c2")).toBe(false);
    expect(archiveEl()!.textContent).toContain("No archived cards");

    // The header button closes it again.
    await toggleArchive();
    expect(archiveEl()).toBeNull();
  });

  it("a pending delete is not listed in the Archive view", async () => {
    const db = backend();
    db.archiveElsewhere("c3");
    await renderBoard();
    await deleteFromMenu(1);
    await toggleArchive();
    expect(db.archivedIds().sort()).toEqual(["c1", "c3"]);
    expect(listed()).toEqual(["c3"]);
    // Undo brings it back to the board, still not into the archive.
    await clickUndo();
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2"]);
    expect(listed()).toEqual(["c3"]);
  });

  it("a held delete doesn't block deleting its column and is carried out first", async () => {
    const db = backend({
      ...fixture(),
      cards: [card("only", "review", 0, 5, "Only")],
    });
    await renderBoard();
    await deleteFromMenu(5);
    const item = await columnAction("review", "Delete column");
    await flush();
    expect(item.disabled).toBe(false);
    act(() => item.click());
    await flush();
    await flush();
    const order = vi
      .mocked(invoke)
      .mock.calls.map(([cmd]) => cmd)
      .filter(
        (cmd) =>
          cmd === "board_card_delete_archived" || cmd === "board_column_delete",
      );
    expect(order).toEqual([
      "board_card_delete_archived",
      "board_column_delete",
    ]);
    expect(db.exists("only")).toBe(false);
  });

  it("the error toast still shows beside the undo toast", async () => {
    backend();
    handlers.board_column_set_display = () => {
      throw "database is locked";
    };
    await renderBoard();
    await drop("c1", "done", 0);
    // Collapsing isn't undoable, so it leaves the undo toast up.
    await runColumnAction("backlog", "Collapse column");
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      "Couldn't collapse the column: database is locked",
    );
    expect(toastText()).toBe("Moved #1 to Done");
  });
});

describe("BoardView full page", () => {
  const cardEl = (id: string) =>
    container.querySelector<HTMLElement>(`[data-board-card='${id}']`)!;
  const drawerEl = () =>
    document.querySelector<HTMLElement>("[data-card-drawer]");
  const boardArea = () => columnEl("backlog").closest<HTMLElement>(".hidden");
  const press = (key: string) =>
    act(async () => {
      drawerEl()!
        .querySelector("input")!
        .dispatchEvent(
          new KeyboardEvent("keydown", {
            key,
            bubbles: true,
            cancelable: true,
          }),
        );
    });

  it("the toggle expands the card over the board and Escape returns to the drawer", async () => {
    await renderBoard();
    act(() => cardEl("c1").click());
    await flush();
    expect(drawerEl()!.hasAttribute("data-full-page")).toBe(false);
    expect(boardArea()).toBeNull();

    await act(async () =>
      drawerEl()!
        .querySelector<HTMLButtonElement>(
          'button[aria-label="Open full page"]',
        )!
        .click(),
    );
    expect(drawerEl()!.hasAttribute("data-full-page")).toBe(true);
    expect(boardArea()).not.toBeNull();

    // The first Escape goes back to the drawer, the second closes it.
    await press("Escape");
    expect(drawerEl()!.hasAttribute("data-full-page")).toBe(false);
    expect(boardArea()).toBeNull();
    await press("Escape");
    expect(drawerEl()).toBeNull();
  });

  it("the Open cards in setting opens full page by default", async () => {
    saveBoardCardOpen("page");
    await renderBoard();
    act(() => cardEl("c1").click());
    await flush();
    expect(drawerEl()!.hasAttribute("data-full-page")).toBe(true);
    // Back to the drawer for this card only; the next card follows the setting.
    await press("Escape");
    expect(drawerEl()!.hasAttribute("data-full-page")).toBe(false);
    await press("Escape");
    act(() => cardEl("c2").click());
    await flush();
    expect(drawerEl()!.hasAttribute("data-full-page")).toBe(true);
  });

  it("opens as a drawer when the setting is Drawer", async () => {
    saveBoardCardOpen("drawer");
    await renderBoard();
    act(() => cardEl("c1").click());
    await flush();
    expect(drawerEl()!.hasAttribute("data-full-page")).toBe(false);
  });
});

describe("BoardView New card shortcut", () => {
  const modKey = IS_MAC ? { metaKey: true } : { ctrlKey: true };
  const dialog = () => document.querySelector('[role="dialog"]');
  const searchBox = () =>
    container.querySelector<HTMLInputElement>('[aria-label="Search cards"]')!;
  const cardEl = (id: string) =>
    container.querySelector<HTMLElement>(`[data-kanban-card="${id}"]`)!;

  function keydown(target: HTMLElement, init: KeyboardEventInit) {
    const event = new KeyboardEvent("keydown", {
      key: "n",
      bubbles: true,
      cancelable: true,
      ...init,
    });
    act(() => {
      target.dispatchEvent(event);
    });
    return event;
  }

  it("opens the New card modal when focus is in the board", async () => {
    await renderBoard();
    const event = keydown(cardEl("c1"), modKey);
    expect(event.defaultPrevented).toBe(true);
    expect(dialog()?.getAttribute("aria-labelledby")).toBeTruthy();
    expect(dialog()?.textContent).toContain("New card");
    expect(document.activeElement).toBe(
      document.querySelector('[aria-label="Title"]'),
    );
  });

  it("opens from the search box without typing an n", async () => {
    await renderBoard();
    searchBox().focus();
    const event = keydown(searchBox(), modKey);
    expect(event.defaultPrevented).toBe(true);
    expect(dialog()).not.toBeNull();
    expect(searchBox().value).toBe("");
  });

  it("does nothing outside the board", async () => {
    await renderBoard();
    const event = keydown(document.body, modKey);
    expect(event.defaultPrevented).toBe(false);
    expect(dialog()).toBeNull();
  });

  it("ignores a plain n, and Shift or Alt with the modifier (New Window)", async () => {
    await renderBoard();
    for (const init of [{}, { ...modKey, shiftKey: true }, { ...modKey, altKey: true }]) {
      const event = keydown(searchBox(), init);
      expect(event.defaultPrevented).toBe(false);
    }
    expect(dialog()).toBeNull();
  });

  it("does not stack a second modal when pressed again", async () => {
    await renderBoard();
    keydown(cardEl("c1"), modKey);
    keydown(document.querySelector<HTMLElement>('[aria-label="Title"]')!, modKey);
    expect(document.querySelectorAll('[role="dialog"]')).toHaveLength(1);
  });

  it("creates the card in the todo column and closes", async () => {
    await renderBoard();
    keydown(cardEl("c1"), modKey);
    const title = document.querySelector<HTMLInputElement>('[aria-label="Title"]')!;
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!;
    act(() => {
      setter.call(title, "From the shortcut");
      title.dispatchEvent(new Event("input", { bubbles: true }));
    });
    act(() => {
      title.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Enter",
          ...modKey,
          bubbles: true,
          cancelable: true,
        }),
      );
    });
    await flush();
    expect(commands("board_card_create")).toHaveLength(1);
    expect(commands("board_card_create")[0][1]).toMatchObject({
      columnId: "backlog",
      title: "From the shortcut",
    });
    expect(dialog()).toBeNull();
  });

  const modalTitle = () =>
    document.querySelector<HTMLInputElement>(
      '[role="dialog"] [aria-label="Title"]',
    )!;

  it("opens from the column composer without typing an n", async () => {
    await renderBoard();
    const add = [...columnEl("backlog").querySelectorAll("button")].find((b) =>
      b.textContent?.includes("Add card"),
    )!;
    act(() => add.click());
    const composer = container.querySelector<HTMLTextAreaElement>(
      '[aria-label="New card in Backlog"]',
    )!;
    composer.focus();
    const event = keydown(composer, modKey);
    expect(event.defaultPrevented).toBe(true);
    expect(dialog()).not.toBeNull();
    expect(composer.value).toBe("");
  });

  it("opens from the card title field in the drawer without typing an n", async () => {
    await renderBoard();
    act(() =>
      container
        .querySelector<HTMLElement>("[data-board-card='c1']")!
        .click(),
    );
    await flush();
    const title = document.querySelector<HTMLInputElement>(
      '[data-card-drawer] [aria-label="Title"]',
    )!;
    title.focus();
    const event = keydown(title, modKey);
    expect(event.defaultPrevented).toBe(true);
    expect(dialog()).not.toBeNull();
    expect(title.value).toBe("Fix login");
  });

  it("opens with focus on a card's menu button, which stops other keys", async () => {
    await renderBoard();
    const priority = container.querySelector<HTMLButtonElement>(
      '[aria-label="Priority of #1"]',
    )!;
    priority.focus();
    const event = keydown(priority, modKey);
    expect(event.defaultPrevented).toBe(true);
    expect(dialog()).not.toBeNull();
  });

  it("returns focus to the board after Create, so the shortcut works again", async () => {
    await renderBoard();
    const card = cardEl("c1");
    card.focus();
    keydown(card, modKey);
    const title = modalTitle();
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!;
    act(() => {
      setter.call(title, "Next");
      title.dispatchEvent(new Event("input", { bubbles: true }));
    });
    act(() => {
      title.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Enter",
          ...modKey,
          bubbles: true,
          cancelable: true,
        }),
      );
    });
    await flush();
    expect(dialog()).toBeNull();
    expect(container.contains(document.activeElement)).toBe(true);
    keydown(document.activeElement as HTMLElement, modKey);
    expect(dialog()).not.toBeNull();
  });

  it("returns focus to the board after Escape, so the shortcut works again", async () => {
    await renderBoard();
    searchBox().focus();
    keydown(searchBox(), modKey);
    press(modalTitle(), "Escape");
    expect(dialog()).toBeNull();
    expect(document.activeElement).toBe(searchBox());
    keydown(document.activeElement as HTMLElement, modKey);
    expect(dialog()).not.toBeNull();
  });

  it("falls back to the board when the opener is gone", async () => {
    await renderBoard();
    const card = cardEl("c2");
    card.focus();
    keydown(card, modKey);
    card.remove();
    press(modalTitle(), "Escape");
    expect(dialog()).toBeNull();
    expect(container.contains(document.activeElement)).toBe(true);
    keydown(document.activeElement as HTMLElement, modKey);
    expect(dialog()).not.toBeNull();
  });

  it("under StrictMode keeps focus in the Title field, and still returns it on Escape", async () => {
    const props: ComponentProps<typeof BoardView> = {
      onClose: vi.fn(),
      linkTargets: [],
      onOpenLinkedWork: vi.fn(() => Promise.resolve()),
      lookupLinkTargets: vi.fn(async () => []),
      projects: [],
    };
    await act(async () => {
      root.render(
        createElement(StrictMode, null, createElement(BoardView, props)),
      );
    });
    await flush();
    searchBox().focus();
    keydown(searchBox(), modKey);
    await flush();
    expect(document.activeElement).toBe(modalTitle());
    press(modalTitle(), "Escape");
    expect(dialog()).toBeNull();
    expect(document.activeElement).toBe(searchBox());
  });
});

describe("BoardView filter builder, sort and saved views", () => {
  const DAY = 86_400_000;
  const extras = (over: Partial<CardExtras> = {}): CardExtras => ({
    priority: 0,
    dueAt: null,
    projects: [],
    labels: [],
    subtasksDone: 0,
    subtasksTotal: 0,
    comments: 0,
    ...over,
  });
  const label = (id: string, name: string): BoardLabel => ({
    id,
    boardId: "b1",
    name,
    color: "e5484d",
    position: 0,
  });

  /**
   * #1 Fix login (High, Bug, due in 3 days), #2 Write docs (Low, due
   * tomorrow, a description), #4 Deploy (Medium, due in 2 days) in Backlog;
   * #3 Ship board (undated) in In Progress; Review and Done are empty.
   */
  function seed(over: Partial<Record<string, Partial<BoardCard>>> = {}) {
    const at = (id: string, base: BoardCard) => ({ ...base, ...over[id] });
    handlers.board_get = () => ({
      board: BOARD,
      columns: fixture().columns,
      cards: [
        at("c1", card("c1", "backlog", 0, 1, "Fix login")),
        at("c2", {
          ...card("c2", "backlog", 1, 2, "Write docs"),
          body: "Retry after the timeout",
        }),
        at("c4", card("c4", "backlog", 2, 4, "Deploy")),
        at("c3", card("c3", "progress", 0, 3, "Ship board")),
      ],
    });
    const now = Date.now();
    handlers.board_card_extras = () => ({
      c1: extras({ priority: 3, labels: ["l1"], dueAt: now + 3 * DAY }),
      c2: extras({ priority: 1, dueAt: now + DAY }),
      c4: extras({ priority: 2, dueAt: now + 2 * DAY }),
    });
    handlers.board_labels = () => [label("l1", "Bug")];
  }

  /** Writes to the saved views, as the store does. */
  function viewStore() {
    let next = 0;
    handlers.board_view_create = (args) => {
      const view: BoardViewRow = {
        id: `v${++next}`,
        boardId: "b1",
        name: String(args.name),
        position: savedViews.length,
        specJson: String(args.specJson),
        layoutJson: String(args.layoutJson),
        createdAt: 1,
        updatedAt: 1,
      };
      savedViews = [...savedViews, view];
      return view;
    };
    handlers.board_view_update = (args) => {
      savedViews = savedViews.map((view) =>
        view.id === args.viewId
          ? {
              ...view,
              name: (args.name as string | null) ?? view.name,
              specJson: (args.specJson as string | null) ?? view.specJson,
              layoutJson: (args.layoutJson as string | null) ?? view.layoutJson,
            }
          : view,
      );
    };
    handlers.board_view_delete = (args) => {
      savedViews = savedViews.filter((view) => view.id !== args.viewId);
    };
  }

  const stored = (
    id: string,
    name: string,
    spec: Partial<ViewSpec>,
    layoutJson = '{"layout":"board"}',
  ): BoardViewRow => ({
    id,
    boardId: "b1",
    name,
    position: savedViews.length,
    specJson: JSON.stringify({
      query: "",
      rows: [],
      sort: { key: "manual", dir: "asc" },
      ...spec,
    }),
    layoutJson,
    createdAt: 1,
    updatedAt: 1,
  });
  const row = (
    field: string,
    values: string[],
    op: "is" | "is_not" = "is",
  ) => ({ field, op, values });

  const searchInput = () =>
    container.querySelector<HTMLInputElement>('input[aria-label="Search cards"]')!;
  const chips = () =>
    [...container.querySelectorAll("[data-filter-chip]")].map((el) =>
      el.textContent?.trim(),
    );
  const badge = () =>
    container.querySelector("[data-filter-count]")?.textContent ?? null;
  const tab = (id: string) =>
    container.querySelector<HTMLElement>(`[data-view-tab="${id}"]`)!;
  const dot = () => container.querySelector("[data-view-dot]");
  const tabsBar = () => container.querySelector("[data-view-tabs]")!;
  const tabButton = (text: string) =>
    [...tabsBar().querySelectorAll("button")].find(
      (el) => el.textContent?.trim() === text,
    )!;
  const lastQuery = () =>
    commands("board_query").at(-1)![1] as {
      spec: ViewSpec;
      running: string[];
    };

  async function click(el: Element) {
    act(() => (el as HTMLElement).click());
    await flush();
  }

  /** Opens the Filter menu unless it is open already. */
  async function openFilter() {
    const trigger = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Filter"]',
    )!;
    if (trigger.getAttribute("aria-expanded") !== "true") await click(trigger);
  }

  async function saveAs(name: string) {
    await click(tabButton("Save view"));
    const input = tabsBar().querySelector<HTMLInputElement>(
      'input[aria-label="View name"]',
    )!;
    act(() => {
      Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!.call(input, name);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    press(input, "Enter");
    await flush();
  }

  async function viewMenu(name: string, item: string) {
    await click(
      container.querySelector(`button[aria-label="View options for ${name}"]`)!,
    );
    const entry = [
      ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
    ].find((el) => el.textContent?.trim() === item)!;
    expect(entry, item).toBeDefined();
    await click(entry);
  }

  beforeEach(() => {
    seed();
    viewStore();
  });

  it("search_finds_text_in_the_description", async () => {
    await renderBoard();
    await search("timeout");
    expect(cardIdsIn("backlog")).toEqual(["c2"]);
    // Case-insensitive, and the title still matches.
    await search("TIMEOUT");
    expect(cardIdsIn("backlog")).toEqual(["c2"]);
    await search("login");
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    await search("#4");
    expect(cardIdsIn("backlog")).toEqual(["c4"]);
    // Matching goes through the engine, and no time zone goes with it.
    const [, args] = commands("board_query").at(-1)!;
    expect(Object.keys(args as object).sort()).toEqual([
      "boardId",
      "running",
      "spec",
    ]);
    expect(lastQuery().spec.query).toBe("#4");
  });

  it("filter_builder_adds_where_and_rows_and_shows_chips", async () => {
    await renderBoard();
    await filterBy("Priority", "High");
    expect(chips()).toEqual(["Priority is High"]);
    await filterBy("Labels", "Bug");
    expect(chips()).toEqual(["Priority is High", "Labels is Bug"]);
    expect(badge()).toBe("2");
    expect(
      [...document.querySelectorAll("[data-filter-lead]")].map(
        (el) => el.textContent,
      ),
    ).toEqual(["Where", "and"]);
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    expect(cardIdsIn("progress")).toEqual([]);
    expect(lastQuery().spec.rows).toEqual([
      row("priority", ["3"]),
      row("labels", ["l1"]),
    ]);
    // Values within a row OR together.
    await filterBy("Priority", "Low");
    expect(chips()[0]).toBe("Priority is High, Low");
    // (High or Low) and Bug: still only #1.
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    expect(lastQuery().spec.rows[0]).toEqual(row("priority", ["3", "1"]));
  });

  it("is_not_flips_a_row_and_a_chip_removes_its_row", async () => {
    await renderBoard();
    await filterBy("Priority", "High");
    await click(document.querySelector('button[aria-label="Match or exclude"]')!);
    expect(chips()).toEqual(["Priority is not High"]);
    expect(cardIdsIn("backlog")).toEqual(["c2", "c4"]);
    expect(cardIdsIn("progress")).toEqual(["c3"]);
    await click(
      container.querySelector('[data-filter-chip="0"] button[aria-label="Remove Priority filter"]')!,
    );
    expect(chips()).toEqual([]);
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2", "c4"]);
  });

  it("clear_all_removes_every_row", async () => {
    await renderBoard();
    await filterBy("Priority", "High");
    await filterBy("Labels", "Bug");
    await search("fix");
    await click(
      container.querySelector('[data-filter-chips] button[aria-label="Clear all"]')!,
    );
    expect(chips()).toEqual([]);
    expect(badge()).toBeNull();
    // The search is not a row: it stays.
    expect(searchInput().value).toBe("fix");
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    expect(lastQuery().spec.rows).toEqual([]);
    await search("");
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2", "c4"]);

    // The builder's own Clear all does the same.
    await filterBy("Priority", "Low");
    await pick("Filter", "Clear all");
    expect(chips()).toEqual([]);
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2", "c4"]);
  });

  it("a_row_with_no_values_is_not_a_filter", async () => {
    savedViews = [stored("v1", "Mine", { rows: [row("priority", ["3"])] })];
    await renderBoard();
    await click(tab("v1"));
    const before = commands("board_query").length;
    // Add a Labels row and pick nothing.
    await openFilter();
    await click(
      [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(
        (el) => el.textContent?.trim() === "Labels",
      )!,
    );
    expect(chips()).toEqual(["Priority is High", "Labels is …"]);
    // Not in the badge, not narrowing, not asked of the engine, not unsaved.
    expect(badge()).toBe("1");
    expect(lastQuery().spec.rows).toEqual([row("priority", ["3"])]);
    expect(commands("board_query")).toHaveLength(before);
    expect(dot()).toBeNull();
    // Alone, an empty row leaves the board as it was.
    await pick("Filter", "Clear all");
    await click(tab("all"));
    await openFilter();
    await click(
      [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(
        (el) => el.textContent?.trim() === "Labels",
      )!,
    );
    expect(badge()).toBeNull();
    expect(countOf("backlog")).toBe("3");
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2", "c4"]);
  });

  const countOf = (id: string) =>
    columnEl(id).querySelector("[data-column-count]")?.textContent;

  it("sort_by_due_orders_columns_and_disables_in_column_drag", async () => {
    handlers.board_card_move = () => undefined;
    await renderBoard();
    expect(kanban.props!.sorted).toBe(false);
    await pick("Sort", "Due date");
    expect(container.querySelector("[data-sorted-by]")?.textContent).toBe(
      "Sorted by Due date",
    );
    expect(lastQuery().spec.sort).toEqual({ key: "due", dir: "asc" });
    // Soonest first inside each column; counts are still plain totals.
    expect(cardIdsIn("backlog")).toEqual(["c2", "c4", "c1"]);
    expect(countOf("backlog")).toBe("3");
    expect(kanban.props!.sorted).toBe(true);

    // Descending reverses the dated cards; an undated one is last either way.
    await pick("Sort", "Descending");
    expect(cardIdsIn("backlog")).toEqual(["c1", "c4", "c2"]);

    // A drop inside a column changes nothing.
    act(() =>
      kanban.props!.onCardMove("c1", { columnId: "backlog", index: 2 }),
    );
    await flush();
    expect(commands("board_card_move")).toHaveLength(0);

    // Back to Manual: the column order returns and dragging inside works.
    await pick("Sort", "Manual");
    expect(container.querySelector("[data-sorted-by]")).toBeNull();
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2", "c4"]);
    expect(kanban.props!.sorted).toBe(false);
    act(() =>
      kanban.props!.onCardMove("c1", { columnId: "backlog", index: 2 }),
    );
    await flush();
    expect(commands("board_card_move")).toHaveLength(1);
  });

  it("names_each_remove_button_after_its_row", async () => {
    await renderBoard();
    await filterBy("Priority", "High");
    await filterBy("Labels", "Bug");
    for (const name of ["Priority", "Labels"]) {
      // One in the builder, one on the chip.
      expect(
        document.querySelectorAll(`button[aria-label="Remove ${name} filter"]`),
      ).toHaveLength(2);
    }
    await click(
      container.querySelector(
        '[data-filter-chip="1"] button[aria-label="Remove Labels filter"]',
      )!,
    );
    expect(chips()).toEqual(["Priority is High"]);
  });

  it("sorts_priority_none_last_and_orders_the_rest", async () => {
    handlers.board_card_extras = () => ({
      c1: extras({ priority: 3 }),
      c2: extras({ priority: 1 }),
    });
    await renderBoard();
    await pick("Sort", "Priority");
    expect(cardIdsIn("backlog")).toEqual(["c2", "c1", "c4"]);
    await pick("Sort", "Descending");
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2", "c4"]);
  });

  it("keyboard_drop_inside_a_sorted_column_moves_nothing", async () => {
    handlers.board_card_move = () => undefined;
    await renderBoard();
    const rect = (x: number, y: number, w: number, h: number) =>
      ({
        x,
        y,
        left: x,
        top: y,
        width: w,
        height: h,
        right: x + w,
        bottom: y + h,
        toJSON() {},
      }) as DOMRect;
    const spy = vi
      .spyOn(Element.prototype, "getBoundingClientRect")
      .mockImplementation(function (this: Element) {
        const section = this.closest("[data-kanban-column]");
        if (!section) return rect(0, 0, 10, 10);
        const x =
          [...container.querySelectorAll("[data-kanban-column]")].indexOf(
            section,
          ) * 300;
        if (this === section) return rect(x, 0, 280, 500);
        if (this.hasAttribute("data-kanban-card")) {
          const above = [...this.parentElement!.children].indexOf(this);
          return rect(x + 6, 50 + above * 70, 268, 60);
        }
        if (
          this.querySelector("[data-kanban-card]") ||
          this.matches(".overflow-y-auto")
        ) {
          return rect(x, 40, 280, 420);
        }
        return rect(x, 0, 280, 36);
      });
    const tick = () =>
      act(() => new Promise((resolve) => setTimeout(resolve, 0)));
    const key = (target: EventTarget, code: string) =>
      act(() => {
        target.dispatchEvent(
          new KeyboardEvent("keydown", {
            code,
            key: code === "Space" ? " " : code,
            bubbles: true,
            cancelable: true,
          }),
        );
      });
    /** Picks the card up, nudges it up one place and drops it. */
    const dropOneUp = async (id: string) => {
      const el = container.querySelector<HTMLElement>(
        `[data-kanban-card='${id}']`,
      )!;
      act(() => el.focus());
      key(el, "Space");
      await tick();
      key(el, "ArrowUp");
      await tick();
      key(document, "Space");
      await tick();
      await flush();
    };
    try {
      // Control: manual order, the same gesture is a move.
      await dropOneUp("c4");
      expect(commands("board_card_move")).toHaveLength(1);

      // Sorted by due, the same gesture inside the column is ignored.
      await pick("Sort", "Due date");
      expect(cardIdsIn("backlog")).toEqual(["c2", "c4", "c1"]);
      await dropOneUp("c1");
      expect(commands("board_card_move")).toHaveLength(1);
      expect(cardIdsIn("backlog")).toEqual(["c2", "c4", "c1"]);
    } finally {
      spy.mockRestore();
    }
  });

  it("cross_column_drop_still_works_while_sorted", async () => {
    handlers.board_card_move = () => undefined;
    await renderBoard();
    await pick("Sort", "Due date");
    // Dropped at the top of In Progress, it is appended: the sort decides
    // where it shows.
    act(() =>
      kanban.props!.onCardMove("c1", { columnId: "progress", index: 0 }),
    );
    await flush();
    expect(commands("board_card_move").at(-1)![1]).toMatchObject({
      cardId: "c1",
      columnId: "progress",
      index: 1,
      actor: "user",
    });
    // An empty column takes it at 0.
    act(() => kanban.props!.onCardMove("c2", { columnId: "done", index: 0 }));
    await flush();
    expect(commands("board_card_move").at(-1)![1]).toMatchObject({
      cardId: "c2",
      columnId: "done",
      index: 0,
    });
  });

  it("save_view_creates_a_tab_and_switching_restores_spec_and_layout", async () => {
    await renderBoard();
    // All cards comes first and is selected.
    const tabs = () =>
      [...container.querySelectorAll<HTMLElement>('[role="tab"]')];
    expect(tabs().map((el) => el.textContent)).toEqual(["All cards"]);
    expect(tabs()[0].getAttribute("aria-selected")).toBe("true");

    await filterBy("Priority", "High");
    await pick("Sort", "Due date");
    await search(" login ");
    await saveAs("Urgent");

    const [, created] = commands("board_view_create")[0];
    expect(created).toMatchObject({
      boardId: "b1",
      name: "Urgent",
      layoutJson: '{"layout":"board"}',
    });
    expect(JSON.parse((created as { specJson: string }).specJson)).toEqual({
      query: "login",
      rows: [row("priority", ["3"])],
      sort: { key: "due", dir: "asc" },
    });
    expect(tabs().map((el) => el.textContent)).toEqual(["All cards", "Urgent"]);
    expect(tabs()[1].getAttribute("aria-selected")).toBe("true");
    expect(dot()).toBeNull();

    // Switching away clears everything; switching back restores it all.
    await click(tab("all"));
    expect(chips()).toEqual([]);
    expect(searchInput().value).toBe("");
    expect(container.querySelector("[data-sorted-by]")).toBeNull();
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2", "c4"]);

    await click(tab("v1"));
    expect(chips()).toEqual(["Priority is High"]);
    expect(searchInput().value).toBe("login");
    expect(container.querySelector("[data-sorted-by]")?.textContent).toBe(
      "Sorted by Due date",
    );
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    expect(dot()).toBeNull();
  });

  it("keeps_layout_keys_it_does_not_know_when_saving_a_view", async () => {
    const layout = '{"layout":"board","columns":{"title":120},"future":[1,2]}';
    savedViews = [stored("v1", "Mine", {}, layout)];
    await renderBoard();
    await click(tab("v1"));
    await filterBy("Priority", "High");
    await click(tabButton("Save"));
    const [, update] = commands("board_view_update").at(-1)!;
    expect(JSON.parse((update as { layoutJson: string }).layoutJson)).toEqual(
      JSON.parse(layout),
    );
    // A duplicate carries the layout as it is too.
    await viewMenu("Mine", "Duplicate");
    expect(commands("board_view_create").at(-1)![1]).toMatchObject({
      name: "Mine copy",
      layoutJson: layout,
    });
  });

  it("duplicate_copies_unsaved_edits_and_leaves_the_original_stored", async () => {
    savedViews = [stored("v1", "Mine", {})];
    await renderBoard();
    await click(tab("v1"));
    await pick("Sort", "Due date");
    expect(dot()).not.toBeNull();
    await viewMenu("Mine", "Duplicate");
    // The copy is made from the edited sort, and nothing was written to v1.
    const [, created] = commands("board_view_create").at(-1)!;
    expect(created).toMatchObject({ name: "Mine copy" });
    expect(
      JSON.parse((created as { specJson: string }).specJson).sort,
    ).toEqual({ key: "due", dir: "asc" });
    expect(commands("board_view_update")).toHaveLength(0);
    expect(
      JSON.parse(savedViews.find((view) => view.id === "v1")!.specJson).sort,
    ).toEqual({ key: "manual", dir: "asc" });
    // The copy is shown, sorted, and clean.
    expect(container.querySelector("[data-sorted-by]")?.textContent).toBe(
      "Sorted by Due date",
    );
    expect(dot()).toBeNull();
  });

  it("unsaved_changes_show_a_dot_with_save_and_revert", async () => {
    savedViews = [stored("v1", "Mine", { rows: [row("priority", ["3"])] })];
    await renderBoard();
    await click(tab("v1"));
    expect(dot()).toBeNull();
    expect(
      [...tabsBar().querySelectorAll("button")].some(
        (el) => el.textContent?.trim() === "Revert",
      ),
    ).toBe(false);

    await search("docs");
    expect(dot()?.textContent).toBe("•");
    expect(dot()?.getAttribute("aria-label")).toBe("Unsaved changes");
    // Undoing the edit by hand clears the dot.
    await search("");
    expect(dot()).toBeNull();

    // Revert drops the changes.
    await filterBy("Priority", "Low");
    expect(dot()).not.toBeNull();
    await click(tabButton("Revert"));
    expect(dot()).toBeNull();
    expect(chips()).toEqual(["Priority is High"]);

    // Save writes them over the view.
    await pick("Sort", "Title");
    expect(dot()).not.toBeNull();
    await click(tabButton("Save"));
    expect(dot()).toBeNull();
    expect(commands("board_view_update").at(-1)![1]).toMatchObject({
      viewId: "v1",
      name: null,
    });
    expect(
      JSON.parse(
        (commands("board_view_update").at(-1)![1] as { specJson: string })
          .specJson,
      ).sort,
    ).toEqual({ key: "title", dir: "asc" });
    expect(
      [...tabsBar().querySelectorAll("button")].some(
        (el) => el.textContent?.trim() === "Revert",
      ),
    ).toBe(false);
  });

  it("all_cards_tab_cannot_be_deleted", async () => {
    savedViews = [stored("v1", "Mine", {}), stored("v2", "Other", {})];
    await renderBoard();
    const tabs = [...container.querySelectorAll('[role="tab"]')].map(
      (el) => el.textContent,
    );
    expect(tabs).toEqual(["All cards", "Mine", "Other"]);
    // Every saved view has a menu; All cards has none.
    expect(
      container.querySelectorAll('button[aria-label^="View options for"]'),
    ).toHaveLength(2);
    expect(
      container.querySelector('button[aria-label="View options for All cards"]'),
    ).toBeNull();
    // Deleting the showing view falls back to All cards.
    await click(tab("v1"));
    await viewMenu("Mine", "Delete");
    expect(commands("board_view_delete")[0][1]).toEqual({ viewId: "v1" });
    expect(
      [...container.querySelectorAll('[role="tab"]')].map((el) => el.textContent),
    ).toEqual(["All cards", "Other"]);
    expect(tab("all").getAttribute("aria-selected")).toBe("true");
  });

  it("renames_a_view_from_its_menu", async () => {
    savedViews = [stored("v1", "Mine", {})];
    await renderBoard();
    await viewMenu("Mine", "Rename");
    const input = tabsBar().querySelector<HTMLInputElement>(
      'input[aria-label="View name"]',
    )!;
    act(() => {
      Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!.call(input, "Renamed");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    press(input, "Enter");
    await flush();
    expect(commands("board_view_update").at(-1)![1]).toMatchObject({
      viewId: "v1",
      name: "Renamed",
      specJson: null,
      layoutJson: null,
    });
    expect(tab("v1").textContent).toBe("Renamed");
  });

  it("a_view_with_a_deleted_label_still_loads", async () => {
    savedViews = [
      stored("v1", "Gone", { rows: [row("labels", ["deleted"])] }),
      stored("v2", "Mixed", { rows: [row("labels", ["deleted", "l1"])] }),
    ];
    await renderBoard();
    // Only missing values: the row is ignored, never zero results.
    await click(tab("v1"));
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2", "c4"]);
    expect(chips()).toEqual(["Labels is …"]);
    expect(badge()).toBeNull();
    expect(commands("board_query")).toHaveLength(0);
    expect(dot()).toBeNull();
    // Some values left: those still filter.
    await click(tab("v2"));
    expect(chips()).toEqual(["Labels is Bug"]);
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    expect(lastQuery().spec.rows).toEqual([row("labels", ["l1"])]);
    expect(dot()).toBeNull();
  });

  it("filters_by_a_select_custom_field_and_encodes_the_value_as_json", async () => {
    handlers.board_fields = () => [
      {
        id: "f1",
        boardId: "b1",
        name: "Area",
        kind: "select",
        optionsJson: '["Backend","Web"]',
        position: 0,
      },
      {
        id: "f2",
        boardId: "b1",
        name: "Blocked",
        kind: "checkbox",
        optionsJson: null,
        position: 1,
      },
    ];
    fieldValues = {
      c1: { f1: '"Backend"', f2: "true" },
      c2: { f1: '"Web"', f2: "false" },
      c3: { f1: '"Backend"' },
    };
    await renderBoard();
    await click(container.querySelector('button[aria-label="Filter"]')!);
    // One entry per field, named after it.
    const entries = [
      ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
    ].map((el) => el.textContent?.trim());
    expect(entries).toContain("Area");
    expect(entries).toContain("Blocked");

    await filterBy("Area", "Backend");
    expect(chips()).toEqual(["Area is Backend"]);
    expect(lastQuery().spec.rows).toEqual([row("field:f1", ['"Backend"'])]);
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    expect(cardIdsIn("progress")).toEqual(["c3"]);

    // A checkbox offers Yes and No, sent as true and false.
    await filterBy("Blocked", "Yes");
    expect(chips()).toEqual(["Area is Backend", "Blocked is Yes"]);
    expect(lastQuery().spec.rows.at(-1)).toEqual(row("field:f2", ["true"]));
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    expect(cardIdsIn("progress")).toEqual([]);
  });

  it("a_custom_field_row_for_a_deleted_field_says_so_and_filters_nothing", async () => {
    handlers.board_fields = () => [
      {
        id: "f3",
        boardId: "b1",
        name: "Notes",
        kind: "text",
        optionsJson: null,
        position: 0,
      },
    ];
    savedViews = [
      stored("v1", "Old", { rows: [row("field:f9", ['"x"'])] }),
      stored("v2", "Text", { rows: [row("field:f3", ['"alpha"'])] }),
    ];
    fieldValues = { c1: { f3: '"alpha"' } };
    await renderBoard();
    await click(tab("v1"));
    expect(chips()).toEqual(["Deleted field is …"]);
    expect(badge()).toBeNull();
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2", "c4"]);
    expect(dot()).toBeNull();
    // A free-form field lists only what the row already holds.
    await click(tab("v2"));
    expect(chips()).toEqual(["Notes is alpha"]);
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    await click(container.querySelector('button[aria-label="Filter"]')!);
    await click(
      document.querySelector('button[aria-label="Pick Notes values"]')!,
    );
    expect(
      [...document.querySelectorAll('[role="menuitemcheckbox"]')].map(
        (el) => el.textContent?.trim(),
      ),
    ).toEqual(["alpha"]);
  });

  it("re_applies_an_untouched_view_changed_in_the_other_app", async () => {
    savedViews = [stored("v1", "Mine", { rows: [row("priority", ["3"])] })];
    await renderBoard();
    await click(tab("v1"));
    expect(chips()).toEqual(["Priority is High"]);
    expect(cardIdsIn("backlog")).toEqual(["c1"]);

    savedViews = [stored("v1", "Mine", { rows: [row("priority", ["1"])] })];
    await act(async () => emitBoardChanged("b1"));
    await flush();
    expect(chips()).toEqual(["Priority is Low"]);
    expect(cardIdsIn("backlog")).toEqual(["c2"]);
    expect(dot()).toBeNull();
  });

  it("keeps_local_edits_and_the_dot_when_the_view_changes_elsewhere", async () => {
    savedViews = [stored("v1", "Mine", { rows: [row("priority", ["3"])] })];
    await renderBoard();
    await click(tab("v1"));
    await search("fix");
    expect(dot()).not.toBeNull();

    savedViews = [stored("v1", "Mine", { rows: [row("priority", ["1"])] })];
    await act(async () => emitBoardChanged("b1"));
    await flush();
    // Still what the user made, and still unsaved against the new version.
    expect(searchInput().value).toBe("fix");
    expect(chips()).toEqual(["Priority is High"]);
    expect(dot()).not.toBeNull();
    // Revert shows the new version.
    await click(tabButton("Revert"));
    expect(chips()).toEqual(["Priority is Low"]);
    expect(searchInput().value).toBe("");
    expect(dot()).toBeNull();
  });

  it("a_view_deleted_elsewhere_leaves_the_screen_under_all_cards", async () => {
    savedViews = [stored("v1", "Mine", { rows: [row("priority", ["3"])] })];
    await renderBoard();
    await click(tab("v1"));
    savedViews = [];
    await act(async () => emitBoardChanged("b1"));
    await flush();
    expect(tab("all").getAttribute("aria-selected")).toBe("true");
    expect(chips()).toEqual(["Priority is High"]);
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    expect(container.querySelector('[data-view-tab="v1"]')).toBeNull();
  });

  it("requeries_on_board_changes", async () => {
    await renderBoard();
    await search("docs");
    expect(cardIdsIn("backlog")).toEqual(["c2"]);
    const before = commands("board_query").length;
    seed({ c4: { title: "Docs again" } });
    await act(async () => emitBoardChanged("b1"));
    await flush();
    expect(commands("board_query").length).toBeGreaterThan(before);
    expect(cardIdsIn("backlog")).toEqual(["c2", "c4"]);
  });

  it("ignores_an_answer_that_arrives_after_a_newer_request", async () => {
    let late!: (ids: string[]) => void;
    handlers.board_query = (args) => {
      const { query } = args.spec as ViewSpec;
      return query === "d"
        ? new Promise<string[]>((resolve) => {
            late = resolve;
          })
        : fakeEngine(args);
    };
    await renderBoard();
    await search("d"); // never answers until later
    await search("deploy"); // answers at once
    expect(cardIdsIn("backlog")).toEqual(["c4"]);
    // The older, wider answer lands now and must not win.
    await act(async () => late(["c1", "c2", "c4", "c3"]));
    await flush();
    expect(cardIdsIn("backlog")).toEqual(["c4"]);
    expect(cardIdsIn("progress")).toEqual([]);
  });

  it("a_failed_query_shows_every_card_and_says_why", async () => {
    handlers.board_query = () => {
      throw new Error("database is locked");
    };
    await renderBoard();
    await search("docs");
    expect(cardIdsIn("backlog")).toEqual(["c1", "c2", "c4"]);
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(
      "database is locked",
    );
  });

  it("passes_the_cards_with_running_linked_work_to_the_query", async () => {
    seed({
      c3: { linkKind: "session", linkId: "s1" },
      c4: { linkKind: "session", linkId: "s2" },
      c2: { linkKind: "flow-run", linkId: "r1" },
    });
    const target = (id: string, status: BoardLinkTarget["status"]) => ({
      kind: "session" as const,
      id,
      label: id,
      status,
    });
    await renderBoard({
      linkTargets: [target("s1", "running"), target("s2", "completed")],
    });
    await filterBy("Linked work", "Running");
    expect(lastQuery().running).toEqual(["c3"]);
    expect(cardIdsIn("backlog")).toEqual([]);
    expect(cardIdsIn("progress")).toEqual(["c3"]);
  });
});
