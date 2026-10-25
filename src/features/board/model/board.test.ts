import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

import {
  addComment,
  addSubtask,
  BOARD_CHANGED,
  createLabel,
  deleteSubtask,
  getCardExtras,
  linkCard,
  listComments,
  listLabels,
  listSubtasks,
  PRIORITIES,
  setCardDue,
  setCardLabels,
  setCardPriority,
  setCardProjects,
  subscribeBoard,
  toggleSubtask,
} from "./board";

describe("subscribeBoard", () => {
  let handler: (event: { payload: { boardId: string } }) => void;
  const stop = vi.fn();

  beforeEach(() => {
    stop.mockReset();
    listen.mockReset();
    listen.mockImplementation(async (_name: string, cb: typeof handler) => {
      handler = cb;
      return stop;
    });
  });

  it("only fires for changes to its own board", async () => {
    const onChange = vi.fn();
    subscribeBoard("b1", onChange);
    await Promise.resolve();

    expect(listen).toHaveBeenCalledWith(BOARD_CHANGED, expect.any(Function));
    handler({ payload: { boardId: "b2" } });
    expect(onChange).not.toHaveBeenCalled();
    handler({ payload: { boardId: "b1" } });
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("stops listening on unsubscribe", async () => {
    const unsubscribe = subscribeBoard("b1", vi.fn());
    await Promise.resolve();
    await Promise.resolve();
    unsubscribe();
    expect(stop).toHaveBeenCalledTimes(1);
  });

  it("stops listening when unsubscribed before listen resolves", async () => {
    const unsubscribe = subscribeBoard("b1", vi.fn());
    unsubscribe();
    await Promise.resolve();
    await Promise.resolve();
    expect(stop).toHaveBeenCalledTimes(1);
  });
});

describe("invoke wrappers", () => {
  beforeEach(() => invoke.mockReset());

  it("sends null link fields to clear a card link", async () => {
    await linkCard("c1", null);
    expect(invoke).toHaveBeenCalledWith("board_card_link", {
      cardId: "c1",
      linkKind: null,
      linkId: null,
    });
  });

  it("sends the link kind and id to link a card", async () => {
    await linkCard("c1", { kind: "pilot", id: "p1" });
    expect(invoke).toHaveBeenCalledWith("board_card_link", {
      cardId: "c1",
      linkKind: "pilot",
      linkId: "p1",
    });
  });
});

describe("card extras wrappers", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
  });

  it.each([
    [
      "getCardExtras",
      () => getCardExtras("b1"),
      "board_card_extras",
      { boardId: "b1" },
    ],
    [
      "setCardPriority",
      () => setCardPriority("c1", 3),
      "board_card_set_priority",
      { cardId: "c1", priority: 3 },
    ],
    [
      "setCardDue",
      () => setCardDue("c1", 5),
      "board_card_set_due",
      { cardId: "c1", dueAt: 5 },
    ],
    [
      "setCardDue clear",
      () => setCardDue("c1", null),
      "board_card_set_due",
      { cardId: "c1", dueAt: null },
    ],
    [
      "setCardProjects",
      () => setCardProjects("c1", ["/a"]),
      "board_card_set_projects",
      { cardId: "c1", projects: ["/a"] },
    ],
    ["listLabels", () => listLabels("b1"), "board_labels", { boardId: "b1" }],
    [
      "createLabel",
      () => createLabel("b1", "Bug", "ff0000"),
      "board_label_create",
      { boardId: "b1", name: "Bug", color: "ff0000" },
    ],
    [
      "setCardLabels",
      () => setCardLabels("c1", ["l1"]),
      "board_card_set_labels",
      { cardId: "c1", labelIds: ["l1"] },
    ],
    [
      "listSubtasks",
      () => listSubtasks("c1"),
      "board_subtasks",
      { cardId: "c1" },
    ],
    [
      "addSubtask",
      () => addSubtask("c1", "t"),
      "board_subtask_add",
      { cardId: "c1", title: "t" },
    ],
    [
      "toggleSubtask",
      () => toggleSubtask("s1", true),
      "board_subtask_toggle",
      { subtaskId: "s1", done: true },
    ],
    [
      "deleteSubtask",
      () => deleteSubtask("s1"),
      "board_subtask_delete",
      { subtaskId: "s1" },
    ],
    [
      "listComments",
      () => listComments("c1"),
      "board_comments",
      { cardId: "c1" },
    ],
    [
      "addComment",
      () => addComment("c1", "hi"),
      "board_comment_add",
      { cardId: "c1", body: "hi" },
    ],
  ])(
    "%s invokes the command with camelCase args",
    async (_n, call, command, args) => {
      await call();
      expect(invoke).toHaveBeenCalledWith(command, args);
    },
  );

  it("exposes the priority names in store order", () => {
    expect([...PRIORITIES]).toEqual([
      "None",
      "Low",
      "Medium",
      "High",
      "Urgent",
    ]);
  });
});
