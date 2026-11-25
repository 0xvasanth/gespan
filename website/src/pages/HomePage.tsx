import { Link } from "../router";
import { NavSpace } from "../components/SiteNav";
import { SiteFooter } from "../components/SiteFooter";
import { HowItWorks } from "../components/HowItWorks";
import { Macbook } from "../components/Macbook";
import { ScrollShot } from "../components/ScrollShot";
import logoUrl from "../assets/logo.svg";
import { AgentMarquee, FlowDiagram, GlowParticles, HeroAgents, ShinyText, SpotCard, StatBand, TaskFeed } from "../components/fx";
import { GUIDES } from "../docs";
import { changelog, formatDate, highlights, release } from "../release";
import { SoonTag } from "./HubPage";
import "./home.css";
import "./hub.css";

/** The Web Hub teaser's inbox rows: title, project and age, status tone. */
const HUB_PREVIEW = [
  ["Review asked: #212 Cart totals in minor units", "Checkout web · 8 min", "run"],
  ["A run asks: keep the v1 charge endpoint?", "Payments API · 12 min", "wait"],
  ["Decide: retry policy for webhooks", "Payments API · 1 h", "mute"],
] as const;

export function HomePage() {
  return (
    <div className="w-site">
      {/* Hero */}
      <div className="w-frame">
        <section className="w-hero lp-hero" aria-label="Introduction">
          {/* One light source: a spotlight that sweeps in over a faint grid. */}
          <div aria-hidden="true" className="lp-grid" />
          <div aria-hidden="true" className="lp-spot" />
          <NavSpace />
          <div className="w-wrap lp-hero-copy" style={{ paddingTop: "40px", display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", gap: "24px" }}>
            {/* What's new sits in the hero, directly above the headline. */}
            <Link className="w-announce animate-rise motion-reduce:animate-none" to="/changelog">
              <span className="new">New in {release.version}</span>
              <span className="txt"><ShinyText>{highlights(changelog[0], 1)}</ShinyText></span>
              <span className="go">Changelog →</span>
            </Link>
            <h1 className="w-hero-title">
              <span className="animate-rise motion-reduce:animate-none block [animation-delay:90ms]">Hold the reins.</span>
              <span className="animate-rise motion-reduce:animate-none block [animation-delay:220ms]" style={{ color: "#a3a3a3" }}><HeroAgents /></span>
            </h1>
            <p className="w-lead animate-rise motion-reduce:animate-none [animation-delay:360ms]" style={{ maxWidth: "620px" }}>One desktop app to drive all of your coding agents. Run them side by side, plan their work on a board, and let a Pilot keep it moving. It uses the subscriptions you already pay for.</p>
            <div className="lp-cta w-stack-m w-full-m animate-rise motion-reduce:animate-none [animation-delay:480ms]" style={{ display: "flex", gap: "10px", alignItems: "center", justifyContent: "center" }}><Link className="w-btn w-btn-light" to="/download"><svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M16.4 12.6c0-2.3 1.9-3.4 2-3.5-1.1-1.6-2.8-1.8-3.4-1.8-1.4-.1-2.8.9-3.5.9-.7 0-1.8-.9-3-.8-1.5 0-3 .9-3.8 2.3-1.6 2.8-.4 7 1.2 9.3.8 1.1 1.7 2.4 2.9 2.3 1.2 0 1.6-.7 3-.7s1.8.7 3 .7c1.3 0 2.1-1.1 2.8-2.3.9-1.3 1.3-2.6 1.3-2.6s-2.5-1-2.5-3.8zM14.1 5.8c.6-.8 1.1-1.9 1-3-.9 0-2.1.6-2.7 1.4-.6.7-1.1 1.8-1 2.9 1 .1 2.1-.5 2.7-1.3z" /></svg> Download for macOS</Link> <a className="w-btn w-btn-glass" href="https://github.com/0xvasanth/gespan"><svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M12 .5a11.5 11.5 0 0 0-3.64 22.41c.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.37-3.88-1.37-.52-1.33-1.28-1.69-1.28-1.69-1.05-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.77 2.7 1.26 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.24-1.28-5.24-5.69 0-1.26.45-2.29 1.19-3.1-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a11 11 0 0 1 5.77 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.84 1.19 3.1 0 4.42-2.7 5.39-5.26 5.68.41.36.78 1.06.78 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 12 .5z" /></svg> View on GitHub</a></div>
            <p className="w-meta animate-rise motion-reduce:animate-none [animation-delay:580ms]">Free and open source · <a className="w-link" href="#agents">12 agents</a> · macOS, Windows and Linux · No telemetry</p>
          </div>
          <div className="w-wrap lp-laptop">
            <Macbook shot="home" />
          </div>
        </section>
      </div>
      {/* Agents */}
      <section id="agents" className="w-sec" aria-labelledby="agents-h">
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "40px" }}>
          <div data-reveal="" className="w-head center">
            <p className="w-eyebrow">Supported agents</p>
            <h2 id="agents-h" className="w-h2">Runs the agents you already have.</h2>
            <p className="w-lead">If an agent is installed and signed in, Gespan can run it. Your plans, logins, skills and MCP setup carry over. No API keys, no new accounts and no token resale.</p>
          </div>
          <AgentMarquee />
          <p className="w-small" style={{ textAlign: "center" }}><a className="w-link" href={GUIDES.providers}>Install and sign-in steps for each agent</a></p>
        </div>
      </section>
      {/* Flow */}
      <section className="w-sec" aria-labelledby="flow-h" style={{ paddingTop: "0" }}>
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "40px" }}>
          <div data-reveal="" className="w-head center">
            <p className="w-eyebrow">One place for the work</p>
            <h2 id="flow-h" className="w-h2">Work flows in. Your agents pull it through.</h2>
          </div>
          <div data-reveal="">
            <FlowDiagram />
          </div>
          <StatBand />
        </div>
      </section>
      {/* How it works */}
      <section className="w-sec" aria-labelledby="loop-h" style={{ paddingTop: "0" }}>
        <div className="w-glow violet" style={{ width: "900px", height: "500px", left: "-200px", top: "80px" }} />
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "40px" }}>
          <div data-reveal="" className="w-head">
            <p className="w-eyebrow">How it works</p>
            <h2 id="loop-h" className="w-h2">From ticket to merged, with you holding the reins.</h2>
          </div>
          <HowItWorks />
          <p className="w-meta" style={{ maxWidth: "760px" }}>This is where Gespan is heading, and parts of the loop are still being built: adding Inbox items to the board, pull requests moving cards, and Pilots and automations working the board directly. Follow along in <a className="w-link" href="https://github.com/0xvasanth/gespan/issues/211">issue 211</a>.</p>
        </div>
      </section>
      {/* My tasks shot */}
      <section className="w-sec" aria-labelledby="tasks-h" style={{ paddingTop: "0" }}>
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "40px" }}>
          <div className="w-split">
            <div data-reveal="" className="w-head">
              <p className="w-eyebrow">My tasks</p>
              <h2 id="tasks-h" className="w-h2">Everything waiting on you, in one place.</h2>
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
              <p className="w-lead">Approvals, questions and draft reviews from every agent land in one list. Answer from the card and Gespan moves you to the next one. Answered and Decisions keep each request and what you said.</p>
              <TaskFeed />
            </div>
          </div>
          <ScrollShot name="tasks" />
        </div>
      </section>
      {/* Features */}
      <section className="w-sec" aria-labelledby="feat-h" style={{ paddingTop: "0" }}>
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "40px" }}>
          <div className="w-stack-m" style={{ display: "flex", alignItems: "flex-end", gap: "24px" }}>
            <div data-reveal="" className="w-head">
              <p className="w-eyebrow">Features</p>
              <h2 id="feat-h" className="w-h2">One home for every agent.</h2>
            </div>
            <Link className="w-btn w-btn-glass" to="/features" style={{ marginLeft: "auto" }}>See every feature</Link>
          </div>
          <div data-stagger="" className="w-g3">
            <SpotCard>
