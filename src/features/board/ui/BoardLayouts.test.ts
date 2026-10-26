// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { IS_MAC } from "../../../platform/tauri/platform";
import { BoardView } from "./BoardView";
import {
  BOARD,
  COLUMNS,
  createTestBackend,
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

const commands = (cmd: string) =>
  vi.mocked(invoke).mock.calls.filter(([name]) => name === cmd);

const emitBoardChanged = () => backend.emitBoardChanged();

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
    expect(groupKeys()).toEqual([
      "status:backlog",
      "status:progress",
      "status:done",
    ]);
    expect(rowKeys()).toEqual([
      "status:backlog/c1",
      "status:backlog/c2",
      "status:progress/c3",
      "status:done/c4",
    ]);
  });

  it("groups by priority, urgent first", async () => {
    await pick("Group by", "Priority");
    expect(groupKeys()).toEqual([
      "priority:p3",
      "priority:p2",
      "priority:p1",
      "priority:p0",
    ]);
    expect(rowKeys()).toEqual([
      "priority:p3/c1",
      "priority:p2/c4",
      "priority:p1/c2",
      "priority:p0/c3",
    ]);
  });

  it("lists a card with two labels in both groups, with unique keys, and No label last", async () => {
    const errors = vi.spyOn(console, "error").mockImplementation(() => undefined);
    await pick("Group by", "Label");
    expect(groupKeys()).toEqual(["label:l1", "label:l2", "label:none"]);
    expect(rowKeys()).toEqual([
      "label:l1/c1",
      "label:l1/c2",
      "label:l2/c2",
      "label:none/c3",
      "label:none/c4",
    ]);
    expect(new Set(rowKeys()).size).toBe(rowKeys().length);
    expect(q('[data-list-group="label:none"]').textContent).toContain("No label");
    // React warns about duplicate keys through console.error.
    expect(errors).not.toHaveBeenCalled();
  });

  it("groups by project and by due bucket, with the catch-all groups", async () => {
    await pick("Group by", "Project");
    expect(groupKeys()).toEqual(["project:none"]);
    expect(q('[data-list-group="project:none"]').textContent).toContain(
      "No project",
    );
    await pick("Group by", "Due date");
    expect(groupKeys()).toEqual(["due:next7", "due:none"]);
    expect(rowKeys()).toEqual([
      "due:next7/c1",
      "due:none/c2",
      "due:none/c3",
      "due:none/c4",
    ]);
    expect(q('[data-list-group="due:none"]').textContent).toContain(
      "No due date",
    );
  });

  it("collapses and expands a group", async () => {
    const header = q('[data-list-group="status:backlog"]');
    expect(header.getAttribute("aria-expanded")).toBe("true");
    await click(header);
    expect(q('[data-list-group="status:backlog"]').getAttribute("aria-expanded")).toBe(
      "false",
    );
    expect(rowKeys()).toEqual(["status:progress/c3", "status:done/c4"]);
    await click(q('[data-list-group="status:backlog"]'));
    expect(rowKeys()).toContain("status:backlog/c1");
  });

  it("a plain click on a row opens the card", async () => {
    await click(listRow("status:backlog/c1"));
    expect(q("[data-card-drawer]")).not.toBeNull();
  });
});

