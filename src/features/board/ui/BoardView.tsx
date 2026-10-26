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
  createBoardFromTemplate,
  createCard,
  createColumn,
  createLabel,
  createView,
  deleteArchivedCard,
  deleteBoard,
  deleteColumn,
  deleteView,
  duplicateCard,
  getBoard,
  getCardExtras,
  getCardIndex,
  getCardTickets,
  listArchivedCards,
  listFields,
  listLabels,
  listBoards,
  fieldDistinctValues,
  getBlockedCards,
  getRunningCards,
  getLatestRunEvents,
  listBoardLinks,
  type CardLinkRow,
  type LinkedWorkEvent,
  type CardLinkKind,
  getBoardSettings,
  setFieldValue,
  updateBoardSettings,
  listViews,
  moveCard,
  moveColumn,
  moveCards,
  moveCardToBoard,
  queryBoard,
  renameBoard,
  restoreCard,
  setCardDue,
  setCardLabels,
  setCardPriority,
  setCardsLabel,
  setCardsPriority,
  updateColumnDisplay,
  subscribeBoard,
  updateCard,
  updateColumn,
  updateView,
  type Board,
  type BoardCard,
  type BoardColumn,
  type BoardDetail,
  type BoardField,
  type BoardLabel,
  type BoardSettings as StoredSettings,
  type BoardView as SavedView,
  type CardExtras,
  type CardIndexRow,
  type DisplayOptions,
  type ColumnRole,
  type FilterRow,
  type SortSpec,
  type ViewSpec,
} from "../model/board";
import { fullIndex } from "../model/boardFilter";
import {
  DEFAULT_LAYOUT,
  EMPTY_SPEC,
  activeRows,
  currentSpec,
  liveRows,
  orderedViews,
  parseSpec,
  runningCardIds,
  sameJson,
  sameSpec,
  sortLabel,
  type FilterSources,
} from "../model/views";
import { sortedCardIds, type SortKey } from "../model/columnActions";
import {
  GROUP_BYS,
  buildGroups,
  LAYOUTS,
  TABLE_COLUMNS,
  parseLayout,
  patchLayout,
  tableColumns,
  type LayoutState,
} from "../model/layout";
import {
  EMPTY_SELECTION,
  pruneSelection,
  rangeSelect,
  toggleSelect,
  type Selection,
} from "../model/selection";
import {
  focusedCardId,
  isEditableTarget,
  shortcutFor,
} from "../model/shortcuts";
import { createCardFromTicket, newCardColumn } from "../model/ticketCards";
import {
  archivedLabel,
  createUndo,
  deletedLabel,
  deletedManyLabel,
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
import { BoardList } from "./BoardList";
import { BoardTable } from "./BoardTable";
import { SelectionBar } from "./SelectionBar";
import { BoardColumnHeader } from "./BoardColumnHeader";
import { BoardFilterMenu, BoardSortMenu, FilterChips } from "./BoardFilter";
import { ViewTabs } from "./ViewTabs";
import { BoardSettings } from "./settings/BoardSettings";
import { PriorityContext } from "./priorityScheme";
import { overLimitMessage, parsePriorityScheme } from "../model/boardSettings";
import {
  laneBy,
  resolveDisplay,
  serializeDisplay,
  withLayoutDisplay,
} from "../model/display";
import { buildLanes, laneCells, laneOrder, laneValue } from "../model/swimlanes";
import { fieldText } from "../model/cardProps";
import { DisplayEditor } from "./settings/DisplayEditor";
import { Swimlanes } from "./kanban/Swimlanes";
import { NewBoardDialog } from "./NewBoardDialog";
import { RunWithDialog } from "./RunWithDialog";
import { agentValues, AGENT_FIELD_LABEL } from "../model/agentFields";
import {
  extraKey,
  pilotState,
  runningPairs,
  sessionState,
  settledSince,
  type LinkChipView,
  type LinkExtra,
} from "../model/linkStatus";
import {
  MenuButton,
  MenuItem,
  MenuLabel,
  NameInput,
} from "./boardControls";
import {
  errorText,
  linkValue,
  nextLabelColor,
  resolveLink,
  type BoardLinkTarget,
  type LinkRef,
  type RunRequest,
} from "./boardFormat";
import { CardComposer } from "./CardComposer";
import { loadBoardCardOpen } from "../../settings/model/settings";
import { CardDetail, type CardDrawerHandle } from "./CardDetail";
import { SEARCH_DEBOUNCE_MS } from "../model/constants";
import { NewCardModal } from "./NewCardModal";
import { CardMoreItems, type MoreActions } from "./CardMoreMenu";
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

/**
 * Where each card sits now among its column's live cards, and its number:
 * one pass over the board, so a bulk action on every card stays linear.
 */
function placesIn(
  detail: BoardDetail,
): Map<string, { place: Place; number: number }> {
  const byColumn = new Map<string, BoardCard[]>();
  for (const card of detail.cards) {
    const list = byColumn.get(card.columnId);
    if (list) list.push(card);
    else byColumn.set(card.columnId, [card]);
  }
  const out = new Map<string, { place: Place; number: number }>();
  for (const [columnId, list] of byColumn) {
    list.sort((a, b) => a.position - b.position);
    list.forEach((card, index) =>
      out.set(card.id, { place: [card.id, columnId, index], number: card.number }),
    );
  }
  return out;
}


const dragInProgress = () => !!document.querySelector("[data-dragging]");

/** The card a focus event's target is, or sits in, in any layout. */
function focusedRowOf(target: EventTarget | null): string | null {
  if (!(target instanceof Element)) return null;
  const row = target.closest("[data-kanban-card], [data-card-id], [data-table-row]");
  return (
    row?.getAttribute("data-kanban-card") ??
    row?.getAttribute("data-card-id") ??
    row?.getAttribute("data-table-row") ??
    null
  );
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
  /**
   * Reads the state of PR and worktree links (state, checks, branch, diff
   * stats), keyed by `extraKey`. Called when the board opens and when a
   * linked turn settles, never on a timer.
   */
  lookupLinkStatus?: (
    links: { kind: "pr" | "worktree"; reference: string }[],
  ) => Promise<Record<string, LinkExtra>>;
  /** Starts a session for a card (the app builds it from the store's prompt). */
  onRunWith?: (request: RunRequest) => Promise<void>;
  /** A palette command for the board; `nonce` tells repeats apart. */
  command?: BoardCommand | null;
  /** Called once the board has acted on (or dropped) `command`. */
  onCommandHandled?: (nonce: number) => void;
};

export type BoardCommand = { id: "new-card" | "switch-board"; nonce: number };

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
  lookupLinkStatus,
  onRunWith,
  command,
  onCommandHandled,
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
        lookupLinkStatus={lookupLinkStatus}
        onRunWith={onRunWith}
        command={command}
        onCommandHandled={onCommandHandled}
      />
    </div>
  );
}

