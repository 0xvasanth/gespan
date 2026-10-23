import type { BuiltinSkill } from "../../skills/model/skills";
import type { Block } from "./session";

export const OPERATOR_COMMAND: BuiltinSkill = {
  kind: "builtin",
  name: "operator",
  invocation: "operator",
  description:
    "Give this thread access to Rebase sessions, folders, and notes.",
  scope: "builtin",
  source: "rebase",
};

/** Activate app access with a leading composer command. */
export function consumeOperatorCommand(text: string): {
  text: string;
  matched: boolean;
} {
  // /pilot and /rebase are unlisted aliases of /operator, kept so existing
  // drafts and threads that use them still activate app access.
  const match = text.match(/^\s*\/(?:operator|pilot|rebase)(?=\s|$)\s*/i);
  if (!match) return { text, matched: false };
  return { text: text.slice(match[0].length), matched: true };
}

/** The unlisted alias spellings of /operator, found in earlier user turns. */
const ALIAS_COMMAND = /^\s*\/(?:pilot|rebase)(?=\s|$)\s*/i;

/** A submitted /operator turn keeps CLI access available in later turns. */
export function isOperatorUserTurn(block: Block): boolean {
  return (
    block.role === "user" &&
    !block.draft &&
    !block.internal &&
    // This persisted field keeps its original name for saved session compatibility.
    (block.rebase === true || ALIAS_COMMAND.test(block.text))
  );
}

/** Old command messages remain enabled and render without their old prefix. */
export function operatorUserPrompt(block: Block): string {
  const legacy = block.text.match(ALIAS_COMMAND);
  if (!legacy) return block.text;
  return (
    block.text.slice(legacy[0].length).trim() ||
    "Explain what you can do in Rebase with the app CLI."
  );
}

export function operatorEnabledInThread(blocks: Block[]): boolean {
  return blocks.some(isOperatorUserTurn);
}
