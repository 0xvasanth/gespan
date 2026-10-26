// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import type {
  BoardCard,
  BoardColumn,
  BoardDetail,
  BoardLabel,
  CardExtras,
  ColumnRole,
  ViewSpec,
} from "../model/board";
import { IS_MAC } from "../../../platform/tauri/platform";
import { BoardView } from "./BoardView";

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

const DAY = 86_400_000;
const BOARD = { id: "b1", name: "Agents", createdAt: 1, updatedAt: 1 };
const OTHER = { id: "b2", name: "Releases", createdAt: 2, updatedAt: 2 };

const column = (
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
const COLUMNS = [
  column("backlog", "Backlog", 0, "todo"),
  column("progress", "In Progress", 1, "doing"),
  column("review", "Review", 2, "review"),
  column("done", "Done", 3, "done"),
];
const card = (
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
const extra = (over: Partial<CardExtras> = {}): CardExtras => ({
  priority: 0,
  dueAt: null,
  projects: [],
  labels: [],
  subtasksDone: 0,
  subtasksTotal: 0,
  comments: 0,
  ...over,
});
const LABELS: BoardLabel[] = [
  { id: "l1", boardId: "b1", name: "Bug", color: "e5484d", position: 0 },
  { id: "l2", boardId: "b1", name: "UI", color: "3e63dd", position: 1 },
];

type Handler = (args: Record<string, unknown>) => unknown;
let handlers: Record<string, Handler>;
// The backend's cards, extras and archived ids: tests change them to play the other app.
let cards: BoardCard[];
let extras: Record<string, CardExtras>;
let archived: Set<string>;
let savedViews: Record<string, unknown>[];

const live = () => cards.filter((c) => !archived.has(c.id));

/** Stands in for `board_query`: title text, status and label rows, priority and title sorts. */
function fakeEngine(args: Record<string, unknown>): string[] {
  const spec = args.spec as ViewSpec;
  const order = new Map(COLUMNS.map((c) => [c.id, c.position]));
  const manual = [...live()].sort(
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

function seed() {
  cards = [
    card("c1", "backlog", 0, 1, "Fix login"),
    card("c2", "backlog", 1, 2, "Write docs"),
    card("c3", "progress", 0, 3, "Ship board"),
    card("c4", "done", 0, 4, "Deploy"),
  ];
  extras = {
    c1: extra({ priority: 3, labels: ["l1"], dueAt: Date.now() + 3 * DAY }),
    c2: extra({ priority: 1, labels: ["l1", "l2"] }),
    c4: extra({ priority: 2 }),
  };
  archived = new Set();
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
  events.listeners.clear();
  seed();
  savedViews = [];
  handlers = {
    board_query: fakeEngine,
    board_views: () => savedViews,
    board_list: () => [BOARD, OTHER],
    board_get: (args) =>
      args.boardId === "b2"
        ? {
            board: OTHER,
            columns: [{ ...column("t2", "Todo", 0, "todo"), boardId: "b2" }],
            cards: [],
          }
        : { board: BOARD, columns: COLUMNS, cards: live() },
    board_activity: () => [],
    board_card_extras: () => extras,
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
      archived.add(String(args.cardId));
    },
    board_cards_archive: (args) => {
      for (const id of args.cardIds as string[]) archived.add(id);
    },
    board_card_restore: (args) => {
      archived.delete(String(args.cardId));
    },
    board_card_delete_archived: (args) => {
      const id = String(args.cardId);
      if (archived.has(id)) cards = cards.filter((c) => c.id !== id);
    },
    board_archived_cards: () => cards.filter((c) => archived.has(c.id)),
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
        id: `v${savedViews.length + 1}`,
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
    },
    board_view_update: (args) => {
      savedViews = savedViews.map((view) =>
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
  vi.mocked(invoke).mockReset();
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
  vi.restoreAllMocks();
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

const commands = (cmd: string) =>
  vi.mocked(invoke).mock.calls.filter(([name]) => name === cmd);

function emitBoardChanged() {
  for (const listener of [...events.listeners]) {
    listener({ payload: { boardId: "b1" } });
  }
}

const MOD: MouseEventInit = IS_MAC ? { metaKey: true } : { ctrlKey: true };

async function click(el: Element, init: MouseEventInit = {}) {
  act(() => {
    el.dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true, ...init }),
    );
  });
  await flush();
}

function press(target: Element, key: string) {
  act(() => {
    target.dispatchEvent(
      new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }),
    );
  });
}

function typeInto(input: HTMLInputElement, value: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
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
  await click(item);
}

async function search(value: string) {
  typeInto(
    container.querySelector<HTMLInputElement>('input[aria-label="Search cards"]')!,
    value,
  );
  await flush();
}

const q = (selector: string) => container.querySelector<HTMLElement>(selector)!;
const qa = (selector: string) => [
  ...container.querySelectorAll<HTMLElement>(selector),
];
const rowKeys = () => qa("[data-list-row]").map((el) => el.dataset.listRow);
const groupKeys = () => qa("[data-list-group]").map((el) => el.dataset.listGroup);
const tableRows = () => qa("[data-table-row]").map((el) => el.dataset.tableRow);
const headers = () => qa("[data-table-column]").map((el) => el.dataset.tableColumn);
const selectedCount = () =>
  container.querySelector("[data-selection-count]")?.textContent ?? null;
const boardCard = (id: string) => q(`[data-board-card="${id}"]`);
const listRow = (key: string) => q(`[data-list-row="${key}"]`);
const tableRow = (id: string) => q(`[data-table-row="${id}"]`);
const bar = (text: string) =>
  [...q("[data-selection-bar]").querySelectorAll("button")].find(
    (b) => b.textContent?.trim() === text,
  )!;
const toast = () => q("[data-board-undo-toast]");

describe("List layout", () => {
  beforeEach(async () => {
    await renderBoard();
    await pick("Layout", "List");
  });

  it("groups by status, in column order, leaving empty columns out", () => {
    expect(groupKeys()).toEqual(["backlog", "progress", "done"]);
    expect(rowKeys()).toEqual([
      "backlog/c1",
      "backlog/c2",
      "progress/c3",
      "done/c4",
    ]);
  });

  it("groups by priority, urgent first", async () => {
    await pick("Group by", "Priority");
    expect(groupKeys()).toEqual(["p3", "p2", "p1", "p0"]);
    expect(rowKeys()).toEqual(["p3/c1", "p2/c4", "p1/c2", "p0/c3"]);
  });

  it("lists a card with two labels in both groups, with unique keys, and No label last", async () => {
    const errors = vi.spyOn(console, "error").mockImplementation(() => undefined);
    await pick("Group by", "Label");
    expect(groupKeys()).toEqual(["l1", "l2", "none"]);
    expect(rowKeys()).toEqual(["l1/c1", "l1/c2", "l2/c2", "none/c3", "none/c4"]);
    expect(new Set(rowKeys()).size).toBe(rowKeys().length);
    expect(q('[data-list-group="none"]').textContent).toContain("No label");
    // React warns about duplicate keys through console.error.
    expect(errors).not.toHaveBeenCalled();
  });

  it("groups by project and by due bucket, with the catch-all groups", async () => {
    await pick("Group by", "Project");
    expect(groupKeys()).toEqual(["none"]);
    expect(q('[data-list-group="none"]').textContent).toContain("No project");
    await pick("Group by", "Due date");
    expect(groupKeys()).toEqual(["next7", "none"]);
    expect(rowKeys()).toEqual(["next7/c1", "none/c2", "none/c3", "none/c4"]);
    expect(q('[data-list-group="none"]').textContent).toContain("No due date");
  });

  it("collapses and expands a group", async () => {
    const header = q('[data-list-group="backlog"]');
    expect(header.getAttribute("aria-expanded")).toBe("true");
    await click(header);
    expect(q('[data-list-group="backlog"]').getAttribute("aria-expanded")).toBe(
      "false",
    );
    expect(rowKeys()).toEqual(["progress/c3", "done/c4"]);
    await click(q('[data-list-group="backlog"]'));
    expect(rowKeys()).toContain("backlog/c1");
  });

  it("a plain click on a row opens the card", async () => {
    await click(listRow("backlog/c1"));
    expect(q("[data-card-drawer]")).not.toBeNull();
  });
});

describe("layout state in the view", () => {
  it("marks the view dirty and saves layout, groupBy and collapsed, keeping unknown keys", async () => {
    savedViews = [
      {
        id: "v1",
        boardId: "b1",
        name: "Triage",
        position: 0,
        specJson: JSON.stringify({
          query: "",
          rows: [],
          sort: { key: "manual", dir: "asc" },
        }),
        layoutJson: '{"layout":"list","future":{"x":1}}',
        createdAt: 1,
        updatedAt: 1,
      },
    ];
    await renderBoard();
    await click(q('[data-view-tab="v1"]'));
    expect(groupKeys()).toEqual(["backlog", "progress", "done"]);
    expect(q("[data-view-dot]")).toBeNull();
    await pick("Group by", "Priority");
    await click(q('[data-list-group="p3"]'));
    expect(q("[data-view-dot]")).not.toBeNull();
    const save = [...q("[data-view-tabs]").querySelectorAll("button")].find(
      (b) => b.textContent?.trim() === "Save",
    )!;
    await click(save);
    const layout = JSON.parse(
      (commands("board_view_update").at(-1)![1] as { layoutJson: string })
        .layoutJson,
    );
    expect(layout).toEqual({
      layout: "list",
      groupBy: "priority",
      collapsed: ["p3"],
      future: { x: 1 },
    });
  });

  it("returning to the saved layout clears the dot", async () => {
    savedViews = [
      {
        id: "v1",
        boardId: "b1",
        name: "Triage",
        position: 0,
        specJson: JSON.stringify({
          query: "",
          rows: [],
          sort: { key: "manual", dir: "asc" },
        }),
        layoutJson: '{"layout":"board"}',
        createdAt: 1,
        updatedAt: 1,
      },
    ];
    await renderBoard();
    await click(q('[data-view-tab="v1"]'));
    await pick("Layout", "Table");
    expect(q("[data-view-dot]")).not.toBeNull();
    await pick("Layout", "Board");
    expect(q("[data-view-dot]")).toBeNull();
  });
});

describe("Table layout", () => {
  beforeEach(async () => {
    await renderBoard();
    await pick("Layout", "Table");
  });

  it("shows the default columns, and the picker adds and removes columns", async () => {
    expect(headers()).toEqual(["title", "status", "priority", "due", "labels"]);
    expect(tableRows()).toEqual(["c1", "c2", "c3", "c4"]);
    await pick("Columns", "Estimate");
    expect(headers()).toEqual([
      "title",
      "status",
      "priority",
      "due",
      "labels",
      "estimate",
    ]);
    await pick("Columns", "Status");
    expect(headers()).not.toContain("status");
  });

  it("resizes a column from the keyboard and never under 60 px", async () => {
    const handle = () => q('[role="separator"][aria-label="Resize Due"]');
    const width = () => Number(handle().getAttribute("aria-valuenow"));
    expect(width()).toBe(110);
    press(handle(), "ArrowRight");
    expect(width()).toBe(120);
    for (let i = 0; i < 10; i += 1) press(handle(), "ArrowLeft");
    expect(width()).toBe(60);
    expect(q('[data-table-column="due"]').style.width).toBe("60px");
  });

  it("resizes a column by dragging, and clamps at 60 px", () => {
    const handle = q('[role="separator"][aria-label="Resize Title"]');
    const at = (type: string, clientX: number) =>
      act(() => {
        const event = new MouseEvent(type, { bubbles: true, clientX });
        (type === "pointerdown" ? handle : window).dispatchEvent(event);
      });
    at("pointerdown", 500);
    at("pointermove", 550);
    expect(q('[data-table-column="title"]').style.width).toBe("370px");
    at("pointermove", 100);
    expect(q('[data-table-column="title"]').style.width).toBe("60px");
    at("pointerup", 100);
    at("pointermove", 900);
    expect(q('[data-table-column="title"]').style.width).toBe("60px");
  });

  it("edits the title in place and writes it on Enter", async () => {
    await click(q('button[aria-label="Edit title of #1"]'));
    const input = q('input[aria-label="Title of #1"]') as HTMLInputElement;
    expect(input.value).toBe("Fix login");
    typeInto(input, "Fix the login");
    press(input, "Enter");
    await flush();
    expect(commands("board_card_update")).toEqual([
      ["board_card_update", { cardId: "c1", title: "Fix the login", body: "" }],
    ]);
    expect(q('input[aria-label="Title of #1"]')).toBeNull();
  });

  it("Escape drops an edit without writing, and keeps the selection", async () => {
    await click(tableRow("c2"), MOD);
    await click(q('button[aria-label="Edit title of #1"]'));
    const input = q('input[aria-label="Title of #1"]') as HTMLInputElement;
    typeInto(input, "Nope");
    press(input, "Escape");
    await flush();
    expect(commands("board_card_update")).toHaveLength(0);
    expect(q('input[aria-label="Title of #1"]')).toBeNull();
    expect(selectedCount()).toBe("1 selected");
  });

  it("edits priority and status through their menus", async () => {
    await pick("Priority of #3", "High");
    expect(commands("board_card_set_priority")).toEqual([
      ["board_card_set_priority", { cardId: "c3", priority: 3 }],
    ]);
    await pick("Status of #3", "Review");
    expect(commands("board_card_move")[0]![1]).toMatchObject({
      cardId: "c3",
      columnId: "review",
      index: 0,
    });
  });

  it("edits labels through the picker", async () => {
    await pick("Labels of #3", "UI");
    expect(commands("board_card_set_labels")).toEqual([
      ["board_card_set_labels", { cardId: "c3", labelIds: ["l2"] }],
    ]);
  });

  it("edits the due date in place and clears it when emptied", async () => {
    await click(q('button[aria-label="Edit due date of #3"]'));
    typeInto(q('input[aria-label="Due date of #3"]') as HTMLInputElement, "2027-03-04");
    press(q('input[aria-label="Due date of #3"]'), "Enter");
    await flush();
    expect(commands("board_card_set_due")[0]![1]).toEqual({
      cardId: "c3",
      dueAt: new Date(2027, 2, 4).getTime(),
    });
    await click(q('button[aria-label="Edit due date of #1"]'));
    typeInto(q('input[aria-label="Due date of #1"]') as HTMLInputElement, "");
    press(q('input[aria-label="Due date of #1"]'), "Enter");
    await flush();
    expect(commands("board_card_set_due")[1]![1]).toEqual({
      cardId: "c1",
      dueAt: null,
    });
  });

  it("keeps unsaved inline text across a refresh until Enter", async () => {
    await click(q('button[aria-label="Edit title of #1"]'));
    typeInto(q('input[aria-label="Title of #1"]') as HTMLInputElement, "Half typed");
    // The other app changes the board.
    cards = cards.map((c) => (c.id === "c2" ? { ...c, title: "Docs v2" } : c));
    extras.c2 = extra({ priority: 4 });
    emitBoardChanged();
    await flush();
    expect(
      (q('input[aria-label="Title of #1"]') as HTMLInputElement).value,
    ).toBe("Half typed");
    expect(tableRow("c2").textContent).toContain("Docs v2");
    press(q('input[aria-label="Title of #1"]'), "Enter");
    await flush();
    expect(commands("board_card_update")[0]![1]).toMatchObject({
      cardId: "c1",
      title: "Half typed",
    });
  });

  it("an untouched Enter does not overwrite a remote rename", async () => {
    await click(q('button[aria-label="Edit title of #1"]'));
    cards = cards.map((c) => (c.id === "c1" ? { ...c, title: "Renamed away" } : c));
    emitBoardChanged();
    await flush();
    press(q('input[aria-label="Title of #1"]'), "Enter");
    await flush();
    expect(commands("board_card_update")).toHaveLength(0);
    expect(tableRow("c1").textContent).toContain("Renamed away");
  });

  it("closes the edit when its card is deleted remotely", async () => {
    await click(q('button[aria-label="Edit title of #1"]'));
    typeInto(q('input[aria-label="Title of #1"]') as HTMLInputElement, "Lost");
    cards = cards.filter((c) => c.id !== "c1");
    emitBoardChanged();
    await flush();
    expect(q('input[aria-label="Title of #1"]')).toBeNull();
    expect(commands("board_card_update")).toHaveLength(0);
  });
});

describe("bulk select", () => {
  it("toggles with the platform modifier in the board layout", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    await click(boardCard("c3"), MOD);
    expect(selectedCount()).toBe("2 selected");
    expect(boardCard("c1").hasAttribute("data-selected")).toBe(true);
    // A selection click doesn't open the card.
    expect(q("[data-card-drawer]")).toBeNull();
    await click(boardCard("c1"), MOD);
    expect(selectedCount()).toBe("1 selected");
    // The other platform's modifier does nothing.
    await click(boardCard("c2"), IS_MAC ? { ctrlKey: true } : { metaKey: true });
    expect(selectedCount()).toBe("1 selected");
  });

  it("works in list and table the same way", async () => {
    await renderBoard();
    await pick("Layout", "List");
    await click(listRow("backlog/c1"), MOD);
    expect(selectedCount()).toBe("1 selected");
    await pick("Layout", "Table");
    // The selection carries over to the table's rows.
    expect(tableRow("c1").hasAttribute("data-selected")).toBe(true);
    await click(tableRow("c3"), MOD);
    expect(selectedCount()).toBe("2 selected");
  });

  it("Shift-click selects a range in the visible order, and it follows a new sort", async () => {
    await renderBoard();
    // Manual order: c1 c2 c3 c4.
    await click(boardCard("c1"), MOD);
    await click(boardCard("c3"), { shiftKey: true });
    expect(selectedCount()).toBe("3 selected");
    expect(boardCard("c2").hasAttribute("data-selected")).toBe(true);
    expect(boardCard("c4").hasAttribute("data-selected")).toBe(false);
    // Title order: Deploy, Fix login, Ship board, Write docs.
    await click(bar("Clear"));
    await pick("Sort", "Title");
    await click(boardCard("c4"), MOD);
    await click(boardCard("c3"), { shiftKey: true });
    // Deploy..Ship board covers c4, c1, c3 but not c2 (Write docs).
    expect(selectedCount()).toBe("3 selected");
    expect(boardCard("c2").hasAttribute("data-selected")).toBe(false);
    expect(boardCard("c1").hasAttribute("data-selected")).toBe(true);
  });

  it("prunes selected cards a search hides, so only what is on screen is counted", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    await click(boardCard("c3"), { shiftKey: true });
    expect(selectedCount()).toBe("3 selected");
    await search("docs");
    expect(selectedCount()).toBe("1 selected");
    // Clearing the search doesn't bring them back.
    await search("");
    expect(selectedCount()).toBe("1 selected");
    await click(bar("Delete"));
    expect(commands("board_cards_archive")).toEqual([
      ["board_cards_archive", { cardIds: ["c2"] }],
    ]);
  });

  it("drops the whole bar when the selection empties", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    await search("docs");
    expect(selectedCount()).toBeNull();
    expect(q("[data-selection-bar]")).toBeNull();
  });

  it("drops a card deleted in the other app", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    await click(boardCard("c2"), MOD);
    cards = cards.filter((c) => c.id !== "c1");
    emitBoardChanged();
    await flush();
    expect(selectedCount()).toBe("1 selected");
  });

  it("the bar sets priority and labels for every selected card", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    await click(boardCard("c3"), MOD);
    await pick("Priority for selected", "Urgent");
    expect(commands("board_cards_set_priority")).toEqual([
      ["board_cards_set_priority", { cardIds: ["c1", "c3"], priority: 4 }],
    ]);
    // c1 has Bug, c3 doesn't: choosing it adds it to both.
    await pick("Labels for selected", "Bug");
    expect(commands("board_cards_set_label")).toEqual([
      ["board_cards_set_label", { cardIds: ["c1", "c3"], labelId: "l1", on: true }],
    ]);
    // When every selected card has it, choosing it removes it.
    extras.c3 = extra({ labels: ["l1"] });
    emitBoardChanged();
    await flush();
    await pick("Labels for selected", "Bug");
    expect(commands("board_cards_set_label").at(-1)![1]).toMatchObject({
      labelId: "l1",
      on: false,
    });
    // The selection stays for more edits.
    expect(selectedCount()).toBe("2 selected");
  });

  it("the bar moves the selection to a status", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    await click(boardCard("c3"), MOD);
    await pick("Status for selected", "Review");
    expect(commands("board_cards_move")[0]![1]).toMatchObject({
      cardIds: ["c1", "c3"],
      columnId: "review",
      index: 0,
    });
    expect(toast().textContent).toContain("Moved 2 cards to Review");
  });

  it("Clear empties the selection", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    await click(bar("Clear"));
    expect(q("[data-selection-bar]")).toBeNull();
  });

  describe("archive and delete", () => {
    const selectTwo = async () => {
      await renderBoard();
      await click(boardCard("c1"), MOD);
      await click(boardCard("c2"), MOD);
    };
    const undoButton = () =>
      [...toast().querySelectorAll("button")].find(
        (b) => b.textContent === "Undo",
      )!;

    it("bulk archive goes through undo", async () => {
      await selectTwo();
      await click(bar("Archive"));
      expect(commands("board_cards_archive")).toEqual([
        ["board_cards_archive", { cardIds: ["c1", "c2"] }],
      ]);
      expect(toast().textContent).toContain("Archived 2 cards");
      expect(boardCard("c1")).toBeNull();
      expect(q("[data-selection-bar]")).toBeNull();
      await click(undoButton());
      expect(commands("board_card_restore").map(([, a]) => a)).toEqual([
        { cardId: "c1", columnId: "backlog", index: 0 },
        { cardId: "c2", columnId: "backlog", index: 1 },
      ]);
      expect(boardCard("c1")).not.toBeNull();
      expect(commands("board_card_delete_archived")).toHaveLength(0);
    });

    it("bulk delete is held, and undo puts every card back without deleting", async () => {
      await selectTwo();
      await click(bar("Delete"));
      expect(toast().textContent).toContain("Deleted 2 cards");
      expect(boardCard("c1")).toBeNull();
      expect(commands("board_card_delete_archived")).toHaveLength(0);
      await click(undoButton());
      expect(commands("board_card_restore")).toHaveLength(2);
      expect(boardCard("c2")).not.toBeNull();
      expect(commands("board_card_delete_archived")).toHaveLength(0);
    });

    it("a second bulk delete commits the first and holds the second", async () => {
      await selectTwo();
      await click(bar("Delete"));
      await click(boardCard("c3"), MOD);
      await click(boardCard("c4"), MOD);
      await click(bar("Delete"));
      expect(commands("board_card_delete_archived").map(([, a]) => a)).toEqual([
        { cardId: "c1" },
        { cardId: "c2" },
      ]);
      expect(toast().textContent).toContain("Deleted 2 cards");
      expect(cards.map((c) => c.id)).toEqual(["c3", "c4"]);
    });

    it("a single-card delete after a bulk one commits the bulk one", async () => {
      await selectTwo();
      await click(bar("Delete"));
      await click(boardCard("c3").querySelector('button[aria-label^="Card actions"]')!);
      const del = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(
        (el) => el.textContent?.trim() === "Delete",
      )!;
      await click(del);
      expect(commands("board_card_delete_archived").map(([, a]) => a)).toEqual([
        { cardId: "c1" },
        { cardId: "c2" },
      ]);
      expect(toast().textContent).toContain("Deleted #3");
    });

    it("switching boards commits a held bulk delete", async () => {
      await selectTwo();
      await click(bar("Delete"));
      act(() =>
        container
          .querySelector<HTMLButtonElement>('button[aria-label^="Board:"]')!
          .click(),
      );
      const option = [
        ...document.querySelectorAll<HTMLButtonElement>('[role="option"]'),
      ].find((el) => el.textContent?.includes("Releases"))!;
      await click(option);
      await flush();
      expect(commands("board_card_delete_archived").map(([, a]) => a)).toEqual([
        { cardId: "c1" },
        { cardId: "c2" },
      ]);
      expect(q("[data-board-undo-toast]")).toBeNull();
    });

    it("hides the held cards from the Archive view", async () => {
      await selectTwo();
      await click(bar("Delete"));
      await click(
        qa("button").find((b) => b.textContent?.trim() === "Archive")!,
      );
      expect(container.textContent).not.toContain("Fix login");
    });
  });
});

