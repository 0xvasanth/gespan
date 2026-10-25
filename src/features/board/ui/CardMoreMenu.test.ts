// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { copyText } from "../../../platform/tauri/clipboard";
import type { Board, BoardCard } from "../model/board";
import { CardMoreItems, copyLabel, type MoreActions } from "./CardMoreMenu";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));
vi.mock("../../../platform/tauri/clipboard", () => ({
  copyText: vi.fn(() => Promise.resolve()),
}));

const CARD: BoardCard = {
  id: "c7",
  boardId: "b1",
  columnId: "todo",
  number: 7,
  title: "Fix the bug",
  body: "",
  position: 0,
  linkKind: null,
  linkId: null,
  createdAt: 1,
  updatedAt: 1,
};
const board = (id: string, name: string): Board => ({
  id,
  name,
  createdAt: 1,
  updatedAt: 1,
});
const COLUMNS = [
  { id: "todo", name: "Todo", cardCount: 2 },
  { id: "done", name: "Done", cardCount: 0 },
];

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(invoke).mockReset();
  vi.mocked(copyText).mockClear();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

function actions(): MoreActions {
  return {
    open: vi.fn(),
    setStatus: vi.fn(),
    setPriority: vi.fn(),
    setLabels: vi.fn(),
    createLabel: vi.fn(),
    moveToBoard: vi.fn(),
    duplicate: vi.fn(),
    archive: vi.fn(),
    remove: vi.fn(),
    onError: vi.fn(),
  };
}

async function render(
  card: BoardCard = CARD,
  boards: Board[] = [board("b1", "Main"), board("b2", "Side")],
  a: MoreActions = actions(),
) {
  const close = vi.fn();
  await act(async () =>
    root.render(
      createElement(CardMoreItems, {
        card,
        columns: COLUMNS,
        labels: [
          { id: "l1", boardId: "b1", name: "Bug", color: "e5484d", position: 0 },
        ],
        labelIds: [],
        priority: 0,
        boards,
        actions: a,
        close,
      }),
    ),
  );
  return { a, close };
}
const items = () =>
  [...container.querySelectorAll<HTMLButtonElement>("button")].map(
    (b) => b.textContent,
  );
const item = (text: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (b) => b.textContent === text,
  )!;

describe("More menu", () => {
  it("lists the items in order with the real number and title", async () => {
    await render();
    expect(items()).toEqual([
      "Open",
      "Status…",
      "Priority…",
      "Labels…",
      "Move to board…",
      "Copy link",
      "Copy #7 Fix the bug",
      "Duplicate",
      "Archive",
      "Delete",
    ]);
  });

  it("cuts a long title to 40 characters in the row only", async () => {
    const title = "x".repeat(60);
    expect(copyLabel(1, title)).toBe(`Copy #1 ${"x".repeat(40)}…`);
    expect(copyLabel(2, "  short  ")).toBe("Copy #2 short");
    await render({ ...CARD, title });
    expect(items()[6]).toBe(`Copy #7 ${"x".repeat(40)}…`);
    await act(async () => item(`Copy #7 ${"x".repeat(40)}…`).click());
    expect(copyText).toHaveBeenCalledWith(`#7 ${title}`);
  });

  it("copies the link and the reference", async () => {
    const { close } = await render();
    await act(async () => item("Copy link").click());
    expect(copyText).toHaveBeenLastCalledWith("gespan://board/b1/card/c7");
    await act(async () => item("Copy #7 Fix the bug").click());
    expect(copyText).toHaveBeenLastCalledWith("#7 Fix the bug");
    expect(close).toHaveBeenCalledTimes(2);
  });

  it("reports a failed copy", async () => {
    vi.mocked(copyText).mockRejectedValueOnce(new Error("denied"));
    const { a } = await render();
    await act(async () => item("Copy link").click());
    expect(a.onError).toHaveBeenCalledWith(
      "Couldn't copy the link of card #7: denied",
    );
  });

  it("Open, Duplicate, Archive and Delete run their action and close", async () => {
    const { a, close } = await render();
    for (const [label, fn] of [
      ["Open", a.open],
      ["Duplicate", a.duplicate],
      ["Archive", a.archive],
      ["Delete", a.remove],
    ] as const) {
      await act(async () => item(label).click());
      expect(fn).toHaveBeenCalledTimes(1);
    }
    expect(close).toHaveBeenCalledTimes(4);
  });

  it("Status… opens the columns, and choosing one moves the card there", async () => {
    const { a } = await render();
    await act(async () => item("Status…").click());
    expect(items()).toContain("Todo");
    await act(async () => item("Done").click());
    expect(a.setStatus).toHaveBeenCalledWith("done", 0);
  });

  it("Priority… and Labels… open the existing menus", async () => {
    const { a } = await render();
    await act(async () => item("Priority…").click());
    expect(items()).toContain("Urgent");
    await act(async () => item("Urgent").click());
    expect(a.setPriority).toHaveBeenCalledWith(4);
  });

  it("Labels… toggles a label on the card", async () => {
    const { a } = await render();
    await act(async () => item("Labels…").click());
    await act(async () => item("Bug").click());
    expect(a.setLabels).toHaveBeenCalledWith(["l1"]);
  });

  it("Move to board… lists only the other boards", async () => {
    const { a } = await render();
    await act(async () => item("Move to board…").click());
    expect(items()).not.toContain("Main");
    await act(async () => item("Side").click());
    expect(a.moveToBoard).toHaveBeenCalledWith("b2");
  });

  it("Move to board… says when there is no other board", async () => {
    const { a } = await render(CARD, [board("b1", "Main")]);
    await act(async () => item("Move to board…").click());
    const none = item("No other boards");
    expect(none.disabled).toBe(true);
    await act(async () => none.click());
    expect(a.moveToBoard).not.toHaveBeenCalled();
  });
});