<div className="lp-feat">
              <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2" /><line x1="12" y1="4" x2="12" y2="20" /></svg></div>
              <h3 className="w-h4">Multi-agent workspace</h3>
              <p className="w-small">Every conversation is a tab. Split panes to run several agents at once, and set the model, effort and permission mode for each session.</p>
            </div>
</SpotCard>
            <SpotCard>
<div className="lp-feat">
              <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><line x1="6" y1="3" x2="6" y2="15" /><circle cx="18" cy="6" r="3" /><circle cx="6" cy="18" r="3" /><path d="M18 9a9 9 0 0 1-9 9" /></svg></div>
              <h3 className="w-h4">Worktrees built in</h3>
              <p className="w-small">Each parallel task gets its own Git worktree, named after the task. Setup and Teardown scripts prepare and clean up each one.</p>
            </div>
</SpotCard>
            <SpotCard>
<div className="lp-feat">
              <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="1 4 1 10 7 10" /><path d="M3.5 15a9 9 0 1 0 2.1-9.4L1 10" /></svg></div>
              <h3 className="w-h4">Review, undo and fork</h3>
              <p className="w-small">Checkpoints record what each turn changed, shell edits included. Keep or undo a turn's diff, or fork a new chat from any message.</p>
            </div>
</SpotCard>
            <SpotCard>
