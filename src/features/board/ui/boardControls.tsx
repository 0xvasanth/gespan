import {
  useEffect,
  useRef,
  useState,
  type ReactNode,
  type SyntheticEvent,
} from "react";
import { Popover } from "../../../shared/ui/Popover";
import { MoreHorizontal } from "../../../shared/ui/icons";

const stop = (event: SyntheticEvent) => event.stopPropagation();

/**
 * Spread on controls inside a card or column header. A press there must not
 * reach the sortable wrapper (4px of movement would start a drag), and keys
 * must not reach its keyboard sensor or the card's click.
 */
export const noDrag = {
  onPointerDown: stop,
  onKeyDown: stop,
  onClick: stop,
} as const;

export const ICON_BUTTON =
  "grid size-6 shrink-0 place-items-center rounded-md text-content/50 hover:bg-content/10 hover:text-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/60";

const MENU_ITEM =
  "flex h-8 w-full items-center gap-2 rounded-lg px-2 text-left text-[13px] text-content/85 hover:bg-content/8 hover:text-content disabled:cursor-default disabled:opacity-40 disabled:hover:bg-transparent";

/** A "…" button with its menu. `children` gets a `close` callback. */
export function MenuButton({
  label,
  children,
  width = 200,
  className = ICON_BUTTON,
}: {
  label: string;
  children: (close: () => void) => ReactNode;
  width?: number;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const close = () => setOpen(false);
  return (
    <>
      <button
        ref={button}
        type="button"
        aria-label={label}
        title={label}
        aria-haspopup="menu"
        aria-expanded={open}
        data-menu-open={open ? "" : undefined}
        className={className}
        onPointerDown={stop}
        onKeyDown={stop}
        onClick={(event) => {
          event.stopPropagation();
          setOpen((value) => !value);
        }}
      >
        <MoreHorizontal className="size-3.5" strokeWidth={1.75} />
      </button>
      {open ? (
        <Popover
          anchor={button}
          side="bottom"
          align="end"
          gap={4}
          width={width}
          constrainHeight={false}
          role="menu"
          aria-label={label}
          onDismiss={close}
          className="p-1"
          // The menu is portalled but still bubbles through the React tree.
          onPointerDown={stop}
          onKeyDown={stop}
          onClick={stop}
        >
          {children(close)}
        </Popover>
      ) : null}
    </>
  );
}

export function MenuItem({
  children,
  onSelect,
  disabled = false,
  title,
  danger = false,
  checked,
}: {
  children: ReactNode;
  onSelect: () => void;
  disabled?: boolean;
  title?: string;
  danger?: boolean;
  /** Set for a radio item. */
  checked?: boolean;
}) {
  return (
    <button
      type="button"
      role={checked === undefined ? "menuitem" : "menuitemradio"}
      aria-checked={checked}
      disabled={disabled}
      title={title}
      onClick={onSelect}
      className={`${MENU_ITEM} ${danger ? "text-red-300/90 hover:bg-red-500/15 hover:text-red-300" : ""}`}
    >
      {children}
    </button>
  );
}

export function MenuLabel({ children }: { children: ReactNode }) {
  return (
    <div className="px-2 pb-0.5 pt-1.5 text-[11px] text-content/45">
      {children}
    </div>
  );
}

export function MenuSeparator() {
  return <div role="separator" className="mx-1 my-1 h-px bg-stroke" />;
}

/**
 * Inline single-line name editor. Enter or blur saves a non-empty trimmed
 * name; Esc, or an empty name, cancels.
 */
export function NameInput({
  initial = "",
  label,
  placeholder,
  className = "",
  onSubmit,
  onCancel,
}: {
  initial?: string;
  label: string;
  placeholder?: string;
  className?: string;
  onSubmit: (name: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initial);
  const done = useRef(false);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);
  const finish = (save: boolean) => {
    if (done.current) return;
    done.current = true;
    const name = value.trim();
    if (save && name && name !== initial) onSubmit(name);
    else onCancel();
  };
  return (
    <input
      ref={input}
      aria-label={label}
      placeholder={placeholder}
      value={value}
      onChange={(event) => setValue(event.target.value)}
      onPointerDown={stop}
      onClick={stop}
      onKeyDown={(event) => {
        event.stopPropagation();
        if (event.nativeEvent.isComposing) return;
        if (event.key === "Enter") {
          event.preventDefault();
          finish(true);
        } else if (event.key === "Escape") {
          event.preventDefault();
          finish(false);
        }
      }}
      onBlur={() => finish(true)}
      className={`h-6.5 min-w-0 rounded-md border border-stroke bg-background-base px-2 text-[13px] text-content outline-none focus:border-accent/60 ${className}`}
    />
  );
}
