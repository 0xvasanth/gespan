import { ask } from "@tauri-apps/plugin-dialog";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { OverlayNav } from "../../../app/shell/TitleBar";
import { WindowControls } from "../../../app/shell/WindowControls";
import { IS_MAC } from "../../../platform/tauri/platform";
import { Kanban, Plus, Search } from "../../../shared/ui/icons";
import { SearchableSelect } from "../../../shared/ui/SearchableSelect";
import type { TicketRef } from "../../sessions/model/session";
import { LinkSessionWorkItemDialog } from "../../sessions/ui/LinkSessionWorkItemDialog";
import { FileActionError } from "../../files/ui/FileActionError";
import {
  archiveCard,
  archiveCards,
  createBoard,
  createCard,
  createColumn,
  createLabel,
  deleteArchivedCard,
  deleteBoard,
  deleteColumn,
  getBoard,
  getCardExtras,
  getCardTickets,
  listArchivedCards,
  listLabels,
  listBoards,
  moveCard,
  moveColumn,
  moveCards,
  renameBoard,
  restoreCard,
  setCardDue,
  setCardLabels,
  setCardPriority,
  setColumnDisplay,
  subscribeBoard,
  updateColumn,
  type Board,
  type BoardCard,
  type BoardColumn,
  type BoardDetail,
  type BoardLabel,
  type CardExtras,
  type ColumnRole,
} from "../model/board";
import {
  EMPTY_FILTER,
  filterCount,
  fullIndex,
  matchesCard,
  pruneFilter,
  type BoardFilter,
} from "../model/boardFilter";
import { sortedCardIds, type SortKey } from "../model/columnActions";
import { createCardFromTicket, newCardColumn } from "../model/ticketCards";
import {
  archivedLabel,
  createUndo,
  deletedLabel,
  movedLabel,
  runUndo,
  sortedLabel,
  undoFailure,
  type Place,
  type UndoAction,
  type UndoApi,
} from "../model/undo";
import { ArchiveView } from "./ArchiveView";
import { BoardCardItem } from "./BoardCardItem";
import { BoardColumnHeader } from "./BoardColumnHeader";
import { BoardFilterMenu } from "./BoardFilter";
import { MenuButton, MenuItem, NameInput } from "./boardControls";
import {
  errorText,
  linkValue,
  nextLabelColor,
  resolveLink,
  type BoardLinkTarget,
  type LinkRef,
} from "./boardFormat";
import { CardComposer } from "./CardComposer";
import { loadBoardCardOpen } from "../../settings/model/settings";
import { CardDetail } from "./CardDetail";
import { KanbanBoard } from "./kanban/KanbanBoard";
import { UndoToast } from "./UndoToast";
import {
  applyCardMove,
  applyColumnMove,
  columnCards,
  type DropTarget,
} from "./kanban/kanbanMove";

export type { BoardLinkTarget, LinkRef } from "./boardFormat";

/** Where a card sits now among its column's live cards, and its number. */
function placeOf(
  detail: BoardDetail,
  cardId: string,
): { place: Place; number: number } | null {
  const card = detail.cards.find((other) => other.id === cardId);
  if (!card) return null;
  const index = columnCards(detail, card.columnId).findIndex(
    (other) => other.id === cardId,
  );
  return { place: [card.id, card.columnId, index], number: card.number };
}

const OPEN_BOARD_KEY = "gespan:open-board";

function loadOpenBoardId(): string | null {
  try {
    return localStorage.getItem(OPEN_BOARD_KEY);
  } catch {
    return null;
  }
}

function saveOpenBoardId(id: string | null) {
  try {
    if (id) localStorage.setItem(OPEN_BOARD_KEY, id);
    else localStorage.removeItem(OPEN_BOARD_KEY);
  } catch {
    // Storage can be unavailable; the first board opens next time.
  }
}

function pickBoard(boards: readonly Board[], preferred: string | null) {
  if (preferred && boards.some((board) => board.id === preferred)) {
    return preferred;
  }
  return boards[0]?.id ?? null;
}

const BUTTON =
  "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-content/15 px-2.5 text-[12px] text-content/80 hover:border-content/30 hover:bg-content/10 hover:text-content";

