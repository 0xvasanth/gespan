/**
 * One hold-to-confirm press for the app: the status-bar button and the held
 * ⌘⇧H shortcut drive it together, and the button renders it.
 */
import type { HoldPhase } from "./hold";
import { Press, type PressLabel } from "./holdToConfirm";

const PRESS_FRAME_MS = 16;
/** How often a waiting hint or armed label checks whether it expired. */
const LABEL_FRAME_MS = 250;

export class HoldPress {
  private press = new Press();
  private listeners = new Set<() => void>();
  private timer: ReturnType<typeof setTimeout> | null = null;
  private changes = 0;
  /** ⌘⇧H is down: its repeats are ignored until it is released. */
  keyHeld = false;

  constructor(
    private onConfirm: () => void,
    private now: () => number = () => Date.now(),
  ) {}

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  /** Changes on every repaint, for `useSyncExternalStore`. */
  version = (): number => this.changes;

  progress(): number {
    return this.press.progress(this.now());
  }
  label(): PressLabel {
    return this.press.label(this.now());
  }

  start(): void {
    this.press.start(this.now());
    this.drive();
  }
  release(): void {
    if (!this.press.pressing()) return;
    if (this.press.release(this.now()) === "confirmed") this.confirmed();
    else this.drive();
  }
  /** Blur, pointer left, or the press otherwise abandoned: never confirms. */
  cancel(): void {
    this.keyHeld = false;
    this.press.cancel(this.now());
    this.drive();
  }
  /** A click with no press behind it (keyboard, VoiceOver). */
  click(): void {
    if (this.press.click(this.now())) this.confirmed();
    else this.drive();
  }
  /** "Hold to confirm", for the palette and menus, which can't be held. */
  hint(): void {
    this.press.hint(this.now());
    this.drive();
  }

  /** ⌘⇧H went down (or repeated). */
  shortcutDown(phase: HoldPhase | null, resume: () => void): void {
    if (this.keyHeld) return;
    this.keyHeld = true;
    if (phase === null) this.start();
    else if (phase === "onHold") resume();
  }
  /**
   * The key or one of its modifiers went up. macOS sends no key-up for H
   * while ⌘ is down, so releasing ⌘ or ⇧ is what usually ends the press.
   */
  shortcutUp(): void {
    if (!this.keyHeld) return;
    this.keyHeld = false;
    this.release();
  }

  dispose(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    this.listeners.clear();
  }

  private confirmed(): void {
    this.press = new Press();
    this.notify();
    this.onConfirm();
  }

  private notify(): void {
    this.changes++;
    for (const listener of this.listeners) listener();
  }

  private drive(): void {
    this.notify();
    if (this.timer !== null) return;
    const step = () => {
      this.timer = null;
      const now = this.now();
      if (this.press.tick(now) === "confirmed") {
        this.confirmed();
        return;
      }
      this.notify();
      const moving = this.press.pressing() || this.press.progress(now) > 0;
      if (moving) this.timer = setTimeout(step, PRESS_FRAME_MS);
      else if (this.press.animating(now)) this.timer = setTimeout(step, LABEL_FRAME_MS);
    };
    this.timer = setTimeout(step, PRESS_FRAME_MS);
  }
}
