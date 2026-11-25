import { useReducedMotion } from "motion/react";
import { createRef, useMemo, useRef } from "react";
import logoUrl from "../assets/logo.svg";
import { AnimatedBeam } from "@/components/ui/animated-beam";
import "./fx.css";

export type Hosting = "hosted" | "self";

/** What the switch shows for each way of running Cloud. */
export const HOSTING: Record<Hosting, { label: string; where: string; inline: string; detail: string }> = {
  hosted: { label: "Hosted", where: "Run by Gespan", inline: "run by Gespan", detail: "Sign up and your team gets a workspace. Gespan runs the web app; your Devboxes connect to it." },
  self: { label: "Self-hosted", where: "On your own servers", inline: "on your own servers", detail: "Run the same web app on your own servers, behind your own sign-in. Your Devboxes connect to it." },
};

const PEOPLE = ["Desktop app", "Browser", "Phone"];
const BOXES = [
  { name: "ada-devbox", tone: "run" },
  { name: "alice-devbox", tone: "ok" },
  { name: "carol-devbox", tone: "run" },
];

/** Where each kind of thing would live. Only the Cloud row changes with hosting. */
export function whereThingsLive(hosting: Hosting): [string, string][] {
  return [
    ["Your code", "Stays on your Devboxes. They clone it, run the agents and push to your code host."],
    ["Agent logins", "On each Devbox, with the same CLIs and plans as your terminal."],
    ["Tasks, decisions and run status", `In Gespan Cloud, ${HOSTING[hosting].inline}.`],
  ];
}

/**
 * How Gespan Cloud would fit together: people reach it from the desktop app,
 * a browser or a phone; it hands runs to the team's Devboxes. A switch shows
 * it hosted by Gespan or on your own servers.
 */
export function CloudMap({ hosting, onHosting }: { hosting: Hosting; onHosting: (h: Hosting) => void }) {
  const box = useRef<HTMLDivElement>(null);
  const hub = useRef<HTMLDivElement>(null);
  const from = useMemo(() => PEOPLE.map(() => createRef<HTMLDivElement>()), []);
  const to = useMemo(() => BOXES.map(() => createRef<HTMLDivElement>()), []);
  const reduced = useReducedMotion();
  const repeat = reduced ? 0 : Infinity;
  const mode = HOSTING[hosting];
  return (
    <div className="cm">
      <fieldset className="cm-switch">
        <legend className="sr-only">How Gespan Cloud would be run</legend>
        {(Object.keys(HOSTING) as Hosting[]).map((h) => (
          <button key={h} type="button" className={h === hosting ? "on" : undefined} aria-pressed={h === hosting} onClick={() => onHosting(h)}>
            {HOSTING[h].label}
          </button>
        ))}
      </fieldset>
      <figure className="fx-flow" aria-label={`Your team reaches Gespan Cloud from the desktop app, a browser or a phone. Cloud, ${mode.inline}, hands runs to the team's Devboxes, which run the agents.`}>
        <div ref={box} className="fx-flow-box" aria-hidden="true">
          <div className="fx-col fx-from">
            {PEOPLE.map((name, i) => (
              <div key={name} ref={from[i]} className="fx-node">
                <span className="fx-dot" />
                {name}
              </div>
            ))}
          </div>
          <div className="fx-beam-label" data-flow-label="in" data-cloud-label="in">Tasks and reviews</div>
          <div className="fx-col fx-center">
            <div ref={hub} className={reduced ? "fx-hub" : "fx-hub pulse"}>
              <img src={logoUrl} width="44" height="44" alt="" />
            </div>
            <div className="fx-hub-name">Gespan Cloud</div>
            <div className="fx-hub-sub" data-cloud-label="where">{mode.where}</div>
          </div>
          <div className="fx-beam-label" data-flow-label="out" data-cloud-label="out">Runs</div>
          <div className="fx-col fx-to">
            {BOXES.map((d, i) => (
              <div key={d.name} ref={to[i]} className="fx-node w-mono">
                <span className={`cm-dot ${d.tone}`} />
                {d.name}
              </div>
            ))}
          </div>
          {from.map((ref, i) => (
            <AnimatedBeam key={`in-${i}`} containerRef={box} fromRef={ref} toRef={hub} curvature={(i - 1) * -22} duration={4} delay={i * 0.4} repeat={repeat} pathColor="#ebebeb" pathOpacity={0.09} gradientStartColor="#c0affc" gradientStopColor="#9f7ef8" />
          ))}
          {to.map((ref, i) => (
            <AnimatedBeam key={`out-${i}`} containerRef={box} fromRef={hub} toRef={ref} curvature={0} duration={4} delay={1.4 + i * 0.4} repeat={repeat} pathColor="#ebebeb" pathOpacity={0.09} gradientStartColor="#9f7ef8" gradientStopColor="#459bf7" />
          ))}
        </div>
      </figure>
      <p className="cm-detail" aria-live="polite">{mode.detail}</p>
      <dl className="cm-where">
        {whereThingsLive(hosting).map(([what, where]) => (
          <div key={what} className="w-glass-flat">
            <dt>{what}</dt>
            <dd>{where}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
