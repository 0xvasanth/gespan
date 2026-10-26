// @vitest-environment happy-dom
// Deferred drawer, Escape, estimate and column fixes (C6-15): each test
// fails without its fix, or says it guards behaviour that already held.
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import type {
  BoardCard,
  BoardColumn,
  BoardSettings,
  CardLinkRow,
  PriorityScheme,
} from "../model/board";
import { DEFAULT_PRIORITY_SCHEME, priorityName } from "../model/boardSettings";
import { encodeField, parseEstimate } from "../model/cardProps";
import { SEARCH_DEBOUNCE_MS } from "../model/constants";
import { BoardView, type BoardCommand } from "./BoardView";
import type { BoardLinkTarget } from "./boardFormat";
import { CardDetail } from "./CardDetail";
import { PrioritySection } from "./settings/PrioritySection";
import {
  createTestBackend,
  flush,
  waitForSearch,
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
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(() => Promise.resolve()),
  revealItemInDir: vi.fn(() => Promise.resolve()),
}));
vi.mock("../../../app/shell/WindowControls", () => ({
  WindowControls: () => null,
}));

const ESTIMATE_HELP = "Use minutes or hours, like 90, 90m, 1h 30m or 1.5h";

let container: HTMLDivElement;
let root: Root;
let backend: TestBackend;

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
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

// ---- helpers ----

const q = <T extends Element = HTMLElement>(selector: string) =>
  document.querySelector<T>(selector);
const drawer = () => q("[data-card-drawer]");
const commands = (name: string) =>
  vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === name);
const commandNames = () => vi.mocked(invoke).mock.calls.map(([cmd]) => cmd);
const alerts = () =>
  [...document.querySelectorAll('[role="alert"]')].map((el) => el.textContent);

/** Settles promises until `done` holds (bounded, so a failure can't hang). */
async function until(done: () => boolean) {
  for (let i = 0; i < 40 && !done(); i += 1) await flush();
  expect(done()).toBe(true);
}

const boardProps = (
  over: Partial<ComponentProps<typeof BoardView>> = {},
): ComponentProps<typeof BoardView> => ({
  onClose: vi.fn(),
  linkTargets: [],
  onOpenLinkedWork: vi.fn(() => Promise.resolve()),
  lookupLinkTargets: vi.fn(async () => []),
  projects: [],
  ...over,
});

async function renderBoard(
  over: Partial<ComponentProps<typeof BoardView>> = {},
) {
  const props = boardProps(over);
  await act(async () => root.render(createElement(BoardView, props)));
  await flush();
  await flush();
  return props;
}

async function click(el: Element) {
  await act(async () => (el as HTMLElement).click());
  await flush();
}

function press(el: Element, key: string) {
  const event = new KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
  });
  act(() => {
    el.dispatchEvent(event);
  });
  return event;
}

/** Types into a React-controlled input or textarea. */
function typeInto(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const proto =
    el instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
  act(() => {
    Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function openCardFromBoard(id: string) {
  await click(q(`[data-board-card="${id}"]`)!);
  await until(() => drawer() !== null);
}

const menuEntries = () => [
  ...document.querySelectorAll<HTMLElement>(
    '[role="menu"] [role="menuitem"], [role="menu"] [role="menuitemradio"], [role="menu"] [role="menuitemcheckbox"], [role="menu"] button',
  ),
];
const menuEntry = (text: string) =>
  menuEntries().find((el) => el.textContent?.trim() === text);

function rightClick(id: string) {
  act(() => {
    q(`[data-board-card='${id}']`)!.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 40,
        clientY: 60,
      }),
    );
  });
}

// ---- CardDetail rendered alone ----

const CARD: BoardCard = {
  id: "c3",
  boardId: "b1",
  columnId: "progress",
  number: 3,
  title: "Ship board",
  body: "",
  position: 0,
  linkKind: null,
  linkId: null,
  createdAt: 1,
  updatedAt: 1,
};
const DRAWER_COLUMNS = [
  { id: "backlog", name: "Backlog", cardCount: 2 },
  { id: "progress", name: "In Progress", cardCount: 1 },
  { id: "done", name: "Done", cardCount: 0 },
];

