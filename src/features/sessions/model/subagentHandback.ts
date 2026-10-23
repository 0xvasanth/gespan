export type Handback = {
  report: string;
  agentId?: string;
  subagentTokens?: number;
  toolUses?: number;
  durationMs?: number;
};

const PREAMBLE_LEAD = "[Subagent hand-back]";
const PREAMBLE_END = "The report follows:";
const TRAILER =
  /^agentId:\s*(\S+)[\s\S]*?subagent_tokens:\s*(\d+)\s*tool_uses:\s*(\d+)\s*duration_ms:\s*(\d+)\s*(?:<\/usage>)?\s*$/;

/**
 * Splits `text` into the agent's report and the trailer metadata.
 *
 * The trailer (`agentId: X ... subagent_tokens: N tool_uses: N duration_ms: N`, optionally wrapped
 * in `<usage>`) must sit at the very end and start at a line start; an `agentId:` line inside the
 * report is left alone. The `[Subagent hand-back] ... The report follows:` preamble is removed and
 * the harness's per-line indentation stripped. Anything else is returned as is.
 */
export function parseHandback(text: string): Handback {
  let body = text;
  let meta: RegExpMatchArray | undefined;
  const starts = [0];
  for (let i = text.indexOf("\n"); i !== -1; i = text.indexOf("\n", i + 1)) starts.push(i + 1);
  for (let k = starts.length - 1; k >= 0; k--) {
    const start = starts[k];
    if (!text.startsWith("agentId:", start)) continue;
    const m = TRAILER.exec(text.slice(start));
    if (m) {
      meta = m;
      body = text.slice(0, start);
      break;
    }
  }

  let stripped = false;
  if (body.startsWith(PREAMBLE_LEAD)) {
    const at = body.indexOf(PREAMBLE_END);
    if (at !== -1) {
      body = body.slice(at + PREAMBLE_END.length).replace(/^[\r\n]+/, "");
      stripped = true;
    }
  }

  if (!meta && !stripped) return { report: text };
  const result: Handback = { report: (stripped ? dedent(body) : body).trimEnd() };
  if (meta) {
    result.agentId = meta[1];
    result.subagentTokens = Number(meta[2]);
    result.toolUses = Number(meta[3]);
    result.durationMs = Number(meta[4]);
  }
  return result;
}

/** Removes the leading whitespace shared by every non-blank line; blank lines become empty. */
function dedent(text: string): string {
  const lines = text.split(/\r?\n/);
  const indents = lines.filter((l) => l.trim()).map((l) => l.length - l.trimStart().length);
  const indent = indents.length ? Math.min(...indents) : 0;
  return lines.map((l) => (l.trim() ? l.slice(indent) : "")).join("\n");
}
