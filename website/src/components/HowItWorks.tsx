import { AnimatePresence, motion, useMotionValueEvent, useScroll } from "motion/react";
import { useRef, useState, type ReactNode } from "react";
import "./how.css";

/**
 * The step showing for scroll progress 0…1 through `count` steps: each step
 * owns an equal slice of the scroll, and the ends rest on the first and last.
 */
export function stepAt(progress: number, count: number): number {
  if (count <= 0) return 0;
  const p = Number.isFinite(progress) ? Math.min(1, Math.max(0, progress)) : 0;
  return Math.min(count - 1, Math.floor(p * count));
}

function BoardPicture() {
  const col = (name: string, cards: [string, string][], tone?: string) => (
    <div className="hw-col">
      <div className="hw-col-head"><span>{name}</span><span className="w-meta">{cards.length}</span></div>
      {cards.map(([title, tag]) => (
        <div key={title} className={tone ? `hw-card ${tone}` : "hw-card"}>
          <div className="hw-card-title">{title}</div>
          <span className="w-tag mute">{tag}</span>
        </div>
      ))}
    </div>
  );
  return (
    <div className="hw-board">
      {col("Inbox", [["Fix the login redirect loop", "LIN-482"], ["Cart totals in minor units", "#212"]])}
      {col("In progress", [["Retry policy for webhooks", "PAY-31"]], "run")}
      {col("Review", [["Bump the auth SDK", "#208"]], "ok")}
    </div>
  );
}

function SessionPicture() {
  return (
    <div className="hw-stack">
      <div className="hw-row"><span className="hw-chip" style={{ background: "#d97757" }} /><b>Claude Code</b><span className="w-tag run" style={{ marginLeft: "auto" }}>Running</span></div>
      <div className="hw-row w-mono hw-dim">worktree · fix-login-redirect</div>
      <div className="hw-log">
        <div><span className="hw-ok">✓</span> Read src/auth/redirect.ts</div>
        <div><span className="hw-ok">✓</span> Keep the original path on redirect</div>
        <div><span className="hw-ok">✓</span> Added a test for the loop</div>
        <div><span className="hw-run">●</span> Opening a pull request…</div>
      </div>
      <div className="hw-row"><span className="w-tag ok">+ 24</span><span className="w-tag mute">− 6</span><span className="w-tag mute">2 files</span></div>
    </div>
  );
}

const RUNS = [9, 12, 8, 6, 14, 11, 16, 13, 10, 15];

function AutomationPicture() {
  return (
    <div className="hw-stack">
      <div className="hw-row"><b>Daily dependency audit</b><span className="w-tag mute" style={{ marginLeft: "auto" }}>Codex</span></div>
      <div className="hw-row hw-dim">Every day at 09:00 · next run in 3 h</div>
      <div className="hw-bars" aria-hidden="true">
        {RUNS.map((h, i) => <span key={i} className={i === 3 ? "x" : undefined} style={{ height: `${h * 4}px` }} />)}
      </div>
      <div className="hw-row hw-dim">9 of the last 10 runs passed · each run lands on the board</div>
    </div>
  );
}

function PilotPicture() {
  return (
    <div className="hw-stack">
      <div className="hw-row"><span className="hw-chip" style={{ background: "#f59e0b" }} /><b>ReleasePilot</b><span className="w-tag wait" style={{ marginLeft: "auto" }}>Needs you</span></div>
      <div className="hw-report"><span className="w-tag ok">Reported</span>Fix the login redirect loop · PR #214 open</div>
      <div className="hw-report"><span className="w-tag ok">Reported</span>Bump the auth SDK · merged</div>
      <div className="hw-report"><span className="w-tag run">Running</span>Retry policy for webhooks</div>
      <div className="hw-ask">Ship 0.4.0 from main, or wait for the webhook fix?</div>
    </div>
  );
}

export interface Step {
  key: string;
  title: string;
  body: string;
  /** A shorter name for the picture's tab strip, when the title is long. */
  tab?: string;
  picture: ReactNode;
}

export const STEPS: Step[] = [
  { key: "board", title: "Board", body: "Tickets from GitHub, GitLab, Linear, Jira and Azure DevOps land in the Inbox and become cards.", picture: <BoardPicture /> },
  { key: "sessions", title: "Sessions", body: "Run a card with any agent, each in its own Git worktree. The card moves as the session starts, opens a pull request and finishes.", picture: <SessionPicture /> },
  { key: "automations", title: "Automations", body: "Agents run on a schedule, like a daily dependency audit, and their runs show up on the board.", picture: <AutomationPicture /> },
  { key: "pilots", title: "Pilots", body: "A long-running agent with memory triages the Inbox, starts sessions and gets a report back from each one.", picture: <PilotPicture /> },
];

/**
 * A scroll story: the steps scroll by on the left while the picture on the
 * right stays put and switches to the step in view. On narrow screens each
 * step carries its own picture instead.
 */
/** `wide` gives the picture most of the row, for screenshots that need the room to be read. */
export function ScrollStory({ steps, wide = false }: { steps: Step[]; wide?: boolean }) {
  const track = useRef<HTMLOListElement>(null);
  const [active, setActive] = useState(0);
  const { scrollYProgress } = useScroll({ target: track, offset: ["start 55%", "end 55%"] });
  useMotionValueEvent(scrollYProgress, "change", (p) => setActive(stepAt(p, steps.length)));
  const step = steps[Math.min(active, steps.length - 1)];
  return (
    <div className={wide ? "hw wide" : "hw"} data-how="">
      <ol ref={track} className="hw-steps">
        {steps.map((s, i) => (
          <li key={s.key} className={s === step ? "hw-step on" : "hw-step"} data-step={s.key} aria-current={s === step ? "step" : undefined}>
            <span className="w-num">{String(i + 1).padStart(2, "0")}</span>
            <h3 className="w-h3">{s.title}</h3>
            <p className="w-lead">{s.body}</p>
            <div className="w-glass hw-inline" aria-hidden="true">{s.picture}</div>
          </li>
        ))}
      </ol>
      <div className="hw-sticky" aria-hidden="true">
        <div className="w-glass hw-panel" data-picture={step.key}>
          <div className="hw-panel-head">
            {steps.map((s) => <span key={s.key} className={s === step ? "on" : undefined}>{s.tab ?? s.title}</span>)}
          </div>
          <AnimatePresence mode="wait" initial={false}>
            <motion.div key={step.key} className="hw-panel-body" initial={{ opacity: 0, y: 14 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -14 }} transition={{ duration: 0.3, ease: [0.2, 0.7, 0.2, 1] }}>
              {step.picture}
            </motion.div>
          </AnimatePresence>
        </div>
      </div>
    </div>
  );
}

/** Home's How it works: Board, Sessions, Automations and Pilots. */
export function HowItWorks() {
  return <ScrollStory steps={STEPS} />;
}
