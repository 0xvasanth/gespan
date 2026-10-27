import { useRef, useSyncExternalStore } from "react";
import { Pause } from "../../../shared/ui/icons";
import { MOD } from "../../../platform/tauri/platform";
import type { HoldPress } from "../model/holdPress";
import type { PressLabel } from "../model/holdToConfirm";

const LABELS: Record<PressLabel, string> = {
  rest: "Hold all",
  hint: "Hold to confirm",
  armed: "Press again to confirm",
  done: "Holding…",
};

const face = "inline-flex items-center justify-center gap-1.5";

/**
 * Hold all: press and hold to confirm. The fill is an inverted copy of the
 * label, clipped to the progress, so the text changes colour under the edge.
 * The button keeps one width for every label.
 */
export function HoldToConfirm({ press }: { press: HoldPress }) {
  useSyncExternalStore(press.subscribe, press.version);
  // Space or Enter is down on the button: its click is not a click-only press.
  const keyDown = useRef(false);
  const progress = press.progress();
  const label = LABELS[press.label()];
  const isKey = (key: string) => key === " " || key === "Enter";
  return (
    <button
      type="button"
      className="relative inline-flex h-5 w-44 shrink-0 items-center justify-center overflow-hidden whitespace-nowrap rounded-md text-content/55 hover:bg-content/10 hover:text-content"
      aria-label={`Hold everything: press and hold (${MOD}⇧H)`}
      title={`Hold everything: press and hold (${MOD}⇧H)`}
      onPointerDown={(e) => {
        if (e.button === 0) press.start();
      }}
      onPointerUp={() => press.release()}
      onPointerLeave={() => press.cancel()}
      onPointerCancel={() => press.cancel()}
      onKeyDown={(e) => {
        if (!isKey(e.key)) return;
        e.preventDefault();
        if (keyDown.current || e.repeat) return;
        keyDown.current = true;
        press.start();
      }}
      onKeyUp={(e) => {
        if (!isKey(e.key) || !keyDown.current) return;
        e.preventDefault();
        keyDown.current = false;
        press.release();
      }}
      onBlur={() => {
        keyDown.current = false;
        press.cancel();
      }}
      onClick={(e) => {
        if (e.detail === 0 && !keyDown.current) press.click();
      }}
    >
      <span className={face}>
        <Pause className="size-3.5" strokeWidth={1.75} aria-hidden />
        <span>{label}</span>
      </span>
      <span
        data-hold-fill
        aria-hidden
        className={`${face} absolute inset-0 bg-content text-surface`}
        style={{ clipPath: `inset(0 ${(1 - progress) * 100}% 0 0)` }}
      >
        <Pause className="size-3.5" strokeWidth={1.75} />
        <span>{label}</span>
      </span>
    </button>
  );
}
