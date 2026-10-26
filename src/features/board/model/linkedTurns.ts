import type { CardLinkKind, LinkedWorkEvent } from "./board";

/** How a session turn ended, as the app's turn lifecycle reports it. */
export type TurnSettlement = "completed" | "failed" | "cancelled";

export type LinkedWorkNotify = (
  kind: CardLinkKind,
  id: string,
  event: LinkedWorkEvent,
) => Promise<void>;

/**
 * The board event a settled turn reports. A turn the user stopped (or that a
 * newer turn replaced) is neither done nor broken, so it reports nothing and
 * its card stays where it is.
 */
export function settledTurnEvent(
  status: TurnSettlement,
): LinkedWorkEvent | null {
  return status === "cancelled" ? null : status;
}

/**
 * The Pilot a turn counts for, if any: the Pilot whose own chat the session
 * is, else the Pilot a habit run works for, else the Pilot that spawned (or
 * prompted) the session through the app CLI. Pilot links follow all of them.
 */
export function turnPilotId(sources: {
  own?: string;
  habitRun?: string;
  spawnedBy?: string;
}): string | undefined {
  return sources.own ?? sources.habitRun ?? sources.spawnedBy;
}

/** Comments a completed turn's final reply on the session's linked cards. */
export type LinkedWriteBack = (
  sessionKey: string,
  reply: string,
) => Promise<unknown>;

/**
 * Turns session turns into linked-work events: one "started" when a turn
 * begins, "needs_input" each time a running session starts waiting on the
 * user, and at most one terminal event when it settles, for the session and,
 * when a Pilot is behind the turn (see `turnPilotId`), for that Pilot. Turns
 * are keyed by session and turn number, so a repeated start or settle never
 * fires twice. A completed turn's final reply also goes to `writeBack`, keyed
 * by the session; the store decides which boards take it and trims it.
 * Each call is fire-and-forget: a board failure goes to `report`, never back
 * into the session.
 */
export function createLinkedTurns(
  notify: LinkedWorkNotify,
  report: (error: unknown) => void = console.error,
  writeBack?: LinkedWriteBack,
) {
  /** Open turns, to the Pilot their session belonged to when they started. */
  const open = new Map<string, string | undefined>();
  /** Sessions already reported as waiting on the user. */
  const waiting = new Set<string>();
  const key = (sessionId: string, turn: number) => `${turn}:${sessionId}`;
  const openTurn = (sessionId: string) => {
    for (const [turnKey, pilotId] of open) {
      if (turnKey.slice(turnKey.indexOf(":") + 1) === sessionId)
        return { pilotId };
    }
    return undefined;
  };
  const guard = (run: () => Promise<unknown>) => {
    try {
      void run().catch(report);
    } catch (error) {
      report(error);
    }
  };

  const send = (
    sessionId: string,
    pilotId: string | undefined,
    event: LinkedWorkEvent,
  ) => {
    const targets: [CardLinkKind, string][] = [["session", sessionId]];
    if (pilotId) targets.push(["pilot", pilotId]);
    for (const [kind, id] of targets) guard(() => notify(kind, id, event));
  };

  return {
    started(sessionId: string, turn: number, pilotId?: string) {
      const turnKey = key(sessionId, turn);
      if (open.has(turnKey)) return;
      open.set(turnKey, pilotId);
      send(sessionId, pilotId, "started");
    },
    /**
     * `reply` is the turn's final assistant reply, passed as is: only a
     * completed turn with a non-blank reply writes back.
     */
    settled(
      sessionId: string,
      turn: number,
      status: TurnSettlement,
      reply?: string,
    ) {
      const turnKey = key(sessionId, turn);
      if (!open.has(turnKey)) return;
      const pilotId = open.get(turnKey);
      open.delete(turnKey);
      waiting.delete(sessionId);
      const event = settledTurnEvent(status);
      if (event) send(sessionId, pilotId, event);
      if (status === "completed" && writeBack && reply?.trim())
        guard(() => writeBack(sessionId, reply));
    },
    /**
     * The sessions waiting on the user right now. Each session with an open
     * turn that was not waiting before reports "needs_input" once.
     */
    needsInput(sessionIds: Iterable<string>) {
      const now = new Set(sessionIds);
      for (const sessionId of waiting) {
        if (!now.has(sessionId)) waiting.delete(sessionId);
      }
      for (const sessionId of now) {
        if (waiting.has(sessionId)) continue;
        const turn = openTurn(sessionId);
        if (!turn) continue;
        waiting.add(sessionId);
        send(sessionId, turn.pilotId, "needs_input");
      }
    },
  };
}

export type LinkedTurns = ReturnType<typeof createLinkedTurns>;
