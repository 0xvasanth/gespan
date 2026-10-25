/**
 * The desktop app's Hold: confirm, stop every session at its safe point, keep
 * the hold across restarts, and resume sessions a few at a time. React state
 * stays in App; this class only decides and asks its host to act.
 */
import type { HarnessEvent } from "../../../integrations/harness/core/types";
import {
  afterRestart,
  continuation,
  loadHold,
  newHold,
  resumePlan,
  ResumeQueue,
  saveHold,
  settle,
  track,
  type HoldPhase,
  type HoldReason,
  type HoldState,
} from "./hold";
import { HoldGate } from "./holdGate";

/** At most this many held sessions start again at once. */
export const RESUME_LIMIT = 3;

export type HoldSession = {
  id: string;
  busy: boolean;
  pilotChat: boolean;
  awaitingInput: boolean;
  queued: number;
};

export type HoldHost = {
  sessions(): HoldSession[];
  /** End the turn at its safe point (holdHarnessTurn). */
  holdTurn(sessionId: string): void;
  /** Stop now: the user chose to end the command still running. */
  stop?(sessionId: string): void;
  /**
   * Continue a held session: send `prompt` first when given, then release its
   * queue. Returns whether a turn started.
   */
  resume(sessionId: string, prompt: string | null): boolean;
  /** The "Hold everything?" confirmation. */
  confirm(): Promise<boolean>;
  /** The hold changed; re-render. */
  changed(state: HoldState | null): void;
};

export class HoldCoordinator {
  private state: HoldState | null;
  private gates = new Map<string, HoldGate>();
  private interrupted = new Set<string>();
  private queue: ResumeQueue | null = null;
  private reasons = new Map<string, HoldReason>();
  private prompting = false;
  /** When the last hold began, once it has been resumed. */
  private lastHoldSince: number | undefined;
  /** Settlements of held turns, delivered once the session really stops. */
  private deferred = new Map<string, (later?: unknown) => void>();

  constructor(private host: HoldHost) {
    const saved = loadHold();
    // A restart ends every live turn: keep the hold, nothing is running.
    this.state = saved ? afterRestart(saved) : null;
    if (this.state) saveHold(this.state);
  }

  isHeld(): boolean {
    return this.state !== null;
  }

  phase(): HoldPhase | null {
    return this.state?.phase ?? null;
  }

  /** Held, or waiting for its turn to resume: it has not finished its work. */
  isSessionHeld(sessionId: string): boolean {
    return (
      this.reasons.has(sessionId) ||
      !!this.state?.sessions.some((s) => s.sessionId === sessionId)
    );
  }

  /** ⌘⇧H, the status bar, the palette and the menus all land here. */
  async toggle(): Promise<void> {
    if (this.state?.phase === "holding" || this.prompting) return;
    if (this.state?.phase === "onHold") {
      this.resume();
      return;
    }
    this.prompting = true;
    try {
      if (!(await this.host.confirm())) return;
    } finally {
      this.prompting = false;
    }
    this.holdAll(Date.now());
  }

  holdAll(now: number): void {
    if (this.state) return;
    const sessions = this.host.sessions();
    let state = newHold(now);
    for (const s of sessions) state = track(state, s.id, s.pilotChat);
    // With nothing to wait for, Gespan is on hold at once.
    this.state = sessions.length === 0 ? { ...state, phase: "onHold" } : state;
    for (const s of sessions) {
      if (!s.busy) {
        this.settle(s.id, "idle");
        continue;
      }
      this.gate(s.id).request();
      this.evaluate(s);
    }
    this.save();
  }

  /** Every live harness event, before batching. */
  observe(sessionId: string, event: HarnessEvent): void {
    this.gate(sessionId).observe(event);
    const session = this.host.sessions().find((s) => s.id === sessionId);
    if (session && this.unsettled(sessionId)) this.evaluate(session);
  }

  /** A session queued a message while held: it resumes with the rest. */
  noteQueued(sessionId: string, pilotChat: boolean): void {
    if (!this.state || this.state.sessions.some((s) => s.sessionId === sessionId)) {
      return;
    }
    this.state = settle(
      track(this.state, sessionId, pilotChat),
      sessionId,
      "idle",
      Date.now(),
    );
    this.save();
  }

  /** Hold a turn's settlement until the session really stops after Resume. */
  deferSettlement<T>(sessionId: string, deliver: (later?: T) => void): void {
    this.deferred.set(sessionId, deliver as (later?: unknown) => void);
  }

