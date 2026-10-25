import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Check, Plus, Trash2, X } from "../../../shared/ui/icons";
import {
  addComment,
  addSubtask,
  deleteSubtask,
  listComments,
  listSubtasks,
  subscribeBoard,
  toggleSubtask,
  type BoardComment,
  type BoardLabel,
  type BoardSubtask,
} from "../model/board";
import { MenuItem, MenuSeparator, NameInput } from "./boardControls";
import { actorLabel, errorText, relativeTime } from "./boardFormat";

export const FIELD =
  "w-full rounded-lg border border-stroke bg-background-base px-3 text-[13px] text-content outline-none placeholder:text-content/35 focus:border-accent/60";

/** The button a property row's menu opens from. */
export const PROPERTY_TRIGGER =
  "inline-flex h-7 min-w-0 max-w-full items-center gap-1.5 rounded-md px-2 text-[13px] text-content/85 hover:bg-content/8 hover:text-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/60";

export const SMALL_BUTTON =
  "inline-flex h-7 shrink-0 items-center rounded-md border border-content/15 px-2.5 text-[12px] text-content/80 hover:border-content/30 hover:bg-content/10 hover:text-content disabled:cursor-default disabled:opacity-40 disabled:hover:border-content/15 disabled:hover:bg-transparent";

export function PropertyRow({
  name,
  label,
  children,
}: {
  name: string;
  label: string;
  children: ReactNode;
}) {
  return (
    <div data-property={name} className="flex min-h-8 items-start gap-2">
      <span className="flex h-8 w-20 shrink-0 items-center text-[12px] text-content/50">
        {label}
      </span>
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5 py-0.5">
        {children}
      </div>
    </div>
  );
}

/**
 * A value that follows `value` (what the board last loaded) but can be
 * ahead of it: a write shows at once, and a failed one puts it back.
 */
