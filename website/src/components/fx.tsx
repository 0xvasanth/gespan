import { AnimatePresence, motion, useInView, useReducedMotion } from "motion/react";
import { createRef, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import logoUrl from "../assets/logo.svg";
import { AnimatedBeam } from "@/components/ui/animated-beam";
import { AnimatedList } from "@/components/ui/animated-list";
import { AnimatedShinyText } from "@/components/ui/animated-shiny-text";
import { BorderBeam } from "@/components/ui/border-beam";
import { MagicCard } from "@/components/ui/magic-card";
import { Marquee } from "@/components/ui/marquee";
import { NumberTicker } from "@/components/ui/number-ticker";
import { Particles } from "@/components/ui/particles";
import { cn } from "@/lib/utils";
import { AGENTS } from "./AgentWall";
import "./fx.css";

// Gespan's compositions of the Magic UI components (src/components/ui), in
// the glass palette. Each one shows something true about the app.

const VIOLET = { from: "#c0affc", to: "#7e47ea" };

/** The hero chip's text, with a light sweeping across it. */
export function ShinyText({ children }: { children: ReactNode }) {
  return <AnimatedShinyText className="mx-0 max-w-none text-[#c4c4c4] via-white/90" shimmerWidth={120}>{children}</AnimatedShinyText>;
}

/**
 * "Let ___ pull.": the headline cycles through the agents Gespan runs. Every
 * name sits in the same grid cell, so the slot keeps the longest name's width
 * and the old and new names cross-fade: the line never jumps or goes blank.
 * Screen readers get the plain sentence; reduced motion keeps "the agents".
 */
export const HERO_AGENTS = ["the agents", "Claude Code", "Codex", "Cursor CLI", "OpenCode", "Kiro CLI"];

export function HeroAgents({ interval = 2400 }: { interval?: number }) {
  const reduced = useReducedMotion();
  const [index, setIndex] = useState(0);
  useEffect(() => {
    if (reduced) return;
    const timer = setInterval(() => setIndex((i) => (i + 1) % HERO_AGENTS.length), interval);
    return () => clearInterval(timer);
  }, [reduced, interval]);
  const word = HERO_AGENTS[reduced ? 0 : index];
  return (
    <>
      <span className="sr-only">Let the agents pull.</span>
      <span aria-hidden="true">
        Let{" "}
        <span className="fx-rotor" data-word={word}>
          {HERO_AGENTS.map((w) => (
            <span key={w} className="fx-rotor-size">{w}</span>
          ))}
          <AnimatePresence initial={false}>
            <motion.span key={word} className="fx-word fx-rotor-word" initial={{ opacity: 0, y: "40%" }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: "-40%" }} transition={{ duration: 0.55, ease: [0.2, 0.7, 0.2, 1] }}>
              {word}
            </motion.span>
          </AnimatePresence>
        </span>{" "}
        pull.
      </span>
    </>
  );
}

/** A beam of light running around its parent's border; nothing for reduced motion. */
export function Beam({ duration = 8, size = 160, delay = 0 }: { duration?: number; size?: number; delay?: number }) {
  const reduced = useReducedMotion();
  if (reduced) return null;
  return <BorderBeam size={size} duration={duration} delay={delay} colorFrom={VIOLET.from} colorTo={VIOLET.to} borderWidth={1.5} />;
}

/** A card with a violet spotlight that follows the pointer. */
export function SpotCard({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <MagicCard className={cn("rounded-2xl", className)} gradientSize={260} gradientColor="rgba(143, 100, 246, 0.16)" gradientFrom={VIOLET.from} gradientTo={VIOLET.to}>
      {children}
    </MagicCard>
  );
}

/** The agent logos in two rows drifting in opposite directions. */
export function AgentMarquee() {
  const half = Math.ceil(AGENTS.length / 2);
  const row = (agents: typeof AGENTS) =>
    agents.map((agent) => (
      <div key={agent.name} className="fx-agent">
        <img src={agent.logo} alt="" width="36" height="36" />
        <span>{agent.name}</span>
      </div>
    ));
  return (
    <div className="fx-marquee">
      <ul className="sr-only" aria-label="Supported agents">
        {AGENTS.map((agent) => (
          <li key={agent.name}>{agent.name}</li>
        ))}
      </ul>
      <div aria-hidden="true">
        <Marquee pauseOnHover className="[--duration:38s] [--gap:12px]">{row(AGENTS.slice(0, half))}</Marquee>
        <Marquee reverse pauseOnHover className="[--duration:42s] [--gap:12px]">{row(AGENTS.slice(half))}</Marquee>
      </div>
    </div>
  );
}

const SOURCES = ["GitHub", "GitLab", "Linear", "Jira", "Azure DevOps"];
const TARGETS = AGENTS.filter((a) => ["Claude Code", "Codex", "Cursor CLI", "OpenCode", "Kiro CLI"].includes(a.name));

/**
 * How work moves through Gespan: tickets flow in from the trackers, through
 * Gespan's board and Pilots, out to the agents that run them.
 */
