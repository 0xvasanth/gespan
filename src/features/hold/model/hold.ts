/**
 * Hold: stop every session at a safe point, keep it held across restarts,
 * and resume each one where it stopped. Mirrors Gespan Native's
 * `gespan_core::hold`.
 */
import { CONTINUE_PROMPT } from "../../sessions/model/inFlight";

export type HoldPhase = "holding" | "onHold";
export type HoldReason =
  | "idle"
  | "interrupted"
  | "awaitingInput"
  | "background"
  | "restarted";
export type HeldSession = {
  sessionId: string;
  /** Null while the session is still settling. */
  reason: HoldReason | null;
  heldAt: number;
  lastTool?: string;
  background: string[];
  /** Pilot chats resume after their workers. */
  pilotChat: boolean;
};
export type HoldState = {
  phase: HoldPhase;
  /** Epoch ms when Hold was confirmed. */
  since: number;
  sessions: HeldSession[];
};

const STORAGE_KEY = "gespan:hold";

export function newHold(now: number): HoldState {
  return { phase: "holding", since: now, sessions: [] };
}

export function track(
  state: HoldState,
  sessionId: string,
  pilotChat: boolean,
): HoldState {
  if (state.sessions.some((s) => s.sessionId === sessionId)) return state;
  return {
    ...state,
    sessions: [
      ...state.sessions,
      { sessionId, reason: null, heldAt: 0, background: [], pilotChat },
    ],
  };
}

export function settle(
  state: HoldState,
  sessionId: string,
  reason: HoldReason,
  now: number,
  lastTool?: string,
  background: string[] = [],
): HoldState {
  const sessions = state.sessions.map((s) =>
    s.sessionId === sessionId
      ? { ...s, reason, heldAt: now, lastTool, background }
      : s,
  );
  const settled = sessions.every((s) => s.reason !== null);
  return { ...state, sessions, phase: settled ? "onHold" : state.phase };
}

/**
 * A restart ends every live provider process, so waits that depended on one
 * (an approval, background work, a turn still settling) need a continuation.
 */
export function afterRestart(state: HoldState): HoldState {
  return {
    ...state,
    phase: "onHold",
    sessions: state.sessions.map((s) =>
      s.reason === "interrupted" || s.reason === "idle"
        ? s
        : { ...s, reason: "restarted" },
    ),
  };
}

/** The message that resumes a held turn, if it needs one. */
export function continuation(reason: HoldReason): string | null {
  return reason === "interrupted" || reason === "restarted"
    ? CONTINUE_PROMPT
    : null;
}

/** Resume stages: plain sessions and Pilot workers, then Pilot chats. */
export function resumePlan(
  state: HoldState,
  exists: (sessionId: string) => boolean,
): string[][] {
  const stage = (pilot: boolean) =>
    state.sessions
      .filter((s) => s.pilotChat === pilot && exists(s.sessionId))
      .map((s) => s.sessionId);
  return [stage(false), stage(true)].filter((s) => s.length > 0);
}

/**
 * Starts resumed sessions a few at a time, one stage after another: a later
 * stage begins once every session of the earlier one has started.
 */
export class ResumeQueue {
  private stages: string[][];
  private running = new Set<string>();
  constructor(plan: string[][]) {
    this.stages = plan.map((stage) => [...stage]);
  }
  next(limit: number): string[] {
    const start: string[] = [];
    while (this.running.size < limit) {
      while (this.stages.length > 0 && this.stages[0].length === 0) {
        this.stages.shift();
      }
      const id = this.stages[0]?.shift();
      if (id === undefined) break;
      this.running.add(id);
      start.push(id);
    }
    return start;
  }
  finished(sessionId: string): void {
    this.running.delete(sessionId);
  }
  isRunning(sessionId: string): boolean {
    return this.running.has(sessionId);
  }
  done(): boolean {
    return this.running.size === 0 && this.stages.every((s) => s.length === 0);
  }
}

export function loadHold(): HoldState | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const value = JSON.parse(raw) as HoldState;
    return Array.isArray(value?.sessions) ? value : null;
  } catch {
    return null;
  }
}

export function saveHold(state: HoldState | null): void {
  try {
    if (state) localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    else localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Storage can be unavailable; the hold then lasts until the app quits.
  }
}
