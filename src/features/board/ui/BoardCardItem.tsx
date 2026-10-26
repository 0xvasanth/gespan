import { useRef, useState, type MouseEvent, type ReactNode } from "react";
import { Popover, type PopoverAnchor } from "../../../shared/ui/Popover";
import {
  Bot,
  ExternalLink,
  MessageSquare,
  Play,
} from "../../../shared/ui/icons";
import type { TicketRef } from "../../sessions/model/session";
import { SessionTicketChip } from "../../sessions/ui/SessionTicketChip";
import {
  PRIORITIES,
  setCardTicket,
  type BoardCard,
  type BoardLabel,
  type CardExtras,
  type CardLinkKind,
} from "../model/board";
import { applyFetchedTicket } from "../model/ticketCards";
import { ICON_BUTTON, MenuButton, MenuItem, noDrag } from "./boardControls";
import { LabelPicker, toggled } from "./CardDrawerParts";
import { DueDateInput, DueItems, PriorityItems } from "./CardPropertyMenus";
import {
  dueChip,
  LINK_KIND_LABEL,
  relativeTime,
  type DueTone,
  type BoardLinkTarget,
  type ResolvedLink,
} from "./boardFormat";

const KIND_ICON: Record<CardLinkKind, typeof MessageSquare> = {
  session: MessageSquare,
  pilot: Bot,
  "flow-run": Play,
};

const DOT: Record<NonNullable<BoardLinkTarget["status"]>, string> = {
  running: "bg-accent animate-pulse motion-reduce:animate-none",
  completed: "bg-diff-add",
  failed: "bg-diff-del-fg",
};

const DUE_TONE: Record<DueTone, string> = {
  overdue: "text-red-400",
  soon: "text-amber-400",
  later: "text-content/60",
};

const EMPTY_EXTRAS: CardExtras = {
  priority: 0,
  dueAt: null,
  projects: [],
  labels: [],
  subtasksDone: 0,
  subtasksTotal: 0,
  comments: 0,
};

const HOVER_ONLY =
  "opacity-0 group-hover/card:opacity-100 focus-within:opacity-100 has-[[data-menu-open]]:opacity-100";
const FACE_TRIGGER =
  "inline-flex shrink-0 items-center rounded px-0.5 hover:bg-content/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/60";

/**
 * Priority, due date and subtask progress. Priority and due are quick-edit
 * menus; an unprioritised card shows a faint priority target on hover.
 */
function CardExtrasRow({
  number,
  extras,
  onSetPriority,
  onSetDue,
}: {
  number: number;
  extras: CardExtras;
  onSetPriority: (priority: number) => void;
  onSetDue: (dueAt: number | null) => void;
}) {
  const due = extras.dueAt === null ? null : dueChip(extras.dueAt);
  const priority = PRIORITIES[extras.priority] ?? PRIORITIES[0];
  return (
    <>
      {extras.priority ? (
        <MenuButton
          label={`Priority of #${number}`}
          width={160}
          className={FACE_TRIGGER}
          trigger={
            <span
              data-card-priority={extras.priority}
              className={
                extras.priority >= 4 ? "text-red-400" : "text-content/70"
              }
            >
              {priority}
            </span>
          }
        >
          {(close) => (
            <PriorityItems
              priority={extras.priority}
              onChoose={onSetPriority}
              close={close}
            />
          )}
        </MenuButton>
      ) : (
        <span data-card-priority-target="" className={HOVER_ONLY}>
          <MenuButton
            label={`Priority of #${number}`}
            width={160}
            className={`${FACE_TRIGGER} text-content/35 hover:text-content/70`}
            trigger="Priority"
          >
            {(close) => (
              <PriorityItems
                priority={0}
                onChoose={onSetPriority}
                close={close}
              />
            )}
          </MenuButton>
        </span>
      )}
      {due ? (
        <MenuButton
          label={`Due date of #${number}`}
          width={200}
          className={FACE_TRIGGER}
          trigger={
            <span data-card-due={due.tone} className={DUE_TONE[due.tone]}>
              {due.text}
            </span>
          }
        >
          {(close) => (
            <>
              <DueItems
                dueAt={extras.dueAt}
                onChoose={onSetDue}
                close={close}
              />
              <div className="px-1 pt-1">
                <DueDateInput
                  dueAt={extras.dueAt}
                  onChoose={onSetDue}
                  className="h-7 w-full rounded-md border border-stroke bg-background-base px-2 text-[12px] text-content/80 outline-none focus:border-accent/60"
                />
              </div>
            </>
          )}
        </MenuButton>
      ) : null}
      {extras.subtasksTotal ? (
        <span
          data-card-subtasks=""
          title="Subtasks done"
          className="shrink-0 tabular-nums text-content/60"
        >
          {extras.subtasksDone}/{extras.subtasksTotal}
        </span>
      ) : null}
      {extras.comments ? (
        <span
          data-card-comments=""
          title="Comments"
          className="shrink-0 tabular-nums text-content/60"
        >
          {extras.comments} {extras.comments === 1 ? "comment" : "comments"}
        </span>
      ) : null}
    </>
  );
}

