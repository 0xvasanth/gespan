import { useReducedMotion } from "motion/react";
import { useEffect, useRef, type ReactNode } from "react";
import "./bento.css";

/**
 * Where a card's border glow sits for a pointer at (x, y): `angle` is the
 * direction from the card's centre to the pointer, in degrees clockwise from
 * the top (what a conic-gradient's `from` takes), and `active` is whether the
 * pointer is on the card or within `reach` px of it.
 */
export function glowAt(rect: { left: number; top: number; width: number; height: number }, x: number, y: number, reach = 64): { angle: number; active: boolean } {
  const cx = rect.left + rect.width / 2;
  const cy = rect.top + rect.height / 2;
  const angle = ((Math.atan2(y - cy, x - cx) * 180) / Math.PI + 90 + 360) % 360;
  const active = x >= rect.left - reach && x <= rect.left + rect.width + reach && y >= rect.top - reach && y <= rect.top + rect.height + reach;
  return { angle, active };
}

const icon = (paths: ReactNode) => (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{paths}</svg>
);

function PanesPicture() {
  const pane = (name: string, color: string, lines: string[], tag: string) => (
    <div className="bt-pane">
      <div className="bt-pane-head"><span className="bt-chip" style={{ background: color }} />{name}<span className="w-tag run" style={{ marginLeft: "auto" }}>{tag}</span></div>
      {lines.map((l) => <div key={l} className="bt-line">{l}</div>)}
    </div>
  );
  return (
    <div className="bt-panes" aria-hidden="true">
      {pane("Claude Code", "#d97757", ["Reading src/auth/redirect.ts", "Keeping the original path"], "Running")}
      {pane("Codex", "#ebebeb", ["Updating 2 snapshot tests", "Running the test suite"], "Running")}
    </div>
  );
}

function BrowserPicture() {
  return (
    <div className="bt-browser" aria-hidden="true">
      <div className="bt-browser-bar"><span /><span /><span /><div className="bt-url">localhost:5173/checkout</div></div>
      <div className="bt-browser-body">
        <div className="bt-skel w60" />
        <div className="bt-skel w90" />
        <div className="bt-skel w40" />
        <div className="bt-cursor">Agent clicked “Pay now”</div>
      </div>
    </div>
  );
}

export interface Feature {
  title: string;
  body: string;
  icon: ReactNode;
  /** Wide cards span two of the four columns and carry a picture. */
  wide?: boolean;
  picture?: ReactNode;
}

export const FEATURES: Feature[] = [
  {
    title: "Multi-agent workspace",
    body: "Every conversation is a tab. Split panes to run several agents at once, and set the model, effort and permission mode for each session.",
    icon: icon(<><rect x="3" y="4" width="18" height="16" rx="2" /><line x1="12" y1="4" x2="12" y2="20" /></>),
    wide: true,
    picture: <PanesPicture />,
  },
  {
    title: "Worktrees built in",
    body: "Each parallel task gets its own Git worktree, named after the task. Setup and Teardown scripts prepare and clean up each one.",
    icon: icon(<><line x1="6" y1="3" x2="6" y2="15" /><circle cx="18" cy="6" r="3" /><circle cx="6" cy="18" r="3" /><path d="M18 9a9 9 0 0 1-9 9" /></>),
  },
  {
    title: "Review, undo and fork",
    body: "Checkpoints record what each turn changed, shell edits included. Keep or undo a turn's diff, or fork a new chat from any message.",
    icon: icon(<><polyline points="1 4 1 10 7 10" /><path d="M3.5 15a9 9 0 1 0 2.1-9.4L1 10" /></>),
  },
  {
    title: "Inbox and automations",
    body: "Bring in GitHub, GitLab, Linear, Jira and Azure DevOps work. Run agents on a schedule or when an Inbox event arrives.",
    icon: icon(<><polyline points="22 12 16 12 14 15 10 15 8 12 2 12" /><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" /></>),
  },
  {
    title: "MCP in one place",
    body: "Add an MCP server once and share it with every session. Servers need your approval, and their secrets stay in the keychain.",
    icon: icon(<><path d="M12 22v-5" /><path d="M9 8V2" /><path d="M15 8V2" /><path d="M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8z" /></>),
  },
  {
    title: "In-app browser",
    body: "Agents open, read and click through pages in a browser Gespan runs, and you use the same tab in a pane beside the chat.",
    icon: icon(<><circle cx="12" cy="12" r="10" /><line x1="2" y1="12" x2="22" y2="12" /><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" /></>),
    wide: true,
    picture: <BrowserPicture />,
  },
];

/**
 * The features as a bento grid: rows of four columns, wide + narrow +
 * narrow, then narrow + narrow + wide. Each card's border lights up on the
 * side facing the pointer as it comes near; with reduced motion, or no
 * pointer, the borders stay still.
 */
export function FeatureBento({ features = FEATURES }: { features?: Feature[] }) {
  const grid = useRef<HTMLUListElement>(null);
  const reduced = useReducedMotion();
  useEffect(() => {
    const el = grid.current;
    if (!el || reduced) return;
    let frame = 0;
    let last: { x: number; y: number } | null = null;
    const paint = () => {
      frame = 0;
      if (!last) return;
      for (const card of el.querySelectorAll<HTMLElement>(".bt-card")) {
        const { angle, active } = glowAt(card.getBoundingClientRect(), last.x, last.y);
        card.style.setProperty("--glow-angle", `${angle}deg`);
        card.dataset.glow = active ? "on" : "off";
      }
    };
    const move = (e: PointerEvent) => {
      last = { x: e.clientX, y: e.clientY };
      if (!frame) frame = requestAnimationFrame(paint);
    };
    window.addEventListener("pointermove", move, { passive: true });
    return () => {
      window.removeEventListener("pointermove", move);
      cancelAnimationFrame(frame);
    };
  }, [reduced]);
  return (
    <ul ref={grid} data-stagger="" className="bt-grid">
      {features.map((f) => (
        <li key={f.title} className={f.wide ? "bt-card wide" : "bt-card"} data-glow="off">
          <div className="bt-inner">
            <div className="w-tile">{f.icon}</div>
            <h3 className="w-h4">{f.title}</h3>
            <p className="w-small">{f.body}</p>
            {f.picture}
          </div>
        </li>
      ))}
    </ul>
  );
}
