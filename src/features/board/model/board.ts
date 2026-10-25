import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { TicketRef } from "../../sessions/model/session";

export const BOARD_CHANGED = "gespan:board-changed";

export type ColumnRole = "todo" | "doing" | "review" | "done";
export type CardLinkKind = "session" | "pilot" | "flow-run";
export type LinkedWorkEvent = "started" | "completed" | "failed";
export type BoardActor = "user" | "system";

export type Board = {
  id: string;
  name: string;
  createdAt: number;
  updatedAt: number;
};

export type BoardColumn = {
  id: string;
  boardId: string;
  name: string;
  position: number;
  role: ColumnRole | null;
  color: string | null;
  wipLimit: number | null;
  collapsed: boolean;
};

export type BoardCard = {
  id: string;
  boardId: string;
  columnId: string;
  number: number;
  title: string;
  body: string;
  position: number;
  linkKind: CardLinkKind | null;
  linkId: string | null;
  createdAt: number;
  updatedAt: number;
};

export type BoardDetail = {
  board: Board;
  columns: BoardColumn[];
  cards: BoardCard[];
};

export type BoardActivityKind =
  | "created"
  | "edited"
  | "moved"
  | "linked"
  | "unlinked"
  | "deleted"
  | LinkedWorkEvent;

/** Activity rows store column names (not ids) in fromColumn/toColumn. */
export type BoardActivity = {
  id: string;
  boardId: string;
  cardId: string | null;
  kind: BoardActivityKind;
  actor: BoardActor;
  fromColumn: string | null;
  toColumn: string | null;
  detail: string | null;
  createdAt: number;
};

export function listBoards(): Promise<Board[]> {
  return invoke<Board[]>("board_list");
}

export function getBoard(boardId: string): Promise<BoardDetail> {
  return invoke<BoardDetail>("board_get", { boardId });
}

export function createBoard(name: string): Promise<Board> {
  return invoke<Board>("board_create", { name });
}

export function renameBoard(boardId: string, name: string): Promise<void> {
  return invoke("board_rename", { boardId, name });
}

export function deleteBoard(boardId: string): Promise<void> {
  return invoke("board_delete", { boardId });
}

export function createColumn(
  boardId: string,
  name: string,
  role: ColumnRole | null = null,
): Promise<BoardColumn> {
  return invoke<BoardColumn>("board_column_create", { boardId, name, role });
}

export function updateColumn(
  columnId: string,
  name: string,
  role: ColumnRole | null,
): Promise<void> {
  return invoke("board_column_update", { columnId, name, role });
}

export function moveColumn(columnId: string, index: number): Promise<void> {
  return invoke("board_column_move", { columnId, index });
}

export function deleteColumn(columnId: string): Promise<void> {
  return invoke("board_column_delete", { columnId });
}

export function createCard(
  columnId: string,
  title: string,
  body?: string,
): Promise<BoardCard> {
  return invoke<BoardCard>("board_card_create", { columnId, title, body });
}

export function updateCard(
  cardId: string,
  title: string,
  body: string,
): Promise<void> {
  return invoke("board_card_update", { cardId, title, body });
}

export function moveCard(
  cardId: string,
  columnId: string,
  index: number,
  actor: BoardActor = "user",
): Promise<void> {
  return invoke("board_card_move", { cardId, columnId, index, actor });
}

export function linkCard(
  cardId: string,
  link: { kind: CardLinkKind; id: string } | null,
): Promise<void> {
  return invoke("board_card_link", {
    cardId,
    linkKind: link?.kind ?? null,
    linkId: link?.id ?? null,
  });
}

/** The ticket the card links to, or null. */
export function getCardTicket(cardId: string): Promise<TicketRef | null> {
  return invoke<TicketRef | null>("board_card_ticket", { cardId });
}

/** Every ticket-linked card on the board, by card id. */
export function getCardTickets(
  boardId: string,
): Promise<Record<string, TicketRef>> {
  return invoke<Record<string, TicketRef>>("board_card_tickets", { boardId });
}

/** Links the card to a ticket (replacing any other); null removes it. */
export function setCardTicket(
  cardId: string,
  ticket: TicketRef | null,
): Promise<void> {
  return invoke("board_card_set_ticket", { cardId, ticket });
}

export function deleteCard(cardId: string): Promise<void> {
  return invoke("board_card_delete", { cardId });
}

export function cardsForLink(
  linkKind: CardLinkKind,
  linkId: string,
): Promise<BoardCard[]> {
  return invoke<BoardCard[]>("board_cards_for_link", { linkKind, linkId });
}