type Props = {
  tabbed?: boolean;
  besideRail?: boolean;
  compactRail?: boolean;
  onClose: () => void;
  onToggleSidebar?: () => void;
  /** Sessions and Pilots a card can link to, with their live status. */
  linkTargets: readonly BoardLinkTarget[];
  /** Rejects when the work can't be opened; the board shows the error. */
  onOpenLinkedWork: (target: BoardLinkTarget) => Promise<void>;
  /**
   * Finds cards' linked work that `linkTargets` doesn't hold (sessions of
   * projects not loaded this run, say). Work it doesn't return is gone.
   */
  lookupLinkTargets: (links: LinkRef[]) => Promise<BoardLinkTarget[]>;
  /** The workspace's projects, for the filter. */
  projects: readonly { path: string; name: string }[];
};

export function BoardView({
  tabbed = false,
  besideRail = false,
  compactRail = false,
  onClose,
  onToggleSidebar,
  linkTargets,
  onOpenLinkedWork,
  lookupLinkTargets,
  projects,
}: Props) {
  return (
    <div
      role="region"
      aria-label="Board"
      data-app-board
      className="flex min-h-0 min-w-0 flex-1 flex-col text-content"
    >
      {!tabbed ? (
        <div
          data-app-view-header
          className="flex h-10 shrink-0 select-none items-center border-b border-stroke"
          data-tauri-drag-region="deep"
        >
          {IS_MAC && compactRail ? <div className="w-4 shrink-0" /> : null}
          {IS_MAC && !besideRail ? <div className="w-[78px] shrink-0" /> : null}
          {besideRail ? null : (
            <OverlayNav onBack={onClose} onToggleSidebar={onToggleSidebar} />
          )}
          <div className="flex min-w-0 flex-1 items-center gap-2 px-3 text-[13px]">
            <Kanban
              className="size-3.5 shrink-0 text-content/45"
              strokeWidth={1.75}
            />
            <span className="min-w-0 truncate text-content">Board</span>
          </div>
          {IS_MAC ? null : <WindowControls />}
        </div>
      ) : null}
      <BoardContent
        linkTargets={linkTargets}
        onOpenLinkedWork={onOpenLinkedWork}
        lookupLinkTargets={lookupLinkTargets}
        projects={projects}
      />
    </div>
  );
}