describe("Escape layering", () => {
  it("closes a menu, then clears the selection", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    act(() =>
      container
        .querySelector<HTMLButtonElement>('button[aria-label="Priority for selected"]')!
        .click(),
    );
    const menu = document.querySelector('[role="menu"]')!;
    press(menu, "Escape");
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(selectedCount()).toBe("1 selected");
    press(boardCard("c1"), "Escape");
    expect(selectedCount()).toBeNull();
  });

  it("closes the drawer first, then clears the selection", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    await click(boardCard("c2"));
    const drawer = q("[data-card-drawer]");
    expect(drawer).not.toBeNull();
    press(drawer, "Escape");
    await flush();
    expect(q("[data-card-drawer]")).toBeNull();
    expect(selectedCount()).toBe("1 selected");
    press(boardCard("c1"), "Escape");
    expect(selectedCount()).toBeNull();
  });

  it("does nothing special without a selection", async () => {
    await renderBoard();
    press(boardCard("c1"), "Escape");
    expect(q("[data-selection-bar]")).toBeNull();
  });
});

describe("windowing", () => {
  function thousand() {
    cards = Array.from({ length: 1000 }, (_, i) =>
      card(`k${i}`, "backlog", i, i + 1, `Card ${i + 1}`),
    );
    extras = {};
  }

  it("renders_only_visible_rows in the list", async () => {
    thousand();
    await renderBoard();
    await pick("Layout", "List");
    const rendered = rowKeys().length;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(60);
    // The scroller still spans every row (1 header + 1000 cards).
    expect(
      (q("[data-board-list] > div") as HTMLElement).style.height,
    ).toBe(`${1001 * 32}px`);
  });

  it("renders_only_visible_rows in the table", async () => {
    thousand();
    await renderBoard();
    await pick("Layout", "Table");
    expect(tableRows().length).toBeGreaterThan(0);
    expect(tableRows().length).toBeLessThan(60);
  });

  it("renders the rows at the scroll position", async () => {
    thousand();
    await renderBoard();
    await pick("Layout", "Table");
    expect(tableRows()[0]).toBe("k0");
    const scroller = q("[data-board-table]");
    act(() => {
      scroller.scrollTop = 32 * 500;
      scroller.dispatchEvent(new Event("scroll"));
    });
    const rows = tableRows();
    expect(rows.length).toBeLessThan(60);
    expect(rows).toContain("k500");
    expect(rows).not.toContain("k0");
  });

  it("selects a range across rows that are not rendered", async () => {
    thousand();
    await renderBoard();
    await pick("Layout", "Table");
    await click(tableRow("k0"), MOD);
    const scroller = q("[data-board-table]");
    act(() => {
      scroller.scrollTop = 32 * 500;
      scroller.dispatchEvent(new Event("scroll"));
    });
    await click(tableRow("k500"), { shiftKey: true });
    expect(selectedCount()).toBe("501 selected");
  });
});
