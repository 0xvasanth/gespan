/**
 * Hold-to-confirm: press and hold for HOLD_MS to confirm, release early to
 * drain back. Pure, so the status-bar control and the held ⌘⇧H shortcut
 * drive the same press. Mirrors Gespan Native's `hold_confirm::Press`.
 */
export const HOLD_MS = 1200;
/** A release sooner than this is a tap: show the hint instead. */
export const TAP_MS = 250;
export const HINT_MS = 1800;
/** Click-only input confirms on a second press within this window. */
export const ARMED_MS = 4000;
const DRAIN_MIN_MS = 180;
const DRAIN_FULL_MS = 500;

export type Released = "tap" | "drain" | "confirmed";
export type PressLabel = "rest" | "hint" | "armed" | "done";

type Fill =
  | { kind: "idle" }
  | { kind: "filling"; base: number; at: number }
  | { kind: "draining"; from: number; at: number; ms: number }
  | { kind: "done" };

export class Press {
  private fill: Fill = { kind: "idle" };
  private pressedAt: number | null = null;
  private hintUntil: number | null = null;
  private armedUntil: number | null = null;

  /** Resumes from whatever a drain left; repeats while filling are ignored. */
  start(now: number): void {
    if (this.fill.kind === "filling" || this.fill.kind === "done") return;
    this.fill = { kind: "filling", base: this.progress(now), at: now };
    this.pressedAt = now;
    this.hintUntil = null;
    this.armedUntil = null;
  }

  release(now: number): Released {
    if (this.fill.kind !== "filling") return "drain";
    if (this.tick(now)) return "confirmed";
    const tap = this.pressedAt !== null && now - this.pressedAt < TAP_MS;
    this.drain(now);
    if (tap) {
      this.hintUntil = now + HINT_MS;
      return "tap";
    }
    return "drain";
  }

  /** Abandoned (blur, pointer left, modifier released): drain, no hint. */
  cancel(now: number): void {
    if (this.fill.kind === "filling") this.drain(now);
  }

  private drain(now: number): void {
    const from = this.progress(now);
    this.pressedAt = null;
    this.fill =
      from > 0
        ? {
            kind: "draining",
            from,
            at: now,
            ms: Math.max(DRAIN_MIN_MS, from * DRAIN_FULL_MS),
          }
        : { kind: "idle" };
  }

  /** A click with no press behind it: the first arms, a second confirms. */
  click(now: number): boolean {
    if (this.fill.kind === "done") return false;
    if (this.armedUntil !== null && now < this.armedUntil) {
      this.fill = { kind: "done" };
      this.armedUntil = null;
      return true;
    }
    this.armedUntil = now + ARMED_MS;
    this.hintUntil = null;
    return false;
  }

  /** "Hold to confirm" without pressing, for the palette and menus. */
  hint(now: number): void {
    if (this.fill.kind !== "done") this.hintUntil = now + HINT_MS;
  }

  progress(now: number): number {
    const fill = this.fill;
    switch (fill.kind) {
      case "idle":
        return 0;
      case "done":
        return 1;
      case "filling":
        return Math.min(1, fill.base + Math.max(0, now - fill.at) / HOLD_MS);
      case "draining": {
        const t = Math.min(1, Math.max(0, now - fill.at) / fill.ms);
        return fill.from * (1 - t) ** 3;
      }
    }
  }

  tick(now: number): "confirmed" | null {
    if (this.fill.kind === "filling" && this.progress(now) >= 1) {
      this.fill = { kind: "done" };
      this.pressedAt = null;
      return "confirmed";
    }
    if (this.fill.kind === "draining" && this.progress(now) <= 0) {
      this.fill = { kind: "idle" };
    }
    return null;
  }

  label(now: number): PressLabel {
    if (this.fill.kind === "done") return "done";
    if (this.armedUntil !== null && now < this.armedUntil) return "armed";
    if (this.hintUntil !== null && now < this.hintUntil) return "hint";
    return "rest";
  }

  pressing(): boolean {
    return this.fill.kind === "filling";
  }

  /** A fill moving, or a transient label waiting to expire. */
  animating(now: number): boolean {
    const moving =
      this.fill.kind === "filling" ||
      (this.fill.kind === "draining" && this.progress(now) > 0);
    const label = this.label(now);
    return moving || label === "hint" || label === "armed";
  }
}
