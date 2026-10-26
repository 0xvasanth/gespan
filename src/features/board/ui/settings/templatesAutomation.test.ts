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
  TemplateInfo,
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
let templateRows: TemplateInfo[];
let boardRows: { id: string; name: string; createdAt: number; updatedAt: number }[];
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
  templateRows = [
    { id: "blank", name: "Blank", builtIn: true },
    { id: "agent-pipeline", name: "Agent pipeline", builtIn: true },
  ];
  boardRows = [BOARD];
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
    board_list: () => boardRows,
    board_templates: () => templateRows,
    board_create_from_template: (args) => {
      const made = { id: "b2", name: String(args.name), createdAt: 2, updatedAt: 2 };
      boardRows = [...boardRows, made];
      const names =
        args.templateId === "agent-pipeline"
          ? ["Queued", "Running", "Needs review", "Merged", "Done"]
          : ["Backlog", "In Progress", "Review", "Done"];
      detail = {
        board: made,
        columns: names.map((n, i) => ({ ...column(`n${i}`, n, i, null), boardId: "b2" })),
        cards: [],
      };
      return made;
    },
    board_save_as_template: (args) => {
      const saved = { id: "t1", name: String(args.name), builtIn: false };
      templateRows = [...templateRows, saved];
      return saved;
    },
    board_template_delete: (args) => {
      templateRows = templateRows.filter((t) => t.id !== args.templateId);
    },
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



const settingsWith = (patch: Partial<BoardSettings>): BoardSettings => ({
  ...DEFAULTS,
  ...patch,
});

async function openNewBoard() {
  await click(
    [...container.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent?.trim() === "New board",
    )!,
  );
}
const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]');
const radios = () =>
  [...dialog()!.querySelectorAll<HTMLElement>('[role="radio"]')];

describe("new board dialog", () => {
  async function create(templateName: string | null, name: string) {
    await openNewBoard();
    if (templateName) {
      await click(
        radios().find((r) => r.getAttribute("aria-label") === templateName)!,
      );
    }
    await type(byLabel<HTMLInputElement>(dialog()!, "Board name"), name, false);
    await click(buttonNamed(dialog()!, "Create board"));
  }
  const names = () =>
    [...container.querySelectorAll("[data-column-name]")].map((el) => el.textContent);

  it("new_board_from_agent_pipeline_has_its_columns", async () => {
    await renderBoard();
    await create("Agent pipeline", "Pipe");
    expect(commands("board_create_from_template")).toEqual([
      ["board_create_from_template", { name: "Pipe", templateId: "agent-pipeline" }],
    ]);
    expect(dialog()).toBeNull();
    expect(names()).toEqual(["Queued", "Running", "Needs review", "Merged", "Done"]);
  });

  it("blank_template_matches_todays_default_columns", async () => {
    await renderBoard();
    await openNewBoard();
    // Blank is picked until another is.
    expect(radios().map((r) => [r.getAttribute("aria-label"), r.getAttribute("aria-checked")])).toEqual([
      ["Blank", "true"],
      ["Agent pipeline", "false"],
    ]);
    await type(byLabel<HTMLInputElement>(dialog()!, "Board name"), "Plain", false);
    await click(buttonNamed(dialog()!, "Create board"));
    expect(lastArgs("board_create_from_template")).toEqual({
      name: "Plain",
      templateId: "blank",
    });
    expect(names()).toEqual(["Backlog", "In Progress", "Review", "Done"]);
  });

  it("asks for a name and does not create without one", async () => {
    await renderBoard();
    await openNewBoard();
    await click(buttonNamed(dialog()!, "Create board"));
    expect(commands("board_create_from_template")).toEqual([]);
    expect(dialog()!.textContent).toContain("Add a name");
  });

  it("still offers Blank when the template list can't be read", async () => {
    handlers.board_templates = () => {
      throw "store is locked";
    };
    await renderBoard();
    await openNewBoard();
    expect(radios().map((r) => r.getAttribute("aria-label"))).toEqual(["Blank"]);
    expect(dialog()!.textContent).toContain("store is locked");
    await type(byLabel<HTMLInputElement>(dialog()!, "Board name"), "X", false);
    await click(buttonNamed(dialog()!, "Create board"));
    expect(lastArgs("board_create_from_template")).toEqual({ name: "X", templateId: "blank" });
  });

  it("shows a failed create and keeps the board it was on", async () => {
    handlers.board_create_from_template = () => {
      throw "name is taken";
    };
    await renderBoard();
    await create(null, "Agents");
    expect(alertText()).toContain("name is taken");
    expect(names()).toContain("Backlog");
  });

  it("the empty state offers the same dialog", async () => {
    boardRows = [];
    await renderBoard();
    await click(
      [...container.querySelectorAll<HTMLButtonElement>("button")].find(
        (b) => b.textContent === "Create board",
      )!,
    );
    expect(dialog()).not.toBeNull();
    expect(radios()).toHaveLength(2);
  });
});

