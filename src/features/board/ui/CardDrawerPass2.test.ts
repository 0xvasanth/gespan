// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { copyText } from "../../../platform/tauri/clipboard";
import type { BoardCard } from "../model/board";
import { CardDetail } from "./CardDetail";
import { CardMoreItems } from "./CardMoreMenu";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(() => Promise.resolve()),
  revealItemInDir: vi.fn(() => Promise.resolve()),
}));
vi.mock("../../../platform/tauri/clipboard", () => ({
  copyText: vi.fn(() => Promise.resolve()),
}));

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
const COLUMNS = [
  { id: "backlog", name: "Backlog", cardCount: 2 },
  { id: "progress", name: "In Progress", cardCount: 1 },
];

let container: HTMLDivElement;
let root: Root;
let subtasks: { id: string; cardId: string; title: string; done: boolean; position: number }[];
let comments: unknown[];

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(invoke).mockReset();
  vi.mocked(copyText).mockClear();
  subtasks = [];
  comments = [];
  vi.mocked(invoke).mockImplementation(async (cmd) => {
    if (cmd === "board_subtasks") return subtasks;
    if (cmd === "board_comments") return comments;
    if (cmd === "board_card_duplicate") return { ...CARD, id: "c4", number: 4 };
    return [];
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  document.body.replaceChildren();
  vi.unstubAllGlobals();
});

