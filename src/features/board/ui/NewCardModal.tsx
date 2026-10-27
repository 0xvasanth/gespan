import { useEffect, useRef, useState } from "react";
import { Modal } from "../../../shared/ui/Modal";
import {
  createCard,
  createLabel,
  setCardDue,
  setCardLabels,
  setCardPriority,
  setCardProjects,
  type BoardColumn,
  type BoardLabel,
} from "../model/board";
import { dueChip, errorText, nextLabelColor } from "./boardFormat";
import { MenuButton, MenuItem, MenuSeparator } from "./boardControls";
import { DueDateInput, DueItems, PriorityItems } from "./CardPropertyMenus";
import {
  FIELD,
  LabelPicker,
  PROPERTY_TRIGGER,
  PropertyRow,
  toggled,
} from "./CardDrawerParts";
import { priorityName } from "../model/boardSettings";
import { usePriorityScheme } from "./priorityScheme";

/** The column a new card starts in: the first `todo` column, else the first. */
export function defaultColumnId(
  columns: readonly BoardColumn[],
): string | null {
  const ordered = [...columns].sort((a, b) => a.position - b.position);
  return (
    (ordered.find((column) => column.role === "todo") ?? ordered[0])?.id ?? null
  );
}

type Draft = {
  columnId: string;
  title: string;
  body: string;
  priority: number;
  dueAt: number | null;
  labelIds: string[];
  projects: string[];
};

/**
 * Makes the card, then one setter per field that was set. A setter that fails
 * does not undo the card: it is named in the returned list.
 */
export async function createFromDraft(draft: Draft): Promise<string[]> {
  const card = await createCard(draft.columnId, draft.title, draft.body);
  const failed: string[] = [];
  const attempt = async (what: string, run: () => Promise<void>) => {
    try {
      await run();
    } catch (cause) {
      failed.push(`${what}: ${errorText(cause)}`);
    }
  };
  if (draft.priority !== 0) {
    await attempt("priority", () => setCardPriority(card.id, draft.priority));
  }
  if (draft.dueAt !== null) {
    await attempt("due date", () => setCardDue(card.id, draft.dueAt));
  }
  if (draft.labelIds.length) {
    await attempt("labels", () => setCardLabels(card.id, draft.labelIds));
  }
  if (draft.projects.length) {
    await attempt("projects", () => setCardProjects(card.id, draft.projects));
  }
  return failed;
}

/** What is shown after a card was made but some fields were not saved. */
export function notSavedMessage(failed: readonly string[]): string {
  return `Card created, but not saved: ${failed.join("; ")}`;
}

/**
 * The New card modal: title, description, column, priority, due date, labels
 * and projects. Cmd/Ctrl+Enter or the Create button makes the card; "Create
 * more" keeps the modal open with the title and description cleared.
 */