<div className="lp-feat">
              <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="22 12 16 12 14 15 10 15 8 12 2 12" /><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" /></svg></div>
              <h3 className="w-h4">Inbox and automations</h3>
              <p className="w-small">Bring in GitHub, GitLab, Linear, Jira and Azure DevOps work. Run agents on a schedule or when an Inbox event arrives.</p>
            </div>
</SpotCard>
            <SpotCard>
<div className="lp-feat">
              <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M12 22v-5" /><path d="M9 8V2" /><path d="M15 8V2" /><path d="M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8z" /></svg></div>
              <h3 className="w-h4">MCP in one place</h3>
              <p className="w-small">Add an MCP server once and share it with every session. Servers need your approval, and their secrets stay in the keychain.</p>
            </div>
</SpotCard>
            <SpotCard>
<div className="lp-feat">
              <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10" /><line x1="2" y1="12" x2="22" y2="12" /><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" /></svg></div>
              <h3 className="w-h4">In-app browser</h3>
              <p className="w-small">Agents open, read and click through pages in a browser Gespan runs, and you use the same tab in a pane beside the chat.</p>
            </div>
</SpotCard>
          </div>
        </div>
      </section>
      {/* Pilots */}
      <section className="w-sec" aria-labelledby="pilots-h" style={{ paddingTop: "0" }}>
        <div className="w-glow violet" style={{ width: "1000px", height: "600px", right: "-300px", top: "0" }} />
        <div className="w-glow blue" style={{ width: "700px", height: "500px", left: "-200px", bottom: "0" }} />
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "40px" }}>
          <div className="w-split">
            <div data-reveal="" className="w-head">
              <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
                <p className="w-eyebrow">Pilots</p>
                <span className="w-tag">Experimental</span>
              </div>
              <h2 id="pilots-h" className="w-h2">A Pilot keeps the work moving.</h2>
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: "20px" }}>
              <p className="w-lead">A Pilot is a long-running agent that works across your projects. It reads its soul before every run, keeps a memory, writes docs, runs pipelines on a schedule and reports back on the sessions it starts. When a decision is yours, it asks.</p>
              <div style={{ display: "flex", gap: "8px", flexWrap: "wrap", alignItems: "center" }}><span className="w-tag mute">Soul</span><span className="w-tag mute">Memory</span><span className="w-tag mute">Docs</span><span className="w-tag mute">Pipelines</span><span className="w-tag mute">Schedule</span> <Link className="w-btn w-btn-glass sm" to="/pilots" style={{ marginLeft: "8px" }}>Meet Pilots</Link></div>
            </div>
          </div>
          <ScrollShot name="pilot" />
        </div>
      </section>
      {/* Principles */}
      <section className="w-sec" aria-labelledby="why-h" style={{ paddingTop: "0" }}>
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "40px" }}>
          <div data-reveal="" className="w-head">
            <p className="w-eyebrow">Why Gespan</p>
            <h2 id="why-h" className="w-h2">A GUI over agents you already have. Nothing in between.</h2>
          </div>
          <div data-stagger="" className="w-g4">
            <SpotCard>
