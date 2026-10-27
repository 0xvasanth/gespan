// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import type {
  BoardCard,
  BoardColumn,
  BoardDetail,
  BoardLabel,
  BoardSettings,
  BoardView as SavedView,
  CardExtras,
  ColumnRole,
} from "../../model/board";
import type { KanbanBoardProps } from "../kanban/KanbanBoard";
import { BoardView } from "../BoardView";
import { waitForSearch } from "../testBackend";

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

const kanban: { props: KanbanBoardProps | null } = { props: null };
vi.mock("../kanban/KanbanBoard", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../kanban/KanbanBoard")>();
  return {
    KanbanBoard: (props: KanbanBoardProps) => {
      kanban.props = props;
      return createElement(actual.KanbanBoard, props);
    },
  };
});

function column(
  id: string,
  name: string,
  position: number,
  role: ColumnRole | null,
  extra: Partial<BoardColumn> = {},
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
    ...extra,
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
    updatedAt: 1,
  };
}

const BOARD = { id: "b1", name: "Agents", createdAt: 1, updatedAt: 1 };

const DEFAULTS: BoardSettings = {
  runWith: true,
  writeBack: true,
  ticketStatusChanges: true,
  autoMoveJson: null,
  icon: null,
  color: null,
  defaultViewId: null,
  displayJson: null,
  priorityJson: null,
};

const extras = (priority: number): CardExtras => ({
  priority,
  dueAt: null,
  projects: [],
  labels: [],
  subtasksDone: 0,
  subtasksTotal: 0,
  comments: 0,
});

type Handler = (args: Record<string, unknown>) => unknown;
let handlers: Record<string, Handler>;
let detail: BoardDetail;
let settings: BoardSettings;
let labelRows: BoardLabel[];
let savedViews: SavedView[];
let cardExtras: Record<string, CardExtras>;

const label = (
  id: string,
  name: string,
  position: number,
  color = "8a867e",
): BoardLabel => ({
  id,
  boardId: "b1",
  name,
  color,
  position,
});

function view(id: string, name: string, rows: unknown[] = []): SavedView {
  return {
    id,
    boardId: "b1",
    name,
    position: savedViews.length,
    specJson: JSON.stringify({
      query: "",
      rows,
      sort: { key: "manual", dir: "asc" },
    }),
    layoutJson: '{"layout":"board"}',
    createdAt: 1,
    updatedAt: 1,
  };
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
  settings = { ...DEFAULTS };
  labelRows = [];
  savedViews = [];
  cardExtras = {};
  detail = {
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
  handlers = {
    board_list: () => [BOARD],
    board_get: () => detail,
    board_query: (args) => {
      const spec = args.spec as { query: string };
      const q = spec.query.trim().toLowerCase();
      return detail.cards
        .filter((c) => !q || c.title.toLowerCase().includes(q))
        .map((c) => c.id);
    },
    board_views: () => savedViews,
    board_activity: () => [],
    board_card_extras: () => cardExtras,
    board_labels: () => labelRows,
    board_fields: () => [],
    board_settings: () => settings,
    board_settings_update: (args) => {
      settings = { ...settings, ...(args.patch as Partial<BoardSettings>) };
      return settings;
    },
    board_rename: () => undefined,
    board_column_create: (args) =>
      column("new", String(args.name), 9, (args.role as ColumnRole) ?? null),
    board_column_update: () => undefined,
    board_column_move: () => undefined,
    board_column_delete: () => undefined,
    board_column_set_display: () => undefined,
    board_column_update_display: () => undefined,
    board_column_card_count: () => ({ live: 0, archived: 0 }),
    board_cards_move: () => undefined,
    board_card_move: () => undefined,
    board_label_create: (args) => label("new", String(args.name), 9),
    board_label_update: () => undefined,
    board_label_move: () => undefined,
    board_label_delete: () => undefined,
    board_labels_merge: () => undefined,
    board_card_set_priority: () => undefined,
  };
  vi.mocked(invoke).mockReset();
  vi.mocked(ask).mockReset();
  vi.mocked(ask).mockResolvedValue(true);
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
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
}

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

function emitBoardChanged() {
  for (const listener of [...events.listeners]) {
    listener({ payload: { boardId: "b1" } });
  }
}

const commands = (cmd: string) =>
  vi.mocked(invoke).mock.calls.filter(([name]) => name === cmd);
const lastArgs = (cmd: string) => commands(cmd).at(-1)?.[1];

async function openSettings() {
  const trigger = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Board actions"]',
  )!;
  act(() => trigger.click());
  const item = [
    ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
  ].find((el) => el.textContent?.trim() === "Board settings");
  expect(item, "Board settings menu item").toBeDefined();
  act(() => item!.click());
  await flush();
  expect(page()).not.toBeNull();
}

