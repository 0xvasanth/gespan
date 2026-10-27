import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ComponentProps,
  type MutableRefObject,
  type ReactNode,
} from "react";
import {
  SearchableSelect,
  type SearchableSelectOption,
} from "../../../shared/ui/SearchableSelect";
import { ExternalLink, Maximize2, Play, Trash2 } from "../../../shared/ui/icons";
import { applyFetchedTicket } from "../model/ticketCards";
import { LinkSessionWorkItemDialog } from "../../sessions/ui/LinkSessionWorkItemDialog";
import { SessionTicketChip } from "../../sessions/ui/SessionTicketChip";
import type { LinkedWorkItem, TicketRef } from "../../sessions/model/session";
import {
  createLabel,
  linkCard,
  listActivity,
  setCardDue,
  setCardLabels,
  setCardPriority,
  setCardProjects,
  setCardTicket,
  subscribeBoard,
  updateCard,
  type BoardActivity,
  type BoardCard,
  type BoardLabel,
  type CardExtras,
} from "../model/board";
import { coalesce } from "../model/coalesce";
import { LinkChip } from "./BoardCardItem";
import { MarkdownToolbar } from "./MarkdownToolbar";
import {
  CloseButton,
  Comments,
  FIELD,
  LabelDot,
  LabelPicker,
  PROPERTY_TRIGGER,
  PropertyRow,
  SMALL_BUTTON,
  Subtasks,
  toggled,
  useAhead,
} from "./CardDrawerParts";
import { DueDateInput, DueItems, PriorityItems } from "./CardPropertyMenus";
import {
  FieldsSection,
  LinksSection,
  RelationsSection,
  StartAndEstimateRows,
  type BoardCardRef,
} from "./CardWorkSections";
import { ICON_BUTTON, MenuButton, MenuItem } from "./boardControls";
import {
  activityText,
  dueChip,
  errorText,
  LINK_KIND_LABEL,
  linkValue,
  nextLabelColor,
  parseLinkValue,
  type BoardLinkTarget,
  type ResolvedLink,
} from "./boardFormat";
import { priorityName } from "../model/boardSettings";
import { usePriorityScheme } from "./priorityScheme";

const ACTIVITY_LIMIT = 50;
const NO_LINK = "";

type Tab = "comments" | "activity";

/** What the board can ask of an open drawer. */
export type CardDrawerHandle = {
  /** Escape from the board: the drawer's own Escape (full page steps back first). */
  escape: () => void;
  /** Saves a typed but unsaved title or description; resolves once written. */
  flushDrafts: () => Promise<void>;
};

/**
 * A card opened from the board, docked beside it: title, properties,
 * description, subtasks, and the comments and activity tabs. The board stays
 * live. Title and description are drafts that save on blur (or Enter, or
 * close); a refresh never overwrites one being edited.
 */
