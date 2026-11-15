import { Link } from "../router";
import { SiteNav } from "../components/SiteNav";
import { SiteFooter } from "../components/SiteFooter";
import { ScrollShot } from "../components/ScrollShot";
import { Beam } from "../components/fx";
import "./pilots.css";

export function PilotsPage() {
  return (
    <div className="w-site">
      <div className="w-frame">
        <section className="w-hero" aria-label="Pilots">
          <SiteNav active="pilots" onHero />
          <div className="w-wrap" style={{ paddingTop: "56px", display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", gap: "22px" }}>
            <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
              <p className="w-eyebrow">Pilots</p>
              <span className="w-tag">Experimental</span>
            </div>
            <h1 className="w-hero-title" style={{ maxWidth: "980px" }}>A Pilot keeps the work moving.</h1>
            <p className="w-lead" style={{ maxWidth: "660px" }}>A Pilot is a long-running agent with a soul, a memory and a schedule. It works across your projects, starts sessions with any agent, gets a report back from each one and asks you when a decision is yours.</p>
            <div className="w-stack-m w-full-m" style={{ display: "flex", gap: "10px" }}><Link className="w-btn w-btn-light" to="/download">Download Gespan</Link> <a className="w-btn w-btn-glass" href="#anatomy">What a Pilot is made of</a></div>
          </div>
          <div className="w-wrap pl-bleed" style={{ marginTop: "56px" }}>
            <ScrollShot name="pilot" />
          </div>
        </section>
      </div>
      {/* Anatomy */}
      <section id="anatomy" className="w-sec" aria-labelledby="an-h">
        <div className="w-glow violet" style={{ width: "900px", height: "600px", left: "-300px", top: "120px" }} />
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "40px" }}>
          <div data-reveal="" className="w-head">
            <p className="w-eyebrow">What a Pilot is made of</p>
            <h2 id="an-h" className="w-h2">Five parts, all plain files you can read and edit.</h2>
          </div>
          <div data-stagger="" className="w-g2">
            <div className="w-glass pl-part">
              <div className="hd">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M20.8 4.6a5.5 5.5 0 0 0-7.8 0L12 5.7l-1-1.1a5.5 5.5 0 0 0-7.8 7.8l1 1.1L12 21l7.8-7.5 1-1.1a5.5 5.5 0 0 0 0-7.8z" /></svg></div>
                <div>
                  <h3 className="w-h4">Soul</h3>
                  <p className="w-meta">SOUL.md · read before every run</p>
                </div>
              </div>
              <p className="w-small">Who the Pilot is, what it's responsible for and the rules it never breaks.</p>
              <div className="pl-inset">
                <div style={{ fontSize: "13px", fontWeight: "600", color: "#f9a8c9" }}>Role</div>
                <p className="w-small" style={{ color: "#cfcfcf" }}>I am the release expert responsible for managing the release process for the acme-web project. I follow the Release Process SOP exactly and keep the user informed at every step.</p>
                <div style={{ fontSize: "13px", fontWeight: "600", color: "#f9a8c9", marginTop: "4px" }}>Critical rules</div>
                <p className="w-small" style={{ color: "#cfcfcf" }}>Never delete or move a tag; tags are immutable. Never merge a pull request; ask the user to merge.</p>
              </div>
            </div>
            <div className="w-glass pl-part">
              <div className="hd">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><ellipse cx="12" cy="5" rx="9" ry="3" /><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" /><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" /></svg></div>
                <div>
                  <h3 className="w-h4">Memory</h3>
                  <p className="w-meta">Learned while it works</p>
                </div>
              </div>
              <p className="w-small">What the Pilot has learned, kept as files. It updates them as it goes; you can edit or forget any part.</p>
              <div className="pl-inset" style={{ flexDirection: "row", gap: "12px", padding: "8px" }}>
                <div style={{ width: "160px", flexShrink: "0", display: "flex", flexDirection: "column", gap: "2px" }}>
                  <div className="pl-file on">MEMORY.md</div>
                  <div className="pl-file">release-process</div>
                  <div className="pl-file">ci-checks</div>
                  <div className="pl-file">mobile-signing</div>
                </div>
                <div style={{ flex: "1", minWidth: "0", padding: "6px 4px", display: "flex", flexDirection: "column", gap: "6px" }}>
                  <div style={{ fontSize: "13px", fontWeight: "600", color: "#f9a8c9" }}>CI</div>
                  <p className="w-small" style={{ color: "#cfcfcf" }}>Native builds take about 30 minutes on Windows; do not report them as stuck before 45.</p>
                </div>
              </div>
            </div>
          </div>
          <div className="w-glass pl-part">
            <div className="hd">
              <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><circle cx="5" cy="6" r="2" /><circle cx="5" cy="18" r="2" /><circle cx="19" cy="12" r="2" /><path d="M7 6h4a4 4 0 0 1 4 4v0a2 2 0 0 0 2 2" /><path d="M7 18h4a4 4 0 0 0 4-4" /></svg></div>
              <div>
                <h3 className="w-h4">Pipelines</h3>
                <p className="w-meta">Release · v3 · step 3 of 5</p>
              </div>
              <span className="w-meta w-hide-m" style={{ marginLeft: "auto" }}>A run keeps the version it started with. Editing the flow saves a new one.</span>
            </div>
            <div className="pl-flow">
              <div className="pl-step"><span className="pl-ck"><svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12" /></svg></span>Plan<span className="w-meta" style={{ marginLeft: "auto" }}>4m</span></div>
              <div className="pl-arrow" />
              <div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
                <div className="pl-step"><span className="pl-ck"><svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12" /></svg></span>macOS build<span className="w-meta" style={{ marginLeft: "auto" }}>12m</span></div>
                <div className="pl-step"><span className="pl-ck"><svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12" /></svg></span>Windows build<span className="w-meta" style={{ marginLeft: "auto" }}>14m</span></div>
                <div className="pl-step"><span className="pl-ck"><svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12" /></svg></span>Linux tests<span className="w-meta" style={{ marginLeft: "auto" }}>9m</span></div>
                <p className="w-meta" style={{ paddingLeft: "4px" }}>A failed branch is fixed and retried, up to twice.</p>
              </div>
              <div className="pl-arrow" />
              <div className="pl-step wait" style={{ position: "relative" }}><span className="w-dot" style={{ background: "#e8c547", boxShadow: "0 0 0 3px rgba(232,197,71,.2)" }} />Cut branch<span className="w-tag wait" style={{ marginLeft: "auto" }}>You</span><Beam duration={3.5} size={70} /></div>
              <div className="pl-arrow" />
              <div className="pl-step dim">Release notes</div>
              <div className="pl-arrow" />
              <div className="pl-step dim">Publish</div>
            </div>
          </div>
          <div data-stagger="" className="w-g2">
            <div className="w-glass pl-part">
              <div className="hd">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z" /><polyline points="14 2 14 8 20 8" /></svg></div>
                <div>
                  <h3 className="w-h4">Docs</h3>
                  <p className="w-meta">Markdown and HTML reports</p>
                </div>
              </div>
              <p className="w-small">What the Pilot writes: its SOP pinned on top, then each run's plans and reports.</p>
              <div className="pl-inset" style={{ gap: "0", padding: "4px 16px" }}>
                <div className="pl-sched"><span className="w-tag mute">Pinned</span><span className="w-small" style={{ color: "#ebebeb" }}>Release Process SOP</span><span className="w-meta" style={{ marginLeft: "auto" }}>used by every run</span></div>
                <div className="pl-sched"><span className="w-tag wait">Draft</span><span className="w-small" style={{ color: "#ebebeb" }}>Release plan 0.2.1</span><span className="w-meta" style={{ marginLeft: "auto" }}>Today</span></div>
                <div className="pl-sched"><span className="w-tag ok">Done</span><span className="w-small" style={{ color: "#ebebeb" }}>Release 0.2.0</span><span className="w-meta" style={{ marginLeft: "auto" }}>Nov 12</span></div>
              </div>
            </div>
            <div className="w-glass pl-part">
              <div className="hd">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10" /><polyline points="12 6 12 12 16 14" /></svg></div>
                <div>
                  <h3 className="w-h4">Schedule</h3>
                  <p className="w-meta">Runs by itself</p>
                </div>
              </div>
              <p className="w-small">Habits that run on a timer, or now with the play button, and report back in the chat.</p>
              <div className="pl-inset" style={{ gap: "0", padding: "4px 16px" }}>
                <div className="pl-sched"><span className="w-small" style={{ color: "#ebebeb" }}>Nightly release check</span><span className="w-meta" style={{ marginLeft: "auto" }}>Daily at 02:00</span></div>
                <div className="pl-sched"><span className="w-small" style={{ color: "#ebebeb" }}>Draft the weekly changelog</span><span className="w-meta" style={{ marginLeft: "auto" }}>Mondays at 09:00</span></div>
                <div className="pl-sched"><span className="w-small" style={{ color: "#ebebeb" }}>Prune old release branches</span><span className="w-tag mute" style={{ marginLeft: "auto" }}>Paused</span></div>
              </div>
            </div>
          </div>
        </div>
      </section>
      {/* How it works with you */}
      <section className="w-sec" aria-labelledby="hw-h" style={{ paddingTop: "0" }}>
        <div className="w-wrap w-split">
          <div style={{ display: "flex", flexDirection: "column", gap: "32px" }}>
            <div data-reveal="" className="w-head">
              <p className="w-eyebrow">Working with a Pilot</p>
              <h2 id="hw-h" className="w-h2">Give it the goal. Keep the decisions.</h2>
            </div>
            <ol className="pl-tl" style={{ listStyle: "none", margin: "0", padding: "0" }}>
              <li>
                <span className="n">1</span>
                <div>
                  <h3 className="w-h4">Say what you want in its chat</h3>
                  <p className="w-small">A release, a triage pass, a sweep of failing checks.</p>
                </div>
              </li>
              <li>
                <span className="n">2</span>
                <div>
                  <h3 className="w-h4">It runs its pipeline</h3>
                  <p className="w-small">Steps run in order or in parallel, and it starts sessions with any agent you have.</p>
                </div>
              </li>
              <li>
                <span className="n">3</span>
                <div>
                  <h3 className="w-h4">Sessions report back</h3>
                  <p className="w-small">Each session it starts reports to the Pilot when it finishes, so the Pilot knows what happened.</p>
                </div>
              </li>
              <li>
                <span className="n">4</span>
                <div>
                  <h3 className="w-h4">It asks when the call is yours</h3>
                  <p className="w-small">The question lands in My tasks, and you answer with one tap.</p>
                </div>
              </li>
            </ol>
          </div>
          <div className="w-glass" style={{ padding: "22px", display: "flex", flexDirection: "column", gap: "14px" }}>
            <div style={{ display: "flex", alignItems: "center", gap: "10px", paddingBottom: "12px", borderBottom: "1px solid rgba(235,235,235,.07)" }}><span style={{ width: "18px", height: "18px", borderRadius: "6px", background: "#f59e0b" }} /><span style={{ fontSize: "14px", fontWeight: "600" }}>ReleasePilot</span><span className="w-tag wait" style={{ marginLeft: "auto" }}>Needs you</span></div>
            <div className="pl-bub" style={{ alignSelf: "flex-end", maxWidth: "380px", background: "rgba(69,155,247,.18)", border: "1px solid rgba(69,155,247,.24)" }}>Ship 0.2.1 with the sidebar ordering fix and the inbox label fix.</div>
            <div className="pl-bub" style={{ paddingLeft: "0", color: "#cfcfcf" }}>I ran the Release pipeline. Plan and Build & test are done: two fixes since v0.2.0, so a patch bump to 0.2.1. macOS, Windows and Linux ran in parallel, all green.</div>
            <div className="pl-bub" style={{ paddingLeft: "0", color: "#cfcfcf" }}>Next is <b style={{ color: "#ebebeb" }}>Cut branch</b>. main has 2 commits that aren't on release/0.2, so I need you to pick one.</div>
            <div className="w-stack-m" style={{ display: "flex", gap: "8px" }}><span className="w-btn w-btn-light sm">Cut from main</span><span className="w-btn w-btn-glass sm">Cut from release/0.2</span></div>
          </div>
        </div>
      </section>
      {/* Wake */}
      <section className="w-sec" aria-labelledby="wk-h" style={{ paddingTop: "0" }}>
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "32px" }}>
          <div data-reveal="" className="w-head">
            <p className="w-eyebrow">Always there</p>
            <h2 id="wk-h" className="w-h3">Reach it from wherever the work starts.</h2>
          </div>
          <div data-stagger="" className="w-g3">
            <div className="w-glass-flat pl-part">
              <h3 className="w-h4">Automations wake it</h3>
              <p className="w-small">An automation can deliver its run to a Pilot instead of starting a session. The prompt waits while the Pilot is busy, then wakes it.</p>
            </div>
            <div className="w-glass-flat pl-part">
              <h3 className="w-h4">Chats survive restarts</h3>
              <p className="w-small">A Pilot's chat is recovered after a quit, crash or restart, so a long job picks up where it was.</p>
            </div>
            <div className="w-glass-flat pl-part">
              <h3 className="w-h4">On your phone</h3>
              <p className="w-small">Paired with the iPhone companion, you can open a Pilot's conversation and send it a message from anywhere.</p>
            </div>
          </div>
          <p className="w-meta">Pilots are experimental. They work today and their setup is still changing between releases.</p>
        </div>
      </section>
      <section className="w-frame" style={{ paddingBottom: "12px" }}>
        <div className="w-hero soft">
          <div className="w-wrap w-stack-m" style={{ paddingTop: "64px", paddingBottom: "64px", display: "flex", alignItems: "center", gap: "32px" }}>
            <div style={{ flex: "1", display: "flex", flexDirection: "column", gap: "12px" }}>
              <h2 className="w-h2">Start your first Pilot.</h2>
              <p className="w-lead">Download Gespan and add a Pilot from the Pilots section of the sidebar.</p>
            </div>
            <div className="w-stack-m w-full-m" style={{ display: "flex", gap: "10px" }}><Link className="w-btn w-btn-light" to="/download">Download Gespan</Link></div>
          </div>
        </div>
      </section>
      <SiteFooter />
    </div>
  );
}
