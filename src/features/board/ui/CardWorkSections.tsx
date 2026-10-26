import { useEffect, useState } from "react";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { SearchableSelect } from "../../../shared/ui/SearchableSelect";
import { Plus, X } from "../../../shared/ui/icons";
import {
  addCardLink,
  addRelation,
  getCardMeta,
  listCardLinks,
  listCardRelations,
  listFieldValues,
  listFields,
  removeCardLink,
  removeRelation,
  setCardEstimate,
  setCardStart,
  setFieldValue,
  type BoardCard,
  type BoardField,
  type CardLinkRow,
  type CardMeta,
  type CardRelation,
  type FieldValue,
} from "../model/board";
import {
  RELATION_CHOICES,
  encodeField,
  encodePicks,
  fieldPicks,
  fieldChecked,
  fieldOptions,
  fieldText,
  formatEstimate,
  groupRelations,
  parseEstimate,
  relationCandidates,
  relationWrite,
  validPrUrl,
  visibleLinks,
  type RelationChoice,
} from "../model/cardProps";
import { MenuButton, MenuItem } from "./boardControls";
import {
  PROPERTY_TRIGGER,
  PropertyRow,
  SMALL_BUTTON,
  useAhead,
  useLive,
} from "./CardDrawerParts";
import { DueDateInput, DueItems } from "./CardPropertyMenus";
import {
  LINK_KIND_LABEL,
  errorText,
  linkValue,
  type BoardLinkTarget,
} from "./boardFormat";

const INPUT =
  "h-7 min-w-0 rounded-md border border-stroke bg-background-base px-2 text-[12px] text-content/80 outline-none focus:border-accent/60";

function InlineError({ message }: { message: string | null }) {
  return message ? (
    <span role="alert" className="text-[12px] text-red-300/90">
      {message}
    </span>
  ) : null;
}

const ADD_TRIGGER = (
  <span className="inline-flex items-center gap-1">
    <Plus className="size-3.5" strokeWidth={1.75} />
    Add
  </span>
);

/** The card's start date and estimate rows. */
export function StartAndEstimateRows({
  card,
  onError,
}: {
  card: BoardCard;
  onError: (message: string) => void;
}) {
  const [meta] = useLive<CardMeta>(
    card.boardId,
    () => getCardMeta(card.id),
    (cause) =>
      onError(
        `Couldn't load details of card #${card.number}: ${errorText(cause)}`,
      ),
  );
  const [startAt, setStartAt, revertStart] = useAhead(
    meta?.startAt ?? null,
    String(meta?.startAt ?? null),
  );
  const [minutes, setMinutes, revertMinutes] = useAhead(
    meta?.estimateMinutes ?? null,
    String(meta?.estimateMinutes ?? null),
  );
  const [text, setText] = useState("");
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!editing) setText(minutes === null ? "" : formatEstimate(minutes));
  }, [minutes, editing]);

  const chooseStart = (next: number | null) => {
    setStartAt(next);
    setCardStart(card.id, next).catch((cause) => {
      revertStart();
      onError(
        `Couldn't set the start date of card #${card.number}: ${errorText(cause)}`,
      );
    });
  };

  const writeEstimate = (next: number | null) => {
    setMinutes(next);
    setCardEstimate(card.id, next).catch((cause) => {
      revertMinutes();
      setError(errorText(cause));
    });
  };
  const commitEstimate = () => {
    if (!text.trim()) {
      setEditing(false);
      setError(null);
      if (minutes !== null) writeEstimate(null);
      return;
    }
    const parsed = parseEstimate(text);
    if (!parsed.ok) {
      // Still editing: the typed text stays to be corrected, not replaced
      // by the saved estimate.
      setError(parsed.error);
      return;
    }
    setEditing(false);
    setError(null);
    setText(formatEstimate(parsed.minutes));
    if (parsed.minutes !== minutes) writeEstimate(parsed.minutes);
  };

  const startText =
    (startAt === null ? null : fieldText("date", String(startAt))) ?? "None";

  return (
    <>
      <PropertyRow name="start" label="Start">
        <MenuButton
          label={`Start: ${startText}`}
          className={PROPERTY_TRIGGER}
          trigger={<span className="truncate">{startText}</span>}
        >
          {(closeMenu) => (
            <DueItems
              dueAt={startAt}
              onChoose={chooseStart}
              close={closeMenu}
            />
          )}
        </MenuButton>
        <DueDateInput
          dueAt={startAt}
          onChoose={chooseStart}
          label="Start date"
        />
      </PropertyRow>
      <PropertyRow name="estimate" label="Estimate">
        <input
          aria-label="Estimate"
          value={text}
          placeholder="1h 30m"
          onFocus={() => setEditing(true)}
          onChange={(event) => {
            setText(event.target.value);
            setError(null);
          }}
          onBlur={commitEstimate}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.nativeEvent.isComposing) {
              event.preventDefault();
              event.currentTarget.blur();
            } else if (event.key === "Escape" && editing) {
              // Escape puts the saved estimate back instead of closing.
              event.stopPropagation();
              setError(null);
              setText(minutes === null ? "" : formatEstimate(minutes));
              setEditing(false);
              event.currentTarget.blur();
            }
          }}
          className={`${INPUT} w-28`}
        />
        <InlineError message={error} />
      </PropertyRow>
    </>
  );
}

