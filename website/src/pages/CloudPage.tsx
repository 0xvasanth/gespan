import { useState } from "react";
import { Link } from "../router";
import { NavSpace } from "../components/SiteNav";
import { SiteFooter } from "../components/SiteFooter";
import { Macbook } from "../components/Macbook";
import { CloudMap } from "../components/CloudMap";
import { ScrollStory, type Step } from "../components/HowItWorks";
import { HeroLight, SpotCard } from "../components/fx";
import { REPO } from "../docs";
import homeShot from "../assets/cloud/home.webp";
import forYouShot from "../assets/cloud/for-you.webp";
import decisionsShot from "../assets/cloud/decisions.webp";
import boardShot from "../assets/cloud/board.webp";
import devboxesShot from "../assets/cloud/devboxes.webp";
import adminShot from "../assets/cloud/admin.webp";
import "./cloud.css";

// Gespan Cloud is a design concept only: a web app for a team, used hosted or
// run on the team's own servers, with Devboxes that run the agents. Work on
// it has not started, so every section says "Coming soon" and "Concept", and
// the copy never promises it. The screens are captured from the Gespan Cloud
// design canvas (each artboard rendered at 1440×900, 2x); the demo team works
// on Payments API and Checkout web.

/** The pinned issue where visitors register interest in Gespan Cloud. */
export const CLOUD_INTEREST = `${REPO}/issues/554`;

/** The badges every Cloud section carries: it is a concept, not yet being built. */
export function SoonTag() {
  return (
    <span style={{ display: "inline-flex", gap: "6px" }}>
      <span className="w-tag wait">Coming soon</span>
      <span className="w-tag mute">Concept</span>
    </span>
  );
}

/** A screen from the design canvas, at the canvas's 1440×900 frame size. */
function Screen({ src, label, eager = false }: { src: string; label: string; eager?: boolean }) {
  return <img className="w-shot gc-screen" src={src} alt={label} width={1440} height={900} loading={eager ? "eager" : "lazy"} decoding="async" />;
}

/** The concept's screens, one per step of the scroll story. */
export const CLOUD_STEPS: Step[] = [
  {
    key: "for-you",
    title: "Everything waiting on you, from every project.",
    tab: "For you",
    body: "Questions from runs, review requests from GitHub and GitLab, and decisions to make would land in one list, oldest first, with Snoozed and Handled views.",
    picture: <Screen src={forYouShot} label="The concept's For you page: questions from runs, review requests and decisions waiting on you, with your tasks beside them." />,
  },
  {
    key: "decisions",
    title: "The rules agents follow, written down once.",
    tab: "Decisions",
    body: "Org, Project and Personal decisions would be written once and given to every run they apply to. Agents could propose new ones for review, and a replaced decision would keep its history.",
    picture: <Screen src={decisionsShot} label="The concept's Decisions page: Org, Project and Personal rules, with the ones awaiting review." />,
  },
  {
    key: "project",
    title: "A board the whole team works from.",
    tab: "Board",
    body: "Each project would have a board that tasks move across from Backlog to Done while agents run them, plus Runs, Team, Activity, Insights for owners and leads, and Settings.",
    picture: <Screen src={boardShot} label="The concept's project board for Payments API: tasks in Backlog, Assigned, Ready and Claimed, with who owns each." />,
  },
  {
    key: "devboxes",
    title: "Agents would run on Devboxes, not in a browser tab.",
    tab: "Devboxes",
    body: "A Devbox is a machine that runs the agents for its owner, with the same CLIs and plans as their terminal. It would be enrolled with a one-time code, and Cloud would show which are online, the slots in use and the runner version.",
    picture: <Screen src={devboxesShot} label="The concept's Devboxes page: enrol a Devbox with a one-time code, and every Devbox with its slots, disk, agents and runner version." />,
  },
  {
    key: "admin",
    title: "One console for the people who run it.",
    tab: "Admin",
    body: "An admin console for adding projects and users, setting who can do what, connecting code hosts and trackers, choosing how people sign in and looking after Devboxes. Developers, leads and admins would each see the sidebar their role needs.",
    picture: <Screen src={adminShot} label="The concept's admin console on Connections: GitHub, GitLab, Jira and Linear, with one that needs attention." />,
  },
];

/** What the concept covers, as a bento grid; `wide` cards span two columns, so rows go wide + narrow, then narrow + wide. */
const COVERS: { title: string; body: string; wide?: boolean }[] = [
  { title: "Hosted, or on your servers", body: "Use Gespan Cloud as a hosted workspace, or run the same app on your own servers. Your code stays on your Devboxes either way.", wide: true },
  { title: "Bring your own agents", body: "Devboxes run the agent CLIs your team already pays for. No API keys to hand over and no token resale." },
  { title: "Sign-in and roles", body: "Email, GitHub or your company's single sign-on. Developers, leads and admins each get the views their role needs." },
  { title: "Same app on your desk", body: "The desktop app stays the place you work. Cloud adds the team around it: one inbox, one board, shared decisions.", wide: true },
];

