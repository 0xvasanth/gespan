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

/**
 * Turns session turns into linked-work events: one "started" when a turn
 * begins and at most one terminal event when it settles, for the session and,
 * when a Pilot is behind the turn (see `turnPilotId`), for that Pilot. Turns
 * are keyed by session and turn number, so a repeated start or settle never
 * fires twice.
 * Each call is fire-and-forget: a board failure goes to `report`, never back
 * into the session.
 */
export function createLinkedTurns(
  notify: LinkedWorkNotify,
  report: (error: unknown) => void = console.error,
) {
  /** Open turns, to the Pilot their session belonged to when they started. */
  const open = new Map<string, string | undefined>();
  const key = (sessionId: string, turn: number) => `${turn}:${sessionId}`;

  const send = (
    sessionId: string,
    pilotId: string | undefined,
    event: LinkedWorkEvent,
  ) => {
    const targets: [CardLinkKind, string][] = [["session", sessionId]];
    if (pilotId) targets.push(["pilot", pilotId]);
    for (const [kind, id] of targets) {
      try {
        void notify(kind, id, event).catch(report);
      } catch (error) {
        report(error);
      }
    }
  };

  return {
    started(sessionId: string, turn: number, pilotId?: string) {
      const turnKey = key(sessionId, turn);
      if (open.has(turnKey)) return;
      open.set(turnKey, pilotId);
      send(sessionId, pilotId, "started");
    },
    settled(sessionId: string, turn: number, status: TurnSettlement) {
      const turnKey = key(sessionId, turn);
      if (!open.has(turnKey)) return;
      const pilotId = open.get(turnKey);
      open.delete(turnKey);
      const event = settledTurnEvent(status);
      if (event) send(sessionId, pilotId, event);
    },
  };
}

export type LinkedTurns = ReturnType<typeof createLinkedTurns>;