export function listActivity(
  boardId: string,
  cardId: string | null,
  limit: number,
): Promise<BoardActivity[]> {
  return invoke<BoardActivity[]>("board_activity", { boardId, cardId, limit });
}

export function recordCardEvent(
  cardId: string,
  kind: LinkedWorkEvent,
  detail?: string,
): Promise<void> {
  return invoke("board_card_record", { cardId, kind, detail });
}

/**
 * Calls `onChange` whenever the board with `boardId` is written to.
 * Returns an unsubscribe function.
 */
export function subscribeBoard(
  boardId: string,
  onChange: () => void,
): () => void {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  void listen<{ boardId: string }>(BOARD_CHANGED, (event) => {
    if (event.payload?.boardId === boardId) onChange();
  })
    .then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    })
    .catch(() => undefined);
  return () => {
    disposed = true;
    unlisten?.();
  };
}

export const PRIORITIES = ["None", "Low", "Medium", "High", "Urgent"] as const;

export type CardExtras = {
  priority: number;
  dueAt: number | null;
  projects: string[];
  labels: string[];
  subtasksDone: number;
  subtasksTotal: number;
  comments: number;
};

export type BoardLabel = {
  id: string;
  boardId: string;
  name: string;
  color: string;
  position: number;
};

export type BoardSubtask = {
  id: string;
  cardId: string;
  title: string;
  done: boolean;
  position: number;
};

export type BoardComment = {
  id: string;
  cardId: string;
  body: string;
  author: string;
  createdAt: number;
  editedAt: number | null;
};

export function getCardExtras(
  boardId: string,
): Promise<Record<string, CardExtras>> {
  return invoke<Record<string, CardExtras>>("board_card_extras", { boardId });
}

export function archiveCard(cardId: string): Promise<void> {
  return invoke("board_card_archive", { cardId });
}

export function archiveCards(cardIds: string[]): Promise<void> {
  return invoke("board_cards_archive", { cardIds });
}

export function unarchiveCard(cardId: string): Promise<void> {
  return invoke("board_card_unarchive", { cardId });
}

/** Unarchives the card and places it at `index` in `columnId`. */
export function restoreCard(
  cardId: string,
  columnId: string,
  index: number,
): Promise<void> {
  return invoke("board_card_restore", { cardId, columnId, index });
}

/** Commits a held delete: deletes the card only if it is still archived. */
export function deleteArchivedCard(cardId: string): Promise<void> {
  return invoke("board_card_delete_archived", { cardId });
}

export function listArchivedCards(boardId: string): Promise<BoardCard[]> {
  return invoke("board_archived_cards", { boardId });
}

export function moveCards(
  cardIds: string[],
  columnId: string,
  index: number,
  actor: BoardActor = "user",
): Promise<void> {
  return invoke("board_cards_move", { cardIds, columnId, index, actor });
}

/** Overwrites all three; pass the current colour and limit to change only `collapsed`. */
export function setColumnDisplay(
  columnId: string,
  color: string | null,
  wipLimit: number | null,
  collapsed: boolean,
): Promise<void> {
  return invoke("board_column_set_display", {
    columnId,
    color,
    wipLimit,
    collapsed,
  });
}

export function columnCardCount(
  columnId: string,
): Promise<{ live: number; archived: number }> {
  return invoke("board_column_card_count", { columnId });
}

export function setCardPriority(
  cardId: string,
  priority: number,
): Promise<void> {
  return invoke("board_card_set_priority", { cardId, priority });
}

export function setCardDue(
  cardId: string,
  dueAt: number | null,
): Promise<void> {
  return invoke("board_card_set_due", { cardId, dueAt });
}

export function setCardProjects(
  cardId: string,
  projects: string[],
): Promise<void> {
  return invoke("board_card_set_projects", { cardId, projects });
}

export function listLabels(boardId: string): Promise<BoardLabel[]> {
  return invoke<BoardLabel[]>("board_labels", { boardId });
}

export function createLabel(
  boardId: string,
  name: string,
  color: string,
): Promise<BoardLabel> {
  return invoke<BoardLabel>("board_label_create", { boardId, name, color });
}

export function setCardLabels(
  cardId: string,
  labelIds: string[],
): Promise<void> {
  return invoke("board_card_set_labels", { cardId, labelIds });
}

export function listSubtasks(cardId: string): Promise<BoardSubtask[]> {
  return invoke<BoardSubtask[]>("board_subtasks", { cardId });
}