export function NewCardModal({
  boardId,
  columns,
  labels,
  projects,
  onClose,
  onNotice,
  returnFocusTo,
}: {
  boardId: string;
  columns: readonly BoardColumn[];
  labels: readonly BoardLabel[];
  projects: readonly { path: string; name: string }[];
  onClose: () => void;
  /** Told when the card was made but a field was not saved, after the modal closes. */
  onNotice: (message: string) => void;
  /** Where focus goes on close when what had it before is gone. */
  returnFocusTo?: () => HTMLElement | null;
}) {
  const scheme = usePriorityScheme();
  // Whatever had focus when the modal opened gets it back on close, so the
  // board keeps its keys (Cmd+N again) instead of focus falling to the page.
  const opener = useRef<Element | null>(document.activeElement);
  const fallback = useRef(returnFocusTo);
  fallback.current = returnFocusTo;
  // Only a real close restores it: an effect cleanup would also run in
  // StrictMode's simulated unmount and pull focus off the Title field.
  const close = () => {
    const before = opener.current;
    if (before instanceof HTMLElement && before.isConnected) before.focus();
    else fallback.current?.()?.focus();
    onClose();
  };
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [columnId, setColumnId] = useState(
    () => defaultColumnId(columns) ?? "",
  );
  const [priority, setPriority] = useState(0);
  const [dueAt, setDueAt] = useState<number | null>(null);
  const [labelIds, setLabelIds] = useState<string[]>([]);
  const [projectPaths, setProjectPaths] = useState<string[]>([]);
  const [createMore, setCreateMore] = useState(false);
  const [titleMissing, setTitleMissing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const titleField = useRef<HTMLInputElement>(null);
  const busy = useRef(false);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    titleField.current?.focus();
    return () => {
      mounted.current = false;
    };
  }, []);

  const closeRef = useRef(close);
  closeRef.current = close;

  const submit = async () => {
    if (busy.current) return;
    const name = title.trim();
    if (!name) {
      setTitleMissing(true);
      titleField.current?.focus();
      return;
    }
    if (!columns.some((column) => column.id === columnId)) {
      setError("Pick a column first");
      return;
    }
    busy.current = true;
    setError(null);
    try {
      const failed = await createFromDraft({
        columnId,
        title: name,
        body: body.trimEnd(),
        priority,
        dueAt,
        labelIds,
        projects: projectPaths,
      });
      if (!mounted.current) return;
      const message = failed.length ? notSavedMessage(failed) : null;
      if (createMore) {
        setTitle("");
        setBody("");
        setTitleMissing(false);
        setError(message);
        titleField.current?.focus();
      } else {
        closeRef.current();
        if (message) onNotice(message);
      }
    } catch (cause) {
      if (mounted.current)
        setError(`Couldn't add the card: ${errorText(cause)}`);
    } finally {
      busy.current = false;
    }
  };

  // Cmd/Ctrl+Enter submits from anywhere in the modal, menu buttons included.
  const submitRef = useRef(submit);
  submitRef.current = submit;
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (
        event.key !== "Enter" ||
        !(event.metaKey || event.ctrlKey) ||
        event.isComposing ||
        event.repeat
      )
        return;
      event.preventDefault();
      event.stopPropagation();
      void submitRef.current();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  const columnName = columns.find((column) => column.id === columnId)?.name;
  const chosenLabels = labels.filter((label) => labelIds.includes(label.id));
  const chosenProjects = projectPaths.map(
    (path) => projects.find((project) => project.path === path)?.name ?? path,
  );

  return (
    <Modal title="New card" size="md" hideClose onClose={close}>
      <form
        className="flex flex-col gap-3 px-4 pb-4 pt-2"
        onSubmit={(event) => event.preventDefault()}
      >
        <div className="flex flex-col gap-1">
          <input
            ref={titleField}
            aria-label="Title"
            aria-invalid={titleMissing || undefined}
            placeholder="Card title"
            value={title}
            onChange={(event) => {
              setTitle(event.target.value);
              setTitleMissing(false);
            }}
            onKeyDown={(event) => {
              if (
                event.key === "Enter" &&
                !event.nativeEvent.isComposing &&
                !event.metaKey &&
                !event.ctrlKey
              ) {
                event.preventDefault();
                void submit();
              }
            }}
            className={`h-9 ${FIELD}`}
          />
          {titleMissing ? (
            <span role="alert" className="text-[12px] text-red-300/90">
              Add a title
            </span>
          ) : null}
        </div>
        <textarea
          aria-label="Description"
          placeholder="Add a description…"
          rows={4}
          value={body}
          onChange={(event) => setBody(event.target.value)}
          className={`resize-none py-2 ${FIELD}`}
        />
        <div className="flex flex-col">
          <PropertyRow name="column" label="Column">
            <MenuButton
              label="Column"
              width={220}
              className={PROPERTY_TRIGGER}
              openOnArrowDown
              trigger={<span className="truncate">{columnName ?? "None"}</span>}
            >
              {(close) =>
                columns.map((column) => (
                  <MenuItem
                    key={column.id}
                    checked={column.id === columnId}
                    onSelect={() => {
                      close();
                      setColumnId(column.id);
                    }}
                  >
                    <span className="min-w-0 truncate">{column.name}</span>
                  </MenuItem>
                ))
              }
            </MenuButton>
          </PropertyRow>
          <PropertyRow name="priority" label="Priority">
            <MenuButton
              label="Priority"
              className={PROPERTY_TRIGGER}
              openOnArrowDown
              trigger={<span className="truncate">{priorityName(scheme, priority)}</span>}
            >
              {(close) => (
                <PriorityItems
                  priority={priority}
                  onChoose={setPriority}
                  close={close}
                />
              )}
            </MenuButton>
          </PropertyRow>
          <PropertyRow name="due" label="Due">
            <MenuButton
              label="Due"
              width={220}
              className={PROPERTY_TRIGGER}
              openOnArrowDown
              trigger={
                <span className="truncate">
                  {dueAt === null ? "None" : dueChip(dueAt).text}
                </span>
              }
            >
              {(close) => (
                <>
                  <DueItems dueAt={dueAt} onChoose={setDueAt} close={close} />
                  <MenuSeparator />
                  <div className="px-1 py-0.5">
                    <DueDateInput
                      dueAt={dueAt}
                      onChoose={setDueAt}
                      className="h-7 w-full rounded-md border border-stroke bg-background-base px-2 text-[12px] text-content/80 outline-none focus:border-accent/60"
                    />
                  </div>
                </>
              )}
            </MenuButton>
          </PropertyRow>
          <PropertyRow name="labels" label="Labels">
            <MenuButton
              label="Labels"
              width={240}
              className={PROPERTY_TRIGGER}
              openOnArrowDown
              trigger={
                <span className="truncate">
                  {chosenLabels.length
                    ? chosenLabels.map((label) => label.name).join(", ")
                    : "None"}
                </span>
              }
            >
              {(close) => (
                <LabelPicker
                  labels={labels}
                  selected={labelIds}
                  onToggle={(id) => setLabelIds((ids) => toggled(ids, id))}
                  onCreate={(name) =>
                    void createLabel(
                      boardId,
                      name,
                      nextLabelColor(labels.length),
                    )
                      .then((created) =>
                        setLabelIds((ids) => [...ids, created.id]),
                      )
                      .catch((cause) =>
                        setError(
                          `Couldn't create the label: ${errorText(cause)}`,
                        ),
                      )
                  }
                  close={close}
                />
              )}
            </MenuButton>
          </PropertyRow>
          <PropertyRow name="projects" label="Projects">
            <MenuButton
              label="Projects"
              width={240}
              className={PROPERTY_TRIGGER}
              openOnArrowDown
              trigger={
                <span className="truncate">
                  {chosenProjects.length ? chosenProjects.join(", ") : "None"}
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
                        setProjectPaths((paths) => toggled(paths, project.path))
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
        </div>
        {error ? (
          <p role="alert" className="text-[12px] text-red-300/90">
            {error}
          </p>
        ) : null}
        <div className="flex items-center gap-3">
          <label className="flex flex-1 items-center gap-2 text-[12px] text-content/70">
            <input
              type="checkbox"
              aria-label="Create more"
              checked={createMore}
              onChange={(event) => setCreateMore(event.target.checked)}
            />
            Create more
          </label>
          <button
            type="button"
            onClick={() => void submit()}
            className="h-7 rounded-md bg-content px-3 text-[12px] font-medium text-background-base hover:bg-content/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
          >
            Create
          </button>
        </div>
      </form>
    </Modal>
  );
}