describe("templates section", () => {
  const row = (id: string) =>
    section("Templates").querySelector<HTMLElement>(`[data-template="${id}"]`);

  it("save_as_template_then_create_copies_labels_fields_views", async () => {
    await renderBoard();
    await openSettings();
    await type(byLabel<HTMLInputElement>(section("Templates"), "Template name"), "My setup");
    expect(commands("board_save_as_template")).toEqual([
      ["board_save_as_template", { boardId: "b1", name: "My setup" }],
    ]);
    // The commands don't announce themselves: the list was read again.
    expect(row("t1")).not.toBeNull();
    expect(
      [...section("Templates").querySelectorAll("[data-template]")].map((el) =>
        el.getAttribute("data-template"),
      ),
    ).toEqual(["blank", "agent-pipeline", "t1"]);

    await click(byLabel(page()!, "Back to board"));
    await openNewBoard();
    expect(radios().map((r) => r.getAttribute("aria-label"))).toEqual([
      "Blank",
      "Agent pipeline",
      "My setup",
    ]);
    await click(radios()[2]);
    await type(byLabel<HTMLInputElement>(dialog()!, "Board name"), "Copy", false);
    await click(buttonNamed(dialog()!, "Create board"));
    expect(lastArgs("board_create_from_template")).toEqual({
      name: "Copy",
      templateId: "t1",
    });
  });

  it("saved_template_deletes_and_built_ins_have_no_delete", async () => {
    templateRows = [
      ...templateRows,
      { id: "t9", name: "Zed", builtIn: false },
      { id: "t1", name: "Alpha", builtIn: false },
    ];
    await renderBoard();
    await openSettings();
    // Built-ins first, then saved ones by name.
    expect(
      [...section("Templates").querySelectorAll("[data-template]")].map((el) =>
        el.getAttribute("data-template"),
      ),
    ).toEqual(["blank", "agent-pipeline", "t1", "t9"]);
    for (const id of ["blank", "agent-pipeline"]) {
      expect(row(id)!.querySelector("button")).toBeNull();
      expect(
        row(id)!.querySelector("[data-template-built-in]")?.getAttribute("title"),
      ).toBe("Built-in templates can't be deleted");
    }
    vi.mocked(ask).mockResolvedValue(false);
    await click(byLabel(row("t1")!, "Delete template Alpha"));
    expect(commands("board_template_delete")).toEqual([]);
    vi.mocked(ask).mockResolvedValue(true);
    await click(byLabel(row("t1")!, "Delete template Alpha"));
    expect(commands("board_template_delete")).toEqual([
      ["board_template_delete", { templateId: "t1" }],
    ]);
    expect(row("t1")).toBeNull();
    expect(row("t9")).not.toBeNull();
  });

  it("says when a save is refused and keeps the list", async () => {
    handlers.board_save_as_template = () => {
      throw "A template named “X” already exists";
    };
    await renderBoard();
    await openSettings();
    await type(byLabel<HTMLInputElement>(section("Templates"), "Template name"), "X");
    expect(alertText()).toContain("already exists");
    expect(section("Templates").querySelectorAll("[data-template]")).toHaveLength(2);
  });
});

