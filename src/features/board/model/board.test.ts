import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

import {
  addComment,
  addCardLink,
  addRelation,
  convertSubtask,
  duplicateCard,
  editComment,
  listCardLinks,
  listCardRelations,
  listFieldValues,
  listFields,
  moveCardToBoard,
  moveSubtask,
  removeCardLink,
  removeRelation,
  getCardMeta,
  setCardEstimate,
  setCardStart,
  setFieldValue,
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
  archiveCard,
  archiveCards,
  columnCardCount,
  deleteArchivedCard,
  listArchivedCards,
  moveCards,
  restoreCard,
  setColumnDisplay,
  unarchiveCard,
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
      "archiveCard",
      () => archiveCard("c1"),
      "board_card_archive",
      { cardId: "c1" },
    ],
    [
      "archiveCards",
      () => archiveCards(["c1", "c2"]),
      "board_cards_archive",
      { cardIds: ["c1", "c2"] },
    ],
    [
      "unarchiveCard",
      () => unarchiveCard("c1"),
      "board_card_unarchive",
      { cardId: "c1" },
    ],
    [
      "restoreCard",
      () => restoreCard("c1", "col1", 2),
      "board_card_restore",
      { cardId: "c1", columnId: "col1", index: 2 },
    ],
    [
      "deleteArchivedCard",
      () => deleteArchivedCard("c1"),
      "board_card_delete_archived",
      { cardId: "c1" },
    ],
    [
      "listArchivedCards",
      () => listArchivedCards("b1"),
      "board_archived_cards",
      { boardId: "b1" },
    ],
    [
      "moveCards",
      () => moveCards(["c1", "c2"], "col1", 0),
      "board_cards_move",
      { cardIds: ["c1", "c2"], columnId: "col1", index: 0, actor: "user" },
    ],
    [
      "moveCards as system",
      () => moveCards(["c1"], "col1", 3, "system"),
      "board_cards_move",
      { cardIds: ["c1"], columnId: "col1", index: 3, actor: "system" },
    ],
    [
      "setColumnDisplay",
      () => setColumnDisplay("col1", null, 5, true),
      "board_column_set_display",
      { columnId: "col1", color: null, wipLimit: 5, collapsed: true },
    ],
    [
      "columnCardCount",
      () => columnCardCount("col1"),
      "board_column_card_count",
      { columnId: "col1" },
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
    [
      "getCardMeta",
      () => getCardMeta("c1"),
      "board_card_meta",
      { cardId: "c1" },
    ],
    [
      "setCardStart",
      () => setCardStart("c1", 5),
      "board_card_set_start",
      { cardId: "c1", startAt: 5 },
    ],
    [
      "setCardStart clear",
      () => setCardStart("c1", null),
      "board_card_set_start",
      { cardId: "c1", startAt: null },
    ],
    [
      "setCardEstimate",
      () => setCardEstimate("c1", 90),
      "board_card_set_estimate",
      { cardId: "c1", minutes: 90 },
    ],
    [
      "addRelation",
      () => addRelation("c1", "c2", "blocks"),
      "board_relation_add",
      { cardId: "c1", relatedCardId: "c2", kind: "blocks" },
    ],
    [
      "removeRelation",
      () => removeRelation("c1", "c2", "relates"),
      "board_relation_remove",
      { cardId: "c1", relatedCardId: "c2", kind: "relates" },
    ],
    [
      "listCardRelations",
      () => listCardRelations("c1"),
      "board_card_relations",
      { cardId: "c1" },
    ],
    ["listFields", () => listFields("b1"), "board_fields", { boardId: "b1" }],
    [
      "setFieldValue",
      () => setFieldValue("c1", "f1", "1"),
      "board_field_value_set",
      { cardId: "c1", fieldId: "f1", valueJson: "1" },
    ],
    [
      "setFieldValue clear",
      () => setFieldValue("c1", "f1", null),
      "board_field_value_set",
      { cardId: "c1", fieldId: "f1", valueJson: null },
    ],
    [
      "listFieldValues",
      () => listFieldValues("c1"),
      "board_card_field_values",
      { cardId: "c1" },
    ],
    [
      "addCardLink",
      () => addCardLink("c1", "pr", "u", null),
      "board_link_add",
      { cardId: "c1", kind: "pr", reference: "u", role: null },
    ],
    [
      "removeCardLink",
      () => removeCardLink("c1", "pr", "u"),
      "board_link_remove",
      { cardId: "c1", kind: "pr", reference: "u" },
    ],
    [
      "listCardLinks",
      () => listCardLinks("c1"),
      "board_card_links",
      { cardId: "c1" },
    ],
    [
      "editComment",
      () => editComment("m1", "x"),
      "board_comment_edit",
      { commentId: "m1", body: "x" },
    ],
    [
      "moveSubtask",
      () => moveSubtask("s1", 2),
      "board_subtask_move",
      { subtaskId: "s1", index: 2 },
    ],
    [
      "convertSubtask",
      () => convertSubtask("s1"),
      "board_subtask_convert",
      { subtaskId: "s1" },
    ],
    [
      "duplicateCard",
      () => duplicateCard("c1"),
      "board_card_duplicate",
      { cardId: "c1" },
    ],
    [
      "moveCardToBoard",
      () => moveCardToBoard("c1", "b2"),
      "board_card_move_to_board",
      { cardId: "c1", boardId: "b2" },
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