async function renderDetail(
  over: Partial<ComponentProps<typeof CardDetail>> = {},
) {
  const props: ComponentProps<typeof CardDetail> = {
    card: CARD,
    columns: DRAWER_COLUMNS,
    labels: [],
    projects: [],
    link: null,
    linkTargets: [],
    onClose: vi.fn(),
    onOpenLinkedWork: vi.fn(),
    onMove: vi.fn(),
    onDelete: vi.fn(),
    onError: vi.fn(),
    ...over,
  };
  await act(async () => root.render(createElement(CardDetail, props)));
  await flush();
  return props;
}

const field = <T extends HTMLElement = HTMLInputElement>(label: string) =>
  drawer()!.querySelector<T>(`[aria-label="${label}"]`)!;

// =====================================================================

describe("Escape layering (R45)", () => {
  it("escape_with_focus_on_the_board_closes_the_open_drawer", async () => {
    await renderBoard();
    await openCardFromBoard("c1");
    // Focus on the board, outside the drawer: the search box, then a card.
    const search = q<HTMLInputElement>('input[aria-label="Search cards"]')!;
    search.focus();
    const event = press(search, "Escape");
    await flush();
    expect(drawer()).toBeNull();
    expect(event.defaultPrevented).toBe(true);

    await openCardFromBoard("c1");
    const other = q("[data-kanban-card='c2']")!;
    other.focus();
    press(other, "Escape");
    await flush();
    expect(drawer()).toBeNull();
    // Focus stayed where the user had put it on the board.
    expect(document.activeElement).toBe(q("[data-kanban-card='c2']"));
  });

  it("escape_in_a_comment_or_subtask_input_keeps_the_text_and_the_drawer", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    for (const label of ["Add a comment", "New subtask"]) {
      const props = await renderDetail();
      const input = field<HTMLInputElement | HTMLTextAreaElement>(label);
      input.focus();
      typeInto(input, "half a thought");
      press(input, "Escape");
      await flush();
      // The first Escape only leaves the field; the text and drawer stay.
      expect(props.onClose).not.toHaveBeenCalled();
      expect(field<HTMLInputElement>(label).value).toBe("half a thought");
      expect(document.activeElement).toBe(drawer());
      // The second closes.
      press(document.activeElement!, "Escape");
      expect(props.onClose).toHaveBeenCalledTimes(1);
      act(() => root.unmount());
      root = createRoot(container);
    }
  });
});

