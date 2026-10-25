/**
 * The description's Markdown toolbar. Each button is a pure text transform:
 * `(text, selection)` in, `(new text, new selection)` out, with UTF-16
 * indices (what a textarea's `selectionStart` and `selectionEnd` use). The
 * stored text stays plain Markdown. Mirrors the native `markdown_toolbar.rs`.
 */

export type Format =
  | "bold"
  | "italic"
  | "heading"
  | "bullets"
  | "numbered"
  | "link"
  | "code";

export const FORMATS: readonly {
  format: Format;
  label: string;
  /** What the button shows. */
  glyph: string;
}[] = [
  { format: "bold", label: "Bold", glyph: "B" },
  { format: "italic", label: "Italic", glyph: "I" },
  { format: "heading", label: "Heading", glyph: "H" },
  { format: "bullets", label: "Bulleted list", glyph: "•" },
  { format: "numbered", label: "Numbered list", glyph: "1." },
  { format: "link", label: "Link", glyph: "Link" },
  { format: "code", label: "Code", glyph: "</>" },
];

export type Edit = { text: string; start: number; end: number };

type Range = { start: number; end: number };

/** Applies `format` to the selection (clamped to the text and to whole characters). */
export function applyFormat(
  format: Format,
  text: string,
  selectionStart: number,
  selectionEnd: number,
): Edit {
  const selection = clamp(text, selectionStart, selectionEnd);
  switch (format) {
    case "bold":
      return wrap(text, selection, "**", "**");
    case "italic":
      return wrap(text, selection, "_", "_");
    case "code":
      return code(text, selection);
    case "link":
      return link(text, selection);
    case "heading":
      return heading(text, selection);
    case "bullets":
      return prefixLines(text, selection, () => "- ");
    case "numbered":
      return prefixLines(text, selection, (n) => `${n + 1}. `);
  }
}

function clamp(text: string, a: number, b: number): Range {
  const fix = (offset: number) => {
    let at = Math.max(0, Math.min(Math.trunc(offset) || 0, text.length));
    // Never land between the halves of a surrogate pair.
    if (at > 0 && at < text.length) {
      const before = text.charCodeAt(at - 1);
      const after = text.charCodeAt(at);
      if (before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff) {
        at -= 1;
      }
    }
    return at;
  };
  const [x, y] = [fix(a), fix(b)];
  return { start: Math.min(x, y), end: Math.max(x, y) };
}

/** The selection without its trailing newlines, which stay outside any marker. */
function trimNewlines(text: string, { start, end }: Range): Range {
  while (end > start && text[end - 1] === "\n") end -= 1;
  return { start, end };
}

function splice(text: string, body: Range, open: string, close: string) {
  return `${text.slice(0, body.start)}${open}${text.slice(body.start, body.end)}${close}${text.slice(body.end)}`;
}

function wrap(text: string, selection: Range, open: string, close: string): Edit {
  const body = trimNewlines(text, selection);
  const start = body.start + open.length;
  return {
    text: splice(text, body, open, close),
    start,
    end: start + (body.end - body.start),
  };
}

function code(text: string, selection: Range): Edit {
  const body = trimNewlines(text, selection);
  if (!text.slice(body.start, body.end).includes("\n")) {
    return wrap(text, body, "`", "`");
  }
  // The fences sit on lines of their own: a selection that starts or ends
  // mid-line gets a line break before the opening or after the closing one.
  const atLineStart = body.start === 0 || text[body.start - 1] === "\n";
  const atLineEnd = body.end === text.length || text[body.end] === "\n";
  const open = atLineStart ? "```\n" : "\n```\n";
  const close = atLineEnd ? "\n```" : "\n```\n";
  const start = body.start + open.length;
  return {
    text: splice(text, body, open, close),
    start,
    end: start + (body.end - body.start),
  };
}

function link(text: string, selection: Range): Edit {
  const body = trimNewlines(text, selection);
  if (body.start === body.end) {
    // Nothing to link: `[]()` with the cursor between the brackets.
    const at = body.start + 1;
    return { text: splice(text, body, "[]()", ""), start: at, end: at };
  }
  const urlStart = body.end + "](".length + 1;
  return {
    text: splice(text, body, "[", "](url)"),
    start: urlStart,
    end: urlStart + "url".length,
  };
}

function lineStart(text: string, at: number) {
  return at === 0 ? 0 : text.lastIndexOf("\n", at - 1) + 1;
}

function heading(text: string, selection: Range): Edit {
  const start = lineStart(text, selection.start);
  const shift = 3;
  return {
    text: `${text.slice(0, start)}## ${text.slice(start)}`,
    start: selection.start + shift,
    end: selection.end + shift,
  };
}

/**
 * Prefixes every line the selection touches. A selection that ends with a
 * newline does not touch the line after it.
 */
function prefixLines(
  text: string,
  selection: Range,
  prefix: (n: number) => string,
): Edit {
  const body = trimNewlines(text, selection);
  const first = lineStart(text, body.start);
  const newline = text.indexOf("\n", body.end);
  const lastEnd = newline === -1 ? text.length : newline;
  const block = text
    .slice(first, lastEnd)
    .split("\n")
    .map((line, n) => `${prefix(n)}${line}`)
    .join("\n");
  const next = `${text.slice(0, first)}${block}${text.slice(lastEnd)}`;
  if (selection.start === selection.end) {
    const at = selection.start + prefix(0).length;
    return { text: next, start: at, end: at };
  }
  return { text: next, start: first, end: first + block.length };
}