describe("automation section", () => {
  const box = (label: string) =>
    byLabel<HTMLInputElement>(section("Automation"), label);
  const picker = (label: string) =>
    byLabel<HTMLSelectElement>(section("Automation"), `Auto-move: ${label}`);

  it("agent_toggles_default_on_for_new_boards", async () => {
    await renderBoard();
    await openSettings();
    for (const label of ["Run with…", "Agent write-back", "Agent may change ticket status"]) {
      expect(box(label).checked, label).toBe(true);
    }
  });

  it("reads the toggles as on before settings have loaded", async () => {
    handlers.board_settings = () => {
      throw "store is locked";
    };
    await renderBoard();
    await openSettings();
    expect(box("Run with…").checked).toBe(true);
    expect(box("Agent write-back").checked).toBe(true);
  });

  it("each toggle writes only its own setting", async () => {
    await renderBoard();
    await openSettings();
    await click(box("Agent write-back"));
    expect(lastArgs("board_settings_update")).toEqual({
      boardId: "b1",
      patch: { writeBack: false },
    });
    await click(box("Agent may change ticket status"));
    expect(lastArgs("board_settings_update")).toEqual({
      boardId: "b1",
      patch: { ticketStatusChanges: false },
    });
    expect(settings).toMatchObject({ runWith: true, writeBack: false, ticketStatusChanges: false });
  });

  it("a failed toggle goes back to what is stored", async () => {
    handlers.board_settings_update = () => {
      throw "store is locked";
    };
    await renderBoard();
    await openSettings();
    await click(box("Run with…"));
    expect(alertText()).toContain("store is locked");
    expect(box("Run with…").checked).toBe(true);
  });

  it("shows each event's default rule", async () => {
    await renderBoard();
    await openSettings();
    expect(
      ["Work starts", "Work needs input", "Work completes", "Work fails", "PR opened", "PR merged"].map(
        (label) => picker(label).value,
      ),
    ).toEqual(["default", "stay", "default", "stay", "stay", "stay"]);
  });

  it("auto_move_rule_picker_saves_and_custom_rule_moves_the_card", async () => {
    await renderBoard();
    await openSettings();
    await choose(picker("PR merged"), "column:review");
    expect(lastArgs("board_settings_update")).toEqual({
      boardId: "b1",
      patch: { autoMoveJson: '{"pr_merged":{"column":"review"}}' },
    });
    // A second edit keeps the first and uses the store's snake_case keys.
    await choose(picker("Work fails"), "column:backlog");
    expect(JSON.parse((lastArgs("board_settings_update") as { patch: { autoMoveJson: string } }).patch.autoMoveJson)).toEqual({
      pr_merged: { column: "review" },
      failed: { column: "backlog" },
    });
    await choose(picker("Work starts"), "stay");
    expect(
      JSON.parse((lastArgs("board_settings_update") as { patch: { autoMoveJson: string } }).patch.autoMoveJson).started,
    ).toBe("stay");
    expect(picker("PR merged").value).toBe("column:review");
  });

  it("two quick rule edits both land, in order", async () => {
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
    await choose(picker("PR merged"), "column:review");
    await choose(picker("PR opened"), "column:progress");
    expect(replies).toHaveLength(1);
    replies.shift()!();
    await flush();
    replies.shift()!();
    await flush();
    const writes = commands("board_settings_update").map(
      ([, a]) => JSON.parse((a as { patch: { autoMoveJson: string } }).patch.autoMoveJson),
    );
    expect(writes[0]).toEqual({ pr_merged: { column: "review" } });
    expect(writes[1]).toEqual({
      pr_merged: { column: "review" },
      pr_opened: { column: "progress" },
    });
  });

  it("rule_picker_shows_stay_for_a_deleted_column", async () => {
    settings = settingsWith({
      autoMoveJson: JSON.stringify({ completed: { column: "gone" }, future_key: 1 }),
    });
    await renderBoard();
    await openSettings();
    expect(picker("Work completes").value).toBe("stay");
    // Showing it doesn't rewrite it, and a later edit keeps what else was stored.
    expect(commands("board_settings_update")).toEqual([]);
    await choose(picker("PR merged"), "stay");
    expect(
      JSON.parse((lastArgs("board_settings_update") as { patch: { autoMoveJson: string } }).patch.autoMoveJson),
    ).toEqual({
      completed: { column: "gone" },
      future_key: 1,
      pr_merged: "stay",
    });
  });
});

describe("the hidden board while settings is open", () => {
  function press(target: EventTarget, key: string, init: KeyboardEventInit = {}) {
    act(() => {
      target.dispatchEvent(
        new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }),
      );
    });
  }

  it("ignores N and Delete, loses its selection and drawer, and is inert", async () => {
    await renderBoard();
    const face = (id: string) =>
      container.querySelector<HTMLElement>(`[data-board-card='${id}']`)!;
    // Select a card, and open another one's drawer.
    act(() =>
      face("c1").dispatchEvent(
        new MouseEvent("click", { bubbles: true, metaKey: true, ctrlKey: true }),
      ),
    );
    await flush();
    expect(container.querySelector("[data-selection-bar]")).not.toBeNull();

    await openSettings();
    expect(container.querySelector("[data-selection-bar]")).toBeNull();
    expect(document.querySelector("[data-card-drawer]")).toBeNull();
    const board = container.querySelector<HTMLElement>("[inert]");
    expect(board?.querySelector("[data-kanban-board]")).not.toBeNull();

    const kanbanCard = container.querySelector<HTMLElement>("[data-kanban-card='c1']")!;
    press(kanbanCard, "Delete");
    press(kanbanCard, "Backspace");
    press(document.body, "n");
    press(kanbanCard, "n");
    press(document.body, "n", { metaKey: true, ctrlKey: true });
    await flush();
    expect(commands("board_card_archive")).toEqual([]);
    expect(commands("board_cards_archive")).toEqual([]);
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });

  it("opening settings closes an open card drawer", async () => {
    await renderBoard();
    act(() =>
      container.querySelector<HTMLElement>("[data-board-card='c1']")!.click(),
    );
    await flush();
    expect(document.querySelector("[data-card-drawer]")).not.toBeNull();
    await openSettings();
    expect(document.querySelector("[data-card-drawer]")).toBeNull();
  });

  it("the shortcuts work again after returning to the board", async () => {
    await renderBoard();
    await openSettings();
    await click(byLabel(page()!, "Back to board"));
    expect(container.querySelector("[inert]")).toBeNull();
    act(() => container.querySelector<HTMLElement>("[data-kanban-card='c1']")!.focus());
    press(container.querySelector<HTMLElement>("[data-kanban-card='c1']")!, "n");
    await flush();
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  });
});