export type BoardCardRef = { id: string; number: number; title: string };

/** Blocks, blocked by and relates to: grouped, removable, with a card picker. */
export function RelationsSection({
  card,
  boardCards,
  onError,
}: {
  card: BoardCard;
  boardCards: readonly BoardCardRef[];
  onError: (message: string) => void;
}) {
  const [relations] = useLive<CardRelation[]>(
    card.boardId,
    () => listCardRelations(card.id),
    (cause) =>
      onError(
        `Couldn't load relations of card #${card.number}: ${errorText(cause)}`,
      ),
  );
  const [picking, setPicking] = useState<RelationChoice | null>(null);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const groups = groupRelations(card.id, relations ?? []);
  const byId = new Map(boardCards.map((other) => [other.id, other]));
  const candidates = relationCandidates(boardCards, card.id, query).slice(0, 8);

  const choose = (other: BoardCardRef) => {
    if (!picking) return;
    const write = relationWrite(picking, card.id, other.id);
    addRelation(write.cardId, write.relatedCardId, write.kind).then(
      () => {
        setError(null);
        setPicking(null);
        setQuery("");
      },
      // A cycle is refused with the store's message; nothing was written.
      (cause) => setError(errorText(cause)),
    );
  };

  const remove = (relation: CardRelation) => {
    removeRelation(
      relation.cardId,
      relation.relatedCardId,
      relation.kind,
    ).catch((cause) =>
      onError(
        `Couldn't remove the relation of card #${card.number}: ${errorText(cause)}`,
      ),
    );
  };

  const pickingName = RELATION_CHOICES.find(([c]) => c === picking)?.[1];

  return (
    <section data-section="relations" className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <span className="flex-1 text-[12px] text-content/50">Relations</span>
        <MenuButton
          label="Add relation"
          width={160}
          className={SMALL_BUTTON}
          trigger={ADD_TRIGGER}
        >
          {(closeMenu) =>
            RELATION_CHOICES.map(([choice, name]) => (
              <MenuItem
                key={choice}
                onSelect={() => {
                  closeMenu();
                  setPicking(choice);
                  setQuery("");
                  setError(null);
                }}
              >
                {name}…
              </MenuItem>
            ))
          }
        </MenuButton>
      </div>
      {RELATION_CHOICES.map(([choice, name]) =>
        groups[choice].length ? (
          <div
            key={choice}
            data-relation-group={choice}
            className="flex flex-col"
          >
            <span className="text-[11px] text-content/45">{name}</span>
            {groups[choice].map(({ otherId, relation }) => {
              const other = byId.get(otherId);
              return (
                <div
                  key={`${relation.cardId}:${relation.relatedCardId}:${relation.kind}`}
                  className="flex h-7 items-center gap-2 text-[13px] text-content/85"
                >
                  <span className="min-w-0 flex-1 truncate">
                    {other ? `#${other.number} ${other.title}` : "Unknown card"}
                  </span>
                  <button
                    type="button"
                    aria-label={`Remove ${name} ${other ? `#${other.number}` : "card"}`}
                    onClick={() => remove(relation)}
                    className="grid size-6 place-items-center rounded-md text-content/50 hover:bg-content/10 hover:text-content"
                  >
                    <X className="size-3.5" strokeWidth={1.75} />
                  </button>
                </div>
              );
            })}
          </div>
        ) : null,
      )}
      {picking ? (
        <div data-relation-picker="" className="flex flex-col gap-1">
          <div className="flex items-center gap-2">
            <input
              autoFocus
              aria-label={`${pickingName} card`}
              value={query}
              placeholder="Search by #number or title"
              onChange={(event) => {
                setQuery(event.target.value);
                setError(null);
              }}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  event.stopPropagation();
                  setPicking(null);
                } else if (
                  event.key === "Enter" &&
                  !event.nativeEvent.isComposing &&
                  candidates[0]
                ) {
                  event.preventDefault();
                  choose(candidates[0]);
                }
              }}
              className={`${INPUT} flex-1`}
            />
            <button
              type="button"
              className={SMALL_BUTTON}
              onClick={() => setPicking(null)}
            >
              Cancel
            </button>
          </div>
          <InlineError message={error} />
          {candidates.length ? (
            <ul className="flex flex-col">
              {candidates.map((other) => (
                <li key={other.id}>
                  <button
                    type="button"
                    onClick={() => choose(other)}
                    className="flex h-7 w-full items-center rounded-md px-2 text-left text-[13px] text-content/85 hover:bg-content/8"
                  >
                    <span className="truncate">
                      #{other.number} {other.title}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <span className="text-[12px] text-content/45">
              No matching cards
            </span>
          )}
        </div>
      ) : null}
    </section>
  );
}

/** One editor per board field, by kind. Values are stored as JSON. */
export function FieldsSection({
  card,
  onError,
}: {
  card: BoardCard;
  onError: (message: string) => void;
}) {
  const fail = (cause: unknown) =>
    onError(
      `Couldn't load fields of card #${card.number}: ${errorText(cause)}`,
    );
  const [fields] = useLive<BoardField[]>(
    card.boardId,
    () => listFields(card.boardId),
    fail,
  );
  const [values] = useLive<FieldValue[]>(
    card.boardId,
    () => listFieldValues(card.id),
    fail,
  );
  if (!fields?.length) return null;
  const byField = new Map((values ?? []).map((v) => [v.fieldId, v.valueJson]));
  return (
    <section data-section="fields" className="flex flex-col">
      {fields.map((field) => (
        <FieldRow
          key={field.id}
          card={card}
          field={field}
          raw={byField.get(field.id)}
          onError={onError}
        />
      ))}
    </section>
  );
}

function FieldRow({
  card,
  field,
  raw,
  onError,
}: {
  card: BoardCard;
  field: BoardField;
  raw: string | undefined;
  onError: (message: string) => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const shown = raw === undefined ? null : fieldText(field.kind, raw);
  const editable =
    field.kind === "text" || field.kind === "number" || field.kind === "url";
  const [text, setText] = useState(shown ?? "");
  useEffect(() => setText(shown ?? ""), [shown]);

  const write = (value: string | null) => {
    setFieldValue(card.id, field.id, value).then(
      () => setError(null),
      (cause) => {
        setError(errorText(cause));
        onError(
          `Couldn't set ${field.name} on card #${card.number}: ${errorText(cause)}`,
        );
      },
    );
  };
  const commitText = () => {
    const encoded = encodeField(field.kind, text);
    if (!encoded.ok) {
      setError(encoded.error);
      return;
    }
    setError(null);
    if (encoded.value === (raw ?? null)) return;
    write(encoded.value);
  };
  const writeDate = (next: number | null) =>
    write(next === null ? null : String(next));

  let editor;
  if (editable) {
    editor = (
      <input
        aria-label={field.name}
        value={text}
        inputMode={
          field.kind === "number"
            ? "decimal"
            : field.kind === "url"
              ? "url"
              : undefined
        }
        onChange={(event) => {
          setText(event.target.value);
          setError(null);
        }}
        onBlur={commitText}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.nativeEvent.isComposing) {
            event.preventDefault();
            event.currentTarget.blur();
          }
        }}
        className={`${INPUT} w-full max-w-60`}
      />
    );
  } else if (field.kind === "checkbox") {
    editor = (
      <input
        type="checkbox"
        aria-label={field.name}
        checked={fieldChecked(raw)}
        onChange={(event) => write(String(event.target.checked))}
        className="size-4 accent-accent"
      />
    );
  } else if (field.kind === "multi-select") {
    const options = fieldOptions(field.optionsJson);
    const picks = fieldPicks(raw);
    // A pick whose option was removed still shows, so it can be unticked.
    const shownOptions = [...options, ...picks.filter((p) => !options.includes(p))];
    editor = (
      <div
        role="group"
        aria-label={field.name}
        className="flex flex-wrap gap-x-3 gap-y-1"
      >
        {shownOptions.map((option) => (
          <label key={option} className="flex items-center gap-1.5 text-[12px] text-content/80">
            <input
              type="checkbox"
              aria-label={`${field.name}: ${option}`}
              checked={picks.includes(option)}
              onChange={(event) =>
                write(
                  encodePicks(
                    shownOptions,
                    event.target.checked
                      ? [...picks, option]
                      : picks.filter((pick) => pick !== option),
                  ),
                )
              }
              className="size-3.5 accent-accent"
            />
            {option}
          </label>
        ))}
        {shownOptions.length === 0 ? (
          <span className="text-[12px] text-content/45">No options</span>
        ) : null}
      </div>
    );
  } else if (field.kind === "select") {
    const options = fieldOptions(field.optionsJson);
    const current = raw === undefined ? "" : (fieldText("select", raw) ?? "");
    editor = (
      <select
        aria-label={field.name}
        value={current}
        onChange={(event) =>
          write(event.target.value ? JSON.stringify(event.target.value) : null)
        }
        className={`${INPUT} max-w-60`}
      >
        <option value="">None</option>
        {current && !options.includes(current) ? (
          <option value={current}>{current}</option>
        ) : null}
        {options.map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
    );
  } else {
    const at = raw === undefined ? NaN : Number(raw);
    const dueAt = Number.isFinite(at) ? at : null;
    editor = (
      <>
        <MenuButton
          label={`${field.name}: ${shown ?? "None"}`}
          className={PROPERTY_TRIGGER}
          trigger={<span className="truncate">{shown ?? "None"}</span>}
        >
          {(closeMenu) => (
            <DueItems dueAt={dueAt} onChoose={writeDate} close={closeMenu} />
          )}
        </MenuButton>
        <DueDateInput
          dueAt={dueAt}
          label={`${field.name} date`}
          onChoose={writeDate}
        />
      </>
    );
  }
  return (
    <PropertyRow name={`field-${field.id}`} label={field.name}>
      {editor}
      <InlineError message={error} />
    </PropertyRow>
  );
}

