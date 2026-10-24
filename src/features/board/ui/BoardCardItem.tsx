import {
  Bot,
  ExternalLink,
  MessageSquare,
  Play,
} from "../../../shared/ui/icons";
import type { BoardCard, CardLinkKind } from "../model/board";
import { ICON_BUTTON, MenuButton, MenuItem, noDrag } from "./boardControls";
import {
  LINK_KIND_LABEL,
  relativeTime,
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
      <div className="mt-1.5 flex min-w-0 items-center gap-2 text-[11px]">
        <span className="shrink-0 font-mono text-content/45">
          #{card.number}
        </span>
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
