// @vitest-environment happy-dom
import {
  act,
  createElement,
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
} from "../model/board";
import type { BoardLinkTarget } from "./boardFormat";
import type { KanbanBoardProps } from "./kanban/KanbanBoard";
import { LAYER } from "../../../shared/lib/layers";
import { BoardView } from "./BoardView";
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
  handlers = {
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
    expect(commands("board_activity")[0][1]).toMatchObject({
      boardId: "b1",
      cardId: "c3",
    });
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

  function search(value: string) {
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
    search("alpha");
    expect(countOf("backlog")).toBe("3/5");
    expect(countOf("done")).toBe("0/0");
    expect(cardIdsIn("backlog")).toEqual(["c1", "c3", "c5"]);
    search("#2");
    expect(cardIdsIn("backlog")).toEqual(["c2"]);
    search("");
    expect(countOf("backlog")).toBe("5");

    await pick("Filter", "Low");
    expect(countOf("backlog")).toBe("2/5");
    expect(cardIdsIn("backlog")).toEqual(["c2", "c4"]);
    expect(container.querySelector("[data-filter-count]")?.textContent).toBe(
      "1",
    );
    // Search and filter narrow together.
    search("beta two");
    expect(cardIdsIn("backlog")).toEqual(["c2"]);
    search("");
    await pick("Filter", "Clear");
    expect(countOf("backlog")).toBe("5");
    expect(container.querySelector("[data-filter-count]")).toBeNull();
  });

  it("drop_while_filtered_lands_between_the_right_cards", async () => {
    five();
    handlers.board_card_move = () => undefined;
    await renderBoard();
    search("alpha"); // shows #1, #3, #5; hides #2 and #4
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
    search("alpha");
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
    await pick("Filter", "Low");
    await pick("Filter", "Bug");
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
    // Clear is an action, not a toggle.
    expect(option("Clear").getAttribute("role")).toBe("menuitem");
  });

  it("keyboard_drop_while_filtered_lands_between_the_right_cards", async () => {
    five();
    handlers.board_card_move = () => undefined;
    await renderBoard();
    search("alpha"); // shows #1, #3, #5; hides #2 and #4
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
    await pick("Filter", "Bug");
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
    await pick("Filter", "api");
    expect(cardIdsIn("backlog")).toEqual(["c1"]);
    await pick("Filter", "web");
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
    search("alpha");
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
    search("alpha");
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