const page = () =>
  container.querySelector<HTMLElement>("[data-board-settings]");
const section = (name: string) =>
  page()!.querySelector<HTMLElement>(`section[aria-label="${name}"]`)!;
const byLabel = <T extends HTMLElement>(root: ParentNode, name: string) =>
  root.querySelector<T>(`[aria-label="${name}"]`)!;
const buttonNamed = (root: ParentNode, text: string) =>
  [...root.querySelectorAll<HTMLButtonElement>("button")].find(
    (el) =>
      el.textContent?.trim() === text || el.getAttribute("aria-label") === text,
  )!;

async function type(input: HTMLInputElement, value: string, commit = true) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )!.set!;
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  if (commit) {
    act(() => {
      input.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Enter",
          bubbles: true,
          cancelable: true,
        }),
      );
    });
  }
  await flush();
}

async function choose(select: HTMLSelectElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLSelectElement.prototype,
    "value",
  )!.set!;
  act(() => {
    setter.call(select, value);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await flush();
}

async function click(el: HTMLElement) {
  act(() => el.click());
  await flush();
}

const columnEl = (id: string) =>
  container.querySelector<HTMLElement>(`[data-kanban-column="${id}"]`)!;
const countOf = (id: string) =>
  columnEl(id).querySelector("[data-column-count]")?.textContent;
const toastText = () =>
  document.querySelector("[data-board-undo-toast]")?.textContent ?? null;
const alertText = () =>
  document.querySelector('[role="alert"]')?.textContent ?? "";

describe("settings page: general", () => {
  it("settings_page_renames_board_and_sets_icon_and_colour", async () => {
    await renderBoard();
    await openSettings();
    const general = section("General");

    await type(byLabel<HTMLInputElement>(general, "Board name"), "Platform");
    expect(commands("board_rename")).toEqual([
      ["board_rename", { boardId: "b1", name: "Platform" }],
    ]);

    // Each choice writes only its own setting: the other app's toggles survive.
    await click(byLabel(general, "rocket"));
    expect(lastArgs("board_settings_update")).toEqual({
      boardId: "b1",
      patch: { icon: "rocket" },
    });
    await click(byLabel(byLabel(general, "Colour"), "Blue"));
    expect(lastArgs("board_settings_update")).toEqual({
      boardId: "b1",
      patch: { color: "4da3f5" },
    });
    expect(settings).toMatchObject({
      icon: "rocket",
      color: "4da3f5",
      writeBack: true,
    });

    emitBoardChanged();
    await flush();
    expect(byLabel(page()!, "rocket").getAttribute("aria-checked")).toBe(
      "true",
    );
    expect(byLabel(page()!, "bug").getAttribute("aria-checked")).toBe("false");
    expect(byLabel(page()!, "Blue").getAttribute("aria-checked")).toBe(
      "true",
    );

    // "No colour" clears it back to the default.
    await click(byLabel(general, "No colour"));
    expect(lastArgs("board_settings_update")).toEqual({
      boardId: "b1",
      patch: { color: null },
    });
  });

  it("offers exactly the eight icons and the eight palette colours", async () => {
    await renderBoard();
    await openSettings();
    const icons = [
      ...byLabel(section("General"), "Icon").querySelectorAll("[role=radio]"),
    ].map((el) => el.getAttribute("aria-label"));
    expect(icons).toEqual([
      "board",
      "bug",
      "rocket",
      "check",
      "star",
      "flag",
      "bolt",
      "book",
    ]);
    const colours = [
      ...byLabel(section("General"), "Colour").querySelectorAll("[role=radio]"),
    ].map((el) => el.getAttribute("aria-label"));
    expect(colours).toEqual([
      "No colour",
      "Grey",
      "Red",
      "Orange",
      "Yellow",
      "Green",
      "Teal",
      "Blue",
      "Purple",
    ]);
  });

  it("does not rename when the name is unchanged or emptied", async () => {
    await renderBoard();
    await openSettings();
    const input = byLabel<HTMLInputElement>(section("General"), "Board name");
    await type(input, "Agents");
    await type(input, "   ");
    expect(commands("board_rename")).toEqual([]);
  });

  it("default_view_opens_on_board_load", async () => {
    savedViews = [
      view("v1", "High", [{ field: "priority", op: "is", values: ["3"] }]),
      view("v2", "Other"),
    ];
    settings = { ...DEFAULTS, defaultViewId: "v1" };
    await renderBoard();
    const tab = (id: string) =>
      container.querySelector<HTMLElement>(`[data-view-tab="${id}"]`)!;
    expect(tab("v1").getAttribute("aria-selected")).toBe("true");
    expect(tab("all").getAttribute("aria-selected")).toBe("false");
    const query = commands("board_query").at(-1)![1] as {
      spec: { rows: unknown[] };
    };
    expect(query.spec.rows).toEqual([
      { field: "priority", op: "is", values: ["3"] },
    ]);
  });

  it("default view is applied once: leaving it for All cards sticks through refreshes", async () => {
    savedViews = [view("v1", "Mine")];
    settings = { ...DEFAULTS, defaultViewId: "v1" };
    await renderBoard();
    const tab = (id: string) =>
      container.querySelector<HTMLElement>(`[data-view-tab="${id}"]`)!;
    await click(tab("all"));
    emitBoardChanged();
    await flush();
    expect(tab("all").getAttribute("aria-selected")).toBe("true");
  });

  it("a default view that was deleted opens All cards without an error", async () => {
    savedViews = [view("v2", "Other")];
    settings = { ...DEFAULTS, defaultViewId: "gone" };
    await renderBoard();
    expect(
      container
        .querySelector<HTMLElement>('[data-view-tab="all"]')!
        .getAttribute("aria-selected"),
    ).toBe("true");
    expect(alertText()).toBe("");
  });

  it("picks the default view from the page and clears it with All cards", async () => {
    savedViews = [view("v1", "Mine")];
    await renderBoard();
    await openSettings();
    const select = byLabel<HTMLSelectElement>(
      section("General"),
      "Default view",
    );
    expect([...select.options].map((o) => o.textContent)).toEqual([
      "All cards",
      "Mine",
    ]);
    await choose(select, "v1");
    expect(lastArgs("board_settings_update")).toEqual({
      boardId: "b1",
      patch: { defaultViewId: "v1" },
    });
    await choose(select, "");
    expect(lastArgs("board_settings_update")).toEqual({
      boardId: "b1",
      patch: { defaultViewId: null },
    });
  });

  it("opens from the board menu and closes back to the board", async () => {
    await renderBoard();
    await openSettings();
    await click(byLabel(page()!, "Back to board"));
    expect(page()).toBeNull();
    expect(columnEl("backlog")).not.toBeNull();
  });
});

describe("settings page: columns", () => {
  const row = (id: string) =>
    section("Columns").querySelector<HTMLElement>(
      `[data-settings-column="${id}"]`,
    )!;

  it("column_manager_adds_recolours_sets_role_and_reorders", async () => {
    await renderBoard();
    await openSettings();

    await type(
      byLabel<HTMLInputElement>(section("Columns"), "New column"),
      "QA",
    );
    expect(commands("board_column_create")).toEqual([
      ["board_column_create", { boardId: "b1", name: "QA", role: null }],
    ]);

    await click(byLabel(byLabel(row("backlog"), "Column colour"), "Red"));
    expect(commands("board_column_update_display")).toEqual([
      [
        "board_column_update_display",
        { columnId: "backlog", patch: { color: "e5534b" } },
      ],
    ]);

    await type(
      byLabel<HTMLInputElement>(row("progress"), "Column name"),
      "Doing",
    );
    expect(lastArgs("board_column_update")).toEqual({
      columnId: "progress",
      name: "Doing",
      role: "doing",
    });

    // Roles other columns hold can't be taken; clearing one is allowed.
    const role = byLabel<HTMLSelectElement>(row("review"), "Role");
    const disabled = [...role.options]
      .filter((o) => o.disabled)
      .map((o) => o.value);
    expect(disabled.sort()).toEqual(["doing", "done", "todo"]);
    await choose(role, "");
    expect(lastArgs("board_column_update")).toEqual({
      columnId: "review",
      name: "Review",
      role: null,
    });

    await click(buttonNamed(row("backlog"), "Move down"));
    expect(lastArgs("board_column_move")).toEqual({
      columnId: "backlog",
      index: 1,
    });
    await click(buttonNamed(row("done"), "Move up"));
    expect(lastArgs("board_column_move")).toEqual({
      columnId: "done",
      index: 2,
    });
    expect(buttonNamed(row("backlog"), "Move up").disabled).toBe(true);
    expect(buttonNamed(row("done"), "Move down").disabled).toBe(true);
  });

  it("sets and clears a WIP limit with a patch that leaves colour and collapsed state alone", async () => {
    detail.columns[1] = column("progress", "In Progress", 1, "doing", {
      color: "3fb950",
      collapsed: false,
    });
    await renderBoard();
    await openSettings();
    const input = byLabel<HTMLInputElement>(row("progress"), "WIP limit");
    await type(input, "3");
    expect(lastArgs("board_column_update_display")).toEqual({
      columnId: "progress",
      patch: { wipLimit: 3 },
    });
    await type(input, "");
    expect(lastArgs("board_column_update_display")).toEqual({
      columnId: "progress",
      patch: { wipLimit: null },
    });
    expect(commands("board_column_set_display")).toEqual([]);
  });

  it("refuses a limit of 0 or a non-number without writing", async () => {
    await renderBoard();
    await openSettings();
    const input = byLabel<HTMLInputElement>(row("progress"), "WIP limit");
    await type(input, "0");
    await type(input, "abc");
    expect(commands("board_column_update_display")).toEqual([]);
    expect(commands("board_column_set_display")).toEqual([]);
    expect(alertText()).toContain("WIP limit");
  });

  it("delete_column_with_cards_offers_move_cards_to", async () => {
    handlers.board_column_card_count = () => ({ live: 2, archived: 0 });
    await renderBoard();
    await openSettings();
    await click(buttonNamed(row("backlog"), "Delete column"));
    // Nothing is deleted yet: the cards need a place to go first.
    expect(commands("board_column_delete")).toEqual([]);
    const panel =
      row("backlog").querySelector<HTMLElement>("[data-move-cards]")!;
    expect(panel.textContent).toContain("Move cards to…");
    const select = byLabel<HTMLSelectElement>(panel, "Move cards to…");
    expect([...select.options].map((o) => o.value)).toEqual([
      "progress",
      "review",
      "done",
    ]);
    await choose(select, "progress");
    await click(buttonNamed(panel, "Move and delete"));
    const log = vi
      .mocked(invoke)
      .mock.calls.map(([name, args]) => [name, args])
      .filter(
        ([name]) =>
          name === "board_cards_move" || name === "board_column_delete",
      );
    // The live cards move first (to the end of the target), then the column goes.
    expect(log).toEqual([
      [
        "board_cards_move",
        {
          cardIds: ["c1", "c2"],
          columnId: "progress",
          index: 1,
          actor: "user",
        },
      ],
      ["board_column_delete", { columnId: "backlog" }],
    ]);
  });

  it("deletes an empty column straight away", async () => {
    await renderBoard();
    await openSettings();
    await click(buttonNamed(row("review"), "Delete column"));
    expect(commands("board_column_delete")).toEqual([
      ["board_column_delete", { columnId: "review" }],
    ]);
    expect(row("review").querySelector("[data-move-cards]")).toBeNull();
  });

  it("says up front that archived cards block the delete, and moves nothing", async () => {
    handlers.board_column_card_count = () => ({ live: 2, archived: 2 });
    await renderBoard();
    await openSettings();
    await click(buttonNamed(row("backlog"), "Delete column"));
    // No "Move cards to…" panel, no move, no delete: nothing half-happens.
    expect(row("backlog").querySelector("[data-move-cards]")).toBeNull();
    expect(commands("board_cards_move")).toEqual([]);
    expect(commands("board_column_delete")).toEqual([]);
    expect(alertText()).toContain("2 archived cards");
  });

  it("a column holding only archived cards is refused before any call", async () => {
    handlers.board_column_card_count = () => ({ live: 0, archived: 1 });
    await renderBoard();
    await openSettings();
    await click(buttonNamed(row("review"), "Delete column"));
    expect(commands("board_cards_move")).toEqual([]);
    expect(commands("board_column_delete")).toEqual([]);
    expect(alertText()).toContain("1 archived card.");
  });
});

describe("board columns", () => {
  async function withCards(limit: number | null, inBacklog: number) {
    detail.columns[0] = column("backlog", "Backlog", 0, "todo", {
      wipLimit: limit,
    });
    detail.cards = [
      ...Array.from({ length: inBacklog }, (_, i) =>
        card(`b${i}`, "backlog", i, i + 1, `Backlog card ${i}`),
      ),
      card("c3", "progress", 0, 90, "Ship board"),
    ];
    await renderBoard();
  }

  it("wip_count_shows_3_of_4_and_highlights_over_limit", async () => {
    await withCards(4, 3);
    expect(countOf("backlog")).toBe("3/4");
    expect(columnEl("backlog").querySelector("[data-wip-over]")).toBeNull();
    // A column with no limit keeps its bare count.
    expect(countOf("progress")).toBe("1");

    detail = {
      ...detail,
      cards: [
        ...detail.cards,
        card("x1", "backlog", 3, 91, "Four"),
        card("x2", "backlog", 4, 92, "Five"),
      ],
    };
    emitBoardChanged();
    await flush();
    expect(countOf("backlog")).toBe("5/4");
    expect(columnEl("backlog").querySelector("[data-wip-over]")).not.toBeNull();
    // At the limit is not over it.
    detail = {
      ...detail,
      cards: detail.cards.filter((c) => c.id !== "x2"),
    };
    emitBoardChanged();
    await flush();
    expect(countOf("backlog")).toBe("4/4");
    expect(columnEl("backlog").querySelector("[data-wip-over]")).toBeNull();
  });

  it("wip_count_uses_total_while_filtered", async () => {
    await withCards(2, 3);
    const search = container.querySelector<HTMLInputElement>(
      'input[aria-label="Search cards"]',
    )!;
    await type(search, "card 1", false);
    await waitForSearch();
    // One of three shown; the limit (2) still compares against the total (3).
    expect(countOf("backlog")).toBe("1/3");
    expect(columnEl("backlog").querySelector("[data-wip-over]")).not.toBeNull();
    await type(search, "", false);
    await waitForSearch();
    expect(countOf("backlog")).toBe("3/2");
  });

  it("drop_over_wip_limit_warns_and_still_saves", async () => {
    detail.columns[1] = column("progress", "In Progress", 1, "doing", {
      wipLimit: 1,
    });
    await renderBoard();
    act(() =>
      kanban.props!.onCardMove("c1", { columnId: "progress", index: 1 }),
    );
    await flush();
    expect(commands("board_card_move")).toEqual([
      [
        "board_card_move",
        { cardId: "c1", columnId: "progress", index: 1, actor: "user" },
      ],
    ]);
    expect(toastText()).toContain("Over the WIP limit (2/1)");
  });

  it("no warning when a drop stays within the limit or only reorders an over-limit column", async () => {
    detail.columns[1] = column("progress", "In Progress", 1, "doing", {
      wipLimit: 2,
    });
    detail.cards.push(
      card("c4", "progress", 1, 4, "Another"),
      card("c5", "progress", 2, 5, "Third"),
    );
    await renderBoard();
    // Already 3/2. Reordering inside it is no new overflow.
    act(() =>
      kanban.props!.onCardMove("c5", { columnId: "progress", index: 0 }),
    );
    await flush();
    expect(toastText() ?? "").not.toContain("WIP");
    // Into a column with no limit.
    act(() => kanban.props!.onCardMove("c1", { columnId: "review", index: 0 }));
    await flush();
    expect(toastText() ?? "").not.toContain("WIP");
  });

  it("shows a column's colour as a stripe", async () => {
    detail.columns[2] = column("review", "Review", 2, "review", {
      color: "a371f7",
    });
    await renderBoard();
    expect(
      columnEl("review")
        .querySelector("[data-column-color]")
        ?.getAttribute("data-column-color"),
    ).toBe("a371f7");
    expect(columnEl("backlog").querySelector("[data-column-color]")).toBeNull();
  });
});

describe("settings page: labels", () => {
  const row = (id: string) =>
    section("Labels").querySelector<HTMLElement>(
      `[data-settings-label="${id}"]`,
    )!;
  beforeEach(() => {
    labelRows = [
      label("l1", "Bug", 0, "e5534b"),
      label("l2", "Docs", 1, "4da3f5"),
      label("l3", "Ops", 2, "2bb3a8"),
    ];
  });

  it("label_manager_creates_renames_recolours_reorders_merges_deletes", async () => {
    await renderBoard();
    await openSettings();

    await type(
      byLabel<HTMLInputElement>(section("Labels"), "New label"),
      "Infra",
    );
    // The next palette colour after three labels.
    expect(commands("board_label_create")).toEqual([
      ["board_label_create", { boardId: "b1", name: "Infra", color: "e8c547" }],
    ]);

    await type(
      byLabel<HTMLInputElement>(row("l2"), "Label name"),
      "Documentation",
    );
    expect(lastArgs("board_label_update")).toEqual({
      labelId: "l2",
      name: "Documentation",
      color: null,
    });

    await click(byLabel(byLabel(row("l1"), "Label colour"), "Purple"));
    expect(lastArgs("board_label_update")).toEqual({
      labelId: "l1",
      name: null,
      color: "a371f7",
    });

    await click(buttonNamed(row("l3"), "Move up"));
    expect(lastArgs("board_label_move")).toEqual({ labelId: "l3", index: 1 });
    expect(buttonNamed(row("l1"), "Move up").disabled).toBe(true);
    expect(buttonNamed(row("l3"), "Move down").disabled).toBe(true);

    await choose(byLabel<HTMLSelectElement>(row("l1"), "Merge into…"), "l2");
    await click(buttonNamed(row("l1"), "Merge"));
    expect(lastArgs("board_labels_merge")).toEqual({
      fromId: "l1",
      intoId: "l2",
    });

    await click(buttonNamed(row("l3"), "Delete label"));
    expect(lastArgs("board_label_delete")).toEqual({ labelId: "l3" });
  });

  it("refuses a name another label has, ignoring case, before writing", async () => {
    await renderBoard();
    await openSettings();
    await type(
      byLabel<HTMLInputElement>(section("Labels"), "New label"),
      " bug ",
    );
    expect(commands("board_label_create")).toEqual([]);
    expect(alertText()).toContain("already exists");
    await type(byLabel<HTMLInputElement>(row("l2"), "Label name"), "OPS");
    expect(commands("board_label_update")).toEqual([]);
    // Changing only the case of its own name is fine.
    await type(byLabel<HTMLInputElement>(row("l2"), "Label name"), "DOCS");
    expect(lastArgs("board_label_update")).toEqual({
      labelId: "l2",
      name: "DOCS",
      color: null,
    });
  });

  it("merge and delete each ask first, and a No changes nothing", async () => {
    vi.mocked(ask).mockResolvedValue(false);
    await renderBoard();
    await openSettings();
    await choose(byLabel<HTMLSelectElement>(row("l1"), "Merge into…"), "l2");
    await click(buttonNamed(row("l1"), "Merge"));
    await click(buttonNamed(row("l3"), "Delete label"));
    expect(ask).toHaveBeenCalledTimes(2);
    expect(commands("board_labels_merge")).toEqual([]);
    expect(commands("board_label_delete")).toEqual([]);
  });

  it("can't merge a label into itself", async () => {
    await renderBoard();
    await openSettings();
    const select = byLabel<HTMLSelectElement>(row("l1"), "Merge into…");
    expect([...select.options].map((o) => o.value)).not.toContain("l1");
    expect(buttonNamed(row("l1"), "Merge").disabled).toBe(true);
  });
});

describe("priority scheme", () => {
  const SCHEME = JSON.stringify({
    names: ["None", "Minor", "Normal", "Major", "Blocker"],
    hidden: [],
  });
  const face = (id: string) =>
    container.querySelector<HTMLElement>(`[data-board-card='${id}']`)!;
  const faceItems = async (id: string, number: number) => {
    act(() =>
      face(id)
        .querySelector<HTMLButtonElement>(
          `button[aria-label="Priority of #${number}"]`,
        )!
        .click(),
    );
    const items = [
      ...document.querySelectorAll<HTMLElement>('[role^="menuitem"]'),
    ].map((el) => el.textContent?.trim());
    return items;
  };

  it("priority_renames_show_on_cards_and_pickers", async () => {
    settings = { ...DEFAULTS, priorityJson: SCHEME };
    cardExtras = { c1: extras(4), c2: extras(1) };
    await renderBoard();
    expect(face("c1").querySelector("[data-card-priority]")?.textContent).toBe(
      "Blocker",
    );
    expect(face("c2").querySelector("[data-card-priority]")?.textContent).toBe(
      "Minor",
    );
    expect(await faceItems("c1", 1)).toEqual([
      "None",
      "Minor",
      "Normal",
      "Major",
      "Blocker",
    ]);
  });

  it("hidden_priority_leaves_pickers_but_cards_keep_showing_it", async () => {
    settings = {
      ...DEFAULTS,
      priorityJson: JSON.stringify({
        names: ["None", "Low", "Medium", "High", "Urgent"],
        hidden: [1, 2],
      }),
    };
    cardExtras = { c2: extras(1) };
    await renderBoard();
    // The card that already has a hidden level still shows it...
    expect(face("c2").querySelector("[data-card-priority]")?.textContent).toBe(
      "Low",
    );
    // ...but no picker offers it.
    expect(await faceItems("c2", 2)).toEqual(["None", "High", "Urgent"]);
  });

  it("edits names and visibility from the page, writing only the scheme", async () => {
    await renderBoard();
    await openSettings();
    const row = (level: number) =>
      section("Priority").querySelector<HTMLElement>(
        `[data-settings-priority="${level}"]`,
      )!;
    expect(
      [0, 1, 2, 3, 4].map(
        (level) => byLabel<HTMLInputElement>(row(level), "Priority name").value,
      ),
    ).toEqual(["None", "Low", "Medium", "High", "Urgent"]);

    await type(byLabel<HTMLInputElement>(row(4), "Priority name"), " Blocker ");
    expect(lastArgs("board_settings_update")).toEqual({
      boardId: "b1",
      patch: {
        priorityJson: JSON.stringify({
          names: ["None", "Low", "Medium", "High", "Blocker"],
          hidden: [],
        }),
      },
    });

    await click(byLabel(row(2), "Show Medium in pickers"));
    expect(
      JSON.parse(
        (
          lastArgs("board_settings_update") as {
            patch: { priorityJson: string };
          }
        ).patch.priorityJson,
      ),
    ).toEqual({
      names: ["None", "Low", "Medium", "High", "Blocker"],
      hidden: [2],
    });

    // Level 0 can't be hidden.
    expect(
      byLabel<HTMLInputElement>(row(0), "Show None in pickers").disabled,
    ).toBe(true);
  });

  it("refuses an empty or 21-character name without writing", async () => {
    await renderBoard();
    await openSettings();
    const input = byLabel<HTMLInputElement>(
      section("Priority").querySelector<HTMLElement>(
        '[data-settings-priority="3"]',
      )!,
      "Priority name",
    );
    await type(input, "");
    await type(input, "x".repeat(21));
    expect(commands("board_settings_update")).toEqual([]);
    expect(alertText()).toContain("1 to 20");
    await type(input, "x".repeat(20));
    expect(commands("board_settings_update")).toHaveLength(1);
  });

  it("reset puts the defaults back by clearing the stored scheme", async () => {
    settings = { ...DEFAULTS, priorityJson: SCHEME };
    await renderBoard();
    await openSettings();
    await click(buttonNamed(section("Priority"), "Reset to defaults"));
    expect(lastArgs("board_settings_update")).toEqual({
      boardId: "b1",
      patch: { priorityJson: null },
    });
  });
});

describe("settings accessibility and rapid edits", () => {
  it("two quick priority renames both survive, written in order", async () => {
    const replies: (() => void)[] = [];
    handlers.board_settings_update = (args) =>
      new Promise((resolve) => {
        replies.push(() => {
          settings = { ...settings, ...(args.patch as Partial<BoardSettings>) };
          resolve(settings);
        });
      });
    await renderBoard();
    await openSettings();
    const name = (level: number) =>
      byLabel<HTMLInputElement>(
        section("Priority").querySelector<HTMLElement>(
          `[data-settings-priority="${level}"]`,
        )!,
        "Priority name",
      );
    await type(name(1), "Minor");
    await type(name(2), "Normal");
    // The second waits for the first reply, then builds on the first edit.
    expect(replies).toHaveLength(1);
    replies.shift()!();
    await flush();
    expect(replies).toHaveLength(1);
    replies.shift()!();
    await flush();
    const writes = commands("board_settings_update").map(
      ([, args]) =>
        JSON.parse(
          (args as { patch: { priorityJson: string } }).patch.priorityJson,
        ).names,
    );
    expect(writes[0]).toEqual(["None", "Minor", "Medium", "High", "Urgent"]);
    expect(writes[1]).toEqual(["None", "Minor", "Normal", "High", "Urgent"]);
    expect(JSON.parse(settings.priorityJson!).names[1]).toBe("Minor");
  });

  it("closing the page returns focus to Board actions", async () => {
    await renderBoard();
    await openSettings();
    await click(byLabel(page()!, "Back to board"));
    expect(document.activeElement).toBe(
      container.querySelector('button[aria-label="Board actions"]'),
    );
  });

  it("names the palette colours instead of hex codes", async () => {
    await renderBoard();
    await openSettings();
    const names = [
      ...byLabel(section("General"), "Colour").querySelectorAll("[role=radio]"),
    ].map((el) => el.getAttribute("aria-label"));
    expect(names).toEqual([
      "No colour",
      "Grey",
      "Red",
      "Orange",
      "Yellow",
      "Green",
      "Teal",
      "Blue",
      "Purple",
    ]);
  });

  it("radio groups have one tab stop and move and choose with the arrow keys", async () => {
    settings = { ...DEFAULTS, icon: "bug" };
    await renderBoard();
    await openSettings();
    const group = byLabel(section("General"), "Icon");
    const radios = () => [...group.querySelectorAll<HTMLElement>("[role=radio]")];
    // The checked one is the tab stop.
    expect(radios().map((el) => el.tabIndex)).toEqual([-1, 0, -1, -1, -1, -1, -1, -1]);
    const key = async (target: HTMLElement, k: string) => {
      act(() => {
        target.dispatchEvent(
          new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }),
        );
      });
      await flush();
    };
    await key(radios()[1], "ArrowRight");
    expect(lastArgs("board_settings_update")).toEqual({
      boardId: "b1",
      patch: { icon: "rocket" },
    });
    expect(document.activeElement).toBe(radios()[2]);
    // Wraps from the first back to the last.
    await key(radios()[0], "ArrowLeft");
    expect(lastArgs("board_settings_update")).toEqual({
      boardId: "b1",
      patch: { icon: "book" },
    });
    // With nothing checked, the first radio is the tab stop.
    settings = { ...DEFAULTS };
    emitBoardChanged();
    await flush();
    expect(radios().map((el) => el.tabIndex)[0]).toBe(0);
  });
});

