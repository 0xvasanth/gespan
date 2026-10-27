import { useLayoutEffect, useRef, type RefObject } from "react";
import { applyFormat, FORMATS } from "../model/markdownToolbar";

/**
 * Formatting buttons for a Markdown textarea. A press rewrites the text
 * through `onChange` and puts the selection where the transform says. The
 * buttons never take focus from the textarea, so its selection survives.
 */
export function MarkdownToolbar({
  textareaRef,
  value,
  onChange,
}: {
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  value: string;
  onChange: (value: string) => void;
}) {
  const pending = useRef<{ start: number; end: number } | null>(null);
  // The new text renders first; then the selection can be set on it.
  useLayoutEffect(() => {
    const selection = pending.current;
    const field = textareaRef.current;
    if (!selection || !field || field.value !== value) return;
    pending.current = null;
    field.focus();
    field.setSelectionRange(selection.start, selection.end);
  }, [value, textareaRef]);
  return (
    <div
      data-markdown-toolbar=""
      role="toolbar"
      aria-label="Formatting"
      className="flex items-center gap-0.5"
    >
      {FORMATS.map(({ format, label, glyph }) => (
        <button
          key={format}
          type="button"
          aria-label={label}
          title={label}
          // Keep the textarea's focus and selection.
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => {
            const field = textareaRef.current;
            if (!field) return;
            const next = applyFormat(
              format,
              field.value,
              field.selectionStart,
              field.selectionEnd,
            );
            pending.current = { start: next.start, end: next.end };
            if (next.text === field.value) {
              field.setSelectionRange(next.start, next.end);
              return;
            }
            onChange(next.text);
          }}
          className={`grid h-6 min-w-6 place-items-center rounded-md px-1.5 text-[12px] text-content/60 hover:bg-content/10 hover:text-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/60 ${format === "bold" ? "font-bold" : ""} ${format === "italic" ? "italic" : ""}`}
        >
          {glyph}
        </button>
      ))}
    </div>
  );
}
