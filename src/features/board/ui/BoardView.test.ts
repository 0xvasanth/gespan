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
  BoardColumn,
  BoardDetail,
  ColumnRole,
} from "../model/board";
import type { BoardLinkTarget } from "./boardFormat";
import type { KanbanBoardProps } from "./kanban/KanbanBoard";
import { BoardView } from "./BoardView";
import { Sidebar } from "../../../app/shell/Sidebar";
import {
  loadBoardEnabled,
  saveBoardEnabled,
  subscribeBoardEnabled,
} from "../../settings/model/settings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
// Delivers `rebase:board-changed` the way the backend emits it.
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
  return { id, boardId: "b1", name, position, role };
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
    expect(localStorage.getItem("rebase:open-board")).toBe("b1");
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
    expect(document.querySelector('[role="dialog"]')).toBeNull();

    act(() => wrapper.focus());
    press(wrapper, "Enter", { code: "Enter" });
    await flush();
    expect(wrapper.hasAttribute("data-dragging")).toBe(false);
    const dialog = document.querySelector('[role="dialog"]');
    expect(dialog?.textContent).toContain("Card #1");
  });

  it("disables Delete column while the column holds cards", async () => {
    await renderBoard();
    const deleteItem = (columnId: string) => {
      const menuButton = columnEl(columnId).querySelector<HTMLButtonElement>(
        'button[aria-label^="Column actions"]',
      )!;
      act(() => menuButton.click());
      const item = [
        ...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
      ].find((el) => el.textContent?.startsWith("Delete"))!;
      return item;
    };
    const blocked = deleteItem("backlog");
    expect(blocked.disabled).toBe(true);
    expect(blocked.getAttribute("title")).toBe("Column has 2 cards");
    press(blocked, "Escape");
    act(() => document.body.dispatchEvent(new PointerEvent("pointerdown")));
    await flush();

    const open = deleteItem("review");
    expect(open.disabled).toBe(false);
  });

  it("asks before deleting a card from its menu", async () => {
    await renderBoard();
    handlers.board_card_delete = () => undefined;
    const deleteFromMenu = async () => {
      const menu = columnEl("backlog").querySelector<HTMLButtonElement>(
        'button[aria-label="Card actions for #2"]',
      )!;
      act(() => menu.click());
      const item = [
        ...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
      ].find((el) => el.textContent === "Delete")!;
      await act(async () => item.click());
      await flush();
    };

    vi.mocked(ask).mockResolvedValue(false);
    await deleteFromMenu();
    expect(ask).toHaveBeenCalledTimes(1);
    expect(vi.mocked(ask).mock.calls[0][0]).toContain("#2");
    expect(vi.mocked(ask).mock.calls[0][0]).toContain("Write docs");
    expect(commands("board_card_delete")).toHaveLength(0);
    // The menu click doesn't also open the card.
    expect(document.querySelector('[role="dialog"]')).toBeNull();

    vi.mocked(ask).mockResolvedValue(true);
    await deleteFromMenu();
    expect(commands("board_card_delete")).toEqual([
      ["board_card_delete", { cardId: "c2" }],
    ]);
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
    expect(localStorage.getItem("rebase:open-board")).toBe("b2");
  });

  it("shows the empty state without an error when the last board is deleted", async () => {
    backend([BOARD]);
    await renderBoard();
    await deleteOpenBoard();
    expect(document.querySelector('[role="alert"]')).toBeNull();
    expect(container.textContent).toContain("No boards yet");
    expect(localStorage.getItem("rebase:open-board")).toBeNull();
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
    expect(document.querySelector('[role="dialog"]')).toBeNull();
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
      ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
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
      detail.cards[0] = { ...detail.cards[0], linkKind: "session", linkId: "s1" };
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
    const dialog = document.querySelector('[role="dialog"]')!;
    expect(dialog.textContent).toContain("Card #3");
    expect(commands("board_activity")[0][1]).toMatchObject({
      boardId: "b1",
      cardId: "c3",
    });
    expect(dialog.querySelector("[data-card-activity]")?.textContent).toBe(
      "Moved from Backlog to In Progress · system · 2m ago",
    );
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