describe("layout state in the view", () => {
  it("marks the view dirty and saves layout, groupBy and collapsed, keeping unknown keys", async () => {
    backend.savedViews = [
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
    expect(groupKeys()).toEqual([
      "status:backlog",
      "status:progress",
      "status:done",
    ]);
    expect(q("[data-view-dot]")).toBeNull();
    await pick("Group by", "Priority");
    await click(q('[data-list-group="priority:p3"]'));
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
      collapsed: ["priority:p3"],
      future: { x: 1 },
    });
  });

  it("returning to the saved layout clears the dot", async () => {
    backend.savedViews = [
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
    backend.cards = backend.cards.map((c) => (c.id === "c2" ? { ...c, title: "Docs v2" } : c));
    backend.extras.c2 = extra({ priority: 4 });
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
    backend.cards = backend.cards.map((c) => (c.id === "c1" ? { ...c, title: "Renamed away" } : c));
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
    backend.cards = backend.cards.filter((c) => c.id !== "c1");
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
    await click(listRow("status:backlog/c1"), MOD);
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
    // Title order: Deploy (c4), Fix login (c1), Ship board (c3), Write docs (c2).
    // Columns still read left to right, so the board's range is by column.
    await click(bar("Clear"));
    await pick("Sort", "Title");
    await click(boardCard("c4"), MOD);
    await click(boardCard("c3"), { shiftKey: true });
    // On screen c3 (In Progress) comes before c4 (Done): the range is {c3, c4}.
    expect(selectedCount()).toBe("2 selected");
    expect(boardCard("c3").hasAttribute("data-selected")).toBe(true);
    expect(boardCard("c4").hasAttribute("data-selected")).toBe(true);
    expect(boardCard("c1").hasAttribute("data-selected")).toBe(false);
  });

  it("a range follows the sort inside a column", async () => {
    await renderBoard();
    // Backlog sorted by title: Fix login (c1), Write docs (c2). Descending flips it.
    await pick("Sort", "Title");
    await pick("Sort", "Descending");
    await click(boardCard("c2"), MOD);
    await click(boardCard("c3"), { shiftKey: true });
    // Backlog reads c2, c1; then In Progress c3: the range is c2, c1, c3.
    expect(selectedCount()).toBe("3 selected");
  });

  it("a list range follows the groups on screen, and a card in two groups counts once", async () => {
    await renderBoard();
    await pick("Layout", "List");
    await pick("Group by", "Priority");
    // Groups: Urgent..: p3 c1, p2 c4, p1 c2, p0 c3.
    await click(listRow("priority:p3/c1"), MOD);
    await click(listRow("priority:p1/c2"), { shiftKey: true });
    expect(selectedCount()).toBe("3 selected");
    expect(listRow("priority:p2/c4").hasAttribute("data-selected")).toBe(true);
    expect(listRow("priority:p0/c3").hasAttribute("data-selected")).toBe(false);
    await click(bar("Clear"));
    await pick("Group by", "Label");
    // c2 has two labels: Bug (c1, c2), UI (c2), No label (c3, c4).
    await click(listRow("label:l1/c1"), MOD);
    await click(listRow("label:none/c3"), { shiftKey: true });
    expect(selectedCount()).toBe("3 selected");
  });

  it("collapsing a list group drops its cards from the selection, and a range skips them", async () => {
    await renderBoard();
    await pick("Layout", "List");
    await click(listRow("status:backlog/c1"), MOD);
    await click(listRow("status:done/c4"), { shiftKey: true });
    expect(selectedCount()).toBe("4 selected");
    await click(q('[data-list-group="status:progress"]'));
    expect(selectedCount()).toBe("3 selected");
    await click(bar("Clear"));
    await click(listRow("status:backlog/c1"), MOD);
    await click(listRow("status:done/c4"), { shiftKey: true });
    expect(selectedCount()).toBe("3 selected");
    await click(q('[data-list-group="status:backlog"]'));
    expect(selectedCount()).toBe("1 selected");
  });

  it("collapsing a board column drops its cards from the selection", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    await click(boardCard("c3"), MOD);
    expect(selectedCount()).toBe("2 selected");
    backend.handlers.board_get = () => ({
      board: BOARD,
      columns: COLUMNS.map((c) =>
        c.id === "progress" ? { ...c, collapsed: true } : c,
      ),
      cards: backend.live(),
    });
    emitBoardChanged();
    await flush();
    expect(selectedCount()).toBe("1 selected");
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
    backend.cards = backend.cards.filter((c) => c.id !== "c1");
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
    backend.extras.c3 = extra({ labels: ["l1"] });
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
      expect(backend.cards.map((c) => c.id)).toEqual(["c3", "c4"]);
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

describe("selection announcements", () => {
  it("marks selected rows with valid ARIA in each layout", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    // Board cards are buttons that report pressed once a selection exists.
    expect(q('[data-kanban-card="c1"]').getAttribute("aria-pressed")).toBe("true");
    expect(q('[data-kanban-card="c2"]').getAttribute("aria-pressed")).toBe("false");
    await pick("Layout", "List");
    expect(listRow("status:backlog/c1").getAttribute("aria-pressed")).toBe("true");
    expect(listRow("status:backlog/c1").hasAttribute("aria-selected")).toBe(false);
    await pick("Layout", "Table");
    expect(q("[data-board-table]").getAttribute("role")).toBe("grid");
    expect(tableRow("c1").getAttribute("aria-selected")).toBe("true");
    // The header is row 1; windowed rows keep their place in the whole table.
    expect(tableRow("c1").getAttribute("aria-rowindex")).toBe("2");
    expect(tableRow("c3").getAttribute("aria-rowindex")).toBe("4");
    expect(q('[data-table-cell="title"]').getAttribute("role")).toBe("gridcell");
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
  // Open the layout under test on the small seed first, then grow the
  // backend to 1,000 cards in one column and let the board-changed refresh
  // feed them to the windowed layout.
  async function openWithThousand(layout: "List" | "Table") {
    await renderBoard();
    await pick("Layout", layout);
    backend.seedThousand(1);
    expect(backend.cards).toHaveLength(1000);
    emitBoardChanged();
    await flush();
    await flush();
  }

  it("renders_only_visible_rows in the list", async () => {
    await openWithThousand("List");
    const rendered = rowKeys().length;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(60);
    // The scroller still spans every row (1 header + 1000 cards).
    expect(
      (q("[data-board-list] > div") as HTMLElement).style.height,
    ).toBe(`${1001 * 32}px`);
  });

  it("renders_only_visible_rows in the table", async () => {
    await openWithThousand("Table");
    expect(tableRows().length).toBeGreaterThan(0);
    expect(tableRows().length).toBeLessThan(60);
  });

  it("renders the rows at the scroll position", async () => {
    await openWithThousand("Table");
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
    await openWithThousand("Table");
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

describe("keyboard shortcuts", () => {
  const key = (
    target: Element,
    k: string,
    init: KeyboardEventInit = {},
  ): KeyboardEvent => {
    const event = new KeyboardEvent("keydown", {
      key: k,
      bubbles: true,
      cancelable: true,
      ...init,
    });
    act(() => {
      target.dispatchEvent(event);
    });
    return event;
  };
  const kanbanCard = (id: string) => q(`[data-kanban-card="${id}"]`);
  const searchBox = () => q('input[aria-label="Search cards"]') as HTMLInputElement;
  const modal = () => document.querySelector('[role="dialog"]');
  const undoButton = () =>
    [...toast().querySelectorAll("button")].find((b) => b.textContent === "Undo")!;

  it("N opens the New card modal when no field has focus", async () => {
    await renderBoard();
    const event = key(kanbanCard("c1"), "n");
    expect(event.defaultPrevented).toBe(true);
    expect(modal()?.textContent).toContain("New card");
  });

  it("N works with nothing focused (body)", async () => {
    await renderBoard();
    key(document.body, "N");
    expect(modal()).not.toBeNull();
  });

  it("N types in the search box instead of opening the modal", async () => {
    await renderBoard();
    searchBox().focus();
    const event = key(searchBox(), "n");
    expect(event.defaultPrevented).toBe(false);
    expect(modal()).toBeNull();
  });

  it("ignores N with a modifier, and does nothing when focus is outside the board", async () => {
    await renderBoard();
    key(kanbanCard("c1"), "n", { altKey: true });
    key(kanbanCard("c1"), "n", { ctrlKey: !IS_MAC, metaKey: false, shiftKey: true });
    const outside = document.createElement("button");
    document.body.append(outside);
    key(outside, "n");
    outside.remove();
    expect(modal()).toBeNull();
  });

  it("/ focuses search and doesn't type a slash", async () => {
    await renderBoard();
    const event = key(kanbanCard("c1"), "/");
    expect(event.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(searchBox());
    expect(searchBox().value).toBe("");
  });

  it("/ types normally inside the search box", async () => {
    await renderBoard();
    searchBox().focus();
    expect(key(searchBox(), "/").defaultPrevented).toBe(false);
  });

  it("E opens the focused card, and does nothing with no focused card", async () => {
    await renderBoard();
    key(document.body, "e");
    expect(q("[data-card-drawer]")).toBeNull();
    key(kanbanCard("c2"), "e");
    await flush();
    expect(q("[data-card-drawer]")?.textContent).toContain("Card #2");
  });

  it("E on a control inside the card leaves the card closed", async () => {
    await renderBoard();
    key(kanbanCard("c1").querySelector('button[aria-label^="Card actions"]')!, "e");
    expect(q("[data-card-drawer]")).toBeNull();
  });

  it("Delete holds a delete of the focused card, with undo", async () => {
    await renderBoard();
    const event = key(kanbanCard("c1"), "Delete");
    await flush();
    expect(event.defaultPrevented).toBe(true);
    expect(toast().textContent).toContain("Deleted #1");
    expect(boardCard("c1")).toBeNull();
    expect(commands("board_card_delete_archived")).toHaveLength(0);
    await click(undoButton());
    expect(commands("board_card_restore")).toHaveLength(1);
    expect(boardCard("c1")).not.toBeNull();
    expect(commands("board_card_delete_archived")).toHaveLength(0);
  });

  it("Backspace deletes the focused card too, but never while typing", async () => {
    await renderBoard();
    searchBox().focus();
    expect(key(searchBox(), "Backspace").defaultPrevented).toBe(false);
    expect(q("[data-board-undo-toast]")).toBeNull();
    act(() => searchBox().blur());
    key(kanbanCard("c2"), "Backspace");
    await flush();
    expect(toast().textContent).toContain("Deleted #2");
  });

  it("Delete with nothing focused or selected does nothing", async () => {
    await renderBoard();
    key(document.body, "Delete");
    await flush();
    expect(q("[data-board-undo-toast]")).toBeNull();
  });

  it("Delete with a selection deletes every selected card, not just the focused one", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    await click(boardCard("c2"), MOD);
    key(kanbanCard("c3"), "Delete");
    await flush();
    expect(toast().textContent).toContain("Deleted 2 cards");
    expect(boardCard("c1")).toBeNull();
    expect(boardCard("c2")).toBeNull();
    expect(boardCard("c3")).not.toBeNull();
    expect(commands("board_cards_archive")).toEqual([
      ["board_cards_archive", { cardIds: ["c1", "c2"] }],
    ]);
    await click(undoButton());
    expect(commands("board_card_delete_archived")).toHaveLength(0);
  });

  it("keys type normally in an inline table edit", async () => {
    await renderBoard();
    await pick("Layout", "Table");
    await click(q('button[aria-label="Edit title of #1"]'));
    const input = q('input[aria-label="Title of #1"]') as HTMLInputElement;
    for (const k of ["n", "e", "/", "Delete", "Backspace"]) {
      expect(key(input, k).defaultPrevented, k).toBe(false);
    }
    expect(modal()).toBeNull();
    expect(q("[data-card-drawer]")).toBeNull();
    expect(q("[data-board-undo-toast]")).toBeNull();
  });

  it("table and list rows take E and Delete like cards", async () => {
    await renderBoard();
    await pick("Layout", "Table");
    key(tableRow("c1"), "e");
    await flush();
    expect(q("[data-card-drawer]")?.textContent).toContain("Card #1");
  });

  it("does nothing while the New card modal is open", async () => {
    await renderBoard();
    key(kanbanCard("c1"), "n");
    const title = document.querySelector('[aria-label="Title"]')!;
    key(title, "n");
    expect(document.querySelectorAll('[role="dialog"]')).toHaveLength(1);
    key(document.body, "Delete");
    expect(q("[data-board-undo-toast]")).toBeNull();
  });

  it("does nothing while a menu is open", async () => {
    await renderBoard();
    act(() =>
      container
        .querySelector<HTMLButtonElement>('button[aria-label="Layout"]')!
        .click(),
    );
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
    key(document.body, "n");
    expect(modal()).toBeNull();
  });

  it("does nothing behind an open drawer unless focus is on the board", async () => {
    await renderBoard();
    await click(boardCard("c1"));
    expect(q("[data-card-drawer]")).not.toBeNull();
    // Focus in the drawer or nowhere: blocked.
    key(q("[data-card-drawer]"), "n");
    key(q("[data-card-drawer]"), "Delete");
    key(document.body, "n");
    key(document.body, "Delete");
    await flush();
    expect(modal()).toBeNull();
    expect(q("[data-board-undo-toast]")).toBeNull();
    expect(boardCard("c1")).not.toBeNull();
    // Focus on a board card: the key acts on the board.
    key(kanbanCard("c2"), "e");
    await flush();
    expect(q("[data-card-drawer]")?.textContent).toContain("Card #2");
    key(kanbanCard("c3"), "Delete");
    await flush();
    expect(toast().textContent).toContain("Deleted #3");
  });

  it("does nothing while a keyboard drag is lifted", async () => {
    await renderBoard();
    const lifted = document.createElement("div");
    lifted.setAttribute("data-dragging", "");
    container.append(lifted);
    key(kanbanCard("c1"), "Delete");
    key(kanbanCard("c1"), "n");
    await flush();
    expect(q("[data-board-undo-toast]")).toBeNull();
    expect(modal()).toBeNull();
    lifted.remove();
  });

  it("ignores a board that is hidden in an inert tab", async () => {
    await renderBoard();
    container.setAttribute("inert", "");
    key(document.body, "n");
    expect(modal()).toBeNull();
    container.removeAttribute("inert");
  });

  it("leaves Escape layering alone", async () => {
    await renderBoard();
    await click(boardCard("c1"), MOD);
    key(kanbanCard("c1"), "Escape");
    expect(selectedCount()).toBeNull();
  });
});

describe("palette commands", () => {
  const renderWith = async (command: ComponentProps<typeof BoardView>["command"]) => {
    const onCommandHandled = vi.fn();
    await act(async () => {
      root.render(
        createElement(BoardView, {
          onClose: vi.fn(),
          linkTargets: [],
          onOpenLinkedWork: vi.fn(() => Promise.resolve()),
          lookupLinkTargets: vi.fn(async () => []),
          projects: [],
          command,
          onCommandHandled,
        }),
      );
    });
    await flush();
    return onCommandHandled;
  };

  it("Create card opens the board and the New card modal, once", async () => {
    const handled = await renderWith({ id: "new-card", nonce: 1 });
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain(
      "New card",
    );
    expect(handled).toHaveBeenCalledWith(1);
    expect(handled).toHaveBeenCalledTimes(1);
  });

  it("Switch board lists the boards", async () => {
    const handled = await renderWith({ id: "switch-board", nonce: 2 });
    const options = [...document.querySelectorAll('[role="option"]')].map((o) =>
      o.textContent?.trim(),
    );
    expect(options.join("|")).toContain("Agents");
    expect(options.join("|")).toContain("Releases");
    expect(handled).toHaveBeenCalledWith(2);
  });

  it("a switch signal fired while renaming does not open the switcher later", async () => {
    await renderWith(undefined);
    const props = (command: ComponentProps<typeof BoardView>["command"]) =>
      createElement(BoardView, {
        onClose: vi.fn(),
        linkTargets: [],
        onOpenLinkedWork: vi.fn(() => Promise.resolve()),
        lookupLinkTargets: vi.fn(async () => []),
        projects: [],
        command,
        onCommandHandled: vi.fn(),
      });
    act(() =>
      (
        document.querySelector(
          'button[aria-label="Board actions"]',
        ) as HTMLButtonElement
      ).click(),
    );
    const rename = [
      ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
    ].find((el) => el.textContent?.trim() === "Rename board")!;
    expect(rename).toBeDefined();
    await act(async () => rename.click());
    const input = document.querySelector('input[aria-label="Board name"]')!;
    expect(input).not.toBeNull();
    await act(async () => root.render(props({ id: "switch-board", nonce: 9 })));
    await flush();
    press(input, "Escape");
    await flush();
    expect(document.querySelector('input[aria-label="Board name"]')).toBeNull();
    expect(document.querySelector('[role="listbox"]')).toBeNull();
    expect(document.querySelectorAll('[role="option"]')).toHaveLength(0);
  });

  it("does not act again for the same command when the board reloads", async () => {
    const handled = await renderWith({ id: "new-card", nonce: 3 });
    act(() => {
      (document.querySelector('[role="dialog"] button[aria-label="Close"]') as
        | HTMLElement
        | null)?.click();
    });
    press(document.body, "Escape");
    emitBoardChanged();
    await flush();
    expect(handled).toHaveBeenCalledTimes(1);
  });

  it("drops a command when there are no boards", async () => {
    backend.handlers.board_list = () => [];
    const handled = await renderWith({ id: "new-card", nonce: 4 });
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(handled).toHaveBeenCalledWith(4);
  });
});
