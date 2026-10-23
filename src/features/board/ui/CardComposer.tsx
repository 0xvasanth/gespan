import { useEffect, useRef, useState } from "react";
import { Plus } from "../../../shared/ui/icons";

/**
 * The foot of a column: a faint "+ Add card" row that opens a textarea.
 * Enter adds the trimmed title (an empty one does nothing) and keeps the
 * composer open for the next card; Shift+Enter is a newline; Esc cancels.
 */
export function CardComposer({
  columnName,
  open,
  onOpen,
  onClose,
  onCreate,
}: {
  columnName: string;
  open: boolean;
  onOpen: () => void;
  onClose: () => void;
  onCreate: (title: string) => void;
}) {
  const [value, setValue] = useState("");
  const field = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (open) field.current?.focus();
    else setValue("");
  }, [open]);

  if (!open) {
    return (
      <button
        type="button"
        onClick={onOpen}
        className="mx-1.5 mb-1.5 flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2 text-left text-[12px] text-content/45 hover:bg-content/6 hover:text-content/80"
      >
        <Plus className="size-3.5" strokeWidth={1.75} />
        Add card
      </button>
    );
  }

  const title = value.trim();
  const submit = () => {
    if (!title) return;
    onCreate(title);
    setValue("");
    field.current?.focus();
  };

  return (
    <div className="mx-1.5 mb-1.5 flex shrink-0 flex-col gap-1.5">
      <textarea
        ref={field}
        aria-label={`New card in ${columnName}`}
        placeholder="Card title"
        rows={2}
        value={value}
        onChange={(event) => setValue(event.target.value)}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) return;
          if (event.key === "Enter" && !event.shiftKey) {
            event.preventDefault();
            submit();
          } else if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            onClose();
          }
        }}
        className="w-full resize-none rounded-lg border border-stroke bg-background-base px-3 py-2 text-[13px] text-content outline-none placeholder:text-content/35 focus:border-accent/60"
      />
      <div className="flex items-center gap-1.5">
        <span className="min-w-0 flex-1 truncate text-[11px] text-content/45">
          Enter to add · Esc to cancel
        </span>
        <button
          type="button"
          onClick={onClose}
          className="h-6 rounded-md px-2 text-[12px] text-content/60 hover:bg-content/8 hover:text-content"
        >
          Cancel
        </button>
        <button
          type="button"
          disabled={!title}
          onClick={submit}
          className="h-6 rounded-md bg-content px-2.5 text-[12px] font-medium text-background-base hover:bg-content/80 disabled:cursor-default disabled:opacity-40"
        >
          Add
        </button>
      </div>
    </div>
  );
}