function BoardContent({
  linkTargets,
  onOpenLinkedWork,
  lookupLinkTargets,
  projects,
}: Pick<
  Props,
  "linkTargets" | "onOpenLinkedWork" | "lookupLinkTargets" | "projects"
>) {
  const [boards, setBoards] = useState<Board[] | null>(null);
  const [openId, setOpenId] = useState<string | null>(null);
  const [detail, setDetail] = useState<BoardDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [naming, setNaming] = useState<"new" | "rename" | null>(null);
  const [addingColumn, setAddingColumn] = useState(false);
  const [tickets, setTickets] = useState<Record<string, TicketRef>>({});
  const [extras, setExtras] = useState<Record<string, CardExtras>>({});
  const [labels, setLabels] = useState<BoardLabel[]>([]);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<BoardFilter>(EMPTY_FILTER);
  const [ticketCardOpen, setTicketCardOpen] = useState(false);
  const [composerColumn, setComposerColumn] = useState<string | null>(null);
  const [openCard, setOpenCard] = useState<{
    id: string;
    focus?: "title" | "link";
  } | null>(null);
  // The card shown as a full page, if any; opening one follows the setting.
  const [pageCardId, setPageCardId] = useState<string | null>(null);
  const [archiveOpen, setArchiveOpen] = useState(false);
  const showCard = (next: { id: string; focus?: "title" | "link" }) => {
    setOpenCard(next);
    setPageCardId(loadBoardCardOpen() === "page" ? next.id : null);
  };

  const detailRef = useRef(detail);
  const openIdRef = useRef(openId);
  useLayoutEffect(() => {
    detailRef.current = detail;
    openIdRef.current = openId;
  });
  const request = useRef(0);

  const fail = useCallback(
    (what: string) => (cause: unknown) =>
      setError(`${what}: ${errorText(cause)}`),
    [],
  );

  // One undo toast at a time. A held delete is committed (only if the card is
  // still archived) when the toast expires, is replaced or the board switches.
  const [undo] = useState(() =>
    createUndo({
      onCommitDelete: (cardId) =>
        deleteArchivedCard(cardId).catch(fail("Couldn't delete the card")),
    }),
  );
  const undoToast = useSyncExternalStore(undo.subscribe, undo.current);
  // Closing the window drops the timer: a held card stays archived.
  // A write that finishes after unmount shows no toast, so its card stays
  // archived. Set in the effect body so StrictMode's remount works.
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    const detach = undo.attach();
    return () => {
      mounted.current = false;
      detach();
    };
  }, [undo]);

  /**
   * Shows the toast for an action on `boardId` that just finished. If the
   * board was switched meanwhile there is no toast, and a delete is carried
   * out at once, as a switch does with a held one.
   */
  const pushUndo = useCallback(
    (boardId: string, label: string, action: UndoAction) => {
      if (!mounted.current) return;
      if (openIdRef.current === boardId) undo.push(label, action);
      else if (action.kind === "delete") {
        void deleteArchivedCard(action.cardId).catch(
          fail("Couldn't delete the card"),
        );
      }
    },
    [fail, undo],
  );

  const openLinkedWork = useCallback(
    (target: BoardLinkTarget) => {
      const failed = fail("Couldn't open linked work");
      try {
        // Called now, so navigation starts in this tick; a synchronous throw
        // lands in the same toast as a rejection.
        void onOpenLinkedWork(target).catch(failed);
      } catch (cause) {
        failed(cause);
      }
    },
    [fail, onOpenLinkedWork],
  );

  const loadBoards = useCallback(
    async (preferred?: string) => {
      try {
        const list = await listBoards();
        setBoards(list);
        setOpenId((current) =>
          pickBoard(list, preferred ?? current ?? loadOpenBoardId()),
        );
      } catch (cause) {
        setBoards((current) => current ?? []);
        fail("Couldn't load boards")(cause);
      }
    },
    [fail],
  );

  /** Fetches the open board; a newer fetch, or a switch, wins. */
  const refresh = useCallback(async () => {
    const id = openIdRef.current;
    if (!id) return;
    const ticket = ++request.current;
    try {
      const [next, linked, cardExtras, boardLabels] = await Promise.all([
        getBoard(id),
        // A failed ticket read leaves the cards without chips, not the board unloaded.
        getCardTickets(id).catch(() => null),
        // Likewise for the card details and labels: the cards show without them.
        getCardExtras(id).catch(() => null),
        listLabels(id).catch(() => null),
      ]);
      if (ticket !== request.current || openIdRef.current !== id) return;
      setDetail(next);
      if (linked) setTickets(linked);
      if (cardExtras) setExtras(cardExtras);
      if (boardLabels) setLabels(boardLabels);
      setBoards(
        (list) =>
          list &&
          list.map((board) =>
            board.id === next.board.id ? next.board : board,
          ),
      );
    } catch (cause) {
      const stale = () =>
        ticket !== request.current || openIdRef.current !== id;
      if (stale()) return;
      // Deleting the board (here or elsewhere) also signals a change to it:
      // a board that is no longer listed is gone, not broken, so open
      // another one quietly.
      const list = await listBoards().catch(() => null);
      if (stale()) return;
      if (list && !list.some((board) => board.id === id)) {
        setBoards(list);
        setOpenId(pickBoard(list, null));
        return;
      }
      fail("Couldn't load the board")(cause);
    }
  }, [fail]);

  useEffect(() => {
    void loadBoards();
  }, [loadBoards]);

  useEffect(() => {
    if (boards) saveOpenBoardId(openId);
  }, [boards, openId]);

  useEffect(() => {
    // Switching boards carries out a held delete now.
    void undo.commitPending();
    openIdRef.current = openId;
    setDetail((current) => (current?.board.id === openId ? current : null));
    setComposerColumn(null);
    setOpenCard(null);
    // Labels belong to a board: its filter and details don't carry over.
    setExtras({});
    setLabels([]);
    setFilter(EMPTY_FILTER);
    setQuery("");
    if (!openId) return;
    void refresh();
    return subscribeBoard(openId, () => void refresh());
  }, [openId, refresh, undo]);

  const targetsByValue = useMemo(
    () =>
      new Map(
        linkTargets.map((target) => [
          linkValue(target.kind, target.id),
          target,
        ]),
      ),
    [linkTargets],
  );

  // Cards' links `linkTargets` doesn't hold, looked up once each: the
  // target, or null when the work is gone. Absent while unconfirmed.
  const [lookedUp, setLookedUp] = useState<
    ReadonlyMap<string, BoardLinkTarget | null>
  >(() => new Map());
  const asked = useRef(new Set<string>());
  useEffect(() => {
    if (!detail) return;
    const wanted = new Map<string, LinkRef>();
    for (const card of detail.cards) {
      if (!card.linkKind || !card.linkId) continue;
      const value = linkValue(card.linkKind, card.linkId);
      // Held by the app now: ask again if it ever drops out (deleted).
      if (targetsByValue.has(value)) asked.current.delete(value);
      if (targetsByValue.has(value) || asked.current.has(value)) continue;
      wanted.set(value, { kind: card.linkKind, id: card.linkId });
    }
    if (!wanted.size) return;
    for (const value of wanted.keys()) asked.current.add(value);
    lookupLinkTargets([...wanted.values()])
      .then((found) => {
        const byValue = new Map(
          found.map((target) => [linkValue(target.kind, target.id), target]),
        );
        setLookedUp((current) => {
          const next = new Map(current);
          for (const value of wanted.keys()) {
            next.set(value, byValue.get(value) ?? null);
          }
          return next;
        });
      })
      // Left unconfirmed: a failed lookup says nothing about the work.
      .catch(fail("Couldn't check linked work"));
  }, [detail, targetsByValue, lookupLinkTargets, fail]);

  // What the app has loaded wins over a lookup: it carries live status.
  const linkStates = useMemo(() => {
    const states = new Map<string, BoardLinkTarget | null>(lookedUp);
    for (const [value, target] of targetsByValue) states.set(value, target);
    return states;
  }, [lookedUp, targetsByValue]);

  // A label or project that is gone leaves the filter, so it can't hide
  // every card behind a choice nobody can see or undo.
  const activeFilter = useMemo(
    () => pruneFilter(filter, labels, projects),
    [filter, labels, projects],
  );
  const narrowing = query.trim() !== "" || filterCount(activeFilter) > 0;
  const matches = useCallback(
    (card: BoardCard) =>
      matchesCard(card, extras[card.id], query, activeFilter),
    [extras, query, activeFilter],
  );
  // The board shows only matching cards. Its drop indexes count those, so
  // `onCardMove` maps them back to places among all the column's cards.
  const shownDetail = useMemo(
    () =>
      detail && narrowing
        ? { ...detail, cards: detail.cards.filter(matches) }
        : detail,
    [detail, narrowing, matches],
  );
  const matchesRef = useRef(matches);
  const narrowingRef = useRef(narrowing);
  useLayoutEffect(() => {
    matchesRef.current = matches;
    narrowingRef.current = narrowing;
  });

  const onCardMove = useCallback(
    (cardId: string, shown: DropTarget) => {
      const before = detailRef.current;
      if (!before) return;
      let to = shown;
      if (narrowingRef.current) {
        const others = columnCards(before, shown.columnId).filter(
          (card) => card.id !== cardId,
        );
        to = {
          columnId: shown.columnId,
          index: fullIndex(
            others.map((card) => card.id),
            others.filter(matchesRef.current).map((card) => card.id),
            shown.index,
          ),
        };
      }
      const from = placeOf(before, cardId);
      const column = before.columns.find((other) => other.id === to.columnId);
      // A drop back onto the same place is no action: a held delete stays held.
      const samePlace =
        from?.place[1] === to.columnId && from.place[2] === to.index;
      if (!samePlace) void undo.commitPending();
      setDetail(applyCardMove(before, cardId, to));
      moveCard(cardId, to.columnId, to.index, "user")
        .then(() => {
          if (!from || !column || samePlace) return;
          const [, fromColumn, fromIndex] = from.place;
          pushUndo(before.board.id, movedLabel([from.number], column.name), {
            kind: "move",
            cardId,
            columnId: fromColumn,
            index: fromIndex,
          });
        })
        .catch((cause) => {
          if (detailRef.current?.board.id === before.board.id) {
            setDetail(before);
          }
          fail("Couldn't move the card")(cause);
          void refresh();
        });
    },
    [fail, refresh, undo, pushUndo],
  );

  const onColumnMove = useCallback(
    (columnId: string, index: number) => {
      const before = detailRef.current;
      if (!before) return;
      setDetail(applyColumnMove(before, columnId, index));
      moveColumn(columnId, index).catch((cause) => {
        if (detailRef.current?.board.id === before.board.id) setDetail(before);
        fail("Couldn't move the column")(cause);
        void refresh();
      });
    },
    [fail, refresh],
  );

  const createNamedBoard = (name: string) => {
    setNaming(null);
    createBoard(name)
      .then((board) => loadBoards(board.id))
      .catch(fail("Couldn't create the board"));
  };

  const removeBoard = async (board: Board) => {
    const confirmed = await ask(
      `Delete “${board.name}”? Its columns, cards and activity are removed.`,
      {
        title: "Delete board",
        kind: "warning",
        okLabel: "Delete board",
        cancelLabel: "Cancel",
      },
    ).catch(() => false);
    if (!confirmed) return;
    deleteBoard(board.id)
      .then(() => loadBoards())
      .catch(fail("Couldn't delete the board"));
  };

  /**
   * Deletes a card without asking: it is archived at once (off the board and
   * out of the Archive view) and deleted for good when the undo toast goes.
   */
  const removeCard = (card: BoardCard) => {
    const before = detailRef.current;
    const from = before && placeOf(before, card.id);
    if (!before || !from) return;
    void undo.commitPending();
    setOpenCard((open) => (open?.id === card.id ? null : open));
    setDetail({
      ...before,
      cards: before.cards.filter((other) => other.id !== card.id),
    });
    archiveCard(card.id)
      .then(() => {
        const [, columnId, index] = from.place;
        pushUndo(before.board.id, deletedLabel(card.number), {
          kind: "delete",
          cardId: card.id,
          columnId,
          index,
        });
      })
      .catch((cause) => {
        fail(`Couldn't delete card #${card.number}`)(cause);
        void refresh();
      });
  };

  /** A move from the card drawer: to `index` in `columnId`, with undo. */
  const moveFromDrawer = (card: BoardCard, columnId: string, index: number) => {
    const before = detailRef.current;
    const from = before && placeOf(before, card.id);
    const column = before?.columns.find((other) => other.id === columnId);
    void undo.commitPending();
    moveCard(card.id, columnId, index, "user")
      .then(() => {
        if (!before || !from || !column) return;
        const [, fromColumn, fromIndex] = from.place;
        pushUndo(before.board.id, movedLabel([card.number], column.name), {
          kind: "move",
          cardId: card.id,
          columnId: fromColumn,
          index: fromIndex,
        });
      })
      .catch(fail(`Couldn't move card #${card.number}`));
  };

  const undoApi: UndoApi = {
    moveCard: (cardId, columnId, index) =>
      moveCard(cardId, columnId, index, "user"),
    moveCards: (cardIds, columnId, index) =>
      moveCards(cardIds, columnId, index, "user"),
    restoreCard,
    snapshot: async () => {
      const id = openIdRef.current ?? "";
      const [board, archived] = await Promise.all([
        getBoard(id),
        listArchivedCards(id),
      ]);
      return {
        live: board.cards,
        archivedIds: archived.map((card) => card.id),
      };
    },
  };

  /** Runs the toast's inverse; what couldn't be undone is said in a toast. */
  const undoLast = () => {
    const action = undo.undo();
    if (!action) return;
    runUndo(action, undoApi)
      .catch((cause) => {
        // A toast pushed while the undo ran is newer: it stays, and with it
        // any delete it holds. The failure goes to the error toast instead.
        const message = undoFailure(cause);
        if (!undo.notify(message) && mounted.current) setError(message);
      })
      .finally(() => void refresh());
  };

  /** Where each card sits now, and the card numbers, read from the board. */
  const placesOf = (ids: readonly string[]) => {
    const current = detailRef.current;
    const found = current
      ? ids.flatMap((id) => placeOf(current, id) ?? [])
      : [];
    return {
      items: found.map((entry) => entry.place),
      numbers: found.map((entry) => entry.number),
    };
  };

  const setCollapsed = (column: BoardColumn, collapsed: boolean) =>
    void setColumnDisplay(
      column.id,
      column.color,
      column.wipLimit,
      collapsed,
    ).catch(
      fail(
        collapsed
          ? "Couldn't collapse the column"
          : "Couldn't expand the column",
      ),
    );

  /** Every live card of a column, in order: filters and search don't narrow it. */
  const wholeColumn = (columnId: string) => {
    const current = detailRef.current;
    return current ? columnCards(current, columnId) : [];
  };

  const sortColumn = (column: BoardColumn, key: SortKey) => {
    const cards = wholeColumn(column.id);
    const ids = sortedCardIds(cards, extras, key);
    if (ids.every((id, index) => id === cards[index]?.id)) return;
    const order = cards.map((card) => card.id);
    void undo.commitPending();
    moveCards(ids, column.id, 0, "user")
      .then(() =>
        pushUndo(column.boardId, sortedLabel(column.name), {
          kind: "sort",
          columnId: column.id,
          order,
        }),
      )
      .catch(fail(`Couldn't sort ${column.name}`));
  };

  const markAllDone = (column: BoardColumn, done: BoardColumn) => {
    const ids = wholeColumn(column.id).map((card) => card.id);
    if (!ids.length) return;
    const { items, numbers } = placesOf(ids);
    void undo.commitPending();
    moveCards(ids, done.id, wholeColumn(done.id).length, "user")
      .then(() =>
        pushUndo(column.boardId, movedLabel(numbers, done.name), {
          kind: "moveMany",
          items,
        }),
      )
      .catch(fail(`Couldn't move the cards of ${column.name} to ${done.name}`));
  };

  const archiveCompleted = (column: BoardColumn) => {
    const ids = wholeColumn(column.id).map((card) => card.id);
    if (!ids.length) return;
    const { items, numbers } = placesOf(ids);
    void undo.commitPending();
    archiveCards(ids)
      .then(() =>
        pushUndo(column.boardId, archivedLabel(numbers), {
          kind: "archive",
          items,
        }),
      )
      .catch(fail(`Couldn't archive the cards of ${column.name}`));
  };

  const toast = (
    <>
      <UndoToast toast={undoToast} onUndo={undoLast} />
      {error ? (
        <FileActionError message={error} onDismiss={() => setError(null)} />
      ) : null}
    </>
  );

  if (!boards) return toast;

  if (!boards.length) {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3">
        <span className="text-[13px] text-content/60">No boards yet</span>
        {naming === "new" ? (
          <NameInput
            label="Board name"
            placeholder="Board name"
            className="w-56"
            onSubmit={createNamedBoard}
            onCancel={() => setNaming(null)}
          />
        ) : (
          <button
            type="button"
            className={BUTTON}
            onClick={() => setNaming("new")}
          >
            Create board
          </button>
        )}
        {toast}
      </div>
    );
  }

  const openBoard = boards.find((board) => board.id === openId) ?? null;
  const columnsById = new Map(
    (detail?.columns ?? []).map((column) => [column.id, column]),
  );
  const shownCard = openCard
    ? detail?.cards.find((card) => card.id === openCard.id)
    : undefined;

  const fullPage = !!shownCard && pageCardId === shownCard.id;

  const labelsOf = (cardId: string) => {
    const ids = extras[cardId]?.labels;
    return ids?.length ? labels.filter((label) => ids.includes(label.id)) : [];
  };

  const renderCard = (card: BoardCard) => (
    <BoardCardItem
      card={card}
      done={columnsById.get(card.columnId)?.role === "done"}
      link={resolveLink(card, linkStates)}
      ticket={tickets[card.id] ?? null}
      extras={extras[card.id]}
      labels={labelsOf(card.id)}
      allLabels={labels}
      onSetPriority={(priority) =>
        void setCardPriority(card.id, priority).catch(
          fail(`Couldn't set the priority of card #${card.number}`),
        )
      }
      onSetDue={(dueAt) =>
        void setCardDue(card.id, dueAt).catch(
          fail(`Couldn't set the due date of card #${card.number}`),
        )
      }
      onSetLabels={(ids) =>
        void setCardLabels(card.id, ids).catch(
          fail(`Couldn't set the labels of card #${card.number}`),
        )
      }
      onCreateLabel={(name) =>
        void createLabel(card.boardId, name, nextLabelColor(labels.length))
          .then((created) =>
            setCardLabels(card.id, [
              ...(extras[card.id]?.labels ?? []),
              created.id,
            ]),
          )
          .catch(fail(`Couldn't create a label for card #${card.number}`))
      }
      onOpen={() => showCard({ id: card.id })}
      onOpenLinkedWork={openLinkedWork}
      onRename={() => showCard({ id: card.id, focus: "title" })}
      onLink={() => showCard({ id: card.id, focus: "link" })}
      onDelete={() => removeCard(card)}
    />
  );

  const doneColumn = detail?.columns.find((column) => column.role === "done");
  const held = undoToast?.action?.kind === "delete" ? undoToast.action : null;
  const heldColumn = held?.columnId ?? null;

  const renderColumnHeader = (column: BoardColumn, shown: number) => (
    <BoardColumnHeader
      column={column}
      count={
        narrowing
          ? (detail?.cards.filter((card) => card.columnId === column.id)
              .length ?? shown)
          : shown
      }
      shown={narrowing ? shown : undefined}
      takenRoles={
        new Set(
          (detail?.columns ?? [])
            .filter((other) => other.id !== column.id && other.role)
            .map((other) => other.role as ColumnRole),
        )
      }
      onAddCard={() => setComposerColumn(column.id)}
      onRename={(name) =>
        void updateColumn(column.id, name, column.role).catch(
          fail("Couldn't rename the column"),
        )
      }
      onSetRole={(role) =>
        void updateColumn(column.id, column.name, role).catch(
          fail("Couldn't set the column's role"),
        )
      }
      heldCards={heldColumn === column.id ? 1 : 0}
      onDelete={() =>
        // A delete held in this column is carried out first.
        void undo
          .commitPending()
          .then(() => deleteColumn(column.id))
          .catch(fail("Couldn't delete the column"))
      }
      onCollapse={() => setCollapsed(column, true)}
      onSort={(key) => sortColumn(column, key)}
      onMarkAllDone={
        doneColumn && doneColumn.id !== column.id
          ? () => markAllDone(column, doneColumn)
          : undefined
      }
      onArchiveCompleted={
        column.role === "done" ? () => archiveCompleted(column) : undefined
      }
    />
  );

  const renderColumnFooter = (column: BoardColumn) => (
    <CardComposer
      columnName={column.name}
      open={composerColumn === column.id}
      onOpen={() => setComposerColumn(column.id)}
      onClose={() =>
        setComposerColumn((current) => (current === column.id ? null : current))
      }
      onCreate={(title) =>
        void createCard(column.id, title).catch(fail("Couldn't add the card"))
      }
    />
  );

  const trailing = addingColumn ? (
    <div className="w-[280px] shrink-0">
      <NameInput
        label="Column name"
        placeholder="Column name"
        className="w-full"
        onSubmit={(name) => {
          setAddingColumn(false);
          if (openId) {
            void createColumn(openId, name).catch(
              fail("Couldn't add the column"),
            );
          }
        }}
        onCancel={() => setAddingColumn(false)}
      />
    </div>
  ) : (
    <button
      type="button"
      aria-label="Add column"
      title="Add column"
      onClick={() => setAddingColumn(true)}
      className="grid size-9 shrink-0 place-items-center rounded-lg border border-dashed border-stroke text-content/45 hover:border-selection-strong hover:text-content"
    >
      <Plus className="size-3.5" strokeWidth={1.75} />
    </button>
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-11 shrink-0 items-center gap-1.5 border-b border-stroke px-4">
        {naming === "rename" && openBoard ? (
          <NameInput
            initial={openBoard.name}
            label="Board name"
            className="w-56"
            onSubmit={(name) => {
              setNaming(null);
              renameBoard(openBoard.id, name)
                .then(() => loadBoards(openBoard.id))
                .catch(fail("Couldn't rename the board"));
            }}
            onCancel={() => setNaming(null)}
          />
        ) : (
          <div className="w-56 min-w-0">
            <SearchableSelect
              label="Board"
              value={openId ?? ""}
              options={boards.map((board) => ({
                value: board.id,
                label: board.name,
              }))}
              onChange={(id) => setOpenId(id)}
              searchPlaceholder="Search boards…"
              emptyLabel="No matching boards"
              variant="transparent"
            />
          </div>
        )}
        {openBoard ? (
          <MenuButton label="Board actions">
            {(close) => (
              <>
                <MenuItem
                  onSelect={() => {
                    close();
                    setNaming("rename");
                  }}
                >
                  Rename board
                </MenuItem>
                <MenuItem
                  danger
                  onSelect={() => {
                    close();
                    void removeBoard(openBoard);
                  }}
                >
                  Delete board
                </MenuItem>
              </>
            )}
          </MenuButton>
        ) : null}
        {detail ? (
          <>
            <label className="relative ml-2 flex items-center">
              <Search
                className="pointer-events-none absolute left-2 size-3.5 text-content/45"
                strokeWidth={1.75}
              />
              <input
                aria-label="Search cards"
                placeholder="Search cards"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Escape" && query) {
                    event.preventDefault();
                    setQuery("");
                  }
                }}
                className="h-7 w-44 rounded-md border border-stroke bg-background-base pl-7 pr-2 text-[12px] text-content outline-none placeholder:text-content/40 focus:border-accent/60"
              />
            </label>
            <BoardFilterMenu
              filter={activeFilter}
              labels={labels}
              projects={projects}
              onChange={setFilter}
            />
          </>
        ) : null}
        <div className="flex-1" />
        {detail ? (
          <button
            type="button"
            className={BUTTON}
            aria-pressed={archiveOpen}
            onClick={() => {
              setOpenCard(null);
              setArchiveOpen((open) => !open);
            }}
          >
            Archive
          </button>
        ) : null}
        {detail ? (
          <button
            type="button"
            className={BUTTON}
            onClick={() => setTicketCardOpen(true)}
          >
            <Plus className="size-3.5" strokeWidth={1.75} />
            New card from ticket…
          </button>
        ) : null}
        {naming === "new" ? (
          <NameInput
            label="Board name"
            placeholder="Board name"
            className="w-48"
            onSubmit={createNamedBoard}
            onCancel={() => setNaming(null)}
          />
        ) : (
          <button
            type="button"
            className={BUTTON}
            onClick={() => setNaming("new")}
          >
            <Plus className="size-3.5" strokeWidth={1.75} />
            New board
          </button>
        )}
      </div>
      <div className="flex min-h-0 flex-1">
        <div className={`min-h-0 min-w-0 flex-1 ${fullPage ? "hidden" : ""}`}>
          {shownDetail ? (
            <KanbanBoard
              detail={shownDetail}
              renderCard={renderCard}
              renderColumnHeader={renderColumnHeader}
              renderColumnFooter={renderColumnFooter}
              onCardMove={onCardMove}
              onColumnMove={onColumnMove}
              onCardOpen={(card) => showCard({ id: card.id })}
              onColumnExpand={(column) => setCollapsed(column, false)}
              trailing={trailing}
            />
          ) : null}
        </div>
        {shownCard ? (
          <CardDetail
            key={shownCard.id}
            card={shownCard}
            columns={(detail?.columns ?? []).map((column) => ({
              id: column.id,
              name: column.name,
              cardCount: (detail?.cards ?? []).filter(
                (other) => other.columnId === column.id,
              ).length,
            }))}
            extras={extras[shownCard.id]}
            labels={labels}
            projects={projects}
            link={resolveLink(shownCard, linkStates)}
            ticket={tickets[shownCard.id] ?? null}
            linkTargets={linkTargets}
            boardCards={(detail?.cards ?? []).map((other) => ({
              id: other.id,
              number: other.number,
              title: other.title,
            }))}
            fullPage={fullPage}
            onToggleFullPage={() =>
              setPageCardId(fullPage ? null : shownCard.id)
            }
            focus={openCard?.focus}
            onClose={() => setOpenCard(null)}
            onOpenLinkedWork={(target) => {
              setOpenCard(null);
              openLinkedWork(target);
            }}
            onMove={(columnId, index) =>
              moveFromDrawer(shownCard, columnId, index)
            }
            onDelete={() => removeCard(shownCard)}
            onError={setError}
          />
        ) : archiveOpen && detail ? (
          <ArchiveView
            key={detail.board.id}
            boardId={detail.board.id}
            columnNames={
              new Map(detail.columns.map((column) => [column.id, column.name]))
            }
            pendingDelete={held?.cardId ?? null}
            onClose={() => setArchiveOpen(false)}
            onError={setError}
          />
        ) : null}
      </div>
      {ticketCardOpen && detail ? (
        <LinkSessionWorkItemDialog
          ticketOnly
          heading="New card from ticket"
          sessionTitle={detail.board.name}
          onSave={(item) => {
            if (!item || item.kind !== "ticket") return;
            setTicketCardOpen(false);
            const column = newCardColumn(detail.columns);
            if (!column) {
              setError("Couldn't add the card: the board has no columns");
              return;
            }
            createCardFromTicket(column, item.ticket).catch(
              fail("Couldn't add the card from the ticket"),
            );
          }}
          onClose={() => setTicketCardOpen(false)}
        />
      ) : null}
      {toast}
    </div>
  );
}
