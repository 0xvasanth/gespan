import type { ReactNode } from "react";
import { Link } from "../router";
import { SiteNav } from "../components/SiteNav";
import { SiteFooter } from "../components/SiteFooter";
import { REPO } from "../docs";
import "./hub.css";

// The Gespan Web Hub is a design concept only: a self-hosted web app for a
// team, with Devboxes that run the agents. Work on it has not started, so every
// section says "Coming soon" and "Concept", and the copy never promises it. The screens are drawn from the Gespan Web Hub design canvas;
// the demo team works on Payments API and Checkout web.

/** The badges every Web Hub section carries: it is a concept, not yet being built. */
export function SoonTag() {
  return (
    <span style={{ display: "inline-flex", gap: "6px" }}>
      <span className="w-tag wait">Coming soon</span>
      <span className="w-tag mute">Concept</span>
    </span>
  );
}

type Tone = "wait" | "run" | "ok" | "bad" | "mute";

const NEEDS: { title: string; detail: string; project: string; age: string; tone: Tone }[] = [
  { title: "Review asked: #212 Cart totals in minor units", detail: "GitHub asked alice-lead to review · from the run on #7", project: "Checkout web", age: "8 min", tone: "run" },
  { title: "A run asks: keep the v1 charge endpoint?", detail: "#1 Add idempotency keys to POST /charges", project: "Payments API", age: "12 min", tone: "wait" },
  { title: "Decide: retry policy for webhooks", detail: "Two options written up by the run on #4", project: "Payments API", age: "1 h", tone: "mute" },
];

const DEVBOXES: { name: string; detail: string; status: string; tone: Tone }[] = [
  { name: "ada-devbox", detail: "Ada Root · #1 on Payments API · 1 of 2 slots", status: "Working", tone: "run" },
  { name: "alice-devbox", detail: "Alice Lead · idle · 2 slots free", status: "Online", tone: "ok" },
  { name: "carol-devbox", detail: "Carol Chen · #7 on Checkout web", status: "Working", tone: "run" },
  { name: "bob-devbox", detail: "Bob Builder · last seen 25 min ago", status: "Offline", tone: "bad" },
];

const BOARD: { column: string; cards: { id: string; title: string; who: string; tone?: Tone }[] }[] = [
  { column: "Backlog", cards: [{ id: "#11", title: "Rate-limit the public API", who: "Unassigned" }] },
  { column: "Ready", cards: [{ id: "#9", title: "Refund webhook signature check", who: "Bob Builder" }] },
  { column: "Running", cards: [{ id: "#1", title: "Add idempotency keys to POST /charges", who: "ada-devbox", tone: "run" }] },
  { column: "In review", cards: [{ id: "#4", title: "Retry failed webhooks", who: "Alice Lead", tone: "wait" }] },
  { column: "Done", cards: [{ id: "#5", title: "Ledger export", who: "Carol Chen", tone: "ok" }] },
];

const DECISIONS: { scope: string; rule: string; sub: string }[] = [
  { scope: "Org", rule: "Every change ships with a test that fails without it.", sub: "Read by every run on the hub" },
  { scope: "Project", rule: "Keep amounts in minor units.", sub: "Read by every run in Checkout web" },
  { scope: "Personal", rule: "Open pull requests as drafts.", sub: "Read by your own runs" },
];

const ADMIN_TABS = ["Projects", "Users", "Access", "Connections", "Sign-in", "Devboxes", "License"];

function Dot({ tone }: { tone: Tone }) {
  return <span className={`hb-dot ${tone}`} aria-hidden="true" />;
}