describe("drawer saves and focus (R46, R47)", () => {
  it("remote_delete_with_a_dirty_draft_shows_no_save_error", async () => {
    await renderBoard();
    await openCardFromBoard("c1");
    const title = field("Title");
    title.focus();
    typeInto(title, "Fix login properly");
    // The other app deletes the card while the title is half edited.
    backend.cards = backend.cards.filter((c) => c.id !== "c1");
    backend.handlers.board_card_update = () => {
      throw "Card not found: c1";
    };
    act(() => backend.emitBoardChanged());
    await until(() => drawer() === null);
    await flush();
    await flush();
    expect(alerts().join(" ")).not.toContain("Couldn't save");
  });

  it("a failed save of a card that is still there still says so", async () => {
    // Guard for the R46 fix: only a vanished card's save is quiet.
    await renderBoard();
    await openCardFromBoard("c1");
    const title = field("Title");
    title.focus();
    typeInto(title, "Fix login properly");
    backend.handlers.board_card_update = () => {
      throw "database is locked";
    };
    await click(q('button[aria-label="Close card"]')!);
    await until(() => alerts().join(" ").includes("Couldn't save"));
  });

  it("drawer_takes_focus_on_open_and_returns_it_on_close", async () => {
    const scrolled: Element[] = [];
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function (this: Element) {
      scrolled.push(this);
    };
    try {
      await renderBoard();
      await openCardFromBoard("c2");
      expect(drawer()!.contains(document.activeElement)).toBe(true);
      await click(q('button[aria-label="Close card"]')!);
      await until(() => drawer() === null);
      const card = q("[data-kanban-card='c2']")!;
      expect(document.activeElement).toBe(card);
      expect(scrolled).toContain(card);
    } finally {
      Element.prototype.scrollIntoView = original;
    }
  });

  it("property_triggers_name_their_value", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await renderDetail({
      extras: {
        priority: 3,
        dueAt: null,
        projects: [],
        labels: [],
        subtasksDone: 0,
        subtasksTotal: 0,
        comments: 0,
      },
    });
    const labels = [
      ...drawer()!.querySelectorAll<HTMLButtonElement>(
        "[data-property] button[aria-haspopup='menu']",
      ),
    ].map((el) => el.getAttribute("aria-label"));
    expect(labels).toEqual(
      expect.arrayContaining([
        "Status: In Progress",
        `Priority: ${priorityName(DEFAULT_PRIORITY_SCHEME, 3)}`,
        "Due: None",
        "Start: None",
        "Projects: None",
        "Labels: None",
      ]),
    );
  });

  it("drawer_tabs_have_tabpanels", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await renderDetail();
    const tabs = [...drawer()!.querySelectorAll<HTMLElement>('[role="tab"]')];
    expect(tabs.map((t) => t.textContent)).toEqual(["Comments", "Activity"]);
    for (const tab of tabs) {
      expect(tab.id).not.toBe("");
      expect(tab.getAttribute("aria-controls")).toBeTruthy();
    }
    const panel = (tab: HTMLElement) =>
      document.getElementById(tab.getAttribute("aria-controls")!);
    expect(panel(tabs[0])?.getAttribute("role")).toBe("tabpanel");
    expect(panel(tabs[0])?.getAttribute("aria-labelledby")).toBe(tabs[0].id);
    await click(tabs[1]);
    expect(panel(tabs[1])?.getAttribute("role")).toBe("tabpanel");
    expect(panel(tabs[1])?.getAttribute("aria-labelledby")).toBe(tabs[1].id);
  });

  it("due_input_saves_only_a_complete_date", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await renderDetail();
    const due = field("Due date");
    due.focus();
    // WebKit reports each typed year digit as a whole date: 0002, 0020, 0202.
    for (const partial of ["0002-03-04", "0020-03-04", "0202-03-04"]) {
      typeInto(due, partial);
    }
    await flush();
    expect(commands("board_card_set_due")).toEqual([]);
    typeInto(due, "2026-03-04");
    await flush();
    expect(commands("board_card_set_due")).toEqual([
      [
        "board_card_set_due",
        { cardId: "c3", dueAt: new Date(2026, 2, 4).getTime() },
      ],
    ]);
    // Leaving a half-typed date writes nothing and shows the saved one.
    typeInto(due, "0020-01-01");
    act(() => due.blur());
    await flush();
    expect(commands("board_card_set_due")).toHaveLength(1);
    expect(field("Due date").value).toBe("2026-03-04");
  });
});

