// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resolveModel, saveLastModelChoice } from "../../sessions/model/models";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import type {
  BoardCard,
  BoardColumn,
  BoardDetail,
  BoardField,
  CardLinkRow,
  BoardFieldKind,
  BoardLabel,
  BoardSettings,
  BoardView as SavedView,
  TemplateInfo,
  CardExtras,
  ColumnRole,
} from "../model/board";
import type { KanbanBoardProps } from "./kanban/KanbanBoard";
import type { ComponentProps as Props } from "react";
import type { Swimlanes } from "./kanban/Swimlanes";
import { BoardView } from "./BoardView";
import { waitForSearch } from "./testBackend";

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

const lanes: { props: Props<typeof Swimlanes> | null } = { props: null };
vi.mock("./kanban/Swimlanes", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./kanban/Swimlanes")>();
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
let linkRows: CardLinkRow[];
let blockedRows: Record<string, number[]>;
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
  linkRows = [];
  blockedRows = {};
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
    board_links: () => linkRows,
    board_blocked: () => blockedRows,
    board_running_cards: (args) =>
      detail.cards
        .filter((c) =>
          (args.running as [string, string][]).some(
            ([kind, id]) =>
              (c.linkKind === kind && c.linkId === id) ||
              linkRows.some(
                (l) => l.cardId === c.id && l.kind === kind && l.reference === id,
              ),
          ),
        )
        .map((c) => c.id),
    board_run_prompt: () => "prompt",
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

let lastProps: ComponentProps<typeof BoardView>;
async function renderBoard(overrides: Partial<ComponentProps<typeof BoardView>> = {}) {
  lastProps = {
    onClose: vi.fn(),
    linkTargets: [],
    onOpenLinkedWork: vi.fn(() => Promise.resolve()),
    lookupLinkTargets: vi.fn(async () => []),
    projects: [{ path: "/work/api", name: "api" }],
    ...overrides,
  };
  await act(async () => {
    root.render(createElement(BoardView, lastProps));
  });
  await flush();
  return lastProps;
}
async function rerender(overrides: Partial<ComponentProps<typeof BoardView>>) {
  lastProps = { ...lastProps, ...overrides };
  await act(async () => {
    root.render(createElement(BoardView, lastProps));
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




const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]');

const target = (
  kind: "session" | "pilot",
  id: string,
  status: BoardLinkTarget["status"],
  agent?: BoardLinkTarget["agent"],
): BoardLinkTarget => ({ kind, id, label: `${kind} ${id}`, status, agent });

const link = (
  cardId: string,
  kind: string,
  reference: string,
  role: string | null = null,
): CardLinkRow => ({ cardId, kind, reference, role, createdAt: 1 });

const settingsWith = (patch: Partial<BoardSettings>): BoardSettings => ({
  ...DEFAULTS,
  ...patch,
});
const faceOf = (id: string) =>
  container.querySelector<HTMLElement>(`[data-board-card='${id}']`)!;
const chipStates = (id: string) =>
  [...faceOf(id).querySelectorAll("[data-link-state]")].map((el) => [
    el.getAttribute("data-link-chip-kind"),
    el.getAttribute("data-link-state"),
  ]);
const primaryDot = (id: string) =>
  faceOf(id)
    .querySelector("[data-link-chip] [aria-label]")
    ?.getAttribute("aria-label");

describe("link chips", () => {
  beforeEach(() => {
    detail = {
      ...detail,
      cards: [
        { ...detail.cards[0], linkKind: "session", linkId: "s1" },
        { ...detail.cards[1], linkKind: "session", linkId: "s3" },
        { ...detail.cards[2], linkKind: "session", linkId: "s4" },
      ],
    };
  });

  it("card_shows_one_chip_per_link_running_waiting_failed_completed", async () => {
    linkRows = [link("c1", "session", "s1", "primary"), link("c1", "session", "s2")];
    handlers.board_latest_run_events = () => ({ c2: "failed" });
    await renderBoard({
      linkTargets: [
        target("session", "s1", "running"),
        target("session", "s2", "waiting"),
        target("session", "s3", "completed"),
        target("session", "s4", "completed"),
      ],
    });
    expect(primaryDot("c1")).toBe("running");
    expect(chipStates("c1")).toEqual([["session", "waiting"]]);
    // The latest event for c2's link is a failure: finished work reads failed.
    expect(primaryDot("c2")).toBe("failed");
    expect(primaryDot("c3")).toBe("completed");
  });

  it("running beats waiting beats failed for a session link", async () => {
    linkRows = [link("c1", "session", "s5"), link("c1", "session", "s6")];
    handlers.board_latest_run_events = () => ({ c1: "failed" });
    await renderBoard({
      linkTargets: [target("session", "s5", "running"), target("session", "s6", "completed")],
    });
    expect(chipStates("c1")).toEqual([
      ["session", "running"],
      ["session", "failed"],
    ]);
  });

  it("pilot_chip_shows_running", async () => {
    linkRows = [link("c1", "pilot", "p1"), link("c1", "pilot", "p2")];
    await renderBoard({
      linkTargets: [target("pilot", "p1", "running"), target("pilot", "p2", "completed")],
    });
    expect(chipStates("c1")).toEqual([
      ["pilot", "running"],
      ["pilot", "idle"],
    ]);
  });

  it("a link whose work is gone reads missing", async () => {
    linkRows = [link("c1", "session", "s9")];
    await renderBoard({ lookupLinkTargets: vi.fn(async () => []) });
    expect(chipStates("c1")).toEqual([["session", "missing"]]);
  });

  it("pr_chip_shows_merged_and_checks_failing", async () => {
    const url = "https://github.com/acme/api/pull/42";
    linkRows = [link("c1", "pr", url)];
    const lookup = vi.fn(async () => ({
      [`pr\n${url}`]: { state: "merged" as const, checks: "failing" as const },
    }));
    await renderBoard({ lookupLinkStatus: lookup });
    expect(lookup).toHaveBeenCalledWith([{ kind: "pr", reference: url }]);
    const chip = faceOf("c1").querySelector("[data-link-chip-kind='pr']")!;
    expect(chip.getAttribute("data-link-state")).toBe("merged");
    expect(chip.textContent).toContain("PR #42");
    expect(chip.querySelector("[data-link-checks]")?.getAttribute("data-link-checks")).toBe("failing");
  });

  it("a PR whose status can't be read shows unknown, not an error", async () => {
    const url = "https://github.com/acme/api/pull/7";
    linkRows = [link("c1", "pr", url)];
    await renderBoard({
      lookupLinkStatus: vi.fn(async () => {
        throw "remote workspaces can't run gh";
      }),
    });
    expect(chipStates("c1")).toEqual([["pr", "unknown"]]);
    expect(alertText()).toBe("");
  });

  it("worktree_chip_shows_branch_and_diff_stats", async () => {
    linkRows = [link("c1", "worktree", "/work/api-wt")];
    await renderBoard({
      lookupLinkStatus: vi.fn(async () => ({
        "worktree\n/work/api-wt": {
          state: "idle" as const,
          branch: "feat/login",
          diff: { additions: 12, deletions: 3 },
        },
      })),
    });
    const chip = faceOf("c1").querySelector("[data-link-chip-kind='worktree']")!;
    expect(chip.textContent).toContain("feat/login");
    expect(chip.querySelector("[data-link-diff]")?.textContent).toBe("+12 −3");
  });

  it("the Linked work display toggle hides the chips", async () => {
    linkRows = [link("c1", "session", "s2")];
    settings = settingsWith({
      displayJson: JSON.stringify({ face: { linkStatus: false } }),
    });
    await renderBoard({ linkTargets: [target("session", "s2", "waiting")] });
    expect(chipStates("c1")).toEqual([]);
  });
});

describe("running set and refresh", () => {
  it("linked_running_filter_matches_cards_with_open_turns", async () => {
    detail = {
      ...detail,
      cards: [
        { ...detail.cards[0], linkKind: "session", linkId: "s1" },
        detail.cards[1],
        detail.cards[2],
      ],
    };
    linkRows = [link("c2", "session", "s2")];
    await renderBoard({
      linkTargets: [
        target("session", "s1", "running"),
        target("session", "s2", "waiting"),
        target("session", "s3", "completed"),
        target("pilot", "p1", "completed"),
      ],
    });
    // Every link counts, not just the primary; stopped work doesn't.
    expect(lastArgs("board_running_cards")).toEqual({
      boardId: "b1",
      running: [
        ["session", "s1"],
        ["session", "s2"],
      ],
    });
    const search = container.querySelector<HTMLInputElement>('input[aria-label="Search cards"]')!;
    await type(search, "o", false);
    await waitForSearch();
    expect((lastArgs("board_query") as { running: string[] }).running).toEqual(["c1", "c2"]);
  });

  it("falls back to primary links when the store can't answer", async () => {
    detail = {
      ...detail,
      cards: [{ ...detail.cards[0], linkKind: "session", linkId: "s1" }, detail.cards[1], detail.cards[2]],
    };
    handlers.board_running_cards = () => {
      throw "store is locked";
    };
    await renderBoard({ linkTargets: [target("session", "s1", "running")] });
    const search = container.querySelector<HTMLInputElement>('input[aria-label="Search cards"]')!;
    await type(search, "o", false);
    await waitForSearch();
    expect((lastArgs("board_query") as { running: string[] }).running).toEqual(["c1"]);
    expect(alertText()).toBe("");
  });

  it("only work linked to this board's cards triggers a re-read when it settles", async () => {
    linkRows = [link("c1", "pr", "https://github.com/acme/api/pull/42")];
    detail = {
      ...detail,
      cards: [{ ...detail.cards[0], linkKind: "session", linkId: "s1" }, detail.cards[1], detail.cards[2]],
    };
    const lookup = vi.fn(async () => ({}));
    await renderBoard({
      lookupLinkStatus: lookup,
      linkTargets: [
        target("session", "s1", "running"),
        target("session", "elsewhere", "running"),
      ],
    });
    const reads = () => [lookup.mock.calls.length, commands("board_links").length];
    const before = reads();
    // Some other session in the app finishes: this board has no reason to read again.
    await rerender({
      linkTargets: [target("session", "s1", "running"), target("session", "elsewhere", "completed")],
    });
    expect(reads()).toEqual(before);
    // A linked one finishing does.
    await rerender({
      linkTargets: [target("session", "s1", "completed"), target("session", "elsewhere", "completed")],
    });
    expect(reads()).toEqual([before[0] + 1, before[1] + 1]);
  });

  it("link_status_refresh_has_no_interval", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    detail = {
      ...detail,
      cards: [{ ...detail.cards[0], linkKind: "session", linkId: "s1" }, detail.cards[1], detail.cards[2]],
    };
    try {
      const url = "https://github.com/acme/api/pull/42";
      linkRows = [link("c1", "pr", url)];
      const lookup = vi.fn(async () => ({}));
      await renderBoard({
        lookupLinkStatus: lookup,
        linkTargets: [target("session", "s1", "running")],
      });
      const counts = () => [
        lookup.mock.calls.length,
        commands("board_links").length,
        commands("board_running_cards").length,
        commands("board_blocked").length,
      ];
      const before = counts();
      expect(before[0]).toBe(1);
      await act(async () => {
        vi.advanceTimersByTime(30 * 60_000);
      });
      await flush();
      // Half an hour of quiet: nothing was asked again.
      expect(counts()).toEqual(before);
      // A linked turn settling reads it once.
      await rerender({ linkTargets: [target("session", "s1", "completed")] });
      expect(lookup.mock.calls.length).toBe(2);
      expect(commands("board_links").length).toBe(before[1] + 1);
      await act(async () => {
        vi.advanceTimersByTime(30 * 60_000);
      });
      await flush();
      expect(lookup.mock.calls.length).toBe(2);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("blocked cards and Run with…", () => {
  const open = async (id: string) => {
    act(() => faceOf(id).click());
    await flush();
  };
  const runWithButton = () =>
    document.querySelector<HTMLButtonElement>("[data-card-drawer] [data-run-with]");

  it("blocked_card_shows_lock_until_blocker_is_done", async () => {
    blockedRows = { c2: [1, 3] };
    await renderBoard({ onRunWith: vi.fn(async () => undefined) });
    const lock = faceOf("c2").querySelector("[data-card-blocked]")!;
    expect(lock.getAttribute("aria-label")).toBe("Blocked by #1, #3");
    expect(faceOf("c1").querySelector("[data-card-blocked]")).toBeNull();
    blockedRows = {};
    emitBoardChanged();
    await flush();
    expect(faceOf("c2").querySelector("[data-card-blocked]")).toBeNull();
  });

  it("run_with_on_blocked_card_asks_first_and_cancel_starts_nothing", async () => {
    blockedRows = { c2: [1, 3] };
    vi.mocked(ask).mockResolvedValue(false);
    const onRunWith = vi.fn(async () => undefined);
    await renderBoard({ onRunWith });
    await open("c2");
    await click(runWithButton()!);
    expect(ask).toHaveBeenCalledWith(
      "#2 is blocked by #1, #3. Run anyway?",
      expect.objectContaining({ okLabel: "Run anyway", cancelLabel: "Cancel" }),
    );
    expect(dialog()).toBeNull();
    expect(onRunWith).not.toHaveBeenCalled();

    vi.mocked(ask).mockResolvedValue(true);
    await click(runWithButton()!);
    expect(dialog()).not.toBeNull();
  });

  it("asks the store at click time, so a stale map can't let a blocked card run", async () => {
    vi.mocked(ask).mockResolvedValue(false);
    await renderBoard({ onRunWith: vi.fn(async () => undefined) });
    // Nothing blocked when the board loaded; a blocker appeared since.
    expect(faceOf("c2").querySelector("[data-card-blocked]")).toBeNull();
    blockedRows = { c2: [1] };
    await open("c2");
    await click(runWithButton()!);
    expect(ask).toHaveBeenCalledWith(
      "#2 is blocked by #1. Run anyway?",
      expect.anything(),
    );
    expect(dialog()).toBeNull();
  });

  it("if the store can't say whether the card is blocked, nothing starts", async () => {
    await renderBoard({ onRunWith: vi.fn(async () => undefined) });
    await open("c1");
    handlers.board_blocked = () => {
      throw "store is locked";
    };
    await click(runWithButton()!);
    expect(ask).not.toHaveBeenCalled();
    expect(dialog()).toBeNull();
    expect(alertText()).toContain("store is locked");
  });

  it("the picker starts on the user's default provider", async () => {
    saveLastModelChoice("codex", resolveModel("codex").id);
    await renderBoard({ onRunWith: vi.fn(async () => undefined) });
    await open("c1");
    await click(runWithButton()!);
    expect(byLabel<HTMLSelectElement>(dialog()!, "Provider").value).toBe("codex");
  });

  it("run_with_starts_linked_session_with_card_prompt_and_inherits_ticket: the picker hands the app what to start", async () => {
    const onRunWith = vi.fn(async () => undefined);
    await renderBoard({ onRunWith });
    await open("c1");
    await click(runWithButton()!);
    // An unblocked card asks nothing.
    expect(ask).not.toHaveBeenCalled();
    const picker = dialog()!;
    expect(
      ["Provider", "Model", "Project", "New worktree"].every((l) =>
        picker.querySelector(`[aria-label="${l}"]`),
      ),
    ).toBe(true);
    await choose(byLabel<HTMLSelectElement>(picker, "Provider"), "codex");
    await click(byLabel(picker, "New worktree"));
    await click(buttonNamed(picker, "Run"));
    expect(onRunWith).toHaveBeenCalledTimes(1);
    const request = onRunWith.mock.calls[0][0] as Record<string, unknown>;
    expect(request).toMatchObject({
      cardId: "c1",
      harness: "codex",
      cwd: "/work/api",
      worktree: true,
    });
    expect(typeof request.model).toBe("string");
    // The prompt is the store's: the board never builds or carries one.
    expect(request).not.toHaveProperty("prompt");
    expect(dialog()).toBeNull();
  });

  it("shows a failed start", async () => {
    const onRunWith = vi.fn(async () => {
      throw "no provider is signed in";
    });
    await renderBoard({ onRunWith });
    await open("c1");
    await click(runWithButton()!);
    await click(buttonNamed(dialog()!, "Run"));
    expect(alertText()).toContain("no provider is signed in");
  });

  it("run_with_hidden_when_board_turns_it_off", async () => {
    settings = settingsWith({ runWith: false });
    await renderBoard({ onRunWith: vi.fn(async () => undefined) });
    await open("c1");
    expect(runWithButton()).toBeNull();
  });

  it("is hidden without a way to start, and while settings are unknown", async () => {
    await renderBoard();
    await open("c1");
    expect(runWithButton()).toBeNull();
  });

  it("is hidden when the board's settings couldn't be read", async () => {
    handlers.board_settings = () => {
      throw "store is locked";
    };
    await renderBoard({ onRunWith: vi.fn(async () => undefined) });
    await open("c1");
    expect(runWithButton()).toBeNull();
  });
});

describe("agent fields", () => {
  const facts = { provider: "Claude", model: "Sonnet", runTimeMs: 75 * 60_000 };

  it("agent_fields_show_provider_model_cost_and_run_time", async () => {
    detail = {
      ...detail,
      cards: [{ ...detail.cards[0], linkKind: "session", linkId: "s1" }, detail.cards[1], detail.cards[2]],
    };
    linkRows = [link("c1", "session", "s2")];
    settings = settingsWith({
      displayJson: JSON.stringify({
        face: { agentFields: ["provider", "model", "cost", "runTime"] },
      }),
    });
    await renderBoard({
      linkTargets: [
        target("session", "s1", "completed", facts),
        target("session", "s2", "completed", {
          provider: "Codex",
          model: "GPT",
          runTimeMs: 5 * 60_000,
          costUsd: 1.5,
        }),
      ],
    });
    const chips = [...faceOf("c1").querySelectorAll("[data-card-field]")].map(
      (el) => el.textContent,
    );
    expect(chips).toEqual([
      "Provider: Claude, Codex",
      "Model: Sonnet, GPT",
      "Cost: $1.50",
      "Run time: 1h 20m",
    ]);
    // A card with no linked session shows none.
    expect(faceOf("c2").querySelector("[data-card-field]")).toBeNull();
  });

  it("a cost nobody reported isn't shown, and the toggles choose what shows", async () => {
    detail = {
      ...detail,
      cards: [{ ...detail.cards[0], linkKind: "session", linkId: "s1" }, detail.cards[1], detail.cards[2]],
    };
    settings = settingsWith({
      displayJson: JSON.stringify({ face: { agentFields: ["cost", "model"] } }),
    });
    await renderBoard({ linkTargets: [target("session", "s1", "completed", facts)] });
    expect(
      [...faceOf("c1").querySelectorAll("[data-card-field]")].map((el) => el.textContent),
    ).toEqual(["Model: Sonnet"]);
  });

  it("table columns show the agent fields from the linked sessions", async () => {
    detail = {
      ...detail,
      cards: [{ ...detail.cards[0], linkKind: "session", linkId: "s1" }, detail.cards[1], detail.cards[2]],
    };
    savedViews = [
      {
        id: "v1",
        boardId: "b1",
        name: "Agents",
        position: 0,
        specJson: JSON.stringify({ query: "", rows: [], sort: { key: "manual", dir: "asc" } }),
        layoutJson: JSON.stringify({
          layout: "table",
          columns: ["title", "agent:provider", "agent:model", "agent:runTime", "agent:cost"],
        }),
        createdAt: 1,
        updatedAt: 1,
      },
    ];
    settings = settingsWith({ defaultViewId: "v1" });
    await renderBoard({ linkTargets: [target("session", "s1", "completed", facts)] });
    const cell = (key: string) =>
      [...container.querySelectorAll(`[data-agent-field='${key}']`)].map((el) => el.textContent);
    expect(cell("provider")[0]).toBe("Claude");
    expect(cell("model")[0]).toBe("Sonnet");
    expect(cell("runTime")[0]).toBe("1h 15m");
    expect(cell("cost")[0]).toBe("");
    const headers = [...container.querySelectorAll('[role="columnheader"]')].map((el) => el.textContent?.trim());
    expect(headers).toEqual(expect.arrayContaining(["Provider", "Model", "Run time", "Cost"]));
  });
});