/** The hub's Home screen, drawn in page markup so it reflows at every width. */
function HubHomeMock() {
  return (
    <div className="hb-app" aria-hidden="true">
      <aside className="hb-side">
        <div className="hb-team"><span className="hb-av">A</span><span>Acme</span></div>
        <span className="hb-side-h">Workspace</span>
        <span className="hb-nav on">Home</span>
        <span className="hb-nav">For you <b>3</b></span>
        <span className="hb-nav">Decisions</span>
        <span className="hb-nav">Devboxes</span>
        <span className="hb-side-h">Payments API</span>
        <span className="hb-nav">Board</span>
        <span className="hb-nav">Runs</span>
        <span className="hb-nav">Team</span>
        <span className="hb-nav">Insights</span>
      </aside>
      <div className="hb-main">
        <div className="hb-top"><span>Home</span><span className="hb-search">Search tasks, decisions, people, Devboxes</span></div>
        <div className="hb-stats">
          {[["3", "Need you"], ["3", "Running now"], ["3", "In review"], ["3 of 4", "Devboxes online"]].map(([n, l]) => (
            <div key={l} className="hb-card hb-stat"><span className="hb-muted">{l}</span><b>{n}</b></div>
          ))}
        </div>
        <div className="hb-cols">
          <div className="hb-col">
            <span className="hb-h">Needs you</span>
            <div className="hb-card hb-list">
              {NEEDS.map((n) => (
                <div key={n.title} className="hb-row">
                  <Dot tone={n.tone} />
                  <span className="hb-grow"><span className="hb-strong">{n.title}</span><span className="hb-muted">{n.detail}</span></span>
                  <span className="hb-end"><span className="hb-badge">{n.project}</span><span className="hb-muted">{n.age}</span></span>
                </div>
              ))}
            </div>
          </div>
          <div className="hb-col">
            <span className="hb-h">Devboxes</span>
            <div className="hb-card hb-list">
              {DEVBOXES.map((d) => (
                <div key={d.name} className="hb-row">
                  <span className="hb-grow"><span className="hb-strong hb-mono">{d.name}</span><span className="hb-muted">{d.detail}</span></span>
                  <span className={`hb-badge out ${d.tone}`}><Dot tone={d.tone} />{d.status}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function Feature({ id, eyebrow, title, lead, children }: { id: string; eyebrow: string; title: string; lead: string; children: ReactNode }) {
  return (
    <section className="w-sec" aria-labelledby={id} style={{ paddingTop: "0" }}>
      <div className="w-wrap w-split hb-feature">
        <div data-reveal="" className="w-head">
          <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
            <p className="w-eyebrow">{eyebrow}</p>
            <SoonTag />
          </div>
          <h2 id={id} className="w-h2">{title}</h2>
          <p className="w-lead">{lead}</p>
        </div>
        <div data-reveal="" className="w-glass hb-panel" aria-hidden="true">{children}</div>
      </div>
    </section>
  );
}

export function HubPage() {
  return (
    <div className="w-site">
      <div className="w-frame">
        <section className="w-hero" aria-label="Gespan Web Hub">
          <SiteNav active="hub" onHero />
          <div className="w-wrap" style={{ paddingTop: "56px", display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", gap: "22px" }}>
            <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
              <p className="w-eyebrow">Gespan Web Hub</p>
              <SoonTag />
            </div>
            <h1 className="w-hero-title" style={{ maxWidth: "980px" }}>A concept: your whole team's agents, in one place.</h1>
            <p className="w-lead" style={{ maxWidth: "680px" }}>
              An early design for a hub a team would run itself: tasks, reviews and questions from every project in one web app, agents working on Devboxes, and the rules they follow written down once for everyone.
            </p>
            <div className="w-stack-m w-full-m" style={{ display: "flex", gap: "10px" }}>
              <a className="w-btn w-btn-light" href={`${REPO}/releases`}>Follow Gespan on GitHub</a>
              <Link className="w-btn w-btn-glass" to="/download">Get the desktop app today</Link>
            </div>
            <p className="w-meta">This is a design concept only. Work on it hasn't started, there is no release date, and it may change completely or never ship.</p>
          </div>
          <div className="w-wrap" style={{ marginTop: "48px", paddingBottom: "48px" }}>
            <figure className="hb-shot" aria-label="A design concept of the Web Hub Home page: what needs you across projects, and every Devbox with its status.">
              <HubHomeMock />
            </figure>
          </div>
        </section>
      </div>

      <Feature id="foryou-h" eyebrow="For you" title="Everything waiting on you, from every project." lead="In the concept, questions from runs, review requests from GitHub and GitLab, and decisions to make would land in one list, oldest first, with Snoozed and Handled views.">
        <div className="hb-tabs"><span className="on">Waiting <b>3</b></span><span>Snoozed</span><span>Handled</span></div>
        <div className="hb-list">
          {NEEDS.map((n) => (
            <div key={n.title} className="hb-row">
              <Dot tone={n.tone} />
              <span className="hb-grow"><span className="hb-strong">{n.title}</span><span className="hb-muted">{n.project} · {n.age}</span></span>
            </div>
          ))}
        </div>
      </Feature>

      <Feature id="decisions-h" eyebrow="Decisions" title="The rules agents follow, written down once." lead="The concept writes down Org, Project and Personal decisions once and gives them to every run they apply to. Agents could propose new ones for review, and a replaced decision would keep its history.">
        <div className="hb-tabs"><span className="on">All</span><span>Awaiting review <b>2</b></span><span>Rules in force</span><span>Rejected</span><span>Superseded</span></div>
        <div className="hb-list">
          {DECISIONS.map((d) => (
            <div key={d.rule} className="hb-row">
              <span className="hb-badge">{d.scope}</span>
              <span className="hb-grow"><span className="hb-strong">{d.rule}</span><span className="hb-muted">{d.sub}</span></span>
            </div>
          ))}
        </div>
      </Feature>

      <Feature id="project-h" eyebrow="Inside a project" title="A board the whole team works from." lead="Each project would have a board that tasks move across from Backlog to Done while agents run them, plus Runs, Team, Activity, Insights for owners and leads, and Settings.">
        <div className="hb-board">
          {BOARD.map((col) => (
            <div key={col.column} className="hb-bcol">
              <span className="hb-muted hb-bh">{col.column}</span>
              {col.cards.map((c) => (
                <div key={c.id} className="hb-bcard">
                  <span className="hb-mono hb-muted">{c.id}</span>
                  <span className="hb-strong">{c.title}</span>
                  <span className="hb-muted" style={{ display: "flex", alignItems: "center", gap: "6px" }}>{c.tone && <Dot tone={c.tone} />}{c.who}</span>
                </div>
              ))}
            </div>
          ))}
        </div>
        <div className="hb-chips">{["Board", "Task", "Runs", "Team", "Activity", "Insights", "Settings"].map((t) => <span key={t} className="hb-badge">{t}</span>)}</div>
      </Feature>

      <Feature id="devbox-h" eyebrow="Devboxes" title="Agents would run on Devboxes, not in a browser tab." lead="In the design, a Devbox is a machine that runs the agents for its owner. It would be enrolled with a one-time code, and the hub would show which are online, the slots in use, the agents installed and the runner version.">
        <pre className="w-code hb-code"><span className="c"># Run this on the Devbox</span>{"\n"}gespan-runner enrol --hub https://hub.example.com --code <span className="p">••••••••</span></pre>
        <div className="hb-list">
          {DEVBOXES.slice(0, 3).map((d) => (
            <div key={d.name} className="hb-row">
              <span className="hb-grow"><span className="hb-strong hb-mono">{d.name}</span><span className="hb-muted">{d.detail}</span></span>
              <span className={`hb-badge out ${d.tone}`}><Dot tone={d.tone} />{d.status}</span>
            </div>
          ))}
        </div>
      </Feature>

      <Feature id="admin-h" eyebrow="Admin console" title="One console for the people who would run the hub." lead="The design has an admin console for adding projects and users, setting who can do what, connecting code hosts and trackers, choosing how people sign in and looking after Devboxes. Developers, leads and admins would each see the sidebar their role needs.">
        <div className="hb-tabs hb-wrap">{ADMIN_TABS.map((t, i) => <span key={t} className={i === 3 ? "on" : undefined}>{t}</span>)}</div>
        <div className="hb-list">
          {[["GitHub", "Pull requests and reviews", "ok", "Connected"], ["GitLab", "Merge request events stopped 40 min ago", "bad", "Failing"], ["Jira", "Tasks and tickets, both ways", "ok", "Connected"], ["Linear", "Tasks and tickets, both ways", "ok", "Connected"]].map(([name, detail, tone, status]) => (
            <div key={name} className="hb-row">
              <span className="hb-grow"><span className="hb-strong">{name}</span><span className="hb-muted">{detail}</span></span>
              <span className={`hb-badge out ${tone}`}><Dot tone={tone as Tone} />{status}</span>
            </div>
          ))}
        </div>
      </Feature>

      <section className="w-frame" aria-labelledby="hub-cta-h" style={{ paddingBottom: "12px" }}>
        <div className="w-hero soft" style={{ padding: "72px 24px", display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", gap: "18px" }}>
          <SoonTag />
          <h2 id="hub-cta-h" className="w-h2">Today, Gespan runs on your desktop.</h2>
          <p className="w-lead" style={{ maxWidth: "600px" }}>The desktop app drives your agents now, free and open source. The Web Hub is only a concept; follow the project on GitHub to hear if it moves forward.</p>
          <div className="w-stack-m w-full-m" style={{ display: "flex", gap: "10px" }}>
            <Link className="w-btn w-btn-light" to="/download">Download Gespan</Link>
            <a className="w-btn w-btn-glass" href={`${REPO}/releases`}>Watch releases</a>
          </div>
        </div>
      </section>
      <SiteFooter />
    </div>
  );
}