describe("estimate and number fields (R67, R68)", () => {
  it("rejected_estimate_keeps_the_typed_text", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd) =>
      cmd === "board_card_meta"
        ? {
            priority: 0,
            dueAt: null,
            startAt: null,
            estimateMinutes: 90,
            archivedAt: null,
            completedAt: null,
          }
        : [],
    );
    await renderDetail();
    await flush();
    const estimate = field("Estimate");
    expect(estimate.value).toBe("1h 30m");
    act(() => estimate.focus());
    typeInto(estimate, "1.m");
    act(() => estimate.blur());
    await flush();
    expect(field("Estimate").value).toBe("1.m");
    expect(drawer()!.textContent).toContain(ESTIMATE_HELP);
    expect(commands("board_card_set_estimate")).toEqual([]);
    // Fixing it saves.
    act(() => field("Estimate").focus());
    typeInto(field("Estimate"), ".5h");
    act(() => field("Estimate").blur());
    await flush();
    expect(commands("board_card_set_estimate")).toEqual([
      ["board_card_set_estimate", { cardId: "c3", minutes: 30 }],
    ]);
    expect(field("Estimate").value).toBe("30m");
  });

  it("estimate_grammar_matches_the_shared_rule", () => {
    for (const [text, minutes] of [
      ["90", 90],
      ["90m", 90],
      ["1h", 60],
      ["1h 30m", 90],
      ["1.5h", 90],
      [".5h", 30],
      ["0.5h", 30],
      ["1.25h", 75],
      ["0.01h", 1],
    ] as const) {
      expect(parseEstimate(text), text).toEqual({ ok: true, minutes });
    }
    for (const text of [
      "1.m",
      "0x10",
      "1e3",
      "-5",
      "-1h",
      "h",
      "1h m",
      "1.5",
      "1..5h",
      "1.2.3h",
    ]) {
      expect(parseEstimate(text), text).toEqual({
        ok: false,
        error: ESTIMATE_HELP,
      });
    }
    expect(parseEstimate("0m").ok).toBe(false);
  });

  it("field_numbers_reject_hex_and_exponent", () => {
    for (const text of [
      "0x10",
      "1e3",
      "1E3",
      "0b11",
      "0o7",
      "Infinity",
      "1.",
      "+3",
      "1_000",
    ]) {
      expect(encodeField("number", text).ok, text).toBe(false);
    }
    expect(encodeField("number", "-2.5")).toEqual({ ok: true, value: "-2.5" });
    expect(encodeField("number", ".5")).toEqual({ ok: true, value: "0.5" });
    expect(encodeField("number", " 42 ")).toEqual({ ok: true, value: "42" });
  });

  it("close_in_full_page_closes_the_card_and_clears_the_page_state", async () => {
    await renderBoard();
    await openCardFromBoard("c1");
    await click(q('button[aria-label="Open full page"]')!);
    expect(drawer()!.hasAttribute("data-full-page")).toBe(true);
    await click(q('button[aria-label="Close card"]')!);
    expect(drawer()).toBeNull();
    // The board is back, and the next open follows the setting (a drawer).
    expect(q("[data-board-card='c1']")!.closest(".hidden")).toBeNull();
    await openCardFromBoard("c1");
    expect(drawer()!.hasAttribute("data-full-page")).toBe(false);
    // Escape in the full page still steps back to the drawer first.
    await click(q('button[aria-label="Open full page"]')!);
    press(drawer()!, "Escape");
    await flush();
    expect(drawer()!.hasAttribute("data-full-page")).toBe(false);
  });
});