/** `full` drops the card's 140px label cap, for roomier places. */
export function LinkChip({
  link,
  full = false,
}: {
  link: ResolvedLink;
  full?: boolean;
}) {
  const Icon = KIND_ICON[link.kind];
  const { target } = link;
  return (
    <span
      data-link-chip={
        target ? "found" : link.unconfirmed ? "unconfirmed" : "missing"
      }
      className={`inline-flex min-w-0 items-center gap-1 ${
        target ? "text-content/70" : "text-content/45"
      }`}
    >
      <Icon className="size-3 shrink-0" strokeWidth={1.75} />
      <span className={full ? "truncate" : "max-w-[140px] truncate"}>
        {target
          ? target.label
          : link.unconfirmed
            ? LINK_KIND_LABEL[link.kind]
            : "Linked work no longer exists"}
      </span>
      {target?.status ? (
        <span
          aria-label={target.status}
          className={`size-1.5 shrink-0 rounded-full ${DOT[target.status]}`}
        />
      ) : null}
    </span>
  );
}

/** One card's frame and contents; the sortable wrapper owns the drag. */
export function BoardCardItem({
  card,
  done,
  link,
  ticket = null,
  extras,
  labels = [],
  allLabels = [],
  onSetPriority,
  onSetDue,
  onSetLabels,
  onCreateLabel,
  onOpen,
  onOpenLinkedWork,
  onRename,
  onLink,
  onDelete,
  moreItems,
  selected = false,
  onSelectClick,
}: {
  card: BoardCard;
  /** In a column with the `done` role: the title is dimmed. */
  done: boolean;
  link: ResolvedLink | null;
  ticket?: TicketRef | null;
  extras?: CardExtras;
  /** The card's labels, in the board's order. */
  labels?: readonly BoardLabel[];
  /** Every label of the board, for the label quick edit. */
  allLabels?: readonly BoardLabel[];
  onSetPriority: (priority: number) => void;
  onSetDue: (dueAt: number | null) => void;
  onSetLabels: (labelIds: string[]) => void;
  onCreateLabel: (name: string) => void;
  onOpen: () => void;
  onOpenLinkedWork: (target: BoardLinkTarget) => void;
  onRename: () => void;
  onLink: () => void;
  onDelete: () => void;
  /**
   * The More menu's content. It opens from a right-click at the pointer and
   * from "More…" in the card's "…" menu.
   */
  moreItems?: (close: () => void) => ReactNode;
  /** In the bulk selection: the card is outlined. */
  selected?: boolean;
  /**
   * Sees a click first; true means it was a selection click (⌘/Ctrl or
   * Shift), so the card doesn't open.
   */
  onSelectClick?: (event: MouseEvent<HTMLElement>) => boolean;
}) {
  const target = link?.target;
  const actions = useRef<HTMLDivElement>(null);
  const [more, setMore] = useState<PopoverAnchor>(null);
  const closeMore = () => setMore(null);
  return (
    <div
      data-board-card={card.id}
      data-selected={selected ? "" : undefined}
      onClick={(event) => {
        if (onSelectClick?.(event)) return;
        onOpen();
      }}
      onContextMenu={
        moreItems
          ? (event) => {
              event.preventDefault();
              event.stopPropagation();
              setMore({ x: event.clientX, y: event.clientY });
            }
          : undefined
      }
      className={`group/card relative cursor-pointer rounded-lg border bg-background-base px-3 py-2.5 hover:border-selection-strong ${
        selected ? "border-accent ring-1 ring-accent/60" : "border-stroke"
      }`}
    >
      <div
        className={`line-clamp-2 break-words pr-10 text-[13px] leading-snug ${
          done ? "text-content/60" : "text-content"
        }`}
      >
        {card.title}
      </div>
      {ticket ? (
        <div
          data-card-ticket={card.id}
          className="mt-1.5 flex min-w-0"
          {...noDrag}
          onClick={(event) => {
            // The chip's own controls act alone; the rest of the chip opens the card.
            if ((event.target as Element).closest("button, [role='menu']")) {
              event.stopPropagation();
            }
          }}
        >
          {/* Read on the card's drawer, not on the board: never `visible` here. */}
          <SessionTicketChip
            ticket={ticket}
            visible={false}
            onChange={(item) => {
              if (item?.kind === "ticket") {
                void applyFetchedTicket(card, ticket, item.ticket).catch(
                  () => undefined,
                );
              } else if (!item) {
                void setCardTicket(card.id, null).catch(() => undefined);
              }
            }}
            current={() => ({ kind: "ticket", ticket })}
          />
        </div>
      ) : null}
      {labels.length ? (
        <div data-card-labels="" className="mt-1.5 flex">
          <MenuButton
            label={`Labels of #${card.number}`}
            width={240}
            className="flex min-w-0 flex-wrap gap-1 rounded-full text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
            trigger={labels.map((label) => (
              <span
                key={label.id}
                className="inline-flex max-w-full items-center gap-1 rounded-full border border-stroke px-1.5 text-[11px] leading-4 text-content/70 hover:border-selection-strong"
              >
                <span
                  aria-hidden
                  className="size-1.5 shrink-0 rounded-full"
                  style={{ backgroundColor: `#${label.color}` }}
                />
                <span className="truncate">{label.name}</span>
              </span>
            ))}
          >
            {(close) => (
              <LabelPicker
                labels={allLabels}
                selected={labels.map((label) => label.id)}
                onToggle={(id) =>
                  onSetLabels(
                    toggled(
                      labels.map((label) => label.id),
                      id,
                    ),
                  )
                }
                onCreate={onCreateLabel}
                close={close}
              />
            )}
          </MenuButton>
        </div>
      ) : null}
      <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px]">
        <span className="shrink-0 font-mono text-content/45">
          #{card.number}
        </span>
        <CardExtrasRow
          number={card.number}
          extras={extras ?? EMPTY_EXTRAS}
          onSetPriority={onSetPriority}
          onSetDue={onSetDue}
        />
        {link ? <LinkChip link={link} /> : null}
        <span className="ml-auto shrink-0 text-content/45">
          {relativeTime(card.updatedAt)}
        </span>
      </div>
      <div
        ref={actions}
        className="absolute right-1.5 top-1.5 flex items-center rounded-md bg-background-base opacity-0 group-hover/card:opacity-100 focus-within:opacity-100 has-[[data-menu-open]]:opacity-100"
      >
        {target ? (
          <button
            type="button"
            aria-label="Open linked work"
            title="Open linked work"
            className={ICON_BUTTON}
            {...noDrag}
            onClick={(event) => {
              event.stopPropagation();
              onOpenLinkedWork(target);
            }}
          >
            <ExternalLink className="size-3.5" strokeWidth={1.75} />
          </button>
        ) : null}
        <MenuButton label={`Card actions for #${card.number}`} width={180}>
          {(close) => (
            <>
              <MenuItem
                onSelect={() => {
                  close();
                  onRename();
                }}
              >
                Rename
              </MenuItem>
              <MenuItem
                onSelect={() => {
                  close();
                  onLink();
                }}
              >
                Link…
              </MenuItem>
              {moreItems ? (
                <MenuItem
                  onSelect={() => {
                    close();
                    setMore(actions);
                  }}
                >
                  More…
                </MenuItem>
              ) : null}
              <MenuItem
                danger
                onSelect={() => {
                  close();
                  onDelete();
                }}
              >
                Delete
              </MenuItem>
            </>
          )}
        </MenuButton>
      </div>
      {more && moreItems ? (
        <Popover
          anchor={more}
          side="bottom"
          align={more === actions ? "end" : "start"}
          gap={4}
          width={240}
          constrainHeight={false}
          role="menu"
          aria-label={`More actions for #${card.number}`}
          onDismiss={closeMore}
          className="p-1"
          // The menu is portalled but still bubbles through the React tree.
          onPointerDown={(event) => event.stopPropagation()}
          onKeyDown={(event) => event.stopPropagation()}
          onClick={(event) => event.stopPropagation()}
          onContextMenu={(event) => {
            event.preventDefault();
            event.stopPropagation();
          }}
        >
          {moreItems(closeMore)}
        </Popover>
      ) : null}
    </div>
  );
}