export function CardDetail({
  card,
  columns,
  extras,
  labels,
  projects,
  link,
  ticket = null,
  linkTargets,
  boardCards = [],
  focus,
  fullPage = false,
  onToggleFullPage,
  onClose,
  onOpenLinkedWork,
  onMove,
  onDelete,
  renderMore,
  onError,
  onRunWith,
  blockedBy = [],
  handle,
  isGone,
  resolveLinkTarget,
}: {
  /** Filled with this drawer's handle while it is open. */
  handle?: MutableRefObject<CardDrawerHandle | null>;
  /**
   * True once the card is off the board (deleted or archived elsewhere): a
   * draft saved as the drawer goes then fails quietly, as the card is gone.
   */
  isGone?: () => boolean;
  /** Looks up linked work `linkTargets` doesn't hold, for the links list. */
  resolveLinkTarget?: ComponentProps<typeof LinksSection>["resolveLinkTarget"];
  /** Absent when the board turns "Run with…" off. */
  onRunWith?: () => void;
  /** Numbers of the open cards blocking this one. */
  blockedBy?: readonly number[];
  card: BoardCard;
  /** The board's columns in order, with how many cards each holds. */
  columns: readonly { id: string; name: string; cardCount: number }[];
  extras?: CardExtras;
  /** The board's labels. */
  labels: readonly BoardLabel[];
  /** The workspace's projects. */
  projects: readonly { path: string; name: string }[];
  link: ResolvedLink | null;
  /** The ticket the card links to. */
  ticket?: TicketRef | null;
  linkTargets: readonly BoardLinkTarget[];
  /** The board's cards, for the relation picker. */
  boardCards?: readonly BoardCardRef[];
  focus?: "title" | "link";
  /** Fills the board area instead of docking beside it. */
  fullPage?: boolean;
  onToggleFullPage?: () => void;
  onClose: () => void;
  onOpenLinkedWork: (target: BoardLinkTarget) => void;
  /** Moves the card to `index` in `columnId`, with an undo toast. */
  onMove: (columnId: string, index: number) => void;
  /** Deletes the card: held with an undo toast, so it never asks first. */
  onDelete: () => void;
  /**
   * The More menu's content for the top bar's "…". `flushDrafts` saves a
   * typed but unsaved title or description and resolves once it is written,
   * for actions that read the card. `dropDrafts` forgets them, for a delete.
   */
  renderMore?: (
    close: () => void,
    flushDrafts: () => Promise<void>,
    dropDrafts: () => void,
  ) => ReactNode;
  onError: (message: string) => void;
}) {
  const scheme = usePriorityScheme();
  const tabIds = useId();
  const [title, setTitle] = useState(card.title);
  const [body, setBody] = useState(card.body);
  const [tab, setTab] = useState<Tab>("comments");
  // Null until the first load answers; `failed` keeps a failed load from
  // reading as an empty history.
  const [activity, setActivity] = useState<{
    rows: BoardActivity[];
    failed: boolean;
  } | null>(null);
  const onErrorRef = useRef(onError);
  useEffect(() => {
    onErrorRef.current = onError;
  });
  const [linkingTicket, setLinkingTicket] = useState(false);
  const ticketRef = useRef(ticket);
  useEffect(() => {
    ticketRef.current = ticket;
  });
  const asideRef = useRef<HTMLElement>(null);
  const titleField = useRef<HTMLInputElement>(null);
  const bodyField = useRef<HTMLTextAreaElement>(null);
  const linkRow = useRef<HTMLDivElement>(null);
  // What was last saved, so blur and close don't both write the same edit.
  const saved = useRef({ title: card.title, body: card.body });
  // What the fields hold now, and what the board last said.
  const draft = useRef({ title: card.title, body: card.body });
  const latest = useRef({ title: card.title, body: card.body });
  const deleted = useRef(false);

  // A save elsewhere (or an agent) updates the card under us. A field that
  // is being edited, or holds unsaved text, keeps its draft.
  useEffect(() => {
    latest.current = { title: card.title, body: card.body };
    const active = document.activeElement;
    for (const field of ["title", "body"] as const) {
      const dirty = draft.current[field] !== saved.current[field];
      const element = field === "title" ? titleField : bodyField;
      if (dirty) {
        // Saving the draft later is the user's edit on top of this one.
        saved.current[field] = card[field];
      } else if (active !== element.current) {
        saved.current[field] = card[field];
        draft.current[field] = card[field];
        (field === "title" ? setTitle : setBody)(card[field]);
      }
    }
  }, [card.title, card.body]); // eslint-disable-line react-hooks/exhaustive-deps

  // The drawer takes focus as it opens, unless a field asked for it.
  useEffect(() => {
    if (focus === "title") titleField.current?.select();
    else if (focus === "link") {
      linkRow.current?.querySelector<HTMLButtonElement>("button")?.focus();
    } else asideRef.current?.focus({ preventScroll: true });
  }, [focus]);

  useEffect(() => {
    let live = true;
    const load = () =>
      listActivity(card.boardId, card.id, ACTIVITY_LIMIT)
        .then((rows) => {
          if (live) setActivity({ rows, failed: false });
        })
        .catch((error) => {
          if (!live) return;
          setActivity((current) => ({
            rows: current?.rows ?? [],
            failed: true,
          }));
          onErrorRef.current(
            `Couldn't load activity for card #${card.number}: ${errorText(error)}`,
          );
        });
    // One read in flight, plus one trailing read for a burst of changes.
    const reload = coalesce(load);
    void reload();
    const stop = subscribeBoard(card.boardId, () => void reload());
    return () => {
      live = false;
      stop();
    };
  }, [card.boardId, card.id, card.number]);

  const linked = link?.target ?? null;
  const options = useMemo((): SearchableSelectOption[] => {
    const isLinked = (target: BoardLinkTarget) =>
      linked?.kind === target.kind && linked.id === target.id;
    // Archived work stays out of the picker, except the card's own link,
    // which shows wherever the app found it.
    const linkable = linkTargets.filter(
      (target) =>
        (target.kind === "session" || target.kind === "pilot") &&
        (!target.unlisted || isLinked(target)),
    );
    if (linked && !linkable.some(isLinked)) linkable.push(linked);
    return [
      { value: NO_LINK, label: "No linked work" },
      ...linkable.map((target) => ({
        value: linkValue(target.kind, target.id),
        label: `${LINK_KIND_LABEL[target.kind]} · ${target.label}`,
        keywords: target.detail,
      })),
    ];
  }, [linkTargets, linked]);

  const failed = (what: string) => (error: unknown) =>
    onError(`Couldn't ${what} card #${card.number}: ${errorText(error)}`);

  const saveFields = (quietIfGone = false): Promise<void> => {
    // An emptied title goes back to the saved one; the notes still save.
    const nextTitle = draft.current.title.trim() || saved.current.title;
    const nextBody = draft.current.body;
    if (nextTitle === saved.current.title && nextBody === saved.current.body) {
      // Nothing to write: take what the board says now.
      draft.current = { ...latest.current };
      saved.current = { ...latest.current };
      setTitle(latest.current.title);
      setBody(latest.current.body);
      return Promise.resolve();
    }
    saved.current = { title: nextTitle, body: nextBody };
    draft.current.title = nextTitle;
    setTitle(nextTitle);
    return updateCard(card.id, nextTitle, nextBody).catch((error) => {
      // The card went while the drawer had a draft: there is nothing to say.
      if (quietIfGone && isGoneRef.current?.()) return;
      failed("save")(error);
    });
  };
  const isGoneRef = useRef(isGone);
  useEffect(() => {
    isGoneRef.current = isGone;
  });
  const saveRef = useRef(saveFields);
  useEffect(() => {
    saveRef.current = saveFields;
  });
  // Closing, or opening another card, saves a pending draft.
  useEffect(
    () => () => {
      if (!deleted.current) saveRef.current(true);
    },
    [],
  );

  const escape = () => {
    // The ticket dialog owns Escape while it is open.
    if (linkingTicket) return;
    saveFields();
    // Escape in the full page goes back to the drawer; the next one closes.
    if (fullPage && onToggleFullPage) onToggleFullPage();
    else onClose();
  };
  // The X closes the card, from the drawer or the full page.
  const close = () => {
    saveFields();
    onClose();
  };
  const escapeRef = useRef(escape);
  useEffect(() => {
    escapeRef.current = escape;
  });
  useEffect(() => {
    if (!handle) return;
    handle.current = {
      escape: () => escapeRef.current(),
      flushDrafts: () => saveRef.current(),
    };
    return () => {
      handle.current = null;
    };
  }, [handle]);

  const changeLink = (value: string) => {
    linkCard(card.id, value === NO_LINK ? null : parseLinkValue(value)).catch(
      failed("link"),
    );
  };

  const changeTicket = (item: LinkedWorkItem | undefined) => {
    // Only tickets link to a card; the link dialog hands one over.
    if (item && item.kind !== "ticket") return;
    setLinkingTicket(false);
    setCardTicket(card.id, item ? item.ticket : null).catch(
      failed("link a ticket to"),
    );
  };

  // The chip hands over a fetched copy of the linked ticket, or an unlink.
  const chipChange = (item: LinkedWorkItem | undefined) => {
    if (item?.kind === "ticket") {
      const requested = ticketRef.current;
      if (!requested) return;
      applyFetchedTicket(card, requested, item.ticket).catch((error) =>
        onError(
          `Couldn't save the ticket on card #${card.number}: ${errorText(error)}`,
        ),
      );
    } else if (!item) {
      changeTicket(undefined);
    }
  };

  const dropDrafts = () => {
    // A pending draft is dropped with the card, as before the hold.
    deleted.current = true;
  };
  const remove = () => {
    dropDrafts();
    onDelete();
  };

  // ---- properties ----
  const [priority, setPriority, revertPriority] = useAhead(
    extras?.priority ?? 0,
    String(extras?.priority ?? 0),
  );
  const [dueAt, setDueAt, revertDue] = useAhead(
    extras?.dueAt ?? null,
    String(extras?.dueAt ?? null),
  );
  const [projectPaths, setProjectPaths, revertProjects] = useAhead(
    extras?.projects ?? [],
    (extras?.projects ?? []).join("\n"),
  );
  const [labelIds, setLabelIds, revertLabels] = useAhead(
    extras?.labels ?? [],
    (extras?.labels ?? []).join("\n"),
  );

  const choosePriority = (next: number) => {
    setPriority(next);
    setCardPriority(card.id, next).catch((error) => {
      revertPriority();
      failed("set the priority of")(error);
    });
  };
  const chooseDue = (next: number | null) => {
    setDueAt(next);
    setCardDue(card.id, next).catch((error) => {
      revertDue();
      failed("set the due date of")(error);
    });
  };
  const chooseProjects = (next: string[]) => {
    setProjectPaths(next);
    setCardProjects(card.id, next).catch((error) => {
      revertProjects();
      failed("set the projects of")(error);
    });
  };
  const chooseLabels = (next: string[]) => {
    setLabelIds(next);
    return setCardLabels(card.id, next).catch((error) => {
      revertLabels();
      failed("set the labels of")(error);
    });
  };
  const addLabel = (name: string) => {
    createLabel(card.boardId, name, nextLabelColor(labels.length))
      .then((created) => chooseLabels([...labelIds, created.id]))
      .catch(failed("create a label for"));
  };
  const moveTo = (columnId: string, cardCount: number) => {
    if (columnId === card.columnId) return;
    onMove(columnId, cardCount);
  };

  const columnName = columns.find(
    (column) => column.id === card.columnId,
  )?.name;
  const due = dueAt === null ? null : dueChip(dueAt);
  const cardLabels = labels.filter((label) => labelIds.includes(label.id));
  const cardProjects = projectPaths.map(
    (path) => projects.find((project) => project.path === path)?.name ?? path,
  );
  const current =
    card.linkKind && card.linkId ? linkValue(card.linkKind, card.linkId) : "";
  const target = linked;

  return (
    <aside
      ref={asideRef}
      tabIndex={-1}
      data-card-drawer=""
      aria-label={`Card #${card.number}`}
      onKeyDown={(event) => {
        if (event.key === "Escape" && !event.nativeEvent.isComposing) {
          event.preventDefault();
          escape();
        }
      }}
      data-full-page={fullPage ? "" : undefined}
      className={`flex min-h-0 flex-col bg-background-base/40 outline-none ${fullPage ? "min-w-0 flex-1" : "w-[380px] shrink-0 border-l border-stroke"}`}
    >
      <div className="flex h-10 shrink-0 items-center gap-2 border-b border-stroke px-3">
        <span className="min-w-0 flex-1 truncate text-[12px] text-content/50">
          Card #{card.number}
        </span>
        {onToggleFullPage ? (
          <button
            type="button"
            aria-label={fullPage ? "Back to drawer" : "Open full page"}
            aria-pressed={fullPage}
            title={fullPage ? "Back to drawer" : "Open full page"}
            onClick={() => {
              saveFields();
              onToggleFullPage();
            }}
            className={ICON_BUTTON}
          >
            <Maximize2 className="size-3.5" strokeWidth={1.75} />
          </button>
        ) : null}
        {renderMore ? (
          <MenuButton label="More actions" width={240}>
            {(closeMenu) =>
              renderMore(closeMenu, () => saveFields(), dropDrafts)
            }
          </MenuButton>
        ) : null}
        <CloseButton onClick={close} />
      </div>
      <div
        className={`flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 pb-4 pt-3 ${fullPage ? "mx-auto w-full max-w-3xl" : ""}`}
      >
        <input
          ref={titleField}
          aria-label="Title"
          value={title}
          onChange={(event) => {
            draft.current.title = event.target.value;
            setTitle(event.target.value);
          }}
          onBlur={() => void saveFields()}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.nativeEvent.isComposing) {
              event.preventDefault();
              event.currentTarget.blur();
            }
          }}
          className="h-9 w-full rounded-lg border border-transparent bg-transparent px-2 text-[16px] font-medium text-content outline-none hover:border-stroke focus:border-accent/60"
        />
        <div className="flex flex-col">
          <PropertyRow name="status" label="Status">
            <MenuButton
              label={`Status: ${columnName ?? "None"}`}
              className={PROPERTY_TRIGGER}
              trigger={<span className="truncate">{columnName ?? "None"}</span>}
            >
              {(closeMenu) =>
                columns.map((column) => (
                  <MenuItem
                    key={column.id}
                    checked={column.id === card.columnId}
                    onSelect={() => {
                      closeMenu();
                      moveTo(column.id, column.cardCount);
                    }}
                  >
                    {column.name}
                  </MenuItem>
                ))
              }
            </MenuButton>
          </PropertyRow>
          <PropertyRow name="priority" label="Priority">
            <MenuButton
              label={`Priority: ${priorityName(scheme, priority)}`}
              className={PROPERTY_TRIGGER}
              trigger={
                <span className="truncate">
                  {priorityName(scheme, priority)}
                </span>
              }
            >
              {(closeMenu) => (
                <PriorityItems
                  priority={priority}
                  onChoose={choosePriority}
                  close={closeMenu}
                />
              )}
            </MenuButton>
          </PropertyRow>
          <PropertyRow name="due" label="Due">
            <MenuButton
              label={`Due: ${due?.text ?? "None"}`}
              className={PROPERTY_TRIGGER}
              trigger={<span className="truncate">{due?.text ?? "None"}</span>}
            >
              {(closeMenu) => (
                <DueItems
                  dueAt={dueAt}
                  onChoose={chooseDue}
                  close={closeMenu}
                />
              )}
            </MenuButton>
            <DueDateInput dueAt={dueAt} onChoose={chooseDue} />
          </PropertyRow>
          <StartAndEstimateRows card={card} onError={onError} />
          <PropertyRow name="projects" label="Projects">
            <MenuButton
              label={`Projects: ${cardProjects.length ? cardProjects.join(", ") : "None"}`}
              width={240}
              className={PROPERTY_TRIGGER}
              trigger={
                <span className="truncate">
                  {cardProjects.length ? cardProjects.join(", ") : "None"}
                </span>
              }
            >
              {() =>
                projects.length ? (
                  projects.map((project) => (
                    <MenuItem
                      key={project.path}
                      kind="checkbox"
                      checked={projectPaths.includes(project.path)}
                      onSelect={() =>
                        chooseProjects(toggled(projectPaths, project.path))
                      }
                    >
                      <span className="min-w-0 truncate">{project.name}</span>
                    </MenuItem>
                  ))
                ) : (
                  <div className="px-2 py-1.5 text-[12px] text-content/45">
                    No projects in this workspace
                  </div>
                )
              }
            </MenuButton>
          </PropertyRow>
          <PropertyRow name="labels" label="Labels">
            <MenuButton
              label={`Labels: ${cardLabels.length ? cardLabels.map((label) => label.name).join(", ") : "None"}`}
              width={240}
              className={PROPERTY_TRIGGER}
              trigger={
                cardLabels.length ? (
                  <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5">
                    {cardLabels.map((label) => (
                      <span
                        key={label.id}
                        className="inline-flex items-center gap-1"
                      >
                        <LabelDot color={label.color} />
                        {label.name}
                      </span>
                    ))}
                  </span>
                ) : (
                  "None"
                )
              }
            >
              {(closeMenu) => (
                <LabelPicker
                  labels={labels}
                  selected={labelIds}
                  onToggle={(id) => chooseLabels(toggled(labelIds, id))}
                  onCreate={addLabel}
                  close={closeMenu}
                />
              )}
            </MenuButton>
          </PropertyRow>
          <div data-property="linked" className="flex items-start gap-2">
            <span className="flex h-8 w-20 shrink-0 items-center text-[12px] text-content/50">
              Linked work
            </span>
            <div
              ref={linkRow}
              className="flex min-w-0 flex-1 flex-col gap-1 py-0.5"
            >
              <div className="flex items-center gap-2">
                <div className="min-w-0 flex-1">
                  <SearchableSelect
                    label="Linked work"
                    value={current}
                    options={options}
                    onChange={changeLink}
                    placeholder="Link a session or Pilot…"
                    searchPlaceholder="Search sessions and Pilots…"
                    emptyLabel="No matching sessions or Pilots"
                  />
                </div>
                {target ? (
                  <button
                    type="button"
                    onClick={() => onOpenLinkedWork(target)}
                    className={`${SMALL_BUTTON} gap-1.5`}
                  >
                    <ExternalLink className="size-3.5" strokeWidth={1.75} />
                    Open linked work
                  </button>
                ) : null}
              </div>
              {onRunWith ? (
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    data-run-with=""
                    onClick={onRunWith}
                    className={`${SMALL_BUTTON} gap-1.5`}
                  >
                    <Play className="size-3.5" strokeWidth={1.75} />
                    Run with…
                  </button>
                  {blockedBy.length ? (
                    <span data-card-blocked="" className="text-[11px] text-amber-400">
                      Blocked by {blockedBy.map((n) => `#${n}`).join(", ")}
                    </span>
                  ) : null}
                </div>
              ) : null}
              {link ? (
                <div className="text-[11px]">
                  <LinkChip link={link} full />
                </div>
              ) : null}
            </div>
          </div>
          <div data-property="ticket" className="flex items-start gap-2">
            <span className="flex h-8 w-20 shrink-0 items-center text-[12px] text-content/50">
              Ticket
            </span>
            <div
              data-card-ticket=""
              className="flex min-w-0 flex-1 flex-wrap items-center gap-2 py-0.5"
            >
              {ticket ? (
                <SessionTicketChip
                  key={`${ticket.tracker}:${ticket.key}`}
                  ticket={ticket}
                  visible
                  onChange={chipChange}
                  current={() =>
                    ticketRef.current
                      ? { kind: "ticket", ticket: ticketRef.current }
                      : undefined
                  }
                />
              ) : null}
              <button
                type="button"
                onClick={() => setLinkingTicket(true)}
                className={SMALL_BUTTON}
              >
                {ticket ? "Change ticket…" : "Link ticket…"}
              </button>
            </div>
          </div>
          <FieldsSection card={card} onError={onError} />
        </div>
        <RelationsSection
          card={card}
          boardCards={boardCards}
          onError={onError}
        />
        <LinksSection
          card={card}
          linkTargets={linkTargets}
          resolveLinkTarget={resolveLinkTarget}
          onOpenLinkedWork={onOpenLinkedWork}
          onError={onError}
        />
        <div className="flex flex-col gap-1">
          <div className="flex items-center justify-between gap-2">
            <span className="text-[12px] text-content/50">Description</span>
            <MarkdownToolbar
              textareaRef={bodyField}
              value={body}
              onChange={(next) => {
                draft.current.body = next;
                setBody(next);
              }}
            />
          </div>
          <textarea
            ref={bodyField}
            aria-label="Description"
            value={body}
            rows={4}
            placeholder="Add a description…"
            onChange={(event) => {
              draft.current.body = event.target.value;
              setBody(event.target.value);
            }}
            onBlur={() => void saveFields()}
            className={`${FIELD} resize-y py-2`}
          />
        </div>
        <Subtasks
          cardId={card.id}
          boardId={card.boardId}
          number={card.number}
          onError={onError}
        />
        <section className="flex flex-col gap-2">
          <div role="tablist" className="flex gap-1 border-b border-stroke">
            {(
              [
                ["comments", "Comments"],
                ["activity", "Activity"],
              ] as const
            ).map(([id, name]) => (
              <button
                key={id}
                type="button"
                role="tab"
                id={`${tabIds}-${id}-tab`}
                aria-controls={`${tabIds}-${id}-panel`}
                aria-selected={tab === id}
                onClick={() => setTab(id)}
                className={`-mb-px h-8 border-b-2 px-2 text-[12px] ${tab === id ? "border-accent text-content" : "border-transparent text-content/50 hover:text-content"}`}
              >
                {name}
              </button>
            ))}
          </div>
          <div
            role="tabpanel"
            id={`${tabIds}-${tab}-panel`}
            aria-labelledby={`${tabIds}-${tab}-tab`}
          >
            {tab === "comments" ? (
              <Comments
                cardId={card.id}
                boardId={card.boardId}
                number={card.number}
                onError={onError}
              />
            ) : (
              <div className="flex flex-col gap-1">
                {activity?.rows.length ? (
                  <ol data-card-activity="" className="flex flex-col gap-1">
                    {activity.rows.map((row) => (
                      <li key={row.id} className="text-[12px] text-content/70">
                        {activityText(row)}
                      </li>
                    ))}
                  </ol>
                ) : null}
                {activity?.failed ? (
                  <span className="text-[12px] text-diff-del-fg/80">
                    Couldn't load activity
                  </span>
                ) : activity && !activity.rows.length ? (
                  <span className="text-[12px] text-content/45">
                    No activity yet
                  </span>
                ) : null}
              </div>
            )}
          </div>
        </section>
      </div>
      <div className="flex h-11 shrink-0 items-center justify-end border-t border-stroke px-3">
        <button
          type="button"
          onClick={remove}
          className="inline-flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[12px] text-red-300/90 hover:bg-red-500/15"
        >
          <Trash2 className="size-3.5" strokeWidth={1.75} />
          Delete card
        </button>
      </div>
      {linkingTicket ? (
        <LinkSessionWorkItemDialog
          ticketOnly
          heading={ticket ? "Edit ticket link" : "Link ticket"}
          initial={ticket ? { kind: "ticket", ticket } : undefined}
          sessionTitle={`Card #${card.number}`}
          onSave={changeTicket}
          onClose={() => setLinkingTicket(false)}
        />
      ) : null}
    </aside>
  );
}