async function settle() {
  await act(async () => {
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

async function renderDetail(
  overrides: Partial<ComponentProps<typeof CardDetail>> = {},
) {
  const props: ComponentProps<typeof CardDetail> = {
    card: CARD,
    columns: COLUMNS,
    labels: [],
    projects: [],
    link: null,
    linkTargets: [],
    onClose: vi.fn(),
    onOpenLinkedWork: vi.fn(),
    onMove: vi.fn(),
    onDelete: vi.fn(),
    onError: vi.fn(),
    ...overrides,
  };
  await act(async () => root.render(createElement(CardDetail, props)));
  await settle();
  return props;
}

const drawer = () => document.querySelector<HTMLElement>("[data-card-drawer]")!;
const button = (label: string) =>
  document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
const menuItem = (text: string) =>
  [...document.querySelectorAll<HTMLButtonElement>('[role="menu"] button')].find(
    (el) => el.textContent === text,
  )!;
const calls = (cmd: string) =>
  vi.mocked(invoke).mock.calls.filter(([name]) => name === cmd);

function setValue(el: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    "value",
  )!.set!;
  setter.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

describe("Markdown toolbar", () => {
  const field = () =>
    document.querySelector<HTMLTextAreaElement>(
      'textarea[aria-label="Description"]',
    )!;
  const select = (start: number, end: number) => {
    field().focus();
    field().setSelectionRange(start, end);
  };

  it("has the seven buttons in order", async () => {
    await renderDetail();
    const labels = [
      ...drawer().querySelectorAll("[data-markdown-toolbar] button"),
    ].map((el) => el.getAttribute("aria-label"));
    expect(labels).toEqual([
      "Bold",
      "Italic",
      "Heading",
      "Bulleted list",
      "Numbered list",
      "Link",
      "Code",
    ]);
  });

  it("bold wraps the selection and keeps the inside selected", async () => {
    await renderDetail({ card: { ...CARD, body: "make it loud" } });
    select(8, 12);
    await act(async () => button("Bold").click());
    expect(field().value).toBe("make it **loud**");
    expect([field().selectionStart, field().selectionEnd]).toEqual([10, 14]);
  });

  it("with no selection puts the cursor between the markers", async () => {
    await renderDetail({ card: { ...CARD, body: "ab" } });
    select(1, 1);
    await act(async () => button("Italic").click());
    expect(field().value).toBe("a__b");
    expect([field().selectionStart, field().selectionEnd]).toEqual([2, 2]);
  });

  it("bullets a multi-line selection and saves plain Markdown on blur", async () => {
    await renderDetail({ card: { ...CARD, body: "a\nb\nc" } });
    select(0, 5);
    await act(async () => button("Bulleted list").click());
    expect(field().value).toBe("- a\n- b\n- c");
    await act(async () => field().blur());
    expect(calls("board_card_update").at(-1)![1]).toMatchObject({
      body: "- a\n- b\n- c",
    });
  });

  it("link selects the url; code fences several lines; end of text works", async () => {
    await renderDetail({ card: { ...CARD, body: "go site" } });
    select(3, 7);
    await act(async () => button("Link").click());
    expect(field().value).toBe("go [site](url)");
    expect(field().value.slice(field().selectionStart, field().selectionEnd)).toBe(
      "url",
    );
    await act(async () => setValue(field(), "x\na\nb"));
    select(2, 5);
    await act(async () => button("Code").click());
    expect(field().value).toBe("x\n```\na\nb\n```");
  });
});

describe("Subtasks", () => {
  const row = (title: string) =>
    [...drawer().querySelectorAll<HTMLElement>("[data-subtask]")].find((el) =>
      el.textContent?.includes(title),
    )!;
  const seed = () => {
    subtasks = ["one", "two", "three"].map((title, position) => ({
      id: `s${position}`,
      cardId: "c3",
      title,
      done: false,
      position,
    }));
  };

  it("moves a subtask down and up from its menu", async () => {
    seed();
    await renderDetail();
    await act(async () => button("Subtask actions for two").click());
    await act(async () => menuItem("Move down").click());
    expect(calls("board_subtask_move")).toEqual([
      ["board_subtask_move", { subtaskId: "s1", index: 2 }],
    ]);
    await act(async () => button("Subtask actions for two").click());
    await act(async () => menuItem("Move up").click());
    expect(calls("board_subtask_move").at(-1)).toEqual([
      "board_subtask_move",
      { subtaskId: "s1", index: 0 },
    ]);
  });

  it("disables Move up on the first row and Move down on the last", async () => {
    seed();
    await renderDetail();
    await act(async () => button("Subtask actions for one").click());
    expect(menuItem("Move up").disabled).toBe(true);
    expect(menuItem("Move down").disabled).toBe(false);
    await act(async () => menuItem("Move up").click());
    expect(calls("board_subtask_move")).toHaveLength(0);
    await act(async () => document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    await act(async () => button("Subtask actions for three").click());
    expect(menuItem("Move down").disabled).toBe(true);
  });

  it("Alt+Arrow moves the focused row; a plain arrow does nothing", async () => {
    seed();
    await renderDetail();
    const key = (target: HTMLElement, init: KeyboardEventInit) =>
      act(async () => {
        target.dispatchEvent(
          new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }),
        );
      });
    await key(row("two"), { key: "ArrowDown" });
    expect(calls("board_subtask_move")).toHaveLength(0);
    await key(row("two"), { key: "ArrowDown", altKey: true });
    await key(row("two"), { key: "ArrowUp", altKey: true });
    expect(calls("board_subtask_move").map(([, args]) => args)).toEqual([
      { subtaskId: "s1", index: 2 },
      { subtaskId: "s1", index: 0 },
    ]);
    // Past the ends: nothing to move.
    await key(row("one"), { key: "ArrowUp", altKey: true });
    await key(row("three"), { key: "ArrowDown", altKey: true });
    expect(calls("board_subtask_move")).toHaveLength(2);
  });

  it("Convert to card calls the store and reports a failure", async () => {
    seed();
    const props = await renderDetail();
    await act(async () => button("Subtask actions for two").click());
    await act(async () => menuItem("Convert to card").click());
    expect(calls("board_subtask_convert")).toEqual([
      ["board_subtask_convert", { subtaskId: "s1" }],
    ]);
    vi.mocked(invoke).mockImplementation(async (cmd) => {
      if (cmd === "board_subtasks") return subtasks;
      if (cmd === "board_subtask_convert") throw "Subtask not found";
      return [];
    });
    await act(async () => button("Subtask actions for one").click());
    await act(async () => menuItem("Convert to card").click());
    await settle();
    expect(props.onError).toHaveBeenCalledWith(
      "Couldn't convert the subtask on card #3: Subtask not found",
    );
  });
});

describe("Comment authors", () => {
  it("shows You for the user and Agent for an agent, with distinct avatars", async () => {
    comments = [
      { id: "m1", cardId: "c3", body: "mine", author: "user", createdAt: 1, editedAt: null },
      { id: "m2", cardId: "c3", body: "bot says", author: "agent", createdAt: 2, editedAt: null },
    ];
    await renderDetail();
    const authors = [...drawer().querySelectorAll("[data-comment-author]")];
    expect(authors.map((el) => el.textContent)).toEqual(["You", "Agent"]);
    const avatars = [...drawer().querySelectorAll("[data-comment-avatar]")];
    expect(avatars.map((el) => el.textContent)).toEqual(["Y", "A"]);
    expect(avatars[0].className).not.toBe(avatars[1].className);
  });
});

describe("More menu in the drawer", () => {
  const more = (flush: boolean) => (close: () => void, flushDrafts: () => Promise<void>) =>
    createElement(CardMoreItems, {
      card: CARD,
      columns: COLUMNS,
      labels: [],
      labelIds: [],
      priority: 0,
      boards: [],
      close,
      actions: {
        open: vi.fn(),
        setStatus: vi.fn(),
        setPriority: vi.fn(),
        setLabels: vi.fn(),
        createLabel: vi.fn(),
        moveToBoard: vi.fn(),
        duplicate: () => {
          void (flush ? flushDrafts() : Promise.resolve()).then(() =>
            invoke("board_card_duplicate", { cardId: CARD.id }),
          );
        },
        archive: vi.fn(),
        remove: vi.fn(),
        onError: vi.fn(),
      },
    });

  it("opens from the top bar's … button", async () => {
    await renderDetail({ renderMore: more(true) });
    await act(async () => button("More actions").click());
    expect(
      [...document.querySelectorAll('[role="menu"] button')].map((el) => el.textContent),
    ).toEqual([
      "Open",
      "Status…",
      "Priority…",
      "Labels…",
      "Move to board…",
      "Copy link",
      "Copy #3 Ship board",
      "Duplicate",
      "Archive",
      "Delete",
    ]);
  });

  it("flushes an unsaved description before the action runs", async () => {
    await renderDetail({ renderMore: more(true) });
    const field = document.querySelector<HTMLTextAreaElement>(
      'textarea[aria-label="Description"]',
    )!;
    await act(async () => setValue(field, "unsaved words"));
    expect(calls("board_card_update")).toHaveLength(0);
    await act(async () => button("More actions").click());
    await act(async () => menuItem("Duplicate").click());
    await settle();
    const names = vi.mocked(invoke).mock.calls.map(([name]) => name);
    expect(names.indexOf("board_card_update")).toBeGreaterThan(-1);
    expect(names.indexOf("board_card_update")).toBeLessThan(
      names.indexOf("board_card_duplicate"),
    );
    expect(calls("board_card_update")[0][1]).toMatchObject({ body: "unsaved words" });
  });
});