export function useAhead<T>(value: T, key: string) {
  const [ahead, setAhead] = useState(value);
  const latest = useRef(value);
  useEffect(() => {
    latest.current = value;
    setAhead(value);
    // `key` stands in for `value`, which may be a fresh array each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  const revert = useCallback(() => setAhead(latest.current), []);
  return [ahead, setAhead, revert] as const;
}

/** Loads on mount and again whenever the board is written to. */
export function useLive<T>(
  boardId: string,
  load: () => Promise<T>,
  fail: (cause: unknown) => void,
) {
  const [value, setValue] = useState<T | null>(null);
  const loadRef = useRef(load);
  const failRef = useRef(fail);
  useEffect(() => {
    loadRef.current = load;
    failRef.current = fail;
  });
  const live = useRef(true);
  const reload = useCallback(() => {
    loadRef
      .current()
      .then((next) => {
        if (live.current) setValue(next);
      })
      .catch((cause) => {
        if (live.current) failRef.current(cause);
      });
  }, []);
  useEffect(() => {
    live.current = true;
    reload();
    const stop = subscribeBoard(boardId, reload);
    return () => {
      live.current = false;
      stop();
    };
  }, [boardId, reload]);
  return [value, reload] as const;
}

export function toggled<T>(list: readonly T[], item: T): T[] {
  return list.includes(item) ? list.filter((x) => x !== item) : [...list, item];
}

function CheckMark({ checked }: { checked: boolean }) {
  return (
    <span className="grid size-3.5 shrink-0 place-items-center">
      {checked ? <Check className="size-3.5" strokeWidth={2} /> : null}
    </span>
  );
}

export function LabelDot({ color }: { color: string }) {
  return (
    <span
      aria-hidden
      className="size-2 shrink-0 rounded-full"
      style={{ backgroundColor: `#${color}` }}
    />
  );
}

/** The Labels menu: every board label as a checkbox, then inline "Create label". */
export function LabelPicker({
  labels,
  selected,
  onToggle,
  onCreate,
  close,
}: {
  labels: readonly BoardLabel[];
  selected: readonly string[];
  onToggle: (id: string) => void;
  onCreate: (name: string) => void;
  close: () => void;
}) {
  const [creating, setCreating] = useState(false);
  return (
    <>
      {labels.map((label) => (
        <MenuItem
          key={label.id}
          kind="checkbox"
          checked={selected.includes(label.id)}
          onSelect={() => onToggle(label.id)}
        >
          <CheckMark checked={selected.includes(label.id)} />
          <LabelDot color={label.color} />
          <span className="min-w-0 truncate">{label.name}</span>
        </MenuItem>
      ))}
      {labels.length ? <MenuSeparator /> : null}
      {creating ? (
        <div className="px-1 py-0.5">
          <NameInput
            label="Label name"
            placeholder="Label name"
            className="w-full"
            onSubmit={(name) => {
              close();
              onCreate(name);
            }}
            onCancel={() => setCreating(false)}
          />
        </div>
      ) : (
        <MenuItem onSelect={() => setCreating(true)}>
          <Plus className="size-3.5 shrink-0" strokeWidth={1.75} />
          Create label
        </MenuItem>
      )}
    </>
  );
}

/** Subtasks with their `done/total` progress, a checkbox and delete each, and an add field. */
export function Subtasks({
  cardId,
  boardId,
  number,
  onError,
}: {
  cardId: string;
  boardId: string;
  number: number;
  onError: (message: string) => void;
}) {
  const [subtasks, reload] = useLive<BoardSubtask[]>(
    boardId,
    () => listSubtasks(cardId),
    (cause) =>
      onError(
        `Couldn't load subtasks for card #${number}: ${errorText(cause)}`,
      ),
  );
  const [title, setTitle] = useState("");
  const run = (what: string, work: Promise<unknown>) =>
    work
      .then(reload)
      .catch((cause) =>
        onError(`Couldn't ${what} on card #${number}: ${errorText(cause)}`),
      );
  const done = subtasks?.filter((subtask) => subtask.done).length ?? 0;
  const add = () => {
    const next = title.trim();
    if (!next) return;
    setTitle("");
    void run("add the subtask", addSubtask(cardId, next));
  };
  return (
    <section className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2 text-[12px] text-content/50">
        <span>Subtasks</span>
        {subtasks?.length ? (
          <span data-subtask-progress="" className="tabular-nums">
            {done}/{subtasks.length}
          </span>
        ) : null}
      </div>
      {subtasks?.length ? (
        <ul className="flex flex-col">
          {subtasks.map((subtask) => (
            <li
              key={subtask.id}
              className="group flex h-8 items-center gap-2 rounded-md px-1 hover:bg-content/5"
            >
              <input
                type="checkbox"
                aria-label={`Done: ${subtask.title}`}
                checked={subtask.done}
                onChange={() =>
                  void run(
                    "update the subtask",
                    toggleSubtask(subtask.id, !subtask.done),
                  )
                }
                className="size-3.5 shrink-0 accent-accent"
              />
              <span
                className={`min-w-0 flex-1 truncate text-[13px] ${subtask.done ? "text-content/45 line-through" : "text-content/85"}`}
              >
                {subtask.title}
              </span>
              <button
                type="button"
                aria-label={`Delete subtask ${subtask.title}`}
                title="Delete subtask"
                onClick={() =>
                  void run("delete the subtask", deleteSubtask(subtask.id))
                }
                className="grid size-6 shrink-0 place-items-center rounded-md text-content/40 opacity-0 hover:bg-content/10 hover:text-content focus-visible:opacity-100 group-hover:opacity-100"
              >
                <Trash2 className="size-3.5" strokeWidth={1.75} />
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      <input
        aria-label="New subtask"
        placeholder="Add a subtask…"
        value={title}
        onChange={(event) => setTitle(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.nativeEvent.isComposing) {
            event.preventDefault();
            add();
          }
        }}
        className={`${FIELD} h-8`}
      />
    </section>
  );
}

/** The card's comments, newest last, with a box to add one. */
export function Comments({
  cardId,
  boardId,
  number,
  onError,
}: {
  cardId: string;
  boardId: string;
  number: number;
  onError: (message: string) => void;
}) {
  const [comments, reload] = useLive<BoardComment[]>(
    boardId,
    () => listComments(cardId),
    (cause) =>
      onError(
        `Couldn't load comments for card #${number}: ${errorText(cause)}`,
      ),
  );
  const [draft, setDraft] = useState("");
  const body = draft.trim();
  const send = () => {
    if (!body) return;
    addComment(cardId, body)
      .then(() => {
        setDraft("");
        reload();
      })
      .catch((cause) =>
        onError(
          `Couldn't add the comment to card #${number}: ${errorText(cause)}`,
        ),
      );
  };
  return (
    <div className="flex flex-col gap-2">
      {comments?.length ? (
        <ol className="flex flex-col gap-2">
          {comments.map((comment) => (
            <li key={comment.id} className="flex flex-col gap-0.5">
              <span className="text-[11px] text-content/45">
                {actorLabel(comment.author)} · {relativeTime(comment.createdAt)}
              </span>
              <span className="whitespace-pre-wrap break-words text-[13px] text-content/85">
                {comment.body}
              </span>
            </li>
          ))}
        </ol>
      ) : comments ? (
        <span className="text-[12px] text-content/45">No comments yet</span>
      ) : null}
      <div className="flex items-end gap-2">
        <textarea
          aria-label="Add a comment"
          placeholder="Add a comment…"
          rows={2}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (
              event.key === "Enter" &&
              (event.metaKey || event.ctrlKey) &&
              !event.nativeEvent.isComposing
            ) {
              event.preventDefault();
              send();
            }
          }}
          className={`${FIELD} resize-none py-2`}
        />
        <button
          type="button"
          aria-label="Send comment"
          disabled={!body}
          onClick={send}
          className={`${SMALL_BUTTON} h-8`}
        >
          Comment
        </button>
      </div>
    </div>
  );
}

export function CloseButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      aria-label="Close card"
      title="Close"
      onClick={onClick}
      className="grid size-6 shrink-0 place-items-center rounded-md text-content/50 hover:bg-content/10 hover:text-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
    >
      <X className="size-3.5" strokeWidth={1.75} />
    </button>
  );
}
