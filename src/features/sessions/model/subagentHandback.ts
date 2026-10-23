export type Handback = {
  report: string;
  agentId?: string;
  subagentTokens?: number;
  toolUses?: number;
  durationMs?: number;
};

const PREAMBLE_LEAD = "[Subagent hand-back]";
const PREAMBLE_END = "The report follows:";
const TAIL =
  /^subagent_tokens:\s*(\d+)\s*tool_uses:\s*(\d+)\s*duration_ms:\s*(\d+)\s*(?:<\/usage>)?$/;
const ID = /^agentId:\s*(\S+)/;

/**
 * Splits `text` into the agent's report and the trailer metadata.
 *
 * The trailer (`agentId: X ... subagent_tokens: N tool_uses: N duration_ms: N`, optionally wrapped
 * in `<usage>`) must sit at the very end and start at a line start; an `agentId:` line inside the
 * report is left alone. The `[Subagent hand-back] ... The report follows:` preamble is removed and
 * the harness's per-line indentation stripped. Anything else is returned as is.
 */
export function parseHandback(text: string): Handback {
  // Linear: trim the end once, anchor the usage tail at the last `subagent_tokens:`, then walk
  // back to the nearest line-start `agentId:` before it.
  const trimmed = text.trimEnd();
  let body = text;
  let meta: { id: string; usage: RegExpExecArray } | undefined;
  const tailStart = trimmed.lastIndexOf("subagent_tokens:");
  const usage = tailStart === -1 ? null : TAIL.exec(trimmed.slice(tailStart));
  if (usage) {
    let at = trimmed.lastIndexOf("agentId:", tailStart - 1);
    for (
      ;
      at !== -1;
      at = at === 0 ? -1 : trimmed.lastIndexOf("agentId:", at - 1)
    ) {
      if (at !== 0 && trimmed[at - 1] !== "\n") continue;
      const found = ID.exec(trimmed.slice(at));
      if (found) {
        meta = { id: found[1], usage };
        body = text.slice(0, at);
        break;
      }
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
  const result: Handback = {
    report: (stripped ? dedent(body) : body).trimEnd(),
  };
  if (meta) {
    result.agentId = meta.id;
    result.subagentTokens = Number(meta.usage[1]);
    result.toolUses = Number(meta.usage[2]);
    result.durationMs = Number(meta.usage[3]);
  }
  return result;
}

/** Removes the leading whitespace shared by every non-blank line; blank lines become empty. */
function dedent(text: string): string {
  const lines = text.split(/\r?\n/);
  const indents = lines
    .filter((l) => l.trim())
    .map((l) => l.length - l.trimStart().length);
  const indent = indents.length ? Math.min(...indents) : 0;
  return lines.map((l) => (l.trim() ? l.slice(indent) : "")).join("\n");
}