  /** A turn ended outside the hold: deliver a settlement it was holding back. */
  turnEnded<T>(sessionId: string, outcome: T): void {
    if (this.isSessionHeld(sessionId)) return;
    const deliver = this.deferred.get(sessionId);
    if (!deliver) return;
    this.deferred.delete(sessionId);
    deliver(outcome);
  }

  /** Session state changed: settle finished turns, refill the resume queue. */
  sync(): void {
    const sessions = this.host.sessions();
    // A resumed session whose turn ended may drop out of the host's list.
    const present = new Set(sessions.map((s) => s.id));
    for (const id of this.queue?.runningIds() ?? []) {
      if (!present.has(id)) this.finishResumed(id);
    }
    for (const s of sessions) {
      if (s.busy) {
        if (this.unsettled(s.id)) this.evaluate(s);
        continue;
      }
      this.gates.delete(s.id);
      if (this.unsettled(s.id)) {
        this.settle(s.id, this.interrupted.has(s.id) ? "interrupted" : "idle");
        this.save();
      }
      if (this.queue?.isRunning(s.id)) this.finishResumed(s.id);
    }
  }

  stopNow(sessionId: string): void {
    if (!this.unsettled(sessionId)) return;
    this.host.stop?.(sessionId);
    this.settle(sessionId, "interrupted");
    this.save();
  }

  resume(): void {
    const state = this.state;
    if (state?.phase !== "onHold") return;
    // Lift the hold first: resumed turns and schedulers may start work again.
    this.state = null;
    this.lastHoldSince = state.since;
    this.reasons = new Map(
      state.sessions.flatMap((s) => (s.reason ? [[s.sessionId, s.reason]] : [])),
    );
    const known = new Set(this.host.sessions().map((s) => s.id));
    this.queue = new ResumeQueue(resumePlan(state, (id) => known.has(id)));
    this.save();
    this.pump();
  }

  /** Start of the last hold, so schedulers can catch up what came due. */
  heldSince(): number | undefined {
    return this.lastHoldSince;
  }

  /** The status-bar text. */
  summary(): string {
    const state = this.state;
    if (!state) return "Hold all";
    const total = state.sessions.length;
    if (state.phase === "holding") {
      const settled = state.sessions.filter((s) => s.reason).length;
      return `Holding ${settled}/${total}…`;
    }
    const ids = new Set(state.sessions.map((s) => s.sessionId));
    const queued = this.host
      .sessions()
      .filter((s) => ids.has(s.id))
      .reduce((sum, s) => sum + s.queued, 0);
    const sessions = total === 1 ? "1 session" : `${total} sessions`;
    return `On hold · ${sessions} · ${queued} queued`;
  }

  /** Sessions still finishing a step. */
  settling(): string[] {
    return (this.state?.sessions ?? [])
      .filter((s) => s.reason === null)
      .map((s) => s.sessionId);
  }

  private finishResumed(sessionId: string): void {
    this.queue?.finished(sessionId);
    this.pump();
  }

  private gate(sessionId: string): HoldGate {
    let gate = this.gates.get(sessionId);
    if (!gate) {
      gate = new HoldGate();
      if (this.unsettled(sessionId)) gate.request();
      this.gates.set(sessionId, gate);
    }
    return gate;
  }

  private unsettled(sessionId: string): boolean {
    return !!this.state?.sessions.some(
      (s) => s.sessionId === sessionId && s.reason === null,
    );
  }

  private evaluate(session: HoldSession): void {
    const action = this.gate(session.id).decide(session.awaitingInput);
    if (!action) return;
    if (action.kind === "interrupt") {
      this.interrupted.add(session.id);
      this.host.holdTurn(session.id);
      return;
    }
    this.settle(session.id, action.point);
    this.save();
  }

  private settle(sessionId: string, reason: HoldReason): void {
    if (!this.state) return;
    this.state = settle(this.state, sessionId, reason, Date.now());
  }

  private pump(): void {
    const queue = this.queue;
    if (!queue) return;
    for (let next = queue.next(RESUME_LIMIT); next.length; next = queue.next(RESUME_LIMIT)) {
      for (const id of next) {
        const reason = this.reasons.get(id) ?? "idle";
        this.reasons.delete(id);
        this.interrupted.delete(id);
        if (!this.host.resume(id, continuation(reason))) {
          queue.finished(id);
          // Nothing more runs there: the held turn's own outcome stands.
          const deliver = this.deferred.get(id);
          this.deferred.delete(id);
          deliver?.();
        }
      }
    }
    if (queue.done()) this.queue = null;
  }

  private save(): void {
    saveHold(this.state);
    this.host.changed(this.state);
  }
}
