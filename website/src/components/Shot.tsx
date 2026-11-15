import { useEffect, useLayoutEffect, useRef, useState } from "react";

/** The app screens are drawn at the desktop window size and scaled to fit. */
export const SHOT_WIDTH = 1440;
export const SHOT_HEIGHT = 900;

// Each screen loads on demand, so the shots don't weigh on the first page load.
const SHOTS = {
  home: { load: () => import("../shots/home.html?raw"), label: "The Gespan Home page: what needs you, the agents working now, automation runs, plan usage and today's events." },
  tasks: { load: () => import("../shots/tasks.html?raw"), label: "My tasks in Gespan: Claude Code asks to run gh pr create, with Allow once, Allow for this session and Deny." },
  pilot: { load: () => import("../shots/pilot.html?raw"), label: "A Pilot's chat in Gespan: ReleasePilot runs the Release pipeline and asks which branch to cut from." },
};
export type ShotName = keyof typeof SHOTS;

/** The scale that fits the whole screen into a frame `width` px wide. Never above 1, so a shot is never cropped or upscaled. */
export function shotScale(width: number): number {
  if (!(width > 0)) return 0;
  return Math.min(width, SHOT_WIDTH) / SHOT_WIDTH;
}

/**
 * A real Gespan screen, rendered in a shadow root so its styles stay apart
 * from the site's, and scaled to the frame's width at every screen size.
 */
export function Shot({ name, className }: { name: ShotName; className?: string }) {
  const frame = useRef<HTMLDivElement>(null);
  const screen = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState(0);

  useLayoutEffect(() => {
    const el = frame.current;
    if (!el) return;
    const measure = () => setScale(shotScale(el.clientWidth));
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    let live = true;
    SHOTS[name].load().then(({ default: html }) => {
      const el = screen.current;
      if (!live || !el) return;
      const root = el.shadowRoot ?? el.attachShadow({ mode: "open" });
      // Static markup from src/shots, bundled at build time; no user input reaches it.
      root.innerHTML = html;
    });
    return () => {
      live = false;
    };
  }, [name]);

  return (
    // A live screen drawn in HTML, so it can't be an <img>; it is announced as one.
    // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role
    <div ref={frame} className={className ? `w-shot ${className}` : "w-shot"} role="img" aria-label={SHOTS[name].label} data-shot={name}>
      <div ref={screen} className="w-shot-in" inert style={{ transform: `scale(${scale})`, visibility: scale ? "visible" : "hidden" }} />
    </div>
  );
}