<div className="lp-feat">
              <h3 className="w-h4">Bring your own agents</h3>
              <p className="w-small">Gespan runs the CLIs you have installed and signed in to, with the same login and plan as your terminal.</p>
            </div>
</SpotCard>
            <SpotCard>
<div className="lp-feat">
              <h3 className="w-h4">Native and local</h3>
              <p className="w-small">Built in Rust and GPUI. Sessions are stored on your machine, and the app has no telemetry.</p>
            </div>
</SpotCard>
            <SpotCard>
<div className="lp-feat">
              <h3 className="w-h4">No token resale</h3>
              <p className="w-small">Gespan doesn't sell tokens or proxy your requests. Your usage stays between you and your provider.</p>
            </div>
</SpotCard>
            <SpotCard>
<div className="lp-feat">
              <h3 className="w-h4">You stay in charge</h3>
              <p className="w-small">Review diffs, keep or undo each turn, approve tools and stop runaway agents from one window.</p>
            </div>
</SpotCard>
          </div>
        </div>
      </section>
      {/* Mobile companion */}
      <section className="w-sec" aria-labelledby="mobile-h" style={{ paddingTop: "0" }}>
        <div className="w-wrap">
          <div className="w-hero soft w-stack-m" style={{ padding: "56px", display: "flex", alignItems: "center", gap: "56px" }}>
            <div style={{ flex: "1", minWidth: "0", display: "flex", flexDirection: "column", gap: "18px" }}>
              <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
                <p className="w-eyebrow">Mobile companion</p>
                <span className="w-tag">Invite only</span>
              </div>
              <h2 id="mobile-h" className="w-h2">Check in from your phone.</h2>
              <p className="w-lead">Pair your iPhone by scanning a code from the desktop app. Open any session, send a message, start a new one, stop a turn and read the real Git changes.</p>
              <p className="w-small">The phone talks straight to your desktop over QUIC with mutually authenticated TLS 1.3. Approvals and questions still need the desktop for now.</p>
            </div>
            <div className="w-phone" style={{ alignSelf: "center" }}>
              <div className="w-phone-screen">
                <div style={{ padding: "44px 16px 10px", display: "flex", alignItems: "center", gap: "8px", borderBottom: "1px solid rgba(235,235,235,.07)" }}>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#a3a3a3" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="15 18 9 12 15 6" /></svg>
                  <div style={{ minWidth: "0" }}>
                    <div style={{ fontSize: "13px", fontWeight: "600" }}>Fix the login redirect loop</div>
                    <div style={{ fontSize: "11px", color: "#8a8a8a" }}>acme-web · Claude Code</div>
                  </div>
                </div>
                <div style={{ flex: "1", padding: "14px", display: "flex", flexDirection: "column", gap: "10px" }}>
                  <div className="lp-bub" style={{ alignSelf: "flex-end", maxWidth: "200px", background: "rgba(69,155,247,.18)", border: "1px solid rgba(69,155,247,.24)" }}>Is the fix ready for review?</div>
                  <div className="lp-bub" style={{ paddingLeft: "0", color: "#cfcfcf" }}>Yes. The redirect now keeps the original path, and I added a test for the loop. 2 files changed.</div>
                  <div style={{ display: "flex", gap: "6px" }}><span className="w-tag ok">+ 24</span><span className="w-tag mute">− 6</span><span className="w-tag mute">2 files</span></div>
                </div>
                <div style={{ margin: "0 10px 12px", height: "44px", borderRadius: "22px", display: "flex", alignItems: "center", padding: "0 6px 0 14px", background: "rgba(255,255,255,.06)", border: "1px solid rgba(255,255,255,.09)", fontSize: "12.5px", color: "#8a8a8a" }}>Message Claude Code<span style={{ marginLeft: "auto", width: "32px", height: "32px", borderRadius: "50%", background: "#ebebeb", display: "flex", alignItems: "center", justifyContent: "center" }}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#111" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><line x1="12" y1="19" x2="12" y2="5" /><polyline points="5 12 12 5 19 12" /></svg></span></div>
              </div>
            </div>
          </div>
        </div>
      </section>
      {/* FAQ */}
      <section className="w-sec" aria-labelledby="hub-h" style={{ paddingTop: "0" }}>
        <div className="w-wrap">
          <div className="w-hero soft hb-teaser">
            <div style={{ flex: "1", minWidth: "0", display: "flex", flexDirection: "column", gap: "18px" }}>
              <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
                <p className="w-eyebrow">Gespan Web Hub</p>
                <SoonTag />
              </div>
              <h2 id="hub-h" className="w-h2">A concept: your whole team's agents, in one place.</h2>
              <p className="w-lead">An early design, not yet being built, for a hub a team would run itself: one inbox for every project, a shared board, Devboxes that run the agents, and decisions every agent follows.</p>
              <div><Link className="w-btn w-btn-glass" to="/hub">See the concept</Link></div>
            </div>
            <div className="w-glass hb-panel" aria-hidden="true">
              <div className="hb-tabs"><span className="on">Waiting <b>3</b></span><span>Snoozed</span><span>Handled</span></div>
              <div className="hb-list">
                {HUB_PREVIEW.map(([title, meta, tone]) => (
                  <div key={title} className="hb-row">
                    <span className={`hb-dot ${tone}`} />
                    <span className="hb-grow"><span className="hb-strong">{title}</span><span className="hb-muted">{meta}</span></span>
                  </div>
                ))}
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="w-sec" aria-labelledby="faq-h" style={{ paddingTop: "0" }}>
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "40px" }}>
          <div data-reveal="" className="w-head">
            <p className="w-eyebrow">Questions</p>
            <h2 id="faq-h" className="w-h2">The short answers.</h2>
          </div>
          <div data-stagger="" className="w-g2">
            <div className="w-glass-flat lp-q">
              <h3 className="w-h4">Is Gespan free?</h3>
              <p className="w-small">Yes. Gespan is open source under the MIT License. You pay only for the agent plans you already have, directly to their providers.</p>
            </div>
            <div className="w-glass-flat lp-q">
              <h3 className="w-h4">Do I need API keys?</h3>
              <p className="w-small">No. Gespan runs the agent CLIs you have already signed in to, so it uses the same login and plan as your terminal.</p>
            </div>
            <div className="w-glass-flat lp-q">
              <h3 className="w-h4">Does my code go through Gespan?</h3>
              <p className="w-small">No. Each agent talks to its own provider, exactly as it does in your terminal. Sessions stay on your machine.</p>
            </div>
            <div className="w-glass-flat lp-q">
              <h3 className="w-h4">Why not just use terminal tabs?</h3>
              <p className="w-small">You can, and Gespan runs those same CLIs. It adds the layer around them: tabs and split panes, a worktree per task, per-turn undo, approvals in one place, and a board, inbox, automations and Pilots.</p>
            </div>
          </div>
        </div>
      </section>
      {/* Download CTA */}
      <section className="w-frame" aria-labelledby="cta-h" style={{ paddingBottom: "12px" }}>
        <div className="w-hero">
          <GlowParticles />
          <div className="w-wrap" style={{ paddingTop: "96px", paddingBottom: "96px", display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", gap: "22px" }}>
            <img src={logoUrl} width="64" height="64" alt="" />
            <h2 id="cta-h" className="w-h1">Hold the reins.</h2>
            <p className="w-lead" style={{ maxWidth: "520px" }}>Install and sign in to at least one supported agent, then download Gespan for your machine.</p>
            <div className="lp-cta w-stack-m w-full-m" style={{ display: "flex", gap: "10px", flexWrap: "wrap", justifyContent: "center" }}><Link className="w-btn w-btn-light" to="/download">macOS</Link> <Link className="w-btn w-btn-glass" to="/download">Windows</Link> <Link className="w-btn w-btn-glass" to="/download">Linux</Link></div>
            <p className="w-meta">Version {release.version} · Released {formatDate(release.date)} · <a className="w-link" href="https://github.com/0xvasanth/gespan/releases">All releases</a></p>
          </div>
        </div>
      </section>
      <SiteFooter />
    </div>
  );
}