describe("unread settings are never overwritten with defaults (F1)", () => {
  const STORED_SCHEME = JSON.stringify({
    names: ["None", "Minor", "Normal", "Major", "Blocker"],
    hidden: [3],
  });
  const STORED_DISPLAY = JSON.stringify({ density: "compact" });
  const priorityRow = (level: number) =>
    section("Priority").querySelector<HTMLElement>(
      `[data-settings-priority="${level}"]`,
    )!;
  const displayMenu = () => {
    const trigger = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Display"]',
    )!;
    if (trigger.getAttribute("aria-expanded") !== "true") {
      act(() => trigger.click());
    }
    // Only the menu can say it while the settings page is closed.
    return document.body;
  };
  const closeDisplayMenu = () => {
    const trigger = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Display"]',
    )!;
    if (trigger.getAttribute("aria-expanded") === "true") {
      act(() => trigger.click());
    }
  };
  const menuBox = (name: string) =>
    document.querySelector<HTMLInputElement>(`input[aria-label="${name}"]`)!;

  it("when getBoardSettings fails, Priority, Display and the toolbar's board default write nothing", async () => {
    let readable = false;
    settings = {
      ...DEFAULTS,
      priorityJson: STORED_SCHEME,
      displayJson: STORED_DISPLAY,
    };
    handlers.board_settings = () => {
      if (!readable) throw "store is locked";
      return settings;
    };
    await renderBoard();

    // The toolbar's Display menu on "All cards" edits the board default:
    // disabled, with the same message and Retry.
    const menu = displayMenu();
    expect(menu.textContent).toContain("Couldn't load settings");
    expect(menuBox("Show Card number").disabled).toBe(true);
    await click(menuBox("Show Card number"));
    await click(menuBox("Show Priority"));
    await choose(
      document.querySelector<HTMLSelectElement>('select[aria-label="Swimlanes"]')!,
      "priority",
    );
    expect(commands("board_settings_update")).toEqual([]);
    closeDisplayMenu();
    await flush();

    await openSettings();
    // Priority: the message, every control disabled, and no write.
    const priority = section("Priority");
    expect(priority.textContent).toContain("Couldn't load settings");
    const name = byLabel<HTMLInputElement>(priorityRow(4), "Priority name");
    expect(name.disabled).toBe(true);
    await type(name, "Critical");
    const show = byLabel<HTMLInputElement>(priorityRow(2), "Show Medium in pickers");
    expect(show.disabled).toBe(true);
    await click(show);
    const reset = buttonNamed(priority, "Reset to defaults");
    expect(reset.disabled).toBe(true);
    await click(reset);

    // Display: the same.
    const display = section("Display");
    expect(display.textContent).toContain("Couldn't load settings");
    const number = byLabel<HTMLInputElement>(display, "Show Card number");
    expect(number.disabled).toBe(true);
    await click(number);
    await click(buttonNamed(display, "Compact"));
    await choose(byLabel<HTMLSelectElement>(display, "Swimlanes"), "label");

    expect(commands("board_settings_update")).toEqual([]);

    // Retry reads them; edits then build on what is stored.
    readable = true;
    await click(buttonNamed(section("Priority"), "Retry"));
    expect(section("Priority").textContent).not.toContain("Couldn't load settings");
    expect(section("Display").textContent).not.toContain("Couldn't load settings");
    const renamed = byLabel<HTMLInputElement>(priorityRow(4), "Priority name");
    expect(renamed.disabled).toBe(false);
    expect(renamed.value).toBe("Blocker");
    await type(renamed, "Critical");
    expect(commands("board_settings_update")).toHaveLength(1);
    expect(
      JSON.parse(
        (lastArgs("board_settings_update") as { patch: { priorityJson: string } })
          .patch.priorityJson,
      ),
    ).toEqual({
      names: ["None", "Minor", "Normal", "Major", "Critical"],
      hidden: [3],
    });
    await click(byLabel<HTMLInputElement>(section("Display"), "Show Card number"));
    const displayJson = JSON.parse(
      (lastArgs("board_settings_update") as { patch: { displayJson: string } })
        .patch.displayJson,
    );
    // The stored density survives the edit.
    expect(displayJson.density).toBe("compact");
    expect(displayJson.face.number).toBe(false);
  });

  it("with unread settings a saved view's display override stays editable", async () => {
    handlers.board_settings = () => {
      throw "store is locked";
    };
    savedViews = [view("v1", "Mine")];
    await renderBoard();
    await click(container.querySelector<HTMLElement>('[data-view-tab="v1"]')!);
    const menu = displayMenu();
    expect(menu.textContent).not.toContain("Couldn't load settings");
    expect(menuBox("Show Card number").disabled).toBe(false);
    await click(menuBox("Show Card number"));
    // A view override is view state, not a settings write.
    expect(commands("board_settings_update")).toEqual([]);
    expect(container.querySelector("[data-view-dot]")).not.toBeNull();
  });
});
