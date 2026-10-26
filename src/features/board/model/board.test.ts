import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

import {
  boardLinkedEvent,
  boardWriteBack,
  createBoardFromTemplate,
  createField,
  deleteField,
  deleteLabel,
  deleteTemplate,
  fieldDistinctValues,
  getBlockedCards,
  getColumnCardCounts,
  getLatestRunEvents,
  getBoardSettings,
  getRunningCards,
  getRunPrompt,
  getSessionStatusSetting,
  listBoardLinks,
  listTemplates,
  mergeLabels,
  moveField,
  moveLabel,
  renameFieldOption,
  saveBoardAsTemplate,
  setSessionStatusSetting,
  updateBoardSettings,
  updateField,
  updateLabel,
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
  updateColumnDisplay,
  unarchiveCard,
  setCardDue,
  setCardLabels,
  setCardPriority,
  setCardProjects,
  subscribeBoard,
  createView,
  deleteView,
  getCardIndex,
  listViews,
  moveView,
  queryBoard,
  setCardsLabel,
  setCardsPriority,
  updateView,
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
      "updateColumnDisplay collapsing only",
      () => updateColumnDisplay("col1", { collapsed: true }),
      "board_column_update_display",
      { columnId: "col1", patch: { collapsed: true } },
    ],
    [
      "updateColumnDisplay clearing the limit",
      () => updateColumnDisplay("col1", { wipLimit: null, color: "e5534b" }),
      "board_column_update_display",
      { columnId: "col1", patch: { wipLimit: null, color: "e5534b" } },
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
    [
      "queryBoard",
      () =>
        queryBoard(
          "b1",
          { query: "x", rows: [], sort: { key: "manual", dir: "asc" } },
          ["c1"],
        ),
      "board_query",
      {
        boardId: "b1",
        spec: { query: "x", rows: [], sort: { key: "manual", dir: "asc" } },
        running: ["c1"],
      },
    ],
    [
      "queryBoard defaults running to empty",
      () =>
        queryBoard("b1", {
          query: "",
          rows: [],
          sort: { key: "manual", dir: "asc" },
        }),
      "board_query",
      {
        boardId: "b1",
        spec: { query: "", rows: [], sort: { key: "manual", dir: "asc" } },
        running: [],
      },
    ],
    [
      "getCardIndex",
      () => getCardIndex("b1"),
      "board_card_index",
      { boardId: "b1" },
    ],
    ["listViews", () => listViews("b1"), "board_views", { boardId: "b1" }],
    [
      "createView",
      () => createView("b1", "Mine", "{}", "{}"),
      "board_view_create",
      { boardId: "b1", name: "Mine", specJson: "{}", layoutJson: "{}" },
    ],
    [
      "updateView sends null for parts left out",
      () => updateView("v1", { name: "New" }),
      "board_view_update",
      { viewId: "v1", name: "New", specJson: null, layoutJson: null },
    ],
    [
      "deleteView",
      () => deleteView("v1"),
      "board_view_delete",
      { viewId: "v1" },
    ],
    [
      "moveView",
      () => moveView("v1", 2),
      "board_view_move",
      { viewId: "v1", index: 2 },
    ],
    [
      "setCardsPriority",
      () => setCardsPriority(["c1", "c2"], 3),
      "board_cards_set_priority",
      { cardIds: ["c1", "c2"], priority: 3 },
    ],
    [
      "setCardsLabel",
      () => setCardsLabel(["c1"], "l1", false),
      "board_cards_set_label",
      { cardIds: ["c1"], labelId: "l1", on: false },
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

describe("C5 settings, template, label, field and agent wrappers", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
  });

  it.each([
    [
      "getBoardSettings",
      () => getBoardSettings("b1"),
      "board_settings",
      { boardId: "b1" },
    ],
    [
      "updateBoardSettings",
      () => updateBoardSettings("b1", { writeBack: false, icon: null }),
      "board_settings_update",
      { boardId: "b1", patch: { writeBack: false, icon: null } },
    ],
    ["listTemplates", () => listTemplates(), "board_templates", undefined],
    [
      "createBoardFromTemplate",
      () => createBoardFromTemplate("Bugs", "bug-triage"),
      "board_create_from_template",
      { name: "Bugs", templateId: "bug-triage" },
    ],
    [
      "saveBoardAsTemplate",
      () => saveBoardAsTemplate("b1", "Mine"),
      "board_save_as_template",
      { boardId: "b1", name: "Mine" },
    ],
    [
      "deleteTemplate",
      () => deleteTemplate("t1"),
      "board_template_delete",
      { templateId: "t1" },
    ],
    [
      "updateLabel sends null for parts left out",
      () => updateLabel("l1", { color: "e5534b" }),
      "board_label_update",
      { labelId: "l1", name: null, color: "e5534b" },
    ],
    [
      "moveLabel",
      () => moveLabel("l1", 2),
      "board_label_move",
      { labelId: "l1", index: 2 },
    ],
    [
      "deleteLabel",
      () => deleteLabel("l1"),
      "board_label_delete",
      { labelId: "l1" },
    ],
    [
      "mergeLabels",
      () => mergeLabels("l1", "l2"),
      "board_labels_merge",
      { fromId: "l1", intoId: "l2" },
    ],
    [
      "createField",
      () => createField("b1", "Size", "multi-select", '["S","L"]'),
      "board_field_create",
      {
        boardId: "b1",
        name: "Size",
        kind: "multi-select",
        optionsJson: '["S","L"]',
      },
    ],
    [
      "createField without options",
      () => createField("b1", "Link", "url"),
      "board_field_create",
      { boardId: "b1", name: "Link", kind: "url", optionsJson: null },
    ],
    [
      "updateField sends null for parts left out",
      () => updateField("f1", { name: "T-shirt" }),
      "board_field_update",
      { fieldId: "f1", name: "T-shirt", optionsJson: null },
    ],
    [
      "renameFieldOption",
      () => renameFieldOption("f1", "S", "Small"),
      "board_field_rename_option",
      { fieldId: "f1", from: "S", to: "Small" },
    ],
    [
      "moveField",
      () => moveField("f1", 0),
      "board_field_move",
      { fieldId: "f1", index: 0 },
    ],
    [
      "deleteField",
      () => deleteField("f1"),
      "board_field_delete",
      { fieldId: "f1" },
    ],
    [
      "fieldDistinctValues",
      () => fieldDistinctValues("b1", "f1", 20),
      "board_field_distinct_values",
      { boardId: "b1", fieldId: "f1", limit: 20 },
    ],
    [
      "boardLinkedEvent",
      () => boardLinkedEvent("session", "s1", "pr_merged"),
      "board_linked_event",
      { kind: "session", id: "s1", event: "pr_merged" },
    ],
    [
      "boardWriteBack",
      () => boardWriteBack("remote:env:s1", "Done."),
      "board_write_back",
      { sessionKey: "remote:env:s1", summary: "Done." },
    ],
    [
      "getBlockedCards",
      () => getBlockedCards("b1"),
      "board_blocked",
      { boardId: "b1" },
    ],
    [
      "listBoardLinks",
      () => listBoardLinks("b1"),
      "board_links",
      { boardId: "b1" },
    ],
    [
      "getRunningCards",
      () =>
        getRunningCards("b1", [
          ["session", "s1"],
          ["pilot", "p1"],
        ]),
      "board_running_cards",
      {
        boardId: "b1",
        running: [
          ["session", "s1"],
          ["pilot", "p1"],
        ],
      },
    ],
    [
      "getRunPrompt",
      () => getRunPrompt("c1"),
      "board_run_prompt",
      { cardId: "c1" },
    ],
    [
      "getSessionStatusSetting",
      () => getSessionStatusSetting("s1"),
      "board_session_status_setting",
      { sessionKey: "s1" },
    ],
    [
      "setSessionStatusSetting clears with null",
      () => setSessionStatusSetting("s1", null),
      "board_session_status_setting_set",
      { sessionKey: "s1", mayChange: null },
    ],
  ])(
    "%s invokes the command with camelCase args",
    async (_n, call, command, args) => {
      await call();
      if (args === undefined) expect(invoke).toHaveBeenCalledWith(command);
      else expect(invoke).toHaveBeenCalledWith(command, args);
    },
  );

  it("getSessionStatusSetting passes a failed read on, never as null", async () => {
    invoke.mockRejectedValueOnce("no such table: board_session_settings");
    await expect(getSessionStatusSetting("s1")).rejects.toBe(
      "no such table: board_session_settings",
    );
    invoke.mockResolvedValueOnce(null);
    await expect(getSessionStatusSetting("s1")).resolves.toBeNull();
    invoke.mockResolvedValueOnce(false);
    await expect(getSessionStatusSetting("s1")).resolves.toBe(false);
  });

  it("getLatestRunEvents and getColumnCardCounts call their commands", async () => {
    invoke.mockResolvedValueOnce({ c1: "failed", c2: "started" });
    await expect(getLatestRunEvents("b1")).resolves.toEqual({
      c1: "failed",
      c2: "started",
    });
    expect(invoke).toHaveBeenLastCalledWith("board_latest_run_events", {
      boardId: "b1",
    });
    invoke.mockResolvedValueOnce({ col1: [248, 2], col2: [0, 0] });
    await expect(getColumnCardCounts("b1")).resolves.toEqual({
      col1: [248, 2],
      col2: [0, 0],
    });
    expect(invoke).toHaveBeenLastCalledWith("board_column_card_counts", {
      boardId: "b1",
    });
    invoke.mockRejectedValueOnce("Board not found");
    await expect(getColumnCardCounts("gone")).rejects.toBe("Board not found");
  });

  it("leaves settings the patch does not name out of the payload", async () => {
    await updateBoardSettings("b1", { runWith: true });
    const [, payload] = invoke.mock.calls[0];
    expect(Object.keys(payload.patch)).toEqual(["runWith"]);
  });
});
