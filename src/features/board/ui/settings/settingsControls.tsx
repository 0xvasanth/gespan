import { useEffect, useRef, useState, type ReactNode } from "react";
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
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="flex items-center gap-1"
    >
      {none ? (
        <button
          type="button"
          role="radio"
          aria-checked={value === null}
          aria-label="No colour"
          title="No colour"
          onClick={() => onChange(null)}
          className={`${SWATCH} border-dashed border-content/35 ${RING(value === null)}`}
        />
      ) : null}
      {LABEL_COLORS.map((color) => (
        <button
          key={color}
          type="button"
          role="radio"
          aria-checked={value === color}
          aria-label={color}
          title={`#${color}`}
          onClick={() => onChange(color)}
          style={{ backgroundColor: `#${color}` }}
          className={`${SWATCH} border-transparent ${RING(value === color)}`}
        />
      ))}
    </div>
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