function BoardContent({
  linkTargets,
  onOpenLinkedWork,
  lookupLinkTargets,
  projects,
  lookupLinkStatus,
  onRunWith,
  command,
  onCommandHandled,
}: Pick<
  Props,
  | "linkTargets"
  | "onOpenLinkedWork"
  | "lookupLinkTargets"
  | "projects"
  | "lookupLinkStatus"
  | "onRunWith"
  | "command"
  | "onCommandHandled"
>) {
  const [boards, setBoards] = useState<Board[] | null>(null);
  const [openId, setOpenId] = useState<string | null>(null);
  const [detail, setDetail] = useState<BoardDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [naming, setNaming] = useState<"rename" | null>(null);
  const [newBoardOpen, setNewBoardOpen] = useState(false);
  const [addingColumn, setAddingColumn] = useState(false);
  const [tickets, setTickets] = useState<Record<string, TicketRef>>({});
  const [extras, setExtras] = useState<Record<string, CardExtras>>({});
  const [labels, setLabels] = useState<BoardLabel[]>([]);
  // What the search box holds, and the search the board query runs: it
  // catches up `SEARCH_DEBOUNCE_MS` after the last keystroke. Views, Escape
  // and board switches set both at once.
  const [query, setQuery] = useState("");
  const [searched, setSearched] = useState("");
  const [rows, setRows] = useState<FilterRow[]>([]);
  const [sort, setSort] = useState<SortSpec>(EMPTY_SPEC.sort);
  const [layout, setLayout] = useState(DEFAULT_LAYOUT);
  const [fields, setFields] = useState<BoardField[]>([]);
  // Saved views, which one shows (null is "All cards"), and what it looked
  // like when applied or last saved: "unsaved changes" compares against it.
  const [views, setViews] = useState<SavedView[]>([]);
  const [activeView, setActiveView] = useState<string | null>(null);
  const [baseline, setBaseline] = useState({
    spec: EMPTY_SPEC,
    layout: DEFAULT_LAYOUT,
  });
  // Ids the engine matched, in display order; null shows every card.
  const [matched, setMatched] = useState<string[] | null>(null);
  const [ticketCardOpen, setTicketCardOpen] = useState(false);
  const [composerColumn, setComposerColumn] = useState<string | null>(null);
  const [openCard, setOpenCard] = useState<{
    id: string;
    focus?: "title" | "link";
  } | null>(null);
  // The card shown as a full page, if any; opening one follows the setting.
  const [pageCardId, setPageCardId] = useState<string | null>(null);
  const [archiveOpen, setArchiveOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Link status: every link on the board, what blocks which card, each
  // card's latest linked-work event, and what PR and worktree links report.
  const [boardLinks, setBoardLinks] = useState<CardLinkRow[]>([]);
  const [blocked, setBlocked] = useState<Record<string, number[]>>({});
  const [latestEvent, setLatestEvent] = useState<
    Record<string, LinkedWorkEvent>
  >({});
  const [linkExtra, setLinkExtra] = useState<Record<string, LinkExtra>>({});
  const [realRunning, setRealRunning] = useState<string[] | null>(null);
  const [settleTick, setSettleTick] = useState(0);
  const [runFor, setRunFor] = useState<BoardCard | null>(null);
  const [settings, setSettings] = useState<StoredSettings | null>(null);
  // The board whose default view has been looked at: it applies once per open.
  const defaultViewSeen = useRef<string | null>(null);
  const showViewRef = useRef<(view: SavedView | null) => void>(() => {});
  const [newCardOpen, setNewCardOpen] = useState(false);
  const [switchSignal, setSwitchSignal] = useState(0);
  const [selection, setSelection] = useState<Selection>(EMPTY_SELECTION);
  /** The card (board card, list or table row) holding keyboard focus. */
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [cardIndex, setCardIndex] = useState<ReadonlyMap<string, CardIndexRow>>(
    () => new Map(),
  );
  const rootRef = useRef<HTMLDivElement>(null);
  /** The open drawer's handle: Escape from the board, draft flushes. */
  const drawerHandle = useRef<CardDrawerHandle | null>(null);
  /** The card whose drawer shows, as of the last render. */
  const shownCardRef = useRef<BoardCard | null>(null);
  /** The board whose last load failed (cleared when one succeeds). */
  const [loadFailed, setLoadFailed] = useState<string | null>(null);
  /** The card to give focus back to once its drawer has closed. */
  const returnFocus = useRef<string | null>(null);
  const showCard = (next: { id: string; focus?: "title" | "link" }) => {
    setOpenCard(next);
    setPageCardId(loadBoardCardOpen() === "page" ? next.id : null);
  };
  /**
   * Closes the drawer (and its full page). Focus that was in the drawer, or
   * nowhere, goes back to the card; focus elsewhere on the board stays put.
   */
  const closeDrawer = () => {
    const id = openCard?.id ?? null;
    const active = document.activeElement;
    const focusWasInDrawer =
      !active ||
      active === document.body ||
      active === rootRef.current ||
      !!active.closest("[data-card-drawer]");
    setOpenCard(null);
    setPageCardId(null);
    if (id && focusWasInDrawer) {
      returnFocus.current = id;
      // Pinned as the focused card, so it is mounted to take focus.
      setFocusedId(id);
    }
  };

  // The full page belongs to an open card: closing it any way ends it.
  useEffect(() => {
    if (!openCard) setPageCardId(null);
  }, [openCard]);

  const detailRef = useRef(detail);
  const openIdRef = useRef(openId);
  const boardsRef = useRef(boards);
  useLayoutEffect(() => {
    detailRef.current = detail;
    openIdRef.current = openId;
    boardsRef.current = boards;
  });
  const request = useRef(0);
  // What the board shows now, for callbacks that outlive a render.
  const live = useRef({ query, rows, sort, layout, activeView, baseline });
  useLayoutEffect(() => {
    live.current = { query, rows, sort, layout, activeView, baseline };
  });
  const applySpec = useCallback((spec: ViewSpec, nextLayout: string) => {
    setQuery(spec.query);
    setSearched(spec.query);
    setRows(spec.rows);
    setSort(spec.sort);
    setLayout(nextLayout);
  }, []);

  /**
   * Takes the saved views a reload read. A showing view that changed in the
   * other app is re-applied if untouched here; one with local edits keeps
   * them (and its dot). A showing view that is gone leaves what is on screen
   * in place under "All cards".
   */
  const syncViews = useCallback(
    (list: SavedView[]) => {
      const now = live.current;
      const dirty =
        !sameSpec(currentSpec(now), now.baseline.spec) ||
        !sameJson(now.layout, now.baseline.layout);
      setViews(list);
      if (!now.activeView) return;
      const found = list.find((view) => view.id === now.activeView);
      if (!found) {
        setActiveView(null);
        setBaseline({ spec: EMPTY_SPEC, layout: DEFAULT_LAYOUT });
        return;
      }
      const spec = parseSpec(found.specJson);
      if (
        !sameSpec(spec, now.baseline.spec) ||
        !sameJson(found.layoutJson, now.baseline.layout)
      ) {
        setBaseline({ spec, layout: found.layoutJson });
        if (!dirty) applySpec(spec, found.layoutJson);
      }
    },
    [applySpec],
  );

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
    (boardId: string, label: string, action: UndoAction | null) => {
      if (!mounted.current) return;
      if (openIdRef.current === boardId) undo.push(label, action);
      else if (action?.kind === "delete") {
        void deleteArchivedCard(action.cardId).catch(
          fail("Couldn't delete the card"),
        );
      } else if (action?.kind === "deleteMany") {
        for (const [cardId] of action.items) {
          void deleteArchivedCard(cardId).catch(
            fail("Couldn't delete the card"),
          );
        }
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
      const [
        next,
        linked,
        cardExtras,
        boardLabels,
        boardFields,
        boardViews,
        boardSettings,
        links,
        blockers,
        runEvents,
      ] = await Promise.all([
          getBoard(id),
          // A failed ticket read leaves the cards without chips, not the board unloaded.
          getCardTickets(id).catch(() => null),
          // Likewise for the card details and labels: the cards show without them.
          getCardExtras(id).catch(() => null),
          listLabels(id).catch(() => null),
          listFields(id).catch(() => null),
          listViews(id).catch(() => null),
          // Without them the board shows with its defaults.
          getBoardSettings(id).catch(() => null),
          // Chips, locks and failed marks are left as they were if these fail.
          listBoardLinks(id).catch(() => null),
          getBlockedCards(id).catch(() => null),
          // Each card's newest run event, board-wide, however old it is.
          getLatestRunEvents(id).catch(() => null),
        ]);
      if (ticket !== request.current || openIdRef.current !== id) return;
      setDetail(next);
      setLoadFailed(null);
      if (linked) setTickets(linked);
      if (cardExtras) setExtras(cardExtras);
      if (boardLabels) setLabels(boardLabels);
      if (boardFields) setFields(boardFields);
      if (boardViews) syncViews(boardViews);
      if (boardSettings) setSettings(boardSettings);
      if (links) setBoardLinks(links);
      if (blockers) setBlocked(blockers);
      if (runEvents) setLatestEvent(runEvents);
      if (defaultViewSeen.current !== id) {
        defaultViewSeen.current = id;
        const wanted = boardViews?.find(
          (view) => view.id === boardSettings?.defaultViewId,
        );
        // A default that is gone, or one the user already moved off, is skipped.
        if (wanted && !live.current.activeView) showViewRef.current(wanted);
      }
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
      setLoadFailed(id);
      fail("Couldn't load the board")(cause);
    }
  }, [fail, syncViews]);

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
    setFields([]);
    // So are its views, search, filter and sort.
    setViews([]);
    setSettings(null);
    setBoardLinks([]);
    setBlocked({});
    setLatestEvent({});
    setLinkExtra({});
    setRealRunning(null);
    setRunFor(null);
    setSettingsOpen(false);
    defaultViewSeen.current = null;
    setActiveView(null);
    setBaseline({ spec: EMPTY_SPEC, layout: DEFAULT_LAYOUT });
    applySpec(EMPTY_SPEC, DEFAULT_LAYOUT);
    setMatched(null);
    setSelection(EMPTY_SELECTION);
    setCardIndex(new Map());
    if (!openId) return;
    void refresh();
    return subscribeBoard(openId, () => void refresh());
  }, [openId, refresh, undo, applySpec]);

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
    // The cards' other session and Pilot links need their targets too.
    for (const link of boardLinks) {
      if (link.kind !== "session" && link.kind !== "pilot") continue;
      const value = linkValue(link.kind, link.reference);
      if (targetsByValue.has(value)) asked.current.delete(value);
      if (targetsByValue.has(value) || asked.current.has(value)) continue;
      wanted.set(value, { kind: link.kind, id: link.reference });
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
  }, [detail, boardLinks, targetsByValue, lookupLinkTargets, fail]);

  // What the app has loaded wins over a lookup: it carries live status.
  const linkStates = useMemo(() => {
    const states = new Map<string, BoardLinkTarget | null>(lookedUp);
    for (const [value, target] of targetsByValue) states.set(value, target);
    return states;
  }, [lookedUp, targetsByValue]);

  const scheme = useMemo(
    () => parsePriorityScheme(settings?.priorityJson ?? null),
    [settings?.priorityJson],
  );
  const sources: FilterSources = useMemo(
    () => ({
      columns: detail?.columns ?? [],
      labels,
      projects,
      fields,
      priorities: scheme,
    }),
    [detail?.columns, labels, projects, fields, scheme],
  );
  // A label, column, project or field that is gone leaves the filter, so it
  // can't hide every card behind a choice nobody can see or undo. A row left
  // with nothing filters nothing.
  const effectiveRows = useMemo(
    () => liveRows(activeRows(rows), sources),
    [rows, sources],
  );
  const sorted = sort.key !== "manual";
  // Typing settles first: one board query per pause, not one per keystroke.
  useEffect(() => {
    if (searched === query) return;
    const timer = setTimeout(() => setSearched(query), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query, searched]);
  const narrowing = searched.trim() !== "" || effectiveRows.length > 0;

  // The cards whose linked work is running now. The store answers it for
  // every link on every card; the primary links alone stand in until it has.
  const runningNow = useMemo(
    () => runningPairs([...linkStates.values()].flatMap((t) => (t ? [t] : []))),
    [linkStates],
  );
  const runningKey = useMemo(() => JSON.stringify(runningNow), [runningNow]);
  const fallbackRunning = useMemo(
    () =>
      runningCardIds(
        detail?.cards ?? [],
        (kind, id) =>
          kind !== "flow-run" &&
          ["running", "waiting"].includes(
            linkStates.get(linkValue(kind, id))?.status ?? "",
          ),
      ),
    [detail, linkStates],
  );
  const runningIds = realRunning ?? fallbackRunning;
  useEffect(() => {
    if (!openId || detail?.board.id !== openId) return;
    let stale = false;
    getRunningCards(openId, JSON.parse(runningKey))
      .then((ids) => {
        if (!stale) setRealRunning(ids);
      })
      // The primary links stand in.
      .catch(() => {
        if (!stale) setRealRunning(null);
      });
    return () => {
      stale = true;
    };
  }, [openId, detail, runningKey]);

  // A linked turn settling (something that ran no longer does) re-reads the
  // board's link status once. Nothing here polls.
  // Only work linked to this board's cards counts: some other session in the
  // app settling is no reason to read this board (or call `gh`) again.
  const boardWork = useMemo(() => {
    const held = new Set<string>();
    for (const card of detail?.cards ?? []) {
      if (card.linkKind && card.linkId) held.add(linkValue(card.linkKind, card.linkId));
    }
    for (const link of boardLinks) {
      if (link.kind === "session" || link.kind === "pilot") {
        held.add(linkValue(link.kind, link.reference));
      }
    }
    return held;
  }, [detail, boardLinks]);
  // Each card's links and each card by id, built once per read: the card
  // faces look theirs up instead of scanning every link for every card.
  const linksByCard = useMemo(() => {
    const out = new Map<string, CardLinkRow[]>();
    for (const link of boardLinks) {
      const list = out.get(link.cardId);
      if (list) list.push(link);
      else out.set(link.cardId, [link]);
    }
    return out;
  }, [boardLinks]);
  const cardsById = useMemo(
    () => new Map((detail?.cards ?? []).map((card) => [card.id, card])),
    [detail],
  );
  // Every column's live card count, in one pass over the board.
  const columnCounts = useMemo(() => {
    const out = new Map<string, number>();
    for (const card of detail?.cards ?? []) {
      out.set(card.columnId, (out.get(card.columnId) ?? 0) + 1);
    }
    return out;
  }, [detail]);
  const boardRunning = useMemo(
    () => runningNow.filter(([kind, id]) => boardWork.has(linkValue(kind, id))),
    [runningNow, boardWork],
  );
  const lastRunning = useRef<[CardLinkKind, string][]>([]);
  useEffect(() => {
    if (settledSince(lastRunning.current, boardRunning)) {
      setSettleTick((tick) => tick + 1);
      void refresh();
    }
    lastRunning.current = boardRunning;
  }, [boardRunning, refresh]);

  // PR and worktree status: on open, on a changed set of such links, and on settle.
  const lookupStatusRef = useRef(lookupLinkStatus);
  lookupStatusRef.current = lookupLinkStatus;
  const statusLinks = useMemo(
    () =>
      boardLinks.flatMap((link) =>
        link.kind === "pr" || link.kind === "worktree"
          ? [{ kind: link.kind as "pr" | "worktree", reference: link.reference }]
          : [],
      ),
    [boardLinks],
  );
  const statusKey = useMemo(() => JSON.stringify(statusLinks), [statusLinks]);
  useEffect(() => {
    const lookup = lookupStatusRef.current;
    const links = JSON.parse(statusKey) as typeof statusLinks;
    if (!lookup || !openId || !links.length) return;
    let stale = false;
    lookup(links).then(
      (found) => {
        if (!stale) setLinkExtra(found);
      },
      // The chips keep what they had (unknown at first).
      () => undefined,
    );
    return () => {
      stale = true;
    };
  }, [openId, statusKey, settleTick]);

  // Matching, filtering and ordering all come from the board query. A newer
  // request, a board switch or a new board read wins over a late answer.
  const spec: ViewSpec = useMemo(
    () => ({ query: searched.trim(), rows: effectiveRows, sort }),
    [searched, effectiveRows, sort],
  );
  const specKey = useMemo(
    () => JSON.stringify([spec, runningIds]),
    [spec, runningIds],
  );
  const specRef = useRef({ spec, runningIds });
  useLayoutEffect(() => {
    specRef.current = { spec, runningIds };
  });
  const queryTicket = useRef(0);
  useEffect(() => {
    const ticket = ++queryTicket.current;
    if (!openId || detail?.board.id !== openId) return;
    const { spec: asked, runningIds: running } = specRef.current;
    if (!asked.query && !asked.rows.length && asked.sort.key === "manual") {
      setMatched(null);
      return;
    }
    queryBoard(openId, asked, running)
      .then((ids) => {
        if (ticket === queryTicket.current) setMatched(ids);
      })
      .catch((cause) => {
        if (ticket !== queryTicket.current) return;
        setMatched(null);
        fail("Couldn't search the board")(cause);
      });
    // `detail` re-runs it on every board change; `specKey` on every edit.
  }, [openId, detail, specKey, fail]);

  const matchedSet = useMemo(() => matched && new Set(matched), [matched]);
  const rank = useMemo(
    () => new Map((matched ?? []).map((id, index) => [id, index])),
    [matched],
  );
  const matches = useCallback(
    (card: BoardCard) => !narrowing || !matchedSet || matchedSet.has(card.id),
    [narrowing, matchedSet],
  );
  // The board shows only matching cards, in the sort's order. Its drop
  // indexes count those, so `onCardMove` maps them back to places among all
  // the column's cards.
  const shownDetail = useMemo(() => {
    if (!detail || !matched || !matchedSet) return detail;
    const cards: BoardCard[] = [];
    for (const card of detail.cards) {
      const ranked = rank.get(card.id);
      if (ranked === undefined && narrowing) continue;
      cards.push(
        sorted
          ? { ...card, position: ranked ?? matched.length + card.position }
          : card,
      );
    }
    return { ...detail, cards };
  }, [detail, matched, matchedSet, rank, narrowing, sorted]);
  const matchesRef = useRef(matches);
  const narrowingRef = useRef(narrowing);
  const sortedRef = useRef(sorted);
  useLayoutEffect(() => {
    matchesRef.current = matches;
    narrowingRef.current = narrowing;
    sortedRef.current = sorted;
  });

  const layoutState = useMemo(() => parseLayout(layout), [layout]);
  // The board's display, with the showing view's override over it. "All
  // cards" has no override, so it draws with the board default.
  const display = useMemo(
    () =>
      resolveDisplay(settings?.displayJson ?? null, activeView ? layout : null),
    [settings?.displayJson, activeView, layout],
  );
  // Quick Display edits build on the latest display, saved or not, and write
  // in order, so two toggles in a row can't undo each other.
  const displayBase = useRef(display);
  const displayPending = useRef(0);
  const displayQueue = useRef<Promise<unknown>>(Promise.resolve());
  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  useEffect(() => {
    if (displayPending.current === 0) displayBase.current = display;
  }, [display]);
  const lanesBy = useMemo(() => laneBy(display, fields), [display, fields]);
  const cardFieldValues = useMemo(() => {
    const out: Record<string, Record<string, string>> = {};
    for (const [id, row] of cardIndex) out[id] = row.fields;
    return out;
  }, [cardIndex]);
  const lanes = useMemo(
    () =>
      lanesBy && shownDetail
        ? buildLanes({
            by: lanesBy,
            cards: shownDetail.cards,
            extras,
            labels,
            projects,
            fieldValues: cardFieldValues,
            priorities: scheme,
          })
        : [],
    [lanesBy, shownDetail, extras, labels, projects, cardFieldValues, scheme],
  );
  const editLayout = useCallback(
    (patch: (current: LayoutState) => Partial<LayoutState>) =>
      setLayout((json) => patchLayout(json, patch(parseLayout(json)))),
    [],
  );

  // The cards on screen, in the order the board query put them: what a Shift
  // range and the bulk actions follow in every layout.
  const visibleCards = useMemo(() => {
    if (!detail) return [];
    const byId = new Map(detail.cards.map((card) => [card.id, card]));
    const order = new Map(detail.columns.map((col) => [col.id, col.position]));
    const manual = [...detail.cards].sort(
      (a, b) =>
        (order.get(a.columnId) ?? 0) - (order.get(b.columnId) ?? 0) ||
        a.position - b.position,
    );
    if (!matched) return manual;
    const ranked = matched.flatMap((id) => byId.get(id) ?? []);
    if (narrowing) return ranked;
    // A card the last query predates still shows, after the ranked ones.
    const seen = new Set(ranked.map((card) => card.id));
    return [...ranked, ...manual.filter((card) => !seen.has(card.id))];
  }, [detail, matched, narrowing]);
  const selectedSet = useMemo(() => new Set(selection.ids), [selection.ids]);

  // The cards on screen in the order they appear, which differs by layout:
  // the board reads column by column (collapsed columns hide their cards),
  // the list group by group (collapsed groups hide theirs; a card in two
  // groups counts once, at its first place), the table row by row. Ranges,
  // pruning and bulk actions all follow it.
  const displayOrder = useMemo(() => {
    if (!detail) return [];
    if (layoutState.layout === "table") return visibleCards;
    if (layoutState.layout === "list") {
      const byId = new Map(visibleCards.map((card) => [card.id, card]));
      const seen = new Set<string>();
      const out: BoardCard[] = [];
      for (const group of buildGroups({
        by: layoutState.groupBy,
        cards: visibleCards,
        columns: detail.columns,
        extras,
        labels,
        projects,
      })) {
        if (layoutState.collapsed.includes(group.key)) continue;
        for (const id of group.cardIds) {
          if (seen.has(id)) continue;
          seen.add(id);
          out.push(byId.get(id)!);
        }
      }
      return out;
    }
    const shown = shownDetail ?? detail;
    const columns = [...shown.columns]
      .sort((a, b) => a.position - b.position)
      .filter((column) => !column.collapsed);
    if (lanesBy) {
      // Swimlanes read lane by lane, then column by column inside a lane.
      const byId = new Map(shown.cards.map((card) => [card.id, card]));
      return laneOrder(
        lanes,
        laneCells(lanes, shown.cards),
        columns.map((column) => column.id),
      ).flatMap((id) => byId.get(id) ?? []);
    }
    return columns.flatMap((column) => columnCards(shown, column.id));
  }, [
    detail,
    shownDetail,
    visibleCards,
    layoutState,
    extras,
    labels,
    projects,
    lanesBy,
    lanes,
  ]);
  const displayOrderRef = useRef(displayOrder);
  useLayoutEffect(() => {
    displayOrderRef.current = displayOrder;
  });

  // A selection holds only cards on screen: a filter, search or view change
  // that hides some drops them, so "N selected" and bulk actions match it.
  useEffect(() => {
    if (!detail) return;
    const shown = new Set(displayOrder.map((card) => card.id));
    const exists = new Set(detail.cards.map((card) => card.id));
    setSelection((current) => pruneSelection(current, shown, exists));
  }, [detail, displayOrder]);

  const isTable = layoutState.layout === "table";
  // Field values come from the card index: the table, the field chips on
  // faces and a field-lane board all need it.
  const needsIndex =
    isTable ||
    display.face.fields.length > 0 ||
    lanesBy?.kind === "field";
  useEffect(() => {
    if (!needsIndex || !openId || detail?.board.id !== openId) return;
    let stale = false;
    getCardIndex(openId)
      .then((rows) => {
        if (!stale) setCardIndex(new Map(rows.map((row) => [row.cardId, row])));
      })
      // The columns that need it stay empty; the rest of the table shows,
      // and the error says why.
      .catch((cause) => {
        if (!stale) fail("Couldn't load the card fields")(cause);
      });
    return () => {
      stale = true;
    };
  }, [needsIndex, openId, detail, fail]);

  /** ⌘/Ctrl-click toggles a card, Shift-click selects a range; true when it was one. */
  const selectClick = (
    event: { metaKey: boolean; ctrlKey: boolean; shiftKey: boolean },
    card: BoardCard,
  ): boolean => {
    if (IS_MAC ? event.metaKey : event.ctrlKey) {
      setSelection((current) => toggleSelect(current, card.id));
      return true;
    }
    if (event.shiftKey) {
      const order = displayOrderRef.current.map((other) => other.id);
      setSelection((current) => rangeSelect(current, card.id, order));
      return true;
    }
    return false;
  };

  const onCardMove = useCallback(
    /**
     * `undoable: false` shows the move's toast without an Undo: a lane drop
     * that also set a priority or field value can't be undone by moving the
     * card back alone.
     */
    (cardId: string, shown: DropTarget, undoable = true) => {
      const before = detailRef.current;
      if (!before) return;
      let to = shown;
      if (sortedRef.current) {
        // Cards sit where the sort puts them: a drop inside a column moves
        // nothing, and one into another column goes to the end of it.
        if (
          before.cards.find((card) => card.id === cardId)?.columnId ===
          shown.columnId
        ) {
          return;
        }
        to = {
          columnId: shown.columnId,
          index: columnCards(before, shown.columnId).filter(
            (card) => card.id !== cardId,
          ).length,
        };
      } else if (narrowingRef.current) {
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
          const moved = movedLabel([from.number], column.name);
          // A move into a column past its limit still happens; it says so.
          const total =
            columnCards(before, to.columnId).filter(
              (other) => other.id !== cardId,
            ).length + 1;
          const over =
            fromColumn !== to.columnId &&
            column.wipLimit != null &&
            total > column.wipLimit;
          const label = over
            ? `${overLimitMessage(total, column.wipLimit!)} · ${moved}`
            : moved;
          pushUndo(
            before.board.id,
            label,
            undoable
              ? { kind: "move", cardId, columnId: fromColumn, index: fromIndex }
              : null,
          );
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

  const createNamedBoard = (name: string, templateId: string) => {
    setNewBoardOpen(false);
    createBoardFromTemplate(name, templateId)
      .then((board) => loadBoards(board.id))
      .catch(fail("Couldn't create the board"));
  };

  const renameOpenBoard = (board: Board, name: string) =>
    renameBoard(board.id, name)
      .then(() => loadBoards(board.id))
      .catch(fail("Couldn't rename the board"));

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

  /** Archives a card from the More menu: off the board now, with an Undo. */
  const archiveOne = (card: BoardCard) => {
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
      .then(() =>
        pushUndo(before.board.id, archivedLabel([card.number]), {
          kind: "archive",
          items: [from.place],
        }),
      )
      .catch((cause) => {
        fail(`Couldn't archive card #${card.number}`)(cause);
        void refresh();
      });
  };

  /** Copies a card right after itself; `flush` saves a drawer's drafts first. */
  const duplicateOne = (card: BoardCard, flush?: () => Promise<void>) => {
    void undo.commitPending();
    void (flush ? flush() : Promise.resolve())
      .then(() => duplicateCard(card.id))
      .then((copy) => {
        undo.notify(`Duplicated #${card.number} as #${copy.number}`);
      })
      .catch(fail(`Couldn't duplicate card #${card.number}`));
  };

  const moveToBoard = (
    card: BoardCard,
    targetId: string,
    flush?: () => Promise<void>,
  ) => {
    const target = boardsRef.current?.find((board) => board.id === targetId);
    void undo.commitPending();
    void (flush ? flush() : Promise.resolve())
      .then(() => moveCardToBoard(card.id, targetId))
      .then((moved) => {
        setOpenCard((open) => (open?.id === card.id ? null : open));
        undo.notify(
          `Moved #${card.number} to ${target?.name ?? "the board"} as #${moved.number}`,
        );
        void refresh();
      })
      .catch(fail(`Couldn't move card #${card.number} to another board`));
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
    const places = current ? placesIn(current) : null;
    const found = places ? ids.flatMap((id) => places.get(id) ?? []) : [];
    return {
      items: found.map((entry) => entry.place),
      numbers: found.map((entry) => entry.number),
    };
  };

  // A patch: collapsing never resets a colour or WIP limit the other app
  // set since this board last loaded.
  const setCollapsed = (column: BoardColumn, collapsed: boolean) =>
    void updateColumnDisplay(column.id, { collapsed }).catch(
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

  /** The selected cards in the order shown. */
  const selectedCards = () => {
    const chosen = new Set(selection.ids);
    return displayOrderRef.current.filter((card) => chosen.has(card.id));
  };

  const bulkStatus = (column: BoardColumn) => {
    const moving = selectedCards().filter((card) => card.columnId !== column.id);
    if (!moving.length) return;
    const ids = moving.map((card) => card.id);
    const movingIds = new Set(ids);
    const { items, numbers } = placesOf(ids);
    const staying = wholeColumn(column.id).filter(
      (card) => !movingIds.has(card.id),
    ).length;
    void undo.commitPending();
    moveCards(ids, column.id, staying, "user")
      .then(() =>
        pushUndo(column.boardId, movedLabel(numbers, column.name), {
          kind: "moveMany",
          items,
        }),
      )
      .catch(fail(`Couldn't move the cards to ${column.name}`));
  };

  const bulkPriority = (priority: number) => {
    const ids = selectedCards().map((card) => card.id);
    if (ids.length) {
      void setCardsPriority(ids, priority).catch(
        fail("Couldn't set the priority"),
      );
    }
  };

  const bulkLabel = (labelId: string, on: boolean) => {
    const ids = selectedCards().map((card) => card.id);
    if (ids.length) {
      void setCardsLabel(ids, labelId, on).catch(fail("Couldn't set the label"));
    }
  };

  /** Takes the selected cards off the board now, then holds or archives them. */
  const bulkRemove = (hold: boolean) => {
    const before = detailRef.current;
    const ids = selectedCards().map((card) => card.id);
    if (!before || !ids.length) return;
    const { items, numbers } = placesOf(ids);
    if (!items.length) return;
    void undo.commitPending();
    const removing = new Set(ids);
    setSelection(EMPTY_SELECTION);
    setOpenCard((open) => (open && removing.has(open.id) ? null : open));
    setDetail({
      ...before,
      cards: before.cards.filter((card) => !removing.has(card.id)),
    });
    archiveCards(ids)
      .then(() =>
        pushUndo(
          before.board.id,
          hold ? deletedManyLabel(numbers) : archivedLabel(numbers),
          hold ? { kind: "deleteMany", items } : { kind: "archive", items },
        ),
      )
      .catch((cause) => {
        fail(hold ? "Couldn't delete the cards" : "Couldn't archive the cards")(
          cause,
        );
        void refresh();
      });
  };

  const dirty =
    activeView !== null &&
    (!sameSpec(currentSpec({ query, rows, sort }), baseline.spec) ||
      !sameJson(layout, baseline.layout));

  /** Shows a saved view, or "All cards" for null. Unsaved changes are dropped. */
  const showView = (view: SavedView | null) => {
    const next = view ? parseSpec(view.specJson) : EMPTY_SPEC;
    const nextLayout = view?.layoutJson ?? DEFAULT_LAYOUT;
    setActiveView(view?.id ?? null);
    setBaseline({ spec: next, layout: nextLayout });
    applySpec(next, nextLayout);
  };
  showViewRef.current = showView;

  /**
   * Runs a view write, then reads the board again. A read already in flight
   * may predate the write, so it is dropped. Null when the write failed.
   */
  const viewWrite = async <T,>(
    what: string,
    run: () => Promise<T>,
  ): Promise<T | null> => {
    request.current += 1;
    try {
      return await run();
    } catch (cause) {
      fail(what)(cause);
      return null;
    } finally {
      void refresh();
    }
  };

  /** Creates a view and shows it, even before the reload lists it. */
  const adoptNewView = (
    boardId: string,
    what: string,
    create: () => Promise<SavedView>,
  ) =>
    void viewWrite(what, create).then((view) => {
      if (!view || openIdRef.current !== boardId) return;
      setViews((list) => [
        ...list.filter((other) => other.id !== view.id),
        view,
      ]);
      showView(view);
    });

  const saveViewAs = (name: string) => {
    if (!openId) return;
    adoptNewView(openId, "Couldn't save the view", () =>
      createView(
        openId,
        name,
        JSON.stringify(currentSpec({ query, rows, sort })),
        layout,
      ),
    );
  };

  const saveViewChanges = () => {
    const id = activeView;
    if (!id) return;
    const saved = currentSpec({ query, rows, sort });
    const savedLayout = layout;
    void viewWrite("Couldn't save the view", () =>
      updateView(id, {
        specJson: JSON.stringify(saved),
        layoutJson: savedLayout,
      }),
    ).then((done) => {
      if (done !== null) setBaseline({ spec: saved, layout: savedLayout });
    });
  };

  const revertView = () =>
    showView(views.find((view) => view.id === activeView) ?? null);

  const renameView = (id: string, name: string) =>
    void viewWrite("Couldn't rename the view", () => updateView(id, { name }));

  const duplicateView = (id: string) => {
    const source = views.find((view) => view.id === id);
    if (!source || !openId) return;
    // The showing view's unsaved edits go into the copy; the original keeps
    // its stored version. The copy is shown, so nothing is left dirty.
    const editing = id === activeView;
    adoptNewView(openId, "Couldn't duplicate the view", () =>
      createView(
        openId,
        `${source.name} copy`,
        editing
          ? JSON.stringify(currentSpec({ query, rows, sort }))
          : source.specJson,
        editing ? layout : source.layoutJson,
      ),
    );
  };

  const removeView = (id: string) =>
    void viewWrite("Couldn't delete the view", () => deleteView(id)).then(
      (done) => {
        if (done !== null && live.current.activeView === id) showView(null);
      },
    );

  const toast = (
    <>
      <UndoToast toast={undoToast} onUndo={undoLast} />
      {error ? (
        <FileActionError message={error} onDismiss={() => setError(null)} />
      ) : null}
    </>
  );

  // Keyboard shortcuts (N, /, E, Delete). The handler reads the latest state
  // through a ref so the window listener is added once.
  const shortcutRef = useRef<(event: KeyboardEvent) => void>(() => {});
  shortcutRef.current = (event) => {
    const root = rootRef.current;
    const shortcut = shortcutFor(event);
    if (!root || !shortcut || !detail) return;
    const target = event.target;
    // A hidden board (an inert tab) never reacts.
    if (root.closest("[inert]")) return;
    if (isEditableTarget(target) || isEditableTarget(document.activeElement))
      return;
    const inBoard = target instanceof Node && root.contains(target);
    const onPage =
      target === document.body || target === document.documentElement;
    if (!inBoard && !onPage) return;
    if (
      newCardOpen ||
      ticketCardOpen ||
      archiveOpen ||
      naming ||
      newBoardOpen ||
      settingsOpen ||
      addingColumn ||
      composerColumn ||
      dragInProgress() ||
      document.querySelector(
        '[role="dialog"], [role="menu"], [role="listbox"], [data-dialog-popover]',
      )
    )
      return;
    // Behind an open drawer only the board itself keeps the keys.
    const drawerOpen =
      !!openCard && detail.cards.some((card) => card.id === openCard.id);
    if (
      drawerOpen &&
      (!inBoard ||
        (target instanceof Element && target.closest("[data-card-drawer]")))
    )
      return;
    const cardId = focusedCardId(target);
    const card = cardId ? detail.cards.find((c) => c.id === cardId) : undefined;
    if (shortcut === "new") {
      event.preventDefault();
      setNewCardOpen(true);
    } else if (shortcut === "search") {
      const search = root.querySelector<HTMLInputElement>(
        'input[aria-label="Search cards"]',
      );
      if (!search) return;
      event.preventDefault();
      search.focus();
    } else if (shortcut === "open") {
      if (!card) return;
      event.preventDefault();
      showCard({ id: card.id });
    } else if (selection.ids.length) {
      event.preventDefault();
      bulkRemove(true);
    } else if (card) {
      event.preventDefault();
      removeCard(card);
    }
  };
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => shortcutRef.current(event);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The switcher signal is one-shot: the header's select reads it in its own
  // effect (which runs first), then it is cleared. A signal that arrives while
  // the select is unmounted (a rename) is dropped, never kept for later.
  useEffect(() => {
    if (switchSignal > 0) setSwitchSignal(0);
  }, [switchSignal]);

  // A palette command: act once the board it needs has loaded.
  const handledCommand = useRef<number | null>(null);
  useEffect(() => {
    if (!command || handledCommand.current === command.nonce) return;
    if (!boards) return;
    // A board that failed to load drops a "new card" command rather than
    // keeping it to fire whenever the board loads later.
    const boardFailed = !!openId && loadFailed === openId;
    if (boards.length && command.id === "new-card" && !detail && !boardFailed) {
      return;
    }
    handledCommand.current = command.nonce;
    if (boards.length && !(command.id === "new-card" && !detail)) {
      if (command.id === "new-card") setNewCardOpen(true);
      else setSwitchSignal((n) => n + 1);
    }
    onCommandHandled?.(command.nonce);
  }, [command, boards, detail, openId, loadFailed, onCommandHandled]);

  // Focus goes back to the card whose drawer just closed, scrolled into
  // view (it is pinned as the focused card, so it is mounted).
  useLayoutEffect(() => {
    const id = returnFocus.current;
    if (!id || openCard) return;
    returnFocus.current = null;
    const value = id.replace(/["\\]/g, "\\$&");
    const element = rootRef.current?.querySelector<HTMLElement>(
      `[data-kanban-card="${value}"], [data-card-id="${value}"], [data-table-row="${value}"]`,
    );
    if (!element) return;
    element.scrollIntoView?.({ block: "nearest" });
    element.focus({ preventScroll: true });
  });

  if (!boards) return toast;

  if (!boards.length) {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3">
        <span className="text-[13px] text-content/60">No boards yet</span>
        <button
          type="button"
          className={BUTTON}
          onClick={() => setNewBoardOpen(true)}
        >
          Create board
        </button>
        {newBoardOpen ? (
          <NewBoardDialog
            onCreate={createNamedBoard}
            onClose={() => setNewBoardOpen(false)}
          />
        ) : null}
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
  shownCardRef.current = shownCard ?? null;

  const labelsOf = (cardId: string) => {
    const ids = extras[cardId]?.labels;
    return ids?.length ? labels.filter((label) => ids.includes(label.id)) : [];
  };

  /** The More menu of a card; `flush` is the drawer's draft save. */
  const moreItemsFor =
    (card: BoardCard) =>
    (
      close: () => void,
      drawerFlush?: () => Promise<void>,
      dropDrafts?: () => void,
    ) => {
      // From the card face (a right-click, say) while this card's drawer is
      // open, its typed drafts are saved first all the same.
      const flush =
        drawerFlush ??
        (openCard?.id === card.id
          ? () => drawerHandle.current?.flushDrafts() ?? Promise.resolve()
          : undefined);
      const mine = extras[card.id];
      const columnsOf = (detail?.columns ?? []).map((column) => ({
        id: column.id,
        name: column.name,
        cardCount: wholeColumn(column.id).length,
      }));
      const actions: MoreActions = {
        open: () => showCard({ id: card.id }),
        setStatus: (columnId, count) => moveFromDrawer(card, columnId, count),
        setPriority: (priority) =>
          void setCardPriority(card.id, priority).catch(
            fail(`Couldn't set the priority of card #${card.number}`),
          ),
        setLabels: (ids) =>
          void setCardLabels(card.id, ids).catch(
            fail(`Couldn't set the labels of card #${card.number}`),
          ),
        createLabel: (name) =>
          void createLabel(card.boardId, name, nextLabelColor(labels.length))
            .then((created) =>
              setCardLabels(card.id, [...(mine?.labels ?? []), created.id]),
            )
            .catch(fail(`Couldn't create a label for card #${card.number}`)),
        moveToBoard: (targetId) => moveToBoard(card, targetId, flush),
        duplicate: () => duplicateOne(card, flush),
        archive: () =>
          void (flush ? flush() : Promise.resolve()).then(() =>
            archiveOne(card),
          ),
        remove: () => {
          dropDrafts?.();
          removeCard(card);
        },
        onError: setError,
      };
      return (
        <CardMoreItems
          card={card}
          columns={columnsOf}
          labels={labels}
          labelIds={mine?.labels ?? []}
          priority={mine?.priority ?? 0}
          boards={boards}
          actions={actions}
          close={close}
        />
      );
    };

  /** The custom fields the display lists that this card holds a value for. */
  const fieldChipsOf = (cardId: string) =>
    display.face.fields.flatMap((id) => {
      const field = fields.find((found) => found.id === id);
      const raw = cardIndex.get(cardId)?.fields[id];
      const text = field && raw !== undefined ? fieldText(field.kind, raw) : null;
      return field && text ? [{ id, name: field.name, text }] : [];
    });

  const basenameOf = (path: string) => {
    const parts = path.split("/").filter(Boolean);
    return parts[parts.length - 1] ?? path;
  };

  /** The link of a card as its chip shows it: a failed latest event marks finished work. */
  const linkOf = (card: BoardCard) => {
    const link = resolveLink(card, linkStates);
    const target = link?.target;
    return link &&
      target &&
      target.kind === "session" &&
      target.status === "completed" &&
      latestEvent[card.id] === "failed"
      ? { ...link, target: { ...target, status: "failed" as const } }
      : link;
  };

  /** One chip for each link besides the primary one. */
  const linkChipsOf = (card: BoardCard): LinkChipView[] =>
    (linksByCard.get(card.id) ?? []).flatMap((link): LinkChipView[] => {
      if (link.role === "primary") return [];
      // An extra link that is also the primary shows once, as the primary.
      if (link.kind === card.linkKind && link.reference === card.linkId) {
        return [];
      }
      if (link.kind === "pr" || link.kind === "worktree") {
        const found = linkExtra[extraKey(link)];
        const base = {
          kind: link.kind,
          reference: link.reference,
          state: found?.state ?? "unknown",
          checks: found?.checks,
          diff: found?.diff,
        } as const;
        return [
          {
            ...base,
            label:
              link.kind === "worktree"
                ? (found?.branch ?? basenameOf(link.reference))
                : (link.reference.match(/\/pull\/(\d+)/)?.[1]
                    ? `PR #${link.reference.match(/\/pull\/(\d+)/)![1]}`
                    : "PR"),
          },
        ];
      }
      if (link.kind !== "session" && link.kind !== "pilot") return [];
      const target = linkStates.get(linkValue(link.kind, link.reference));
      const state =
        target === null
          ? "missing"
          : link.kind === "pilot"
            ? pilotState(target?.status === "running")
            : sessionState({
                running: target?.status === "running",
                waiting: target?.status === "waiting",
                failed: latestEvent[card.id] === "failed",
              });
      return [
        {
          kind: link.kind,
          reference: link.reference,
          label: target?.label ?? (link.kind === "pilot" ? "Pilot" : "Session"),
          state,
        },
      ];
    });

  /** The sessions linked to a card (primary and extra), as the app knows them. */
  const agentFactsOf = (card: BoardCard) => {
    const ids = new Set<string>();
    if (card.linkKind === "session" && card.linkId) ids.add(card.linkId);
    for (const link of linksByCard.get(card.id) ?? []) {
      if (link.kind === "session") ids.add(link.reference);
    }
    return [...ids].flatMap((id) => {
      const facts = linkStates.get(linkValue("session", id))?.agent;
      return facts ? [facts] : [];
    });
  };
  const agentOf = (cardId: string) => {
    const card = cardsById.get(cardId);
    return agentValues(card ? agentFactsOf(card) : []);
  };
  const faceChipsOf = (card: BoardCard) => {
    const values = agentOf(card.id);
    return [
      ...fieldChipsOf(card.id),
      ...display.face.agentFields.flatMap((field) =>
        values[field]
          ? [{ id: `agent:${field}`, name: AGENT_FIELD_LABEL[field], text: values[field]! }]
          : [],
      ),
    ];
  };

  /** Asks first when open cards block this one, then lets the user pick what runs. */
  const startRunWith = async (card: BoardCard) => {
    // Asked again now: the map on screen may be stale or empty. If the store
    // can't say, the card may be blocked, so nothing starts.
    let blockers: number[];
    try {
      blockers = (await getBlockedCards(card.boardId))[card.id] ?? [];
    } catch (cause) {
      fail("Couldn't check whether the card is blocked")(cause);
      return;
    }
    if (blockers.length) {
      const confirmed = await ask(
        `#${card.number} is blocked by ${blockers.map((n) => `#${n}`).join(", ")}. Run anyway?`,
        {
          title: "Blocked card",
          kind: "warning",
          okLabel: "Run anyway",
          cancelLabel: "Cancel",
        },
      ).catch(() => false);
      if (!confirmed) return;
    }
    setRunFor(card);
  };
  const runWithOn = !!onRunWith && settings?.runWith === true;

  const renderCard = (card: BoardCard) => (
    <BoardCardItem
      card={card}
      done={columnsById.get(card.columnId)?.role === "done"}
      link={linkOf(card)}
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
      moreItems={moreItemsFor(card)}
      selected={selectedSet.has(card.id)}
      onSelectClick={(event) => selectClick(event, card)}
      display={display}
      fieldChips={faceChipsOf(card)}
      linkChips={linkChipsOf(card)}
      blockedBy={blocked[card.id] ?? []}
    />
  );

  /** A click on a list or table row: selects (⌘/Ctrl, Shift) or opens the card. */
  const rowClick = (
    event: { metaKey: boolean; ctrlKey: boolean; shiftKey: boolean },
    card: BoardCard,
  ) => {
    if (!selectClick(event, card)) showCard({ id: card.id });
  };

  const addLabelTo = (card: BoardCard, name: string) =>
    void createLabel(card.boardId, name, nextLabelColor(labels.length))
      .then((created) =>
        setCardLabels(card.id, [
          ...(extras[card.id]?.labels ?? []),
          created.id,
        ]),
      )
      .catch(fail(`Couldn't create a label for card #${card.number}`));

  const labelCounts = new Map<string, number>();
  for (const id of selection.ids) {
    for (const label of extras[id]?.labels ?? []) {
      labelCounts.set(label, (labelCounts.get(label) ?? 0) + 1);
    }
  }

  const doneColumn = detail?.columns.find((column) => column.role === "done");
  // Cards a held delete hides, by column (a bulk delete holds several).
  const heldAction = undoToast?.action;
  const heldPlaces: Place[] =
    heldAction?.kind === "delete"
      ? [[heldAction.cardId, heldAction.columnId, heldAction.index]]
      : heldAction?.kind === "deleteMany"
        ? heldAction.items
        : [];
  const heldIds = heldPlaces.map(([cardId]) => cardId);
  const heldByColumn = new Map<string, number>();
  for (const [, columnId] of heldPlaces) {
    heldByColumn.set(columnId, (heldByColumn.get(columnId) ?? 0) + 1);
  }

  /**
   * A drop into a lane changes the column, and sets the lane's value only for
   * a priority or select-field lane; label and project lanes change the
   * column alone.
   */
  const onLaneDrop = (cardId: string, laneKey: string, columnId: string) => {
    const card = detailRef.current?.cards.find((other) => other.id === cardId);
    if (!card || !lanesBy) return;
    const value = laneValue(lanesBy, laneKey);
    const encoded =
      value?.kind === "field"
        ? value.option === null
          ? null
          : JSON.stringify(value.option)
        : null;
    const setsValue =
      value?.kind === "priority"
        ? (extras[card.id]?.priority ?? 0) !== value.priority
        : value?.kind === "field"
          ? (cardIndex.get(card.id)?.fields[value.fieldId] ?? null) !== encoded
          : false;
    if (card.columnId !== columnId) {
      // The same path as a column drop: optimistic move, WIP warning, and an
      // Undo only when the column is all that changes (a column-only Undo
      // would leave the lane's value changed).
      onCardMove(
        card.id,
        {
          columnId,
          index: wholeColumn(columnId).filter(
            (other) => other.id !== card.id && matches(other),
          ).length,
        },
        !setsValue,
      );
    }
    if (!setsValue) return;
    if (value?.kind === "priority") {
      void setCardPriority(card.id, value.priority).catch(
        fail(`Couldn't set the priority of card #${card.number}`),
      );
    } else if (value?.kind === "field") {
      void setFieldValue(card.id, value.fieldId, encoded).catch(
        fail(`Couldn't set the field of card #${card.number}`),
      );
    }
  };

  /**
   * The toolbar's Display edits: on a saved view they are its override (kept
   * with Save changes); on "All cards" they change the board default.
   */
  const changeDisplay = (next: DisplayOptions) => {
    displayBase.current = next;
    if (activeView) {
      setLayout((json) => withLayoutDisplay(json, next));
    } else if (openId) {
      displayPending.current += 1;
      displayQueue.current = displayQueue.current
        .then(() =>
          updateBoardSettings(openId, { displayJson: serializeDisplay(next) }),
        )
        .then(
          (saved) => {
            setSettings(saved);
            return true;
          },
          (cause) => {
            fail("Couldn't save the display")(cause);
            return false;
          },
        )
        .then((saved) => {
          displayPending.current -= 1;
          // A failed save leaves what is stored; edits stop building on it.
          if (!saved && displayPending.current === 0) {
            displayBase.current = resolveDisplay(
              settingsRef.current?.displayJson ?? null,
              null,
            );
          }
        });
    }
  };

  const renderColumnHeader = (column: BoardColumn, shown: number) => (
    <BoardColumnHeader
      column={column}
      count={
        narrowing && detail ? (columnCounts.get(column.id) ?? 0) : shown
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
      heldCards={heldByColumn.get(column.id) ?? 0}
      onDelete={() =>
        // A delete held in this column is carried out first.
        void undo
          .commitPending()
          .then(() => deleteColumn(column.id))
          .catch(fail("Couldn't delete the column"))
      }
      moveTargets={(detail?.columns ?? []).filter(
        (other) => other.id !== column.id,
      )}
      onMoveAndDelete={(targetId) => {
        // Every live card goes, in order, to the end of the target column;
        // a delete held in this column is carried out first.
        const moving = wholeColumn(column.id).map((card) => card.id);
        const end = wholeColumn(targetId).length;
        void undo
          .commitPending()
          .then(() =>
            moving.length
              ? moveCards(moving, targetId, end, "user")
              : undefined,
          )
          .then(() => deleteColumn(column.id))
          .catch(fail("Couldn't delete the column"));
      }}
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
    <PriorityContext.Provider value={scheme}>
      <div
        ref={rootRef}
        tabIndex={-1}
        className="flex min-h-0 flex-1 flex-col outline-none"
        onKeyDown={(event) => {
          // Escape's last layers: menus, inline edits and the drawer's own
          // fields take it first (they mark it handled). What is left closes
          // an open drawer, wherever focus is on the board, then clears the
          // selection.
          if (
            event.key !== "Escape" ||
            event.defaultPrevented ||
            event.nativeEvent.isComposing ||
            // A drag in progress is cancelled by its own Escape.
            dragInProgress()
          ) {
            return;
          }
          if (drawerHandle.current && shownCardRef.current) {
            event.preventDefault();
            drawerHandle.current.escape();
            return;
          }
          if (!selection.ids.length) return;
          event.preventDefault();
          setSelection(EMPTY_SELECTION);
        }}
        onKeyDownCapture={(event) => {
          // Cmd+N (Ctrl+N elsewhere) with focus anywhere in the board, the
          // search box and menu buttons included (capture, so a control that
          // stops keydown can't swallow it). Shift and Alt variants stay with the app.
          const mod = IS_MAC ? event.metaKey : event.ctrlKey;
          if (
            !mod ||
            event.shiftKey ||
            event.altKey ||
            (IS_MAC && event.ctrlKey) ||
            event.key.toLowerCase() !== "n" ||
            event.nativeEvent.isComposing ||
            !detail ||
            settingsOpen ||
            newBoardOpen ||
            ticketCardOpen
          )
            return;
          event.preventDefault();
          setNewCardOpen(true);
        }}
      >
        <div className="flex h-11 shrink-0 items-center gap-1.5 border-b border-stroke px-4">
          {naming === "rename" && openBoard ? (
            <NameInput
              initial={openBoard.name}
              label="Board name"
              className="w-56"
              onSubmit={(name) => {
                setNaming(null);
                void renameOpenBoard(openBoard, name);
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
                openSignal={switchSignal}
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
                    onSelect={() => {
                      close();
                      setOpenCard(null);
                      setArchiveOpen(false);
                      setSelection(EMPTY_SELECTION);
                      setSettingsOpen(true);
                    }}
                  >
                    Board settings
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
                      setSearched("");
                    }
                  }}
                  className="h-7 w-44 rounded-md border border-stroke bg-background-base pl-7 pr-2 text-[12px] text-content outline-none placeholder:text-content/40 focus:border-accent/60"
                />
              </label>
              <BoardFilterMenu
                rows={rows}
                count={effectiveRows.length}
                sources={sources}
                onChange={setRows}
                distinctValues={(fieldKey) =>
                  fieldDistinctValues(
                    openId!,
                    fieldKey.slice("field:".length),
                    50,
                  )
                }
              />
              <BoardSortMenu sort={sort} onChange={setSort} />
              <MenuButton
                label="Layout"
                width={160}
                className={BUTTON}
                trigger={`Layout: ${LAYOUTS.find(([kind]) => kind === layoutState.layout)?.[1]}`}
              >
                {(close) =>
                  LAYOUTS.map(([kind, name]) => (
                    <MenuItem
                      key={kind}
                      checked={kind === layoutState.layout}
                      onSelect={() => {
                        close();
                        editLayout(() => ({ layout: kind }));
                      }}
                    >
                      {name}
                    </MenuItem>
                  ))
                }
              </MenuButton>
              <MenuButton
                label="Display"
                width={340}
                className={BUTTON}
                trigger="Display"
              >
                {() => (
                  <div className="p-2">
                    <DisplayEditor
                      display={display}
                      fields={fields}
                      onChange={changeDisplay}
                      base={() => displayBase.current}
                    />
                    <div className="pt-2 text-[11px] text-content/45">
                      {activeView
                        ? "Saved with this view."
                        : "Changes the board default."}
                    </div>
                  </div>
                )}
              </MenuButton>
              {layoutState.layout === "list" ? (
                <MenuButton
                  label="Group by"
                  width={180}
                  className={BUTTON}
                  trigger={`Group by ${GROUP_BYS.find(([by]) => by === layoutState.groupBy)?.[1]}`}
                >
                  {(close) =>
                    GROUP_BYS.map(([by, name]) => (
                      <MenuItem
                        key={by}
                        checked={by === layoutState.groupBy}
                        onSelect={() => {
                          close();
                          editLayout(() => ({ groupBy: by }));
                        }}
                      >
                        {name}
                      </MenuItem>
                    ))
                  }
                </MenuButton>
              ) : null}
              {layoutState.layout === "table" ? (
                <MenuButton
                  label="Columns"
                  width={200}
                  className={BUTTON}
                  trigger="Columns"
                >
                  {() => (
                    <>
                      <MenuLabel>Show columns</MenuLabel>
                      {[
                        ...TABLE_COLUMNS.slice(1).map(([id, name]) => [id, name]),
                        ...fields.map((field) => [`field:${field.id}`, field.name]),
                      ].map(([id, name]) => {
                        const shown = tableColumns(layoutState, fields).some(
                          (column) => column.id === id,
                        );
                        return (
                          <MenuItem
                            key={id}
                            kind="checkbox"
                            checked={shown}
                            onSelect={() =>
                              editLayout((current) => {
                                const ids = tableColumns(current, fields).map(
                                  (column) => column.id,
                                );
                                return {
                                  columns: shown
                                    ? ids.filter((other) => other !== id)
                                    : [...ids, id!],
                                };
                              })
                            }
                          >
                            {name}
                          </MenuItem>
                        );
                      })}
                    </>
                  )}
                </MenuButton>
              ) : null}
              {sorted ? (
                <span data-sorted-by="" className="text-[12px] text-content/55">
                  Sorted by {sortLabel(sort.key)}
                </span>
              ) : null}
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
          <button
            type="button"
            className={BUTTON}
            onClick={() => setNewBoardOpen(true)}
          >
            <Plus className="size-3.5" strokeWidth={1.75} />
            New board
          </button>
          {newBoardOpen ? (
            <NewBoardDialog
              onCreate={createNamedBoard}
              onClose={() => setNewBoardOpen(false)}
            />
          ) : null}
        </div>
        {detail && !settingsOpen ? (
          <ViewTabs
            views={orderedViews(views)}
            activeId={activeView}
            dirty={dirty}
            onSelect={(id) =>
              showView(id ? (views.find((view) => view.id === id) ?? null) : null)
            }
            onSaveAs={saveViewAs}
            onSaveChanges={saveViewChanges}
            onRevert={revertView}
            onRename={renameView}
            onDuplicate={duplicateView}
            onDelete={removeView}
          />
        ) : null}
        {settingsOpen ? null : (
          <FilterChips
            rows={rows}
            sources={sources}
            onRemove={(index) =>
              setRows((list) => list.filter((_, at) => at !== index))
            }
            onClear={() => setRows([])}
          />
        )}
        {settingsOpen && detail ? (
          <BoardSettings
            board={detail.board}
            settings={settings}
            scheme={scheme}
            columns={detail.columns}
            cards={detail.cards}
            labels={labels}
            fields={fields}
            views={views}
            onClose={() => {
              setSettingsOpen(false);
              // Back where the page was opened from.
              rootRef.current
                ?.querySelector<HTMLElement>(
                  'button[aria-label="Board actions"]',
                )
                ?.focus();
            }}
            onRename={(name) => void renameOpenBoard(detail.board, name)}
            onError={setError}
            onSettings={setSettings}
            onReloadSettings={() => void refresh()}
            beforeColumnDelete={() => undo.commitPending()}
          />
        ) : null}
        <div
          className={`flex min-h-0 flex-1 ${settingsOpen ? "hidden" : ""}`}
          inert={settingsOpen || undefined}
        >
          <div
            className={`relative min-h-0 min-w-0 flex-1 ${fullPage ? "hidden" : ""}`}
            // The card or row holding keyboard focus, in any layout: each
            // keeps it mounted when it is scrolled out of its window (R81).
            onFocus={(event) => setFocusedId(focusedRowOf(event.target))}
            onBlur={(event) => {
              // The window losing focus (another app, a system dialog) blurs
              // the card with nowhere to go; it gets focus back on return, so
              // keep it pinned rather than let it scroll out and unmount.
              if (!event.relatedTarget && !document.hasFocus()) return;
              setFocusedId(focusedRowOf(event.relatedTarget));
            }}
          >
            {detail && layoutState.layout === "list" ? (
              <BoardList
                cards={visibleCards}
                columns={detail.columns}
                extras={extras}
                labels={labels}
                projects={projects}
                groupBy={layoutState.groupBy}
                collapsed={layoutState.collapsed}
                selected={selectedSet}
                focusedCardId={focusedId}
                onToggleGroup={(key) =>
                  editLayout((current) => ({
                    collapsed: current.collapsed.includes(key)
                      ? current.collapsed.filter((other) => other !== key)
                      : [...current.collapsed, key],
                  }))
                }
                onRowClick={rowClick}
              />
            ) : detail && layoutState.layout === "table" ? (
              <BoardTable
                agent={agentOf}
                cards={visibleCards}
                existing={new Set(detail.cards.map((card) => card.id))}
                columns={tableColumns(layoutState, fields)}
                boardColumns={detail.columns}
                extras={extras}
                index={cardIndex}
                labels={labels}
                fields={fields}
                projects={projects}
                selected={selectedSet}
                focusedCardId={focusedId}
                onRowClick={rowClick}
                onResize={(id, width) =>
                  editLayout((current) => ({
                    widths: { ...current.widths, [id]: width },
                  }))
                }
                onCommitTitle={(card, title) => {
                  const body =
                    detailRef.current?.cards.find((other) => other.id === card.id)
                      ?.body ?? card.body;
                  void updateCard(card.id, title, body).catch(
                    fail(`Couldn't rename card #${card.number}`),
                  );
                }}
                onSetStatus={(card, columnId) =>
                  moveFromDrawer(card, columnId, wholeColumn(columnId).length)
                }
                onSetPriority={(card, priority) =>
                  void setCardPriority(card.id, priority).catch(
                    fail(`Couldn't set the priority of card #${card.number}`),
                  )
                }
                onSetDue={(card, dueAt) =>
                  void setCardDue(card.id, dueAt).catch(
                    fail(`Couldn't set the due date of card #${card.number}`),
                  )
                }
                onSetLabels={(card, ids) =>
                  void setCardLabels(card.id, ids).catch(
                    fail(`Couldn't set the labels of card #${card.number}`),
                  )
                }
                onCreateLabel={addLabelTo}
              />
            ) : shownDetail && lanesBy ? (
              <Swimlanes
                columns={shownDetail.columns}
                cards={shownDetail.cards}
                lanes={lanes}
                renderCard={renderCard}
                renderColumnHeader={renderColumnHeader}
                onDrop={onLaneDrop}
                onCardOpen={(card) => showCard({ id: card.id })}
                onColumnExpand={(column) => setCollapsed(column, false)}
                selectedIds={selectedSet}
                trailing={trailing}
                density={display.density}
                focusedCardId={focusedId}
                openCardId={shownCard?.id ?? null}
              />
            ) : shownDetail ? (
              <KanbanBoard
                detail={shownDetail}
                renderCard={renderCard}
                renderColumnHeader={renderColumnHeader}
                renderColumnFooter={renderColumnFooter}
                onCardMove={onCardMove}
                onColumnMove={onColumnMove}
                onCardOpen={(card) => showCard({ id: card.id })}
                onColumnExpand={(column) => setCollapsed(column, false)}
                sorted={sorted}
                selectedIds={selectedSet}
                trailing={trailing}
                density={display.density}
                focusedCardId={focusedId}
                openCardId={shownCard?.id ?? null}
                columnTotals={narrowing ? columnCounts : undefined}
              />
            ) : null}
            {detail && selection.ids.length ? (
              <SelectionBar
                count={selection.ids.length}
                columns={detail.columns}
                labels={labels}
                labelCounts={labelCounts}
                onStatus={(columnId) => {
                  const column = columnsById.get(columnId);
                  if (column) bulkStatus(column);
                }}
                onPriority={bulkPriority}
                onLabel={bulkLabel}
                onArchive={() => bulkRemove(false)}
                onDelete={() => bulkRemove(true)}
                onClear={() => setSelection(EMPTY_SELECTION)}
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
                cardCount: columnCounts.get(column.id) ?? 0,
              }))}
              extras={extras[shownCard.id]}
              labels={labels}
              projects={projects}
              link={linkOf(shownCard)}
              onRunWith={runWithOn ? () => void startRunWith(shownCard) : undefined}
              blockedBy={blocked[shownCard.id] ?? []}
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
              onClose={closeDrawer}
              handle={drawerHandle}
              isGone={() =>
                !detailRef.current?.cards.some(
                  (other) => other.id === shownCard.id,
                )
              }
              resolveLinkTarget={(kind, id) =>
                linkStates.get(linkValue(kind, id))
              }
              onOpenLinkedWork={(target) => {
                setOpenCard(null);
                openLinkedWork(target);
              }}
              onMove={(columnId, index) =>
                moveFromDrawer(shownCard, columnId, index)
              }
              onDelete={() => removeCard(shownCard)}
              renderMore={moreItemsFor(shownCard)}
              onError={setError}
            />
          ) : archiveOpen && detail ? (
            <ArchiveView
              key={detail.board.id}
              boardId={detail.board.id}
              columnNames={
                new Map(detail.columns.map((column) => [column.id, column.name]))
              }
              pendingDelete={null}
              pendingDeletes={heldIds}
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
        {runFor && onRunWith ? (
          <RunWithDialog
            cardId={runFor.id}
            projects={projects}
            onClose={() => setRunFor(null)}
            onRun={(request) => {
              setRunFor(null);
              onRunWith(request).catch(fail("Couldn't start the session"));
            }}
          />
        ) : null}
        {newCardOpen && detail ? (
          <NewCardModal
            key={detail.board.id}
            boardId={detail.board.id}
            columns={detail.columns}
            labels={labels}
            projects={projects}
            onClose={() => setNewCardOpen(false)}
            onNotice={setError}
            returnFocusTo={() => rootRef.current}
          />
        ) : null}
        {toast}
      </div>
    </PriorityContext.Provider>
  );
}
