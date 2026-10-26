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
  BoardField,
  BoardFieldKind,
  BoardLabel,
  BoardSettings,
  BoardView as SavedView,
  CardExtras,
  ColumnRole,
} from "../../model/board";
import type { KanbanBoardProps } from "../kanban/KanbanBoard";
import type { ComponentProps as Props } from "react";
import type { Swimlanes } from "../kanban/Swimlanes";
import { BoardView } from "../BoardView";

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

const lanes: { props: Props<typeof Swimlanes> | null } = { props: null };
vi.mock("../kanban/Swimlanes", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../kanban/Swimlanes")>();
  return {
    Swimlanes: (props: Props<typeof actual.Swimlanes>) => {
      lanes.props = props;
      return createElement(actual.Swimlanes, props);
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
let fieldRows: BoardField[];
let indexRows: Record<string, Record<string, string>>;
let distinct: Record<string, string[]>;
let valueRows: { cardId: string; fieldId: string; valueJson: string }[];

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
  fieldRows = [];
  indexRows = {};
  distinct = {};
  valueRows = [];
  lanes.props = null;
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
    board_fields: () => fieldRows,
    board_card_index: () =>
      detail.cards.map((c) => ({
        cardId: c.id,
        columnId: c.columnId,
        number: c.number,
        title: c.title,
        priority: 0,
        dueAt: null,
        startAt: null,
        estimateMinutes: null,
        labels: [],
        projects: [],
        createdAt: 1,
        updatedAt: 1,
        fields: indexRows[c.id] ?? {},
      })),
    board_field_distinct_values: (args) => distinct[String(args.fieldId)] ?? [],
    board_field_create: (args) => ({
      id: "nf",
      boardId: "b1",
      name: String(args.name),
      kind: args.kind as BoardFieldKind,
      optionsJson: (args.optionsJson as string | null) ?? null,
      position: 9,
    }),
    board_field_update: () => undefined,
    board_field_rename_option: () => undefined,
    board_field_move: () => undefined,
    board_field_delete: () => undefined,
    board_field_value_set: () => undefined,
    board_card_field_values: () => valueRows,
    board_subtasks: () => [],
    board_comments: () => [],
    board_card_relations: () => [],
    board_card_links: () => [],
    board_card_meta: () => ({
      priority: 0,
      dueAt: null,
      startAt: null,
      estimateMinutes: null,
      archivedAt: null,
      completedAt: null,
    }),
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


const field = (
  id: string,
  name: string,
  kind: BoardFieldKind,
  optionsJson: string | null = null,
  position = 0,
): BoardField => ({ id, boardId: "b1", name, kind, optionsJson, position });

const settingsWith = (display: unknown): BoardSettings => ({
  ...DEFAULTS,
  displayJson: JSON.stringify(display),
});

const faceOf = (id: string) =>
  container.querySelector<HTMLElement>(`[data-board-card='${id}']`)!;

describe("fields manager", () => {
  const row = (id: string) =>
    section("Fields").querySelector<HTMLElement>(`[data-settings-field="${id}"]`)!;

  it("fields_manager_creates_each_kind_and_reorders", async () => {
    fieldRows = [field("f1", "Severity", "select", '["S1","S2"]', 0), field("f2", "Notes", "text", null, 1)];
    await renderBoard();
    await openSettings();
    const kinds: [BoardFieldKind, string | null][] = [
      ["text", null],
      ["number", null],
      ["date", null],
      ["select", "[]"],
      ["multi-select", "[]"],
      ["checkbox", null],
      ["url", null],
    ];
    for (const [kind, options] of kinds) {
      await choose(byLabel<HTMLSelectElement>(section("Fields"), "New field kind"), kind);
      await type(byLabel<HTMLInputElement>(section("Fields"), "New field name"), `F ${kind}`, false);
      await click(buttonNamed(section("Fields"), "Add field"));
      expect(lastArgs("board_field_create")).toEqual({
        boardId: "b1",
        name: `F ${kind}`,
        kind,
        optionsJson: options,
      });
    }
    expect(commands("board_field_create")).toHaveLength(7);

    await click(buttonNamed(row("f1"), "Move down"));
    expect(lastArgs("board_field_move")).toEqual({ fieldId: "f1", index: 1 });
    expect(buttonNamed(row("f1"), "Move up").disabled).toBe(true);
    expect(buttonNamed(row("f2"), "Move down").disabled).toBe(true);

    // The kind is shown, never editable.
    expect(row("f1").querySelector("[data-field-kind]")?.textContent).toBe("Select");
    expect(row("f1").querySelector("select")).toBeNull();
    await type(byLabel<HTMLInputElement>(row("f2"), "Field name"), "Remarks");
    expect(lastArgs("board_field_update")).toEqual({
      fieldId: "f2",
      name: "Remarks",
      optionsJson: null,
    });
  });

  it("refuses an empty field name without writing", async () => {
    await renderBoard();
    await openSettings();
    await click(buttonNamed(section("Fields"), "Add field"));
    expect(commands("board_field_create")).toEqual([]);
    expect(alertText()).toContain("needs a name");
  });

  it("renaming_a_select_option_updates_cards", async () => {
    fieldRows = [field("f1", "Severity", "select", '["S1","S2"]')];
    await renderBoard();
    await openSettings();
    const options = () => row("f1").querySelector<HTMLElement>('[role="group"]')!;
    const optionRow = (name: string) =>
      options().querySelector<HTMLElement>(`[data-field-option="${name}"]`)!;
    // A case-only change goes through the rename, which keeps stored values.
    await type(byLabel<HTMLInputElement>(optionRow("S1"), "Option name"), "s1");
    expect(commands("board_field_rename_option")).toEqual([
      ["board_field_rename_option", { fieldId: "f1", from: "S1", to: "s1" }],
    ]);
    expect(commands("board_field_update")).toEqual([]);

    await type(byLabel<HTMLInputElement>(optionRow("S2"), "Option name"), "S1");
    expect(commands("board_field_rename_option")).toHaveLength(1);
    expect(alertText()).toContain("already exists");

    await type(byLabel<HTMLInputElement>(options(), "New option"), "S3");
    expect(lastArgs("board_field_update")).toEqual({
      fieldId: "f1",
      name: null,
      optionsJson: '["S1","S2","S3"]',
    });
    await click(byLabel(optionRow("S1"), "Remove option S1"));
    expect(lastArgs("board_field_update")).toEqual({
      fieldId: "f1",
      name: null,
      optionsJson: '["S2"]',
    });
  });

  it("asks before deleting a field, and a No deletes nothing", async () => {
    fieldRows = [field("f1", "Severity", "select", '["S1"]')];
    vi.mocked(ask).mockResolvedValue(false);
    await renderBoard();
    await openSettings();
    await click(buttonNamed(row("f1"), "Delete field"));
    expect(commands("board_field_delete")).toEqual([]);
    vi.mocked(ask).mockResolvedValue(true);
    await click(buttonNamed(row("f1"), "Delete field"));
    expect(lastArgs("board_field_delete")).toEqual({ fieldId: "f1" });
  });

  it("deleting_a_field_removes_its_filter_rows_and_table_column", async () => {
    fieldRows = [field("f1", "Severity", "select", '["S1","S2"]')];
    const layout = JSON.stringify({ layout: "table", columns: ["title", "field:f1"] });
    savedViews = [
      {
        ...view("v1", "Triage", [{ field: "field:f1", op: "is", values: ['"S1"'] }]),
        layoutJson: layout,
      },
    ];
    settings = { ...DEFAULTS, defaultViewId: "v1" };
    await renderBoard();
    const headers = () =>
      [...container.querySelectorAll('[role="columnheader"]')].map((el) =>
        el.textContent?.trim(),
      );
    const chips = () =>
      [...container.querySelectorAll("[data-filter-chip]")].map((el) =>
        el.textContent?.trim(),
      );
    expect(chips()).toEqual(["Severity is S1"]);
    expect(headers().some((h) => h?.includes("Severity"))).toBe(true);

    // The store removes the field, its filter rows and its table column.
    handlers.board_field_delete = () => {
      fieldRows = [];
      savedViews = [
        {
          ...view("v1", "Triage"),
          layoutJson: JSON.stringify({ layout: "table", columns: ["title"] }),
        },
      ];
    };
    await openSettings();
    await click(buttonNamed(section("Fields"), "Delete field"));
    await click(byLabel(page()!, "Back to board"));
    emitBoardChanged();
    await flush();
    expect(chips()).toEqual([]);
    expect(headers().some((h) => h?.includes("Severity"))).toBe(false);
  });

  it("a stale view naming a deleted field neither filters nor shows its column", async () => {
    const layout = JSON.stringify({ layout: "table", columns: ["title", "field:gone"] });
    savedViews = [
      {
        ...view("v1", "Stale", [{ field: "field:gone", op: "is", values: ['"S1"'] }]),
        layoutJson: layout,
      },
    ];
    settings = { ...DEFAULTS, defaultViewId: "v1" };
    await renderBoard();
    // Whatever the engine was asked, the dead field was not part of it.
    for (const [, args] of commands("board_query")) {
      expect((args as { spec: { rows: unknown[] } }).spec.rows).toEqual([]);
    }
    expect(
      container.querySelector('[data-view-tab="v1"]')?.getAttribute("aria-selected"),
    ).toBe("true");
    expect(alertText()).toBe("");
    expect(container.querySelectorAll('[role="columnheader"]').length).toBeGreaterThan(0);
  });
});

describe("field editors in the drawer", () => {
  async function openCard(id: string) {
    act(() => faceOf(id).click());
    await flush();
  }
  const editor = (name: string) =>
    document.querySelector<HTMLElement>(`[data-card-drawer] [aria-label="${name}"]`)!;

  it("multi_select_and_url_edit_in_the_drawer_and_reject_bad_values", async () => {
    fieldRows = [
      field("m1", "Areas", "multi-select", '["API","Web","Docs"]', 0),
      field("u1", "Spec", "url", null, 1),
    ];
    await renderBoard();
    await openCard("c1");

    const tick = async (option: string) => {
      const box = document.querySelector<HTMLInputElement>(
        `[data-card-drawer] input[aria-label="Areas: ${option}"]`,
      )!;
      await click(box);
    };
    await tick("Web");
    expect(lastArgs("board_field_value_set")).toEqual({
      cardId: "c1",
      fieldId: "m1",
      valueJson: '["Web"]',
    });
    // Picks are stored in option order, however they were ticked.
    valueRows = [{ cardId: "c1", fieldId: "m1", valueJson: '["Web"]' }];
    emitBoardChanged();
    await flush();
    await tick("API");
    expect(lastArgs("board_field_value_set")).toEqual({
      cardId: "c1",
      fieldId: "m1",
      valueJson: '["API","Web"]',
    });
    // Unticking the last pick clears the value.
    await tick("Web");
    expect(lastArgs("board_field_value_set")).toMatchObject({ valueJson: null });

    const url = editor("Spec") as HTMLInputElement;
    act(() => url.focus());
    const before = commands("board_field_value_set").length;
    await type(url, "ftp://example.com", true);
    act(() => url.blur());
    await flush();
    expect(commands("board_field_value_set")).toHaveLength(before);
    expect(document.querySelector("[data-card-drawer]")!.textContent).toContain(
      "must start with http:// or https://",
    );
    act(() => url.focus());
    await type(url, `https://example.com/${"a".repeat(2000)}`, true);
    act(() => url.blur());
    await flush();
    expect(commands("board_field_value_set")).toHaveLength(before);

    act(() => url.focus());
    await type(url, "https://example.com/spec", true);
    act(() => url.blur());
    await flush();
    expect(lastArgs("board_field_value_set")).toEqual({
      cardId: "c1",
      fieldId: "u1",
      valueJson: '"https://example.com/spec"',
    });
  });

  it("shows the store's refusal of a value", async () => {
    fieldRows = [field("u1", "Spec", "url")];
    handlers.board_field_value_set = () => {
      throw "Value is not valid for this field";
    };
    await renderBoard();
    await openCard("c1");
    const url = editor("Spec") as HTMLInputElement;
    act(() => url.focus());
    await type(url, "https://example.com", true);
    act(() => url.blur());
    await flush();
    expect(alertText()).toContain("Value is not valid for this field");
  });
});

describe("filter rows on free-form fields", () => {
  beforeEach(() => {
    fieldRows = [
      field("t1", "Notes", "text", null, 0),
      field("n1", "Points", "number", null, 1),
      field("d1", "Ship", "date", null, 2),
    ];
  });
  async function addRow(name: string) {
    const trigger = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Filter"]',
    )!;
    if (trigger.getAttribute("aria-expanded") !== "true") act(() => trigger.click());
    const item = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(
      (el) => el.textContent?.trim() === name,
    )!;
    expect(item, name).toBeDefined();
    act(() => item.click());
    await flush();
  }
  const choices = () =>
    [...document.querySelectorAll<HTMLElement>('[role="menuitemcheckbox"]')].map(
      (el) => el.textContent?.trim(),
    );
  const lastRows = () =>
    (commands("board_query").at(-1)![1] as { spec: { rows: { values: string[] }[] } }).spec
      .rows;

  it("text_number_date_filter_rows_offer_distinct_card_values", async () => {
    distinct = {
      t1: ['"alpha"', '"beta"'],
      n1: ["3", "12"],
      d1: [String(new Date(2026, 9, 26).getTime())],
    };
    await renderBoard();
    await addRow("Notes");
    expect(commands("board_field_distinct_values")).toEqual([
      ["board_field_distinct_values", { boardId: "b1", fieldId: "t1", limit: 50 }],
    ]);
    expect(choices()).toEqual(["alpha", "beta"]);
    act(() =>
      [...document.querySelectorAll<HTMLElement>('[role="menuitemcheckbox"]')]
        .find((el) => el.textContent?.trim() === "beta")!
        .click(),
    );
    await flush();
    expect(lastRows()).toEqual([
      { field: "field:t1", op: "is", values: ['"beta"'] },
    ]);

    await addRow("Points");
    expect(choices()).toContain("12");
    expect(choices()).toContain("3");
    await addRow("Ship");
    expect(
      choices().filter((c) => c === new Date(2026, 9, 26).toLocaleDateString()),
    ).toHaveLength(1);
  });

  it("still offers typed input when the distinct values can't be read", async () => {
    handlers.board_field_distinct_values = () => {
      throw "store is locked";
    };
    await renderBoard();
    await addRow("Notes");
    expect(choices()).toEqual([]);
    expect(
      document.querySelector('input[aria-label="Add Notes value"]'),
    ).not.toBeNull();
  });

  it("free_form_filter_value_matches", async () => {
    await renderBoard();
    await addRow("Notes");
    const input = () =>
      document.querySelector<HTMLInputElement>('input[aria-label="Add Notes value"]')!;
    await type(input(), "gamma", true);
    expect(lastRows()).toEqual([{ field: "field:t1", op: "is", values: ['"gamma"'] }]);
    // What was typed stays a choice, checked.
    expect(choices()).toEqual(["gamma"]);

    await addRow("Points");
    const points = () =>
      document.querySelector<HTMLInputElement>('input[aria-label="Add Points value"]')!;
    await type(points(), "abc", true);
    expect(lastRows()).toHaveLength(1);
    expect(document.body.textContent).toContain("is not a number");
    await type(points(), "12", true);
    expect(lastRows()[1]).toEqual({ field: "field:n1", op: "is", values: ["12"] });
  });
});

describe("display options", () => {
  const rich = () => {
    cardExtras = {
      c1: {
        priority: 4,
        dueAt: Date.now() + 86_400_000,
        projects: [],
        labels: ["l1"],
        subtasksDone: 1,
        subtasksTotal: 2,
        comments: 3,
      },
    };
    labelRows = [label("l1", "Bug", 0)];
  };

  it("display_toggles_hide_card_parts", async () => {
    rich();
    await renderBoard();
    const parts = () => ({
      number: !!faceOf("c1").querySelector("[data-card-number]"),
      priority: !!faceOf("c1").querySelector("[data-card-priority]"),
      due: !!faceOf("c1").querySelector("[data-card-due]"),
      labels: !!faceOf("c1").querySelector("[data-card-labels]"),
      subtasks: !!faceOf("c1").querySelector("[data-card-subtasks]"),
      comments: !!faceOf("c1").querySelector("[data-card-comments]"),
    });
    expect(parts()).toEqual({
      number: true, priority: true, due: true, labels: true, subtasks: true, comments: true,
    });

    settings = settingsWith({
      face: { number: false, priority: false, due: false, labels: false, subtasks: false, comments: false },
    });
    emitBoardChanged();
    await flush();
    expect(parts()).toEqual({
      number: false, priority: false, due: false, labels: false, subtasks: false, comments: false,
    });
    // The title stays.
    expect(faceOf("c1").textContent).toContain("Fix login");
  });

  it("shows chosen custom fields on the face and ignores a missing one", async () => {
    fieldRows = [field("f1", "Severity", "select", '["S1"]')];
    indexRows = { c1: { f1: '"S1"' } };
    settings = settingsWith({ face: { fields: ["f1", "gone"] } });
    await renderBoard();
    expect(
      [...faceOf("c1").querySelectorAll("[data-card-field]")].map((el) => el.textContent),
    ).toEqual(["Severity: S1"]);
    expect(faceOf("c2").querySelector("[data-card-field]")).toBeNull();
  });

  it("compact_density_shrinks_cards", async () => {
    await renderBoard();
    expect(faceOf("c1").getAttribute("data-density")).toBe("comfortable");
    expect(faceOf("c1").className).toContain("py-2.5");
    settings = settingsWith({ density: "compact" });
    emitBoardChanged();
    await flush();
    expect(faceOf("c1").getAttribute("data-density")).toBe("compact");
    expect(faceOf("c1").className).toContain("py-1.5");
    expect(faceOf("c1").className).not.toContain("py-2.5");
  });

  it("saved_view_display_overrides_board_default", async () => {
    savedViews = [
      {
        ...view("v1", "Lean"),
        layoutJson: JSON.stringify({
          layout: "board",
          display: { face: { number: false }, density: "compact" },
        }),
      },
    ];
    cardExtras = { c1: extrasOf(4) };
    await renderBoard();
    const tab = (id: string) =>
      container.querySelector<HTMLElement>(`[data-view-tab="${id}"]`)!;
    const numberShown = () => !!faceOf("c1").querySelector("[data-card-number]");
    expect(numberShown()).toBe(true);
    await click(tab("v1"));
    expect(numberShown()).toBe(false);
    expect(faceOf("c1").getAttribute("data-density")).toBe("compact");
    // Parts the override doesn't name keep the board default: the priority shows.
    expect(
      faceOf("c1").querySelector("[data-card-priority]")?.textContent,
    ).toBe("Urgent");
    await click(tab("all"));
    expect(numberShown()).toBe(true);
    expect(faceOf("c1").getAttribute("data-density")).toBe("comfortable");
  });

  it("the Display menu edits the board default on All cards and a view's override on a view", async () => {
    savedViews = [view("v1", "Mine")];
    await renderBoard();
    const toggle = async () => {
      const trigger = container.querySelector<HTMLButtonElement>('button[aria-label="Display"]')!;
      if (trigger.getAttribute("aria-expanded") !== "true") act(() => trigger.click());
      await click(document.querySelector<HTMLElement>('input[aria-label="Show Card number"]')!);
    };
    await toggle();
    const patch = lastArgs("board_settings_update") as { boardId: string; patch: { displayJson: string } };
    expect(Object.keys(patch.patch)).toEqual(["displayJson"]);
    expect(JSON.parse(patch.patch.displayJson).face.number).toBe(false);

    const writes = commands("board_settings_update").length;
    await click(container.querySelector<HTMLElement>('[data-view-tab="v1"]')!);
    await toggle();
    // On a view it is unsaved view state: no settings write, the view is dirty.
    expect(commands("board_settings_update")).toHaveLength(writes);
    expect(container.querySelector("[data-view-dot]")).not.toBeNull();
  });

  it("two quick toggles in the Display menu on All cards both land, written in order", async () => {
    const replies: (() => void)[] = [];
    handlers.board_settings_update = (args) =>
      new Promise((resolve) => {
        replies.push(() => {
          settings = { ...settings, ...(args.patch as Partial<BoardSettings>) };
          resolve(settings);
        });
      });
    await renderBoard();
    const trigger = container.querySelector<HTMLButtonElement>('button[aria-label="Display"]')!;
    act(() => trigger.click());
    const box = (name: string) =>
      document.querySelector<HTMLInputElement>(`input[aria-label="${name}"]`)!;
    act(() => box("Show Card number").click());
    act(() => box("Show Priority").click());
    await flush();
    // The second waits for the first reply, then builds on the first edit.
    expect(replies).toHaveLength(1);
    replies.shift()!();
    await flush();
    expect(replies).toHaveLength(1);
    replies.shift()!();
    await flush();
    const written = commands("board_settings_update").map(
      ([, args]) =>
        JSON.parse((args as { patch: { displayJson: string } }).patch.displayJson).face,
    );
    expect(written[0]).toMatchObject({ number: false, priority: true });
    expect(written[1]).toMatchObject({ number: false, priority: false });
  });

  it("a failed Display save on the settings page isn't built on by the next edit", async () => {
    let calls = 0;
    handlers.board_settings_update = (args) => {
      calls += 1;
      if (calls === 1) throw "store is locked";
      settings = { ...settings, ...(args.patch as Partial<BoardSettings>) };
      return settings;
    };
    await renderBoard();
    await openSettings();
    const box = (name: string) => byLabel<HTMLInputElement>(section("Display"), name);
    await click(box("Show Comments"));
    expect(alertText()).toContain("store is locked");
    await click(box("Show Subtasks"));
    const second = JSON.parse(
      (lastArgs("board_settings_update") as { patch: { displayJson: string } }).patch
        .displayJson,
    );
    expect(second.face.subtasks).toBe(false);
    // The failed change was not carried into the next write.
    expect(second.face.comments).toBe(true);
  });

  it("the settings page writes only the display and keeps quick toggles", async () => {
    await renderBoard();
    await openSettings();
    const box = (name: string) =>
      byLabel<HTMLInputElement>(section("Display"), name);
    await click(box("Show Comments"));
    await click(box("Show Subtasks"));
    const written = commands("board_settings_update").map(
      ([, args]) => (args as { patch: Record<string, string> }).patch,
    );
    expect(written.every((p) => Object.keys(p).join() === "displayJson")).toBe(true);
    const last = JSON.parse(written.at(-1)!.displayJson);
    expect(last.face.comments).toBe(false);
    expect(last.face.subtasks).toBe(false);
  });
});

describe("swimlanes", () => {
  const dropArgs = () => ({
    moves: commands("board_card_move"),
    priority: commands("board_card_set_priority"),
    values: commands("board_field_value_set"),
  });
  const lane = (name: string) =>
    container.querySelector<HTMLElement>(`section[aria-label="Lane ${name}"]`);
  const idsIn = (name: string) =>
    [...lane(name)!.querySelectorAll("[data-kanban-card]")].map((el) =>
      el.getAttribute("data-kanban-card"),
    );

  it("swimlanes_by_priority_group_rows_and_drop_sets_priority", async () => {
    settings = settingsWith({ swimlane: "priority" });
    cardExtras = { c1: { ...extrasOf(4) }, c3: { ...extrasOf(2) } };
    await renderBoard();
    expect(idsIn("Urgent")).toEqual(["c1"]);
    expect(idsIn("Medium")).toEqual(["c3"]);
    expect(idsIn("None")).toEqual(["c2"]);
    expect(container.querySelector("[data-kanban-board]")).toBeNull();

    // Into the Urgent lane, another column: the column changes and so does the priority.
    act(() => lanes.props!.onDrop("c3", "p4", "review"));
    await flush();
    expect(dropArgs().moves).toEqual([
      ["board_card_move", { cardId: "c3", columnId: "review", index: 0, actor: "user" }],
    ]);
    expect(dropArgs().priority).toEqual([
      ["board_card_set_priority", { cardId: "c3", priority: 4 }],
    ]);
    // Same column, other lane: the priority alone.
    act(() => lanes.props!.onDrop("c2", "p3", "backlog"));
    await flush();
    expect(dropArgs().moves).toHaveLength(1);
    expect(dropArgs().priority.at(-1)).toEqual([
      "board_card_set_priority",
      { cardId: "c2", priority: 3 },
    ]);
  });

  it("a hidden priority lane shows only while a card holds that level", async () => {
    settings = {
      ...settingsWith({ swimlane: "priority" }),
      priorityJson: JSON.stringify({
        names: ["None", "Low", "Medium", "High", "Urgent"],
        hidden: [1, 2],
      }),
    };
    cardExtras = { c3: extrasOf(2) };
    await renderBoard();
    expect(lane("Low")).toBeNull();
    expect(lane("Medium")).not.toBeNull();
  });

  it("a lane drop into a column past its WIP limit warns, and the card stays put while it saves", async () => {
    detail.columns[1] = column("progress", "In Progress", 1, "doing", { wipLimit: 1 });
    settings = settingsWith({ swimlane: "priority" });
    let finish!: () => void;
    handlers.board_card_move = () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      });
    await renderBoard();
    act(() => lanes.props!.onDrop("c1", "p0", "progress"));
    await flush();
    // Optimistic: the lane board already shows c1 in In Progress, before the save returns.
    expect(lanes.props!.cards.find((c) => c.id === "c1")?.columnId).toBe("progress");
    expect(commands("board_card_move")).toHaveLength(1);
    finish();
    await flush();
    expect(toastText()).toContain("Over the WIP limit (2/1)");
  });

  it("a failed lane drop puts the card back and says so", async () => {
    settings = settingsWith({ swimlane: "priority" });
    handlers.board_card_move = () => {
      throw "store is locked";
    };
    await renderBoard();
    act(() => lanes.props!.onDrop("c1", "p0", "review"));
    await flush();
    expect(lanes.props!.cards.find((c) => c.id === "c1")?.columnId).toBe("backlog");
    expect(alertText()).toContain("store is locked");
  });

  it("label_lane_drop_changes_only_the_column", async () => {
    labelRows = [label("l1", "Bug", 0), label("l2", "Docs", 1)];
    cardExtras = { c1: { ...extrasOf(3), labels: ["l1"] } };
    settings = settingsWith({ swimlane: "label" });
    await renderBoard();
    expect(idsIn("Bug")).toEqual(["c1"]);
    expect(idsIn("No label")).toEqual(["c2", "c3"]);
    act(() => lanes.props!.onDrop("c1", "l2", "review"));
    await flush();
    const { moves, priority, values } = dropArgs();
    expect(moves).toHaveLength(1);
    expect(priority).toEqual([]);
    expect(values).toEqual([]);
    expect(commands("board_card_set_labels")).toEqual([]);
  });

  it("a project lane also changes only the column", async () => {
    cardExtras = { c1: { ...extrasOf(3), projects: ["/work/api"] } };
    settings = settingsWith({ swimlane: "project" });
    await renderBoard();
    expect(lane("api") ?? lane("/work/api")).not.toBeNull();
    act(() => lanes.props!.onDrop("c1", "/work/api", "review"));
    await flush();
    expect(dropArgs().moves).toHaveLength(1);
    expect(dropArgs().priority).toEqual([]);
    expect(dropArgs().values).toEqual([]);
  });

  it("a select-field lane sets the option, and the no-value lane clears it", async () => {
    fieldRows = [field("f1", "Severity", "select", '["S1","S2"]')];
    indexRows = { c1: { f1: '"S1"' } };
    settings = settingsWith({ swimlane: "field:f1" });
    await renderBoard();
    expect(idsIn("S1")).toEqual(["c1"]);
    expect(idsIn("No Severity")).toEqual(["c2", "c3"]);
    act(() => lanes.props!.onDrop("c2", "opt:S2", "backlog"));
    await flush();
    expect(lastArgs("board_field_value_set")).toEqual({
      cardId: "c2",
      fieldId: "f1",
      valueJson: '"S2"',
    });
    expect(dropArgs().moves).toEqual([]);
    act(() => lanes.props!.onDrop("c1", "none", "backlog"));
    await flush();
    expect(lastArgs("board_field_value_set")).toEqual({
      cardId: "c1",
      fieldId: "f1",
      valueJson: null,
    });
  });

  it("lanes on a missing field fall back to the plain board", async () => {
    settings = settingsWith({ swimlane: "field:gone" });
    await renderBoard();
    expect(container.querySelector("[data-swimlanes]")).toBeNull();
    expect(container.querySelector("[data-kanban-board]")).not.toBeNull();
    expect(alertText()).toBe("");
  });

  it("with swimlanes none the board is the same kanban board", async () => {
    await renderBoard();
    expect(container.querySelector("[data-kanban-board]")).not.toBeNull();
    expect(container.querySelector("[data-swimlanes]")).toBeNull();
  });
});

function extrasOf(priority: number): CardExtras {
  return {
    priority,
    dueAt: null,
    projects: [],
    labels: [],
    subtasksDone: 0,
    subtasksTotal: 0,
    comments: 0,
  };
}
