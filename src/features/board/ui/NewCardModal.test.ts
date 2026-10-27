// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import type { BoardColumn, BoardLabel } from "../model/board";
import { NewCardModal, defaultColumnId } from "./NewCardModal";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));

function column(
  id: string,
  name: string,
  position: number,
  role: BoardColumn["role"],
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

const COLUMNS = [
  column("progress", "In Progress", 1, "doing"),
  column("backlog", "Backlog", 0, "todo"),
  column("done", "Done", 2, "done"),
];
const LABELS: BoardLabel[] = [
  { id: "l1", boardId: "b1", name: "bug", color: "#f00" },
  { id: "l2", boardId: "b1", name: "ui", color: "#0f0" },
];
const PROJECTS = [{ path: "/p/app", name: "app" }];

type Handler = (args: Record<string, unknown>) => unknown;
let handlers: Record<string, Handler>;
let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  handlers = {
    board_card_create: (args) => ({ id: "new", title: args.title }),
    board_card_set_priority: () => undefined,
    board_card_set_due: () => undefined,
    board_card_set_labels: () => undefined,
    board_card_set_projects: () => undefined,
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
});

async function flush() {
  await act(async () => {
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

async function renderModal(
  overrides: Partial<ComponentProps<typeof NewCardModal>> = {},
) {
  const props: ComponentProps<typeof NewCardModal> = {
    boardId: "b1",
    columns: COLUMNS,
    labels: LABELS,
    projects: PROJECTS,
    onClose: vi.fn(),
    onNotice: vi.fn(),
    ...overrides,
  };
  await act(async () => {
    root.render(createElement(NewCardModal, props));
  });
  await flush();
  return props;
}

const calls = (cmd: string) =>
  vi.mocked(invoke).mock.calls.filter(([name]) => name === cmd);
const field = <T extends HTMLElement>(label: string) =>
  document.querySelector<T>(`[aria-label="${label}"]`)!;

function type(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const proto =
    el instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, "value")!.set!;
  act(() => {
    setter.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function press(target: Element, key: string, init: KeyboardEventInit = {}) {
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

function click(el: Element) {
  act(() => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function pick(trigger: string, item: string) {
  click(field(trigger));
  const row = [...document.querySelectorAll('[role^="menuitem"]')].find(
    (el) => el.textContent?.trim() === item,
  )!;
  click(row);
}

const button = (name: string) =>
  [...document.querySelectorAll("button")].find(
    (el) => el.textContent?.trim() === name,
  )!;

describe("defaultColumnId", () => {
  it("is the first todo column by position, else the first column", () => {
    expect(defaultColumnId(COLUMNS)).toBe("backlog");
    expect(
      defaultColumnId([column("b", "B", 1, "doing"), column("a", "A", 0, null)]),
    ).toBe("a");
    expect(defaultColumnId([])).toBeNull();
  });
});

describe("NewCardModal", () => {
  it("starts on the todo column with the title focused", async () => {
    await renderModal();
    expect(document.activeElement).toBe(field("Title"));
    expect(field("Column").textContent).toBe("Backlog");
    expect(field("Priority").textContent).toBe("None");
    expect(field("Due").textContent).toBe("None");
  });

  it("requires a title and writes nothing without one", async () => {
    const props = await renderModal();
    type(field("Title"), "   ");
    press(field("Title"), "Enter", { metaKey: true });
    await flush();
    expect(calls("board_card_create")).toHaveLength(0);
    expect(document.body.textContent).toContain("Add a title");
    expect(props.onClose).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(field("Title"));
    type(field("Title"), "x");
    expect(document.body.textContent).not.toContain("Add a title");
  });

  it("Cmd+Enter makes the card with every field set, then closes", async () => {
    const props = await renderModal();
    type(field("Title"), "  Ship it ");
    type(field("Description"), "Details\n\n");
    pick("Column", "Done");
    pick("Priority", "High");
    pick("Due", "Tomorrow");
    pick("Labels", "bug");
    pick("Projects", "app");
    press(field("Title"), "Enter", { metaKey: true });
    await flush();
    expect(calls("board_card_create")[0][1]).toEqual({
      columnId: "done",
      title: "Ship it",
      body: "Details",
    });
    expect(calls("board_card_set_priority")[0][1]).toEqual({
      cardId: "new",
      priority: 3,
    });
    const due = calls("board_card_set_due")[0][1] as { dueAt: number };
    expect(new Date(due.dueAt).getHours()).toBe(0);
    expect(calls("board_card_set_labels")[0][1]).toEqual({
      cardId: "new",
      labelIds: ["l1"],
    });
    expect(calls("board_card_set_projects")[0][1]).toEqual({
      cardId: "new",
      projects: ["/p/app"],
    });
    expect(props.onClose).toHaveBeenCalledTimes(1);
    expect(props.onNotice).not.toHaveBeenCalled();
  });

  it("skips setters for fields left at their defaults", async () => {
    await renderModal();
    type(field("Title"), "Plain");
    click(button("Create"));
    await flush();
    expect(calls("board_card_create")).toHaveLength(1);
    for (const cmd of [
      "board_card_set_priority",
      "board_card_set_due",
      "board_card_set_labels",
      "board_card_set_projects",
    ]) {
      expect(calls(cmd)).toHaveLength(0);
    }
  });

  it("keeps the card when a setter fails, and says which field", async () => {
    handlers.board_card_set_priority = () => {
      throw new Error("priority is locked");
    };
    const props = await renderModal();
    type(field("Title"), "Half");
    pick("Priority", "Low");
    pick("Projects", "app");
    click(button("Create"));
    await flush();
    // The later setters still run, and the card stays made.
    expect(calls("board_card_set_projects")).toHaveLength(1);
    expect(props.onClose).toHaveBeenCalledTimes(1);
    expect(props.onNotice).toHaveBeenCalledWith(
      expect.stringContaining("Card created, but not saved: priority: "),
    );
    expect(props.onNotice).toHaveBeenCalledWith(
      expect.stringContaining("priority is locked"),
    );
  });

  it("stays open and shows the error when the card can't be created", async () => {
    handlers.board_card_create = () => {
      throw new Error("column is gone");
    };
    const props = await renderModal();
    type(field("Title"), "Doomed");
    click(button("Create"));
    await flush();
    expect(props.onClose).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("column is gone");
    expect(field<HTMLInputElement>("Title").value).toBe("Doomed");
    // Fixing nothing and trying again is allowed.
    handlers.board_card_create = (args) => ({ id: "n", title: args.title });
    click(button("Create"));
    await flush();
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });

  it("Create more clears title and description, keeps the rest, refocuses the title", async () => {
    const props = await renderModal();
    click(field("Create more"));
    type(field("Title"), "One");
    type(field("Description"), "Body");
    pick("Column", "In Progress");
    pick("Priority", "Urgent");
    pick("Labels", "ui");
    pick("Projects", "app");
    field<HTMLButtonElement>("Priority").focus();
    press(field("Priority"), "Enter", { ctrlKey: true });
    await flush();
    expect(props.onClose).not.toHaveBeenCalled();
    expect(field<HTMLInputElement>("Title").value).toBe("");
    expect(field<HTMLTextAreaElement>("Description").value).toBe("");
    expect(document.activeElement).toBe(field("Title"));
    expect(field("Column").textContent).toBe("In Progress");
    expect(field("Priority").textContent).toBe("Urgent");
    expect(field("Labels").textContent).toBe("ui");
    expect(field("Projects").textContent).toBe("app");
    type(field("Title"), "Two");
    click(button("Create"));
    await flush();
    const second = calls("board_card_create")[1][1] as Record<string, unknown>;
    expect(second).toMatchObject({ columnId: "progress", title: "Two" });
    expect(calls("board_card_set_labels")[1][1]).toEqual({
      cardId: "new",
      labelIds: ["l2"],
    });
  });

  it("Create more keeps a failed-setter message in the modal", async () => {
    handlers.board_card_set_labels = () => {
      throw new Error("no such label");
    };
    const props = await renderModal();
    click(field("Create more"));
    type(field("Title"), "A");
    pick("Labels", "bug");
    click(button("Create"));
    await flush();
    expect(props.onClose).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("labels: no such label");
    expect(field<HTMLInputElement>("Title").value).toBe("");
  });

  it("a rapid double Cmd+Enter creates one card", async () => {
    let release!: () => void;
    handlers.board_card_create = (args) =>
      new Promise((resolve) => {
        release = () => resolve({ id: "new", title: args.title });
      });
    const props = await renderModal();
    type(field("Title"), "Once");
    press(field("Title"), "Enter", { metaKey: true });
    press(field("Title"), "Enter", { metaKey: true });
    await act(async () => {
      release();
      await Promise.resolve();
    });
    await flush();
    expect(calls("board_card_create")).toHaveLength(1);
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });

  it("a rapid double Cmd+Enter with Create more makes one card, not two", async () => {
    let release!: () => void;
    handlers.board_card_create = (args) =>
      new Promise((resolve) => {
        release = () => resolve({ id: "new", title: args.title });
      });
    await renderModal();
    click(field("Create more"));
    type(field("Title"), "Once");
    press(field("Title"), "Enter", { metaKey: true });
    press(field("Title"), "Enter", { metaKey: true });
    await act(async () => {
      release();
      await Promise.resolve();
    });
    await flush();
    expect(calls("board_card_create")).toHaveLength(1);
  });

  it("Enter in the title submits; Enter in the description does not", async () => {
    await renderModal();
    type(field("Title"), "Quick");
    press(field("Description"), "Enter");
    await flush();
    expect(calls("board_card_create")).toHaveLength(0);
    press(field("Title"), "Enter");
    await flush();
    expect(calls("board_card_create")).toHaveLength(1);
  });

  it("Escape closes", async () => {
    const props = await renderModal();
    press(field("Title"), "Escape");
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });

  it("Tab order follows the DOM: title, description, the five pills, Create more, Create", async () => {
    await renderModal();
    const order = [
      ...document.querySelectorAll<HTMLElement>(
        "input, textarea, button",
      ),
    ].map((el) => el.getAttribute("aria-label") ?? el.textContent?.trim());
    expect(order).toEqual([
      "Title",
      "Description",
      "Column",
      "Priority",
      "Due",
      "Labels",
      "Projects",
      "Create more",
      "Create",
    ]);
  });

  it("Down opens a pill menu", async () => {
    await renderModal();
    expect(document.querySelector('[role="menu"]')).toBeNull();
    press(field("Priority"), "ArrowDown");
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
  });

  it("the Create more checkbox is a real checkbox that Space toggles", async () => {
    await renderModal();
    const box = field<HTMLInputElement>("Create more");
    expect(box.type).toBe("checkbox");
    expect(box.checked).toBe(false);
    click(box);
    expect(box.checked).toBe(true);
  });
});