type AddingLink = "session" | "pilot" | "pr" | "worktree";

/** The card's links but the primary one, each with Open and Remove, and Add. */
export function LinksSection({
  card,
  linkTargets,
  resolveLinkTarget,
  onOpenLinkedWork,
  onError,
}: {
  card: BoardCard;
  linkTargets: readonly BoardLinkTarget[];
  /**
   * The board's lookup of work `linkTargets` doesn't hold (sessions of
   * projects not loaded this run, say): the target, null when it is gone,
   * undefined while unknown.
   */
  resolveLinkTarget?: (
    kind: "session" | "pilot",
    id: string,
  ) => BoardLinkTarget | null | undefined;
  onOpenLinkedWork: (target: BoardLinkTarget) => void;
  onError: (message: string) => void;
}) {
  const [links] = useLive<CardLinkRow[]>(
    card.boardId,
    () => listCardLinks(card.id),
    (cause) =>
      onError(
        `Couldn't load links of card #${card.number}: ${errorText(cause)}`,
      ),
  );
  const [adding, setAdding] = useState<AddingLink | null>(null);
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const shown = visibleLinks(links ?? [], {
    kind: card.linkKind,
    id: card.linkId,
  });
  /** The link's work: a target, null when it is gone, undefined while unknown. */
  const targetOf = (link: CardLinkRow): BoardLinkTarget | null | undefined => {
    const held = linkTargets.find(
      (target) => target.kind === link.kind && target.id === link.reference,
    );
    if (held) return held;
    if (link.kind !== "session" && link.kind !== "pilot") return undefined;
    return resolveLinkTarget?.(link.kind, link.reference);
  };

  const add = (kind: AddingLink, reference: string) => {
    addCardLink(card.id, kind, reference).then(
      () => {
        setError(null);
        setAdding(null);
        setText("");
      },
      (cause) => setError(errorText(cause)),
    );
  };
  const submitText = () => {
    const value = text.trim();
    if (adding === "pr") {
      if (!validPrUrl(value)) {
        setError("A PR link starts with http:// or https://");
        return;
      }
      add("pr", value);
    } else if (adding === "worktree") {
      if (!value) {
        setError("Enter a folder path");
        return;
      }
      add("worktree", value);
    }
  };
  const open = (link: CardLinkRow) => {
    const failed = (cause: unknown) =>
      onError(`Couldn't open the link: ${errorText(cause)}`);
    if (link.kind === "pr") void openUrl(link.reference).catch(failed);
    else if (link.kind === "worktree") {
      void revealItemInDir(link.reference).catch(failed);
    } else {
      const target = targetOf(link);
      if (target) onOpenLinkedWork(target);
      else onError("That work no longer exists");
    }
  };
  const remove = (link: CardLinkRow) => {
    removeCardLink(card.id, link.kind, link.reference).catch((cause) =>
      onError(
        `Couldn't remove the link from card #${card.number}: ${errorText(cause)}`,
      ),
    );
  };
  const kindLabel = (kind: string) =>
    kind === "pr"
      ? "PR"
      : kind === "worktree"
        ? "Worktree"
        : (LINK_KIND_LABEL[kind as keyof typeof LINK_KIND_LABEL] ?? kind);
  const label = (link: CardLinkRow) => {
    const target = targetOf(link);
    if (target) return target.label;
    if (target === null) return "No longer exists";
    if (link.kind === "session" || link.kind === "pilot") {
      return `${kindLabel(link.kind)} ${link.reference.slice(0, 8)}`;
    }
    return link.reference;
  };

  const pickable = (kind: "session" | "pilot") => {
    const taken = new Set(
      (links ?? [])
        .filter((link) => link.kind === kind)
        .map((link) => link.reference),
    );
    return linkTargets
      .filter(
        (target) =>
          target.kind === kind && !target.unlisted && !taken.has(target.id),
      )
      .map((target) => ({
        value: linkValue(target.kind, target.id),
        label: target.label,
        keywords: target.detail,
      }));
  };

  return (
    <section data-section="links" className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <span className="flex-1 text-[12px] text-content/50">Links</span>
        <MenuButton
          label="Add link"
          width={180}
          className={SMALL_BUTTON}
          trigger={ADD_TRIGGER}
        >
          {(closeMenu) =>
            (
              [
                ["session", "Session…"],
                ["pilot", "Pilot…"],
                ["pr", "PR URL…"],
                ["worktree", "Worktree path…"],
              ] as const
            ).map(([kind, name]) => (
              <MenuItem
                key={kind}
                onSelect={() => {
                  closeMenu();
                  setAdding(kind);
                  setText("");
                  setError(null);
                }}
              >
                {name}
              </MenuItem>
            ))
          }
        </MenuButton>
      </div>
      {shown.map((link) => (
        <div
          key={`${link.kind}:${link.reference}`}
          data-link={link.kind}
          className="flex h-7 items-center gap-2 text-[13px] text-content/85"
        >
          <span className="w-16 shrink-0 text-[11px] text-content/45">
            {kindLabel(link.kind)}
          </span>
          <span className="min-w-0 flex-1 truncate" title={link.reference}>
            {label(link)}
          </span>
          <button
            type="button"
            className={SMALL_BUTTON}
            aria-label={`Open ${kindLabel(link.kind)} link`}
            onClick={() => open(link)}
          >
            Open
          </button>
          <button
            type="button"
            className={SMALL_BUTTON}
            aria-label={`Remove ${kindLabel(link.kind)} link`}
            onClick={() => remove(link)}
          >
            Remove
          </button>
        </div>
      ))}
      {adding === "session" || adding === "pilot" ? (
        <div className="flex items-center gap-2">
          <div className="min-w-0 flex-1">
            <SearchableSelect
              label={adding === "pilot" ? "Pilot" : "Session"}
              value=""
              options={pickable(adding)}
              onChange={(value) => {
                const id = value.slice(value.indexOf(":") + 1);
                if (id) add(adding, id);
              }}
              placeholder={
                adding === "pilot" ? "Choose a Pilot…" : "Choose a session…"
              }
              searchPlaceholder="Search…"
              emptyLabel="Nothing to link"
            />
          </div>
          <button
            type="button"
            className={SMALL_BUTTON}
            onClick={() => setAdding(null)}
          >
            Cancel
          </button>
        </div>
      ) : adding ? (
        <div className="flex items-center gap-2">
          <input
            autoFocus
            aria-label={adding === "pr" ? "PR URL" : "Worktree path"}
            value={text}
            placeholder={
              adding === "pr"
                ? "https://github.com/…/pull/1"
                : "/path/to/worktree"
            }
            onChange={(event) => {
              setText(event.target.value);
              setError(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.stopPropagation();
                setAdding(null);
              } else if (
                event.key === "Enter" &&
                !event.nativeEvent.isComposing
              ) {
                event.preventDefault();
                submitText();
              }
            }}
            className={`${INPUT} flex-1`}
          />
          <button type="button" className={SMALL_BUTTON} onClick={submitText}>
            Add
          </button>
          <button
            type="button"
            className={SMALL_BUTTON}
            onClick={() => setAdding(null)}
          >
            Cancel
          </button>
        </div>
      ) : null}
      <InlineError message={error} />
    </section>
  );
}
