import { useEffect, useMemo, useRef, useState } from "react";
import { Modal } from "../../../shared/ui/Modal";
import {
  SearchableSelect,
  type SearchableSelectOption,
} from "../../../shared/ui/SearchableSelect";
import { ExternalLink, Trash2 } from "../../../shared/ui/icons";
import {
  deleteCard,
  linkCard,
  listActivity,
  subscribeBoard,
  updateCard,
  type BoardActivity,
  type BoardCard,
} from "../model/board";
import { LinkChip } from "./BoardCardItem";
import {
  activityText,
  errorText,
  LINK_KIND_LABEL,
  linkValue,
  parseLinkValue,
  type BoardLinkTarget,
  type ResolvedLink,
} from "./boardFormat";

const ACTIVITY_LIMIT = 50;
const NO_LINK = "";

const FIELD =
  "w-full rounded-lg border border-stroke bg-background-base px-3 text-[13px] text-content outline-none placeholder:text-content/35 focus:border-accent/60";

/**
 * A card opened from the board: its title and notes, its linked work and the
 * activity timeline. Edits save on blur; the link saves on pick.
 */
export function CardDetail({
  card,
  columnName,
  link,
  linkTargets,
  focus,
  onClose,
  onOpenLinkedWork,
  onError,
}: {
  card: BoardCard;
  columnName: string;
  link: ResolvedLink | null;
  linkTargets: readonly BoardLinkTarget[];
  focus?: "title" | "link";
  onClose: () => void;
  onOpenLinkedWork: (target: BoardLinkTarget) => void;
  onError: (message: string) => void;
}) {
  const [title, setTitle] = useState(card.title);
  const [body, setBody] = useState(card.body);
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
  const titleField = useRef<HTMLInputElement>(null);
  const linkRow = useRef<HTMLDivElement>(null);
  // What was last saved, so blur and close don't both write the same edit.
  const saved = useRef({ title: card.title, body: card.body });

  // A save elsewhere (or an agent) updates the card under us.
  useEffect(() => {
    saved.current = { title: card.title, body: card.body };
    setTitle(card.title);
    setBody(card.body);
  }, [card.title, card.body]);

  useEffect(() => {
    if (focus === "title") titleField.current?.select();
    else if (focus === "link") {
      linkRow.current?.querySelector<HTMLButtonElement>("button")?.focus();
    }
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
    void load();
    const stop = subscribeBoard(card.boardId, () => void load());
    return () => {
      live = false;
      stop();
    };
  }, [card.boardId, card.id, card.number]);

  const options = useMemo((): SearchableSelectOption[] => {
    const linkable = linkTargets.filter(
      (target) => target.kind === "session" || target.kind === "pilot",
    );
    return [
      { value: NO_LINK, label: "No linked work" },
      ...linkable.map((target) => ({
        value: linkValue(target.kind, target.id),
        label: `${LINK_KIND_LABEL[target.kind]} · ${target.label}`,
        keywords: target.detail,
      })),
    ];
  }, [linkTargets]);

  const saveFields = () => {
    const nextTitle = title.trim();
    if (!nextTitle) {
      setTitle(saved.current.title);
      return;
    }
    if (nextTitle === saved.current.title && body === saved.current.body) {
      return;
    }
    saved.current = { title: nextTitle, body };
    updateCard(card.id, nextTitle, body).catch((error) =>
      onError(`Couldn't save card #${card.number}: ${errorText(error)}`),
    );
  };

  const changeLink = (value: string) => {
    linkCard(card.id, value === NO_LINK ? null : parseLinkValue(value)).catch(
      (error) =>
        onError(`Couldn't link card #${card.number}: ${errorText(error)}`),
    );
  };

  const remove = () => {
    deleteCard(card.id)
      .then(onClose)
      .catch((error) =>
        onError(`Couldn't delete card #${card.number}: ${errorText(error)}`),
      );
  };

  const current =
    card.linkKind && card.linkId ? linkValue(card.linkKind, card.linkId) : "";
  const target = link?.target ?? null;

  return (
    <Modal
      title={`Card #${card.number}`}
      description={columnName}
      size="md"
      onClose={() => {
        saveFields();
        onClose();
      }}
    >
      <div className="flex flex-col gap-4 px-4 pb-4 pt-3">
        <label className="flex flex-col gap-1">
          <span className="text-[11px] text-content/50">Title</span>
          <input
            ref={titleField}
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            onBlur={saveFields}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                event.preventDefault();
                event.currentTarget.blur();
              }
            }}
            className={`${FIELD} h-8`}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-[11px] text-content/50">Notes</span>
          <textarea
            value={body}
            rows={4}
            placeholder="Add notes…"
            onChange={(event) => setBody(event.target.value)}
            onBlur={saveFields}
            className={`${FIELD} resize-y py-2`}
          />
        </label>
        <div ref={linkRow} className="flex flex-col gap-1">
          <span className="text-[11px] text-content/50">Linked work</span>
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
                className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-content/15 px-2.5 text-[12px] text-content/80 hover:border-content/30 hover:bg-content/10 hover:text-content"
              >
                <ExternalLink className="size-3.5" strokeWidth={1.75} />
                Open linked work
              </button>
            ) : null}
          </div>
          {link ? (
            <div className="text-[11px]">
              <LinkChip link={link} full />
            </div>
          ) : null}
        </div>
        <div className="flex flex-col gap-1">
          <span className="text-[11px] text-content/50">Activity</span>
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
            <span className="text-[12px] text-content/45">No activity yet</span>
          ) : null}
        </div>
        <div className="flex justify-end">
          <button
            type="button"
            onClick={remove}
            className="inline-flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[12px] text-red-300/90 hover:bg-red-500/15"
          >
            <Trash2 className="size-3.5" strokeWidth={1.75} />
            Delete card
          </button>
        </div>
      </div>
    </Modal>
  );
}