describe("board actions (R54, R69, R81, R83)", () => {
  it("right_click_duplicate_flushes_drawer_drafts_first", async () => {
    backend.handlers.board_card_duplicate = () => ({
      ...backend.cards[0],
      id: "c9",
      number: 9,
    });
    await renderBoard();
    await openCardFromBoard("c1");
    const title = field("Title");
    title.focus();
    typeInto(title, "Fix login v2");
    rightClick("c1");
    await click(menuEntry("Duplicate")!);
    await until(() => commands("board_card_duplicate").length === 1);
    const names = commandNames();
    const update = names.indexOf("board_card_update");
    expect(update).toBeGreaterThanOrEqual(0);
    expect(update).toBeLessThan(names.indexOf("board_card_duplicate"));
    expect(commands("board_card_update")[0][1]).toEqual({
      cardId: "c1",
      title: "Fix login v2",
      body: "",
    });
  });

  it("status_picker_move_in_the_drawer_offers_undo", async () => {
    // Guards behaviour that already held in Tauri (the drawer's Status menu
    // goes through the board's move with an undo toast).
    await renderBoard();
    await openCardFromBoard("c1");
    await click(drawer()!.querySelector('button[aria-label^="Status"]')!);
    await click(menuEntry("Done")!);
    await until(() => commands("board_card_move").length === 1);
    expect(commands("board_card_move")[0][1]).toMatchObject({
      cardId: "c1",
      columnId: "done",
    });
    await until(() => q("[data-board-undo-toast]") !== null);
    expect(q("[data-board-undo-toast]")!.textContent).toContain(
      "Moved #1 to Done",
    );
    const undo = [
      ...q("[data-board-undo-toast]")!.querySelectorAll("button"),
    ].find((b) => b.textContent === "Undo")!;
    await click(undo);
    await until(() => commands("board_card_move").length === 2);
    expect(commands("board_card_move")[1][1]).toMatchObject({
      cardId: "c1",
      columnId: "backlog",
      index: 0,
    });
  });

  it("card_index_failure_shows_an_error", async () => {
    backend.savedViews = [
      {
        id: "v1",
        boardId: "b1",
        name: "Table",
        position: 0,
        specJson: JSON.stringify({
          query: "",
          rows: [],
          sort: { key: "manual", dir: "asc" },
        }),
        layoutJson: '{"layout":"table"}',
        createdAt: 1,
        updatedAt: 1,
      },
    ];
    backend.handlers.board_card_index = () => {
      throw "database is locked";
    };
    await renderBoard();
    await click(q('[data-view-tab="v1"]')!);
    await until(() => commands("board_card_index").length > 0);
    await until(() => alerts().join(" ").includes("database is locked"));
  });

  it("pending_palette_command_clears_when_boards_fail_to_load", async () => {
    // The open board fails to load: the "new card" command is dropped, not
    // kept to fire whenever the board loads later.
    backend.handlers.board_get = () => {
      throw "database is locked";
    };
    const onCommandHandled = vi.fn();
    const command: BoardCommand = { id: "new-card", nonce: 7 };
    await renderBoard({ command, onCommandHandled });
    await until(() => onCommandHandled.mock.calls.length === 1);
    expect(onCommandHandled).toHaveBeenCalledWith(7);
    backend.handlers.board_get = () => ({
      board: { id: "b1", name: "Agents", createdAt: 1, updatedAt: 1 },
      columns: backend.columns,
      cards: backend.live(),
    });
    act(() => backend.emitBoardChanged());
    await until(() => q("[data-board-card='c1']") !== null);
    expect(q('[role="dialog"]')).toBeNull();

    // The board list itself failing drops the command too (guard).
    act(() => root.unmount());
    root = createRoot(container);
    backend.handlers.board_list = () => {
      throw "database is locked";
    };
    const second = vi.fn();
    await renderBoard({
      command: { id: "new-card", nonce: 8 },
      onCommandHandled: second,
    });
    await until(() => second.mock.calls.length === 1);
  });

  it("slash_without_a_search_input_does_nothing", async () => {
    // Guards behaviour that already held: with no search box to focus, "/"
    // is left alone (not swallowed) and nothing moves.
    await renderBoard();
    const search = q<HTMLInputElement>('input[aria-label="Search cards"]')!;
    search.removeAttribute("aria-label");
    const before = document.activeElement;
    const event = press(document.body, "/");
    expect(event.defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(before);
    // With the box there, "/" focuses it.
    search.setAttribute("aria-label", "Search cards");
    const again = press(document.body, "/");
    expect(again.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(search);
  });

  it("board_latest_run_events failing leaves the board loaded", async () => {
    // Guard (C6-14 follow-up): the board-wide run-event read is optional.
    backend.handlers.board_latest_run_events = () => {
      throw "database is locked";
    };
    await renderBoard();
    await until(() => q("[data-board-card='c4']") !== null);
    expect(alerts()).toEqual([]);
  });
});

describe("links (R61, controller dedupe)", () => {
  const LINKS: CardLinkRow[] = [
    {
      cardId: "c1",
      kind: "session",
      reference: "s-prim",
      role: null,
      createdAt: 1,
    },
    {
      cardId: "c1",
      kind: "session",
      reference: "s-elsewhere-123456",
      role: null,
      createdAt: 2,
    },
    {
      cardId: "c1",
      kind: "pilot",
      reference: "p-elsewhere-123456",
      role: null,
      createdAt: 3,
    },
  ];
  const TARGETS: BoardLinkTarget[] = [
    { kind: "session", id: "s-prim", label: "Primary work" },
  ];

  it("links_list_names_sessions_and_pilots", async () => {
    backend.cards = backend.cards.map((c) =>
      c.id === "c1" ? { ...c, linkKind: "session", linkId: "s-prim" } : c,
    );
    backend.handlers.board_links = () => LINKS;
    backend.handlers.board_card_links = () => LINKS;
    const lookupLinkTargets = vi.fn(async () => [
      {
        kind: "session" as const,
        id: "s-elsewhere-123456",
        label: "Parser rewrite",
      },
      {
        kind: "pilot" as const,
        id: "p-elsewhere-123456",
        label: "Nightly pilot",
      },
    ]);
    await renderBoard({ linkTargets: TARGETS, lookupLinkTargets });
    await until(() => lookupLinkTargets.mock.calls.length > 0);
    await flush();
    await openCardFromBoard("c1");
    await until(
      () =>
        drawer()!.querySelectorAll('[data-section="links"] [data-link]')
          .length > 0,
    );
    const rows = [
      ...drawer()!.querySelectorAll('[data-section="links"] [data-link]'),
    ].map((el) => el.textContent);
    expect(rows.join(" | ")).toContain("Parser rewrite");
    expect(rows.join(" | ")).toContain("Nightly pilot");
    expect(rows.join(" | ")).not.toContain("s-elsewh");
    // The primary link, also stored as an extra row, shows once: in the
    // Linked work row, not again in the list.
    expect(rows).toHaveLength(2);
    expect(rows.join(" | ")).not.toContain("Primary work");
  });

  it("a link that is both primary and extra shows one chip on the card", async () => {
    backend.cards = backend.cards.map((c) =>
      c.id === "c1" ? { ...c, linkKind: "session", linkId: "s-prim" } : c,
    );
    backend.handlers.board_links = () => LINKS.slice(0, 1);
    await renderBoard({ linkTargets: TARGETS });
    const face = q("[data-board-card='c1']")!;
    expect(face.querySelectorAll("[data-link-chip]")).toHaveLength(1);
    expect(
      face.querySelectorAll("[data-link-chip-kind='session']"),
    ).toHaveLength(0);
  });
});

describe("columns (R56, R84, R86)", () => {
  const collapsedBacklog = (): BoardColumn[] =>
    backend.columns.map((c) =>
      c.id === "backlog" ? { ...c, collapsed: true } : c,
    );

  it("collapsed_strip_label_and_count_read_shown_of_total", async () => {
    backend.columns = collapsedBacklog();
    await renderBoard();
    const strip = () => q("[data-kanban-column='backlog']")!;
    expect(strip().getAttribute("aria-label")).toBe("Backlog, 2 cards");
    expect(strip().querySelector("[data-column-count]")!.textContent).toBe("2");
    const search = q<HTMLInputElement>('input[aria-label="Search cards"]')!;
    typeInto(search, "fix");
    await waitForSearch();
    await flush();
    expect(strip().getAttribute("aria-label")).toBe("Backlog, 1 of 2 cards");
    expect(strip().querySelector("[data-column-count]")!.textContent).toBe(
      "1/2",
    );
    expect(SEARCH_DEBOUNCE_MS).toBe(150);
  });

  it("header_delete_with_cards_offers_move_cards_to", async () => {
    let counts = { live: 2, archived: 0 };
    backend.handlers.board_column_card_count = () => counts;
    backend.handlers.board_column_delete = () => undefined;
    await renderBoard();
    await click(q('button[aria-label="Column actions for Backlog"]')!);
    await flush();
    expect(
      menuEntry("Delete column")!.hasAttribute("disabled") ||
        menuEntry("Delete column")!.getAttribute("aria-disabled") === "true",
    ).toBe(true);
    expect(q('[role="menu"]')!.textContent).toContain("Move cards to…");
    // The move targets follow the label (the Role list also names "Review").
    const targets = menuEntries().filter((el) =>
      el.title.startsWith("Move the cards to"),
    );
    expect(targets.map((el) => el.textContent)).toEqual([
      "In Progress",
      "Review",
      "Done",
    ]);
    await click(targets[1]);
    await until(() => commands("board_column_delete").length === 1);
    expect(commands("board_cards_move")).toEqual([
      [
        "board_cards_move",
        { cardIds: ["c1", "c2"], columnId: "review", index: 0, actor: "user" },
      ],
    ]);
    expect(commandNames().indexOf("board_cards_move")).toBeLessThan(
      commandNames().indexOf("board_column_delete"),
    );
    expect(commands("board_column_delete")[0][1]).toEqual({
      columnId: "backlog",
    });

    // Archived cards still block: no move offer then.
    counts = { live: 1, archived: 1 };
    await click(q('button[aria-label="Column actions for In Progress"]')!);
    await flush();
    expect(q('[role="menu"]')!.textContent).not.toContain("Move cards to…");
  });

  it("collapse_keeps_a_wip_limit_written_by_the_other_app", async () => {
    backend.handlers.board_column_update_display = (args) => {
      const patch = args.patch as Partial<BoardColumn>;
      backend.columns = backend.columns.map((c) =>
        c.id === args.columnId ? { ...c, ...patch } : c,
      );
      return backend.columns.find((c) => c.id === args.columnId);
    };
    await renderBoard();
    // The other app sets a WIP limit; this board hasn't reloaded yet.
    backend.columns = backend.columns.map((c) =>
      c.id === "progress" ? { ...c, wipLimit: 5, color: "3fb950" } : c,
    );
    await click(q('button[aria-label="Column actions for In Progress"]')!);
    await click(menuEntry("Collapse column")!);
    await until(() => commands("board_column_update_display").length === 1);
    expect(commands("board_column_update_display")[0][1]).toEqual({
      columnId: "progress",
      patch: { collapsed: true },
    });
    expect(commands("board_column_set_display")).toEqual([]);
    const progress = backend.columns.find((c) => c.id === "progress")!;
    expect(progress).toMatchObject({
      wipLimit: 5,
      color: "3fb950",
      collapsed: true,
    });
  });
});

describe("priority settings (R85)", () => {
  it("failed_priority_write_resets_to_the_saved_scheme", async () => {
    const scheme: PriorityScheme = DEFAULT_PRIORITY_SCHEME;
    const patches: Partial<BoardSettings>[] = [];
    let accept = false;
    const onPatch = vi.fn(async (patch: Partial<BoardSettings>) => {
      patches.push(patch);
      return accept;
    });
    await act(async () =>
      root.render(
        createElement(PrioritySection, {
          scheme,
          stored: false,
          onPatch,
          onError: vi.fn(),
        }),
      ),
    );
    const name = (level: number) =>
      container.querySelector<HTMLInputElement>(
        `[data-settings-priority="${level}"] input[aria-label="Priority name"]`,
      )!;
    act(() => name(1).focus());
    typeInto(name(1), "Minor");
    press(name(1), "Enter");
    await flush();
    await flush();
    expect(patches).toHaveLength(1);
    // The write failed: the field shows the saved name again.
    expect(name(1).value).toBe("Low");
    // And the next edit builds on the saved scheme, not the failed one.
    accept = true;
    typeInto(name(2), "Normal");
    press(name(2), "Enter");
    await flush();
    await flush();
    expect(patches).toHaveLength(2);
    expect(JSON.parse(patches[1].priorityJson!).names).toEqual([
      "None",
      "Low",
      "Normal",
      "High",
      "Urgent",
    ]);
  });
});
