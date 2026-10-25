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

/** Priority, due date and subtask progress, when the card has any. */
function CardExtrasRow({ extras }: { extras: CardExtras }) {
  const due = extras.dueAt === null ? null : dueChip(extras.dueAt);
  const priority = PRIORITIES[extras.priority] ?? PRIORITIES[0];
  if (!extras.priority && !due && !extras.subtasksTotal && !extras.comments) {
    return null;
  }
  return (
    <>
      {extras.priority ? (
        <span
          data-card-priority={extras.priority}
          className={`shrink-0 ${extras.priority >= 4 ? "text-red-400" : "text-content/70"}`}
        >
          {priority}
        </span>
      ) : null}
      {due ? (
        <span
          data-card-due={due.tone}
          className={`shrink-0 ${DUE_TONE[due.tone]}`}
        >
          {due.text}
        </span>
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
  onOpen,
  onOpenLinkedWork,
  onRename,
  onLink,
  onDelete,
}: {
  card: BoardCard;
  /** In a column with the `done` role: the title is dimmed. */
  done: boolean;
  link: ResolvedLink | null;
  ticket?: TicketRef | null;
  extras?: CardExtras;
  /** The card's labels, in the board's order. */
  labels?: readonly BoardLabel[];
  onOpen: () => void;
  onOpenLinkedWork: (target: BoardLinkTarget) => void;
  onRename: () => void;
  onLink: () => void;
  onDelete: () => void;
}) {
  const target = link?.target;
  return (
    <div
      data-board-card={card.id}
      onClick={onOpen}
      className="group/card relative cursor-pointer rounded-lg border border-stroke bg-background-base px-3 py-2.5 hover:border-selection-strong"
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
        <div data-card-labels="" className="mt-1.5 flex flex-wrap gap-1">
          {labels.map((label) => (
            <span
              key={label.id}
              className="inline-flex max-w-full items-center gap-1 rounded-full border border-stroke px-1.5 text-[11px] leading-4 text-content/70"
            >
              <span
                aria-hidden
                className="size-1.5 shrink-0 rounded-full"
                style={{ backgroundColor: `#${label.color}` }}
              />
              <span className="truncate">{label.name}</span>
            </span>
          ))}
        </div>
      ) : null}
      <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px]">
        <span className="shrink-0 font-mono text-content/45">
          #{card.number}
        </span>
        {extras ? <CardExtrasRow extras={extras} /> : null}
        {link ? <LinkChip link={link} /> : null}
        <span className="ml-auto shrink-0 text-content/45">
          {relativeTime(card.updatedAt)}
        </span>
      </div>
      <div className="absolute right-1.5 top-1.5 flex items-center rounded-md bg-background-base opacity-0 group-hover/card:opacity-100 focus-within:opacity-100 has-[[data-menu-open]]:opacity-100">
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
    </div>
  );
}