export function FlowDiagram() {
  const box = useRef<HTMLDivElement>(null);
  const hub = useRef<HTMLDivElement>(null);
  const from = useMemo(() => SOURCES.map(() => createRef<HTMLDivElement>()), []);
  const to = useMemo(() => TARGETS.map(() => createRef<HTMLDivElement>()), []);
  const reduced = useReducedMotion();
  const repeat = reduced ? 0 : Infinity;
  return (
    <figure className="fx-flow" aria-label="Tickets from GitHub, GitLab, Linear, Jira and Azure DevOps flow into Gespan, which runs them with Claude Code, Codex, Cursor CLI, OpenCode and Kiro CLI.">
      <div ref={box} className="fx-flow-box">
        <div className="fx-col">
          {SOURCES.map((name, i) => (
            <div key={name} ref={from[i]} className="fx-node">
              <span className="fx-dot" />
              {name}
            </div>
          ))}
        </div>
        <div className="fx-col fx-center">
          <div ref={hub} className="fx-hub">
            <img src={logoUrl} width="44" height="44" alt="" />
          </div>
        </div>
        <div className="fx-col">
          {TARGETS.map((agent, i) => (
            <div key={agent.name} ref={to[i]} className="fx-node fx-agent-node">
              <img src={agent.logo} alt="" width="22" height="22" />
              <span>{agent.name}</span>
            </div>
          ))}
        </div>
        {from.map((ref, i) => (
          <AnimatedBeam key={`in-${i}`} containerRef={box} fromRef={ref} toRef={hub} curvature={(i - 2) * -18} duration={4} delay={i * 0.35} repeat={repeat} pathColor="#ebebeb" pathOpacity={0.09} gradientStartColor={VIOLET.from} gradientStopColor="#9f7ef8" />
        ))}
        {to.map((ref, i) => (
          <AnimatedBeam key={`out-${i}`} containerRef={box} fromRef={hub} toRef={ref} curvature={(i - 2) * 18} duration={4} delay={1.6 + i * 0.35} repeat={repeat} pathColor="#ebebeb" pathOpacity={0.09} gradientStartColor="#9f7ef8" gradientStopColor="#459bf7" />
        ))}
      </div>
      <figcaption className="fx-flow-caption">
        <span><b>Inbox</b> Tickets and review requests</span>
        <span><b>Gespan</b> Board, automations and Pilots</span>
        <span><b>Sessions</b> Any agent, each in its own worktree</span>
      </figcaption>
    </figure>
  );
}

const FEED = [
  { kind: "Approval", tone: "wait", title: "Claude Code wants to run gh pr create", meta: "Auto-fix langfuse issues · 2h" },
  { kind: "Question", tone: "mute", title: "ReleasePilot asks which release branch", meta: "acme-web · 1h" },
  { kind: "Review", tone: "run", title: "Draft review ready on gespan#476", meta: "Codex · read-only run" },
  { kind: "Run", tone: "ok", title: "Nightly triage passed", meta: "Automation · 09:00" },
  { kind: "Approval", tone: "wait", title: "Codex wants to edit 2 files", meta: "PilotRobot epic #45 · now" },
];

/** What lands in My tasks, arriving one by one once the list scrolls into view. */
export function TaskFeed() {
  const ref = useRef<HTMLDivElement>(null);
  const seen = useInView(ref, { once: true, amount: 0.4 });
  const reduced = useReducedMotion();
  const items = FEED.map((item) => (
    <div key={item.title} className="fx-task">
      <span className={`w-tag ${item.tone}`}>{item.kind}</span>
      <div style={{ minWidth: 0 }}>
        <div className="fx-task-title">{item.title}</div>
        <div className="w-meta">{item.meta}</div>
      </div>
    </div>
  ));
  return (
    <div ref={ref} className="fx-feed" aria-label="Examples of what waits in My tasks">
      {reduced ? <div className="fx-feed-list">{items.slice().reverse()}</div> : seen ? <AnimatedList delay={1100} className="fx-feed-list">{items}</AnimatedList> : null}
    </div>
  );
}

const STATS = [
  { value: 12, label: "coding agents it runs" },
  { value: 5, label: "trackers in the Inbox" },
  { value: 3, label: "desktop platforms" },
  { value: 0, label: "telemetry events sent" },
];

/** A row of real numbers about Gespan that count up as they scroll into view. */
export function StatBand() {
  return (
    <dl className="fx-stats" data-stagger="">
      {STATS.map((stat) => (
        <div key={stat.label} className="fx-stat">
          <dt className="w-small">{stat.label}</dt>
          <dd>{stat.value === 0 ? <span className="fx-num">0</span> : <NumberTicker value={stat.value} className="fx-num" />}</dd>
        </div>
      ))}
    </dl>
  );
}

/** Drifting violet particles behind a section; nothing for reduced motion. */
export function GlowParticles() {
  const reduced = useReducedMotion();
  if (reduced) return null;
  // Inline position: the hero card's own rule (.w-hero > *) would otherwise make
  // this layer a full-height block and push the content down.
  return <Particles className="pointer-events-none" style={{ position: "absolute", inset: 0 }} quantity={90} ease={70} size={0.6} color="#c0affc" vy={-0.08} />;
}
