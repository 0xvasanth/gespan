import { useRef, useState, type MouseEvent, type ReactNode } from "react";
import { Popover, type PopoverAnchor } from "../../../shared/ui/Popover";
import {
  Bot,
  ExternalLink,
  Lock,
  MessageSquare,
  Play,
} from "../../../shared/ui/icons";
import type { TicketRef } from "../../sessions/model/session";
import { SessionTicketChip } from "../../sessions/ui/SessionTicketChip";
import {
  setCardTicket,
  type BoardCard,
  type BoardLabel,
  type CardExtras,
  type CardLinkKind,
  type DisplayOptions,
} from "../model/board";
import { DEFAULT_DISPLAY } from "../model/display";
import { chipStatusText, type LinkChipView } from "../model/linkStatus";
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
import { priorityName } from "../model/boardSettings";
import { usePriorityScheme } from "./priorityScheme";

const KIND_ICON: Record<CardLinkKind, typeof MessageSquare> = {
  session: MessageSquare,
  pilot: Bot,
  "flow-run": Play,
};

const DOT: Record<NonNullable<BoardLinkTarget["status"]>, string> = {
  running: "bg-accent animate-pulse motion-reduce:animate-none",
  waiting: "bg-amber-400",
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
  face,
  onSetPriority,
  onSetDue,
}: {
  number: number;
  extras: CardExtras;
  face: DisplayOptions["face"];
  onSetPriority: (priority: number) => void;
  onSetDue: (dueAt: number | null) => void;
}) {
  const scheme = usePriorityScheme();
  const due = extras.dueAt === null ? null : dueChip(extras.dueAt);
  const priority = priorityName(scheme, extras.priority);
  return (
    <>
      {!face.priority ? null : extras.priority ? (
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
      {face.due && due ? (
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
      {face.subtasks && extras.subtasksTotal ? (
        <span
          data-card-subtasks=""
          title="Subtasks done"
          className="shrink-0 tabular-nums text-content/60"
        >
          {extras.subtasksDone}/{extras.subtasksTotal}
        </span>
      ) : null}
      {face.comments && extras.comments ? (
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

const CHIP_DOT: Record<LinkChipView["state"], string> = {
  running: DOT.running,
  waiting: DOT.waiting,
  failed: DOT.failed,
  completed: DOT.completed,
  idle: "bg-content/30",
  open: "bg-accent",
  merged: "bg-purple-400",
  closed: "bg-content/40",
  unknown: "bg-content/25",
  missing: "bg-content/25",
};

const CHECKS_TEXT = { passing: "checks passing", failing: "checks failing", pending: "checks pending" };

/** One chip for a link beyond the primary: its label, state, checks or diff. */
function ExtraLinkChip({ chip }: { chip: LinkChipView }) {
  return (
    <span
      data-link-chip-kind={chip.kind}
      data-link-state={chip.state}
      title={chipStatusText(chip)}
      className="inline-flex min-w-0 items-center gap-1 text-content/70"
    >
      <span className="max-w-[120px] truncate">{chip.label}</span>
      <span
        role="img"
        aria-label={chip.state}
        className={`size-1.5 shrink-0 rounded-full ${CHIP_DOT[chip.state]}`}
      />
      {chip.checks ? (
        <span data-link-checks={chip.checks} className="shrink-0 text-content/55">
          {CHECKS_TEXT[chip.checks]}
        </span>
      ) : null}
      {chip.diff ? (
        <span data-link-diff="" className="shrink-0 tabular-nums text-content/55">
          +{chip.diff.additions} −{chip.diff.deletions}
        </span>
      ) : null}
    </span>
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
  display = DEFAULT_DISPLAY,
  fieldChips = [],
  linkChips = [],
  blockedBy = [],
}: {
  /** One chip for each link besides the primary one (sessions, Pilots, PRs, worktrees). */
  linkChips?: readonly LinkChipView[];
  /** Numbers of the open cards blocking this one; any means a lock. */
  blockedBy?: readonly number[];
  card: BoardCard;
  /** What the face shows and how tight it is; the board's default when absent. */
  display?: DisplayOptions;
  /** Custom-field values to show, in the order the display lists them. */
  fieldChips?: readonly { id: string; name: string; text: string }[];
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
      data-density={display.density}
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
      className={`group/card relative cursor-pointer rounded-lg border bg-background-base hover:border-selection-strong ${
        display.density === "compact" ? "px-2.5 py-1.5" : "px-3 py-2.5"
      } ${
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
      {display.face.labels && labels.length ? (
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
      {fieldChips.length ? (
        <div
          data-card-fields=""
          className="mt-1.5 flex flex-wrap gap-x-2 gap-y-0.5 text-[11px] text-content/60"
        >
          {fieldChips.map((chip) => (
            <span
              key={chip.id}
              data-card-field={chip.id}
              className="max-w-full truncate"
              title={chip.name}
            >
              {chip.name}: {chip.text}
            </span>
          ))}
        </div>
      ) : null}
      <div
        className={`flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] ${
          display.density === "compact" ? "mt-1" : "mt-1.5"
        }`}
      >
        {display.face.number ? (
          <span
            data-card-number=""
            className="shrink-0 font-mono text-content/45"
          >
            #{card.number}
          </span>
        ) : null}
        {blockedBy.length ? (
          <span
            data-card-blocked=""
            role="img"
            aria-label={`Blocked by ${blockedBy.map((n) => `#${n}`).join(", ")}`}
            title={`Blocked by ${blockedBy.map((n) => `#${n}`).join(", ")}`}
            className="inline-flex shrink-0 text-amber-400"
          >
            <Lock className="size-3" strokeWidth={1.75} />
          </span>
        ) : null}
        <CardExtrasRow
          number={card.number}
          face={display.face}
          extras={extras ?? EMPTY_EXTRAS}
          onSetPriority={onSetPriority}
          onSetDue={onSetDue}
        />
        {display.face.linkStatus && link ? <LinkChip link={link} /> : null}
        {display.face.linkStatus
          ? linkChips.map((chip) => (
              <ExtraLinkChip key={`${chip.kind}\n${chip.reference}`} chip={chip} />
            ))
          : null}
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