function Head({ id, eyebrow, title, lead }: { id: string; eyebrow: string; title: string; lead?: string }) {
  return (
    <div data-reveal="" className="w-head">
      <div style={{ display: "flex", alignItems: "center", gap: "10px", flexWrap: "wrap" }}>
        <p className="w-eyebrow">{eyebrow}</p>
        <SoonTag />
      </div>
      <h2 id={id} className="w-h2">{title}</h2>
      {lead && <p className="w-lead">{lead}</p>}
    </div>
  );
}

export function CloudPage() {
  const [hosting, setHosting] = useState<"hosted" | "self">("hosted");
  return (
    <div className="w-site">
      <div className="w-frame">
        <section className="w-hero gc-hero" aria-label="Gespan Cloud">
          <HeroLight />
          <NavSpace />
          <div className="w-wrap" style={{ paddingTop: "40px", display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", gap: "22px" }}>
            <div className="animate-rise motion-reduce:animate-none" style={{ display: "flex", alignItems: "center", gap: "10px", flexWrap: "wrap", justifyContent: "center" }}>
              <p className="w-eyebrow">Gespan Cloud</p>
              <SoonTag />
            </div>
            <h1 className="w-hero-title animate-rise motion-reduce:animate-none [animation-delay:90ms]" style={{ maxWidth: "980px" }}>A concept: your whole team's agents, in one place.</h1>
            <p className="w-lead animate-rise motion-reduce:animate-none [animation-delay:220ms]" style={{ maxWidth: "680px" }}>
              An early design for Gespan Cloud: tasks, reviews and questions from every project in one web app, agents working on Devboxes, and the rules they follow written down once for everyone. Teams would use it hosted, or run it on their own servers.
            </p>
            <div className="w-stack-m w-full-m animate-rise motion-reduce:animate-none [animation-delay:340ms]" style={{ display: "flex", gap: "10px" }}>
              <a className="w-btn w-btn-light" href={CLOUD_INTEREST}>Register interest</a>
              <Link className="w-btn w-btn-glass" to="/download">Get the desktop app today</Link>
            </div>
            <p className="w-meta animate-rise motion-reduce:animate-none [animation-delay:440ms]">This is a design concept only. Work on it hasn't started, there is no release date, and it may change completely or never ship.</p>
          </div>
          <div className="w-wrap mb-under-hero">
            <figure className="gc-shot">
              <Macbook>
                <Screen src={homeShot} label="The concept's Home page: what needs you across projects, your projects, and every Devbox with its status." eager />
              </Macbook>
            </figure>
          </div>
        </section>
      </div>

      {/* How it would work */}
      <section className="w-sec" aria-labelledby="cloud-how-h">
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "40px" }}>
          <Head id="cloud-how-h" eyebrow="How it would work" title="Your team in the browser. Your agents on your machines." lead="Cloud would keep the team's shared picture: tasks, decisions and how each run is going. The agents would run on Devboxes, so your code and agent logins stay on machines you own." />
          <div data-reveal="">
            <CloudMap hosting={hosting} onHosting={setHosting} />
          </div>
        </div>
      </section>

      {/* The concept's screens */}
      <section className="w-sec" aria-labelledby="cloud-screens-h" style={{ paddingTop: "0" }}>
        <div className="w-glow violet" style={{ width: "900px", height: "500px", left: "-200px", top: "80px" }} />
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "40px" }}>
          <Head id="cloud-screens-h" eyebrow="Inside the concept" title="One web app for what the whole team is doing." />
          <ScrollStory steps={CLOUD_STEPS} wide />
        </div>
      </section>

      {/* What it covers */}
      <section className="w-sec" aria-labelledby="cloud-covers-h" style={{ paddingTop: "0" }}>
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "40px" }}>
          <Head id="cloud-covers-h" eyebrow="What the concept covers" title="Built around the agents you already have." />
          <div data-stagger="" className="gc-bento">
            {COVERS.map((c) => (
              <SpotCard key={c.title} className={c.wide ? "gc-wide" : undefined}>
                <div className="gc-cover">
                  <h3 className="w-h4">{c.title}</h3>
                  <p className="w-small">{c.body}</p>
                </div>
              </SpotCard>
            ))}
          </div>
        </div>
      </section>

      <section className="w-frame" aria-labelledby="cloud-cta-h" style={{ paddingBottom: "12px" }}>
        <div className="w-hero soft" style={{ padding: "72px 24px", display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", gap: "18px" }}>
          <SoonTag />
          <h2 id="cloud-cta-h" className="w-h2">Want Gespan Cloud for your team?</h2>
          <p className="w-lead" style={{ maxWidth: "600px" }}>It's only a concept for now. Give the interest issue a 👍 and subscribe to hear if it moves forward. Today, the desktop app drives your agents, free and open source.</p>
          <div className="w-stack-m w-full-m" style={{ display: "flex", gap: "10px", flexWrap: "wrap", justifyContent: "center" }}>
            <a className="w-btn w-btn-light" href={CLOUD_INTEREST}>Register interest</a>
            <Link className="w-btn w-btn-glass" to="/download">Download Gespan</Link>
            <a className="w-btn w-btn-glass" href={`${REPO}/releases`}>Watch releases</a>
          </div>
        </div>
      </section>
      <SiteFooter />
    </div>
  );
}
