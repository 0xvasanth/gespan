import type { Block, Session } from "../../sessions/model/session";
import { groupPilotChatTurns } from "../../sessions/model/transcriptActivity";

export type PilotActivitySelection = {
  sessionId: string;
  turnId: string;
  /** Retain an archived turn after the viewer returns to the live window. */
  blocks: Block[];
};

export function resolvePilotActivity(
  selection: PilotActivitySelection | null,
  session: Pick<Session, "id" | "blocks" | "busy"> | undefined,
) {
  if (!selection || selection.sessionId !== session?.id) return null;
  const turns = groupPilotChatTurns(session.blocks);
  const current = turns.find((turn) => turn[0].id === selection.turnId);
  return {
    turnId: selection.turnId,
    blocks: current ?? selection.blocks,
    live: !!session.busy && !!current && current === turns[turns.length - 1],
  };
}
