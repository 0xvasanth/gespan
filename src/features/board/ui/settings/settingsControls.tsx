import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { colorName } from "../../model/boardSettings";
import { LABEL_COLORS } from "../boardFormat";

export const FIELD =
  "h-7 rounded-md border border-stroke bg-background-base px-2 text-[12px] text-content outline-none placeholder:text-content/40 focus:border-accent/60 disabled:opacity-40";

export const SETTINGS_BUTTON =
  "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-content/15 px-2.5 text-[12px] text-content/80 hover:border-content/30 hover:bg-content/10 hover:text-content disabled:cursor-default disabled:opacity-40 disabled:hover:border-content/15 disabled:hover:bg-transparent";

/** A titled block of the settings page, named for assistive tech by its heading. */
export function SettingsSection({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <section aria-label={title} className="flex flex-col gap-3 py-5">
      <div>
        <h2 className="text-[13px] font-medium text-content">{title}</h2>
        {hint ? (
          <p className="mt-0.5 text-[12px] text-content/50">{hint}</p>
        ) : null}
      </div>
      {children}
    </section>
  );
}

/**
 * A text field that saves on Enter or blur and drops its edit on Escape.
 * `onCommit` returns false for text it refused: the draft stays, so the
 * same text asked again says so again.
 */
export function CommitInput({
  value,
  label,
  onCommit,
  placeholder,
  className = "",
  inputMode,
}: {
  value: string;
  label: string;
  onCommit: (text: string) => boolean | void | Promise<boolean | void>;
  placeholder?: string;
  className?: string;
  inputMode?: "numeric";
}) {
  const [draft, setDraft] = useState(value);
  // What was last handed to `onCommit` and not refused: Enter then blur is one save.
  const sent = useRef(value);
  useEffect(() => {
    setDraft(value);
    sent.current = value;
  }, [value]);

  const commit = async () => {
    if (draft === sent.current) return;
    const previous = sent.current;
    sent.current = draft;
    const accepted = await onCommit(draft);
    if (accepted === false) sent.current = previous;
  };

  return (
    <input
      aria-label={label}
      value={draft}
      placeholder={placeholder}
      inputMode={inputMode}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={() => void commit()}
      onKeyDown={(event) => {
        if (event.nativeEvent.isComposing) return;
        if (event.key === "Enter") {
          event.preventDefault();
          void commit();
        } else if (event.key === "Escape" && draft !== value) {
          event.preventDefault();
          setDraft(value);
          sent.current = value;
        }
      }}
      className={`${FIELD} ${className}`}
    />
  );
}

/** One choice of a {@link RadioGroup}. */
export type RadioOption = {
  value: string;
  /** The accessible name. */
  label: string;
  title?: string;
  className: string;
  style?: CSSProperties;
  content?: ReactNode;
};

/**
 * A radio group with a roving tab stop: Tab reaches the checked radio (or the
 * first), arrow keys move to the next or previous one and choose it.
 */
export function RadioGroup({
  label,
  value,
  options,
  onChange,
  className = "flex items-center gap-1",
}: {
  label: string;
  /** The checked option's value, or null when none is. */
  value: string | null;
  options: readonly RadioOption[];
  onChange: (value: string) => void;
  className?: string;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const checked = options.findIndex((option) => option.value === value);
  const stop = checked >= 0 ? checked : 0;
  const step = (event: KeyboardEvent<HTMLDivElement>) => {
    const delta =
      event.key === "ArrowRight" || event.key === "ArrowDown"
        ? 1
        : event.key === "ArrowLeft" || event.key === "ArrowUp"
          ? -1
          : 0;
    if (!delta || event.nativeEvent.isComposing) return;
    event.preventDefault();
    const from = refs.current.findIndex((el) => el === event.target);
    const next = (Math.max(from, 0) + delta + options.length) % options.length;
    refs.current[next]?.focus();
    onChange(options[next].value);
  };
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={className}
      onKeyDown={step}
    >
      {options.map((option, index) => (
        <button
          key={option.value}
          ref={(el) => {
            refs.current[index] = el;
          }}
          type="button"
          role="radio"
          aria-checked={index === checked}
          aria-label={option.label}
          title={option.title ?? option.label}
          tabIndex={index === stop ? 0 : -1}
          onClick={() => onChange(option.value)}
          style={option.style}
          className={option.className}
        >
          {option.content}
        </button>
      ))}
    </div>
  );
}

/** The palette as a radio group; `none` adds a "No colour" choice. */
export function Swatches({
  label,
  value,
  onChange,
  none = false,
}: {
  label: string;
  value: string | null;
  onChange: (color: string | null) => void;
  none?: boolean;
}) {
  const options: RadioOption[] = [
    ...(none
      ? [
          {
            value: "",
            label: "No colour",
            className: `${SWATCH} border-dashed border-content/35 ${RING(value === null)}`,
          },
        ]
      : []),
    ...LABEL_COLORS.map((color) => ({
      value: color,
      label: colorName(color),
      title: `${colorName(color)} (#${color})`,
      style: { backgroundColor: `#${color}` },
      className: `${SWATCH} border-transparent ${RING(value === color)}`,
    })),
  ];
  return (
    <RadioGroup
      label={label}
      value={value ?? (none ? "" : null)}
      options={options}
      onChange={(next) => onChange(next || null)}
    />
  );
}

const SWATCH =
  "size-4 shrink-0 rounded-full border outline-none focus-visible:ring-2 focus-visible:ring-accent/60";
const RING = (on: boolean) =>
  on ? "ring-2 ring-content/80 ring-offset-1 ring-offset-background-base" : "";

/** Up and down buttons for a row in a reorderable list. */
export function MoveButtons({
  index,
  count,
  onMove,
}: {
  index: number;
  count: number;
  onMove: (to: number) => void;
}) {
  return (
    <>
      <button
        type="button"
        aria-label="Move up"
        title="Move up"
        disabled={index === 0}
        onClick={() => onMove(index - 1)}
        className={`${SETTINGS_BUTTON} w-7 justify-center px-0`}
      >
        ↑
      </button>
      <button
        type="button"
        aria-label="Move down"
        title="Move down"
        disabled={index === count - 1}
        onClick={() => onMove(index + 1)}
        className={`${SETTINGS_BUTTON} w-7 justify-center px-0`}
      >
        ↓
      </button>
    </>
  );
}