export function addSubtask(
  cardId: string,
  title: string,
): Promise<BoardSubtask> {
  return invoke<BoardSubtask>("board_subtask_add", { cardId, title });
}

export function toggleSubtask(subtaskId: string, done: boolean): Promise<void> {
  return invoke("board_subtask_toggle", { subtaskId, done });
}

export function deleteSubtask(subtaskId: string): Promise<void> {
  return invoke("board_subtask_delete", { subtaskId });
}

export function listComments(cardId: string): Promise<BoardComment[]> {
  return invoke<BoardComment[]>("board_comments", { cardId });
}

export function addComment(
  cardId: string,
  body: string,
): Promise<BoardComment> {
  return invoke<BoardComment>("board_comment_add", { cardId, body });
}

export type CardMeta = {
  priority: number;
  dueAt: number | null;
  startAt: number | null;
  estimateMinutes: number | null;
  archivedAt: number | null;
  completedAt: number | null;
};

export type RelationKind = "blocks" | "relates" | "duplicates";

export type CardRelation = {
  cardId: string;
  relatedCardId: string;
  kind: RelationKind;
};

export type BoardFieldKind = "text" | "number" | "date" | "select" | "checkbox";

export type BoardField = {
  id: string;
  boardId: string;
  name: string;
  kind: BoardFieldKind;
  optionsJson: string | null;
  position: number;
};

export type FieldValue = {
  cardId: string;
  fieldId: string;
  valueJson: string;
};

export type CardLinkRow = {
  cardId: string;
  kind: string;
  reference: string;
  role: string | null;
  createdAt: number;
};

export function getCardMeta(cardId: string): Promise<CardMeta> {
  return invoke<CardMeta>("board_card_meta", { cardId });
}

export function setCardStart(
  cardId: string,
  startAt: number | null,
): Promise<void> {
  return invoke("board_card_set_start", { cardId, startAt });
}

export function setCardEstimate(
  cardId: string,
  minutes: number | null,
): Promise<void> {
  return invoke("board_card_set_estimate", { cardId, minutes });
}

export function addRelation(
  cardId: string,
  relatedCardId: string,
  kind: RelationKind,
): Promise<void> {
  return invoke("board_relation_add", { cardId, relatedCardId, kind });
}

export function removeRelation(
  cardId: string,
  relatedCardId: string,
  kind: RelationKind,
): Promise<void> {
  return invoke("board_relation_remove", { cardId, relatedCardId, kind });
}

export function listCardRelations(cardId: string): Promise<CardRelation[]> {
  return invoke<CardRelation[]>("board_card_relations", { cardId });
}

export function listFields(boardId: string): Promise<BoardField[]> {
  return invoke<BoardField[]>("board_fields", { boardId });
}

/** Pass `null` to clear the value. */
export function setFieldValue(
  cardId: string,
  fieldId: string,
  valueJson: string | null,
): Promise<void> {
  return invoke("board_field_value_set", { cardId, fieldId, valueJson });
}

export function listFieldValues(cardId: string): Promise<FieldValue[]> {
  return invoke<FieldValue[]>("board_card_field_values", { cardId });
}

export function addCardLink(
  cardId: string,
  kind: string,
  reference: string,
  role: string | null = null,
): Promise<void> {
  return invoke("board_link_add", { cardId, kind, reference, role });
}

export function removeCardLink(
  cardId: string,
  kind: string,
  reference: string,
): Promise<void> {
  return invoke("board_link_remove", { cardId, kind, reference });
}

export function listCardLinks(cardId: string): Promise<CardLinkRow[]> {
  return invoke<CardLinkRow[]>("board_card_links", { cardId });
}

export function editComment(commentId: string, body: string): Promise<void> {
  return invoke("board_comment_edit", { commentId, body });
}

export function moveSubtask(subtaskId: string, index: number): Promise<void> {
  return invoke("board_subtask_move", { subtaskId, index });
}

/** Turns the subtask into a card after its parent, related to it. */
export function convertSubtask(subtaskId: string): Promise<BoardCard> {
  return invoke<BoardCard>("board_subtask_convert", { subtaskId });
}

export function duplicateCard(cardId: string): Promise<BoardCard> {
  return invoke<BoardCard>("board_card_duplicate", { cardId });
}

export function moveCardToBoard(
  cardId: string,
  boardId: string,
): Promise<BoardCard> {
  return invoke<BoardCard>("board_card_move_to_board", { cardId, boardId });
}
