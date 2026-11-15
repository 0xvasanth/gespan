import type { ReactNode } from "react";

const TOKEN = /\*\*(.+?)\*\*|`([^`]+)`|\[([^\]]+)\]\(([^)\s]+)\)/g;

/**
 * Inline Markdown from CHANGELOG.md, as React nodes: **bold**, `code` and
 * [links](https://…). Everything else stays text, so nothing is injected as HTML.
 */
export function renderInline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  for (const match of text.matchAll(TOKEN)) {
    const at = match.index ?? 0;
    if (at > last) out.push(text.slice(last, at));
    const key = out.length;
    if (match[1] !== undefined) out.push(<b key={key}>{match[1]}</b>);
    else if (match[2] !== undefined) out.push(<code key={key} className="w-code-inline">{match[2]}</code>);
    else if (/^https?:\/\//.test(match[4])) out.push(<a key={key} className="w-link" href={match[4]}>{match[3]}</a>);
    else out.push(match[3]);
    last = at + match[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

export function Inline({ text }: { text: string }) {
  return <>{renderInline(text)}</>;
}
