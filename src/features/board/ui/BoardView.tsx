import { ask } from "@tauri-apps/plugin-dialog";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { OverlayNav } from "../../../app/shell/TitleBar";
import { WindowControls } from "../../../app/shell/WindowControls";
import { IS_MAC } from "../../../platform/tauri/platform";
import { Kanban, Plus } from "../../../shared/ui/icons";
import { SearchableSelect } from "../../../shared/ui/SearchableSelect";
import type { TicketRef } from "../../sessions/model/session";
import { LinkSessionWorkItemDialog } from "../../sessions/ui/LinkSessionWorkItemDialog";
import { FileActionError } from "../../files/ui/FileActionError";
import {
  createBoard,
  createCard,
  createColumn,
  deleteBoard,
  deleteCard,
  deleteColumn,
  getBoard,
  getCardTickets,
  listBoards,
  moveCard,
  moveColumn,
  renameBoard,
  subscribeBoard,
  updateColumn,
  type Board,
  type BoardCard,
  type BoardColumn,
  type BoardDetail,
  type ColumnRole,
} from "../model/board";
import { createCardFromTicket, newCardColumn } from "../model/ticketCards";
import { BoardCardItem } from "./BoardCardItem";
import { BoardColumnHeader } from "./BoardColumnHeader";
import {
  confirmDeleteCard,
  MenuButton,
  MenuItem,
  NameInput,
} from "./boardControls";
import {
  errorText,
  linkValue,
  resolveLink,
  type BoardLinkTarget,
  type LinkRef,
} from "./boardFormat";
import { CardComposer } from "./CardComposer";
import { CardDetail } from "./CardDetail";
import { KanbanBoard } from "./kanban/KanbanBoard";
import {
  applyCardMove,
  applyColumnMove,
  type DropTarget,
} from "./kanban/kanbanMove";

export type { BoardLinkTarget, LinkRef } from "./boardFormat";

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
      />
    </div>
  );
}

function BoardContent({
  linkTargets,
  onOpenLinkedWork,
  lookupLinkTargets,
}: Pick<Props, "linkTargets" | "onOpenLinkedWork" | "lookupLinkTargets">) {
  const [boards, setBoards] = useState<Board[] | null>(null);
  const [openId, setOpenId] = useState<string | null>(null);
  const [detail, setDetail] = useState<BoardDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [naming, setNaming] = useState<"new" | "rename" | null>(null);
  const [addingColumn, setAddingColumn] = useState(false);
  const [tickets, setTickets] = useState<Record<string, TicketRef>>({});
  const [ticketCardOpen, setTicketCardOpen] = useState(false);
  const [composerColumn, setComposerColumn] = useState<string | null>(null);
  const [openCard, setOpenCard] = useState<{
    id: string;
    focus?: "title" | "link";
  } | null>(null);

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
      const [next, linked] = await Promise.all([
        getBoard(id),
        // A failed ticket read leaves the cards without chips, not the board unloaded.
        getCardTickets(id).catch(() => null),
      ]);
      if (ticket !== request.current || openIdRef.current !== id) return;
      setDetail(next);
      if (linked) setTickets(linked);
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
    openIdRef.current = openId;
    setDetail((current) => (current?.board.id === openId ? current : null));
    setComposerColumn(null);
    setOpenCard(null);
    if (!openId) return;
    void refresh();
    return subscribeBoard(openId, () => void refresh());
  }, [openId, refresh]);

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

  const onCardMove = useCallback(
    (cardId: string, to: DropTarget) => {
      const before = detailRef.current;
      if (!before) return;
      setDetail(applyCardMove(before, cardId, to));
      moveCard(cardId, to.columnId, to.index, "user").catch((cause) => {
        if (detailRef.current?.board.id === before.board.id) setDetail(before);
        fail("Couldn't move the card")(cause);
        void refresh();
      });
    },
    [fail, refresh],
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

  const removeCard = async (card: BoardCard) => {
    if (!(await confirmDeleteCard(card))) return;
    deleteCard(card.id).catch(fail(`Couldn't delete card #${card.number}`));
  };

  const toast = error ? (
    <FileActionError message={error} onDismiss={() => setError(null)} />
  ) : null;

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

  const renderCard = (card: BoardCard) => (
    <BoardCardItem
      card={card}
      done={columnsById.get(card.columnId)?.role === "done"}
      link={resolveLink(card, linkStates)}
      ticket={tickets[card.id] ?? null}
      onOpen={() => setOpenCard({ id: card.id })}
      onOpenLinkedWork={openLinkedWork}
      onRename={() => setOpenCard({ id: card.id, focus: "title" })}
      onLink={() => setOpenCard({ id: card.id, focus: "link" })}
      onDelete={() => void removeCard(card)}
    />
  );

  const renderColumnHeader = (column: BoardColumn, count: number) => (
    <BoardColumnHeader
      column={column}
      count={count}
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
      onDelete={() =>
        void deleteColumn(column.id).catch(fail("Couldn't delete the column"))
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
        <div className="flex-1" />
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
      <div className="min-h-0 flex-1">
        {detail ? (
          <KanbanBoard
            detail={detail}
            renderCard={renderCard}
            renderColumnHeader={renderColumnHeader}
            renderColumnFooter={renderColumnFooter}
            onCardMove={onCardMove}
            onColumnMove={onColumnMove}
            onCardOpen={(card) => setOpenCard({ id: card.id })}
            trailing={trailing}
          />
        ) : null}
      </div>
      {shownCard ? (
        <CardDetail
          key={shownCard.id}
          card={shownCard}
          columnName={columnsById.get(shownCard.columnId)?.name ?? ""}
          link={resolveLink(shownCard, linkStates)}
          ticket={tickets[shownCard.id] ?? null}
          linkTargets={linkTargets}
          focus={openCard?.focus}
          onClose={() => setOpenCard(null)}
          onOpenLinkedWork={(target) => {
            setOpenCard(null);
            openLinkedWork(target);
          }}
          onError={setError}
        />
      ) : null}
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
