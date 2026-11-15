import { Link } from "../router";
import { SiteNav } from "../components/SiteNav";
import { SiteFooter } from "../components/SiteFooter";
import "./features.css";

export function FeaturesPage() {
  return (
    <div className="w-site">
      <div className="w-frame">
        <section className="w-hero soft" aria-label="Features">
          <SiteNav active="features" onHero />
          <div className="w-wrap" style={{ paddingTop: "56px", paddingBottom: "72px", display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", gap: "22px" }}>
            <p className="w-eyebrow">Features</p>
            <h1 className="w-h1" style={{ maxWidth: "860px" }}>Everything around your agents, in one window.</h1>
            <p className="w-lead" style={{ maxWidth: "640px" }}>Gespan runs the same CLIs as your terminal. What it adds is the layer around them: tabs, worktrees, review, a board, an inbox, automations and Pilots.</p>
            <nav className="fe-jump" aria-label="Feature groups" style={{ marginTop: "8px" }}><a href="#workspace">Workspace</a> <a href="#review">Review and control</a> <a href="#plan">Plan and automate</a> <a href="#connect">Connect</a> <a href="#beyond">Beyond the desktop</a></nav>
          </div>
        </section>
      </div>
      {/* Workspace */}
      <section id="workspace" className="w-sec" aria-labelledby="g1">
        <div className="w-wrap w-aside">
          <div className="fe-group">
            <p className="w-eyebrow">01 · Workspace</p>
            <h2 id="g1" className="w-h3">Many agents, side by side.</h2>
            <p className="w-small">Every conversation is a tab, and every parallel task can have its own worktree so agents never clash.</p>
          </div>
          <div className="w-g2">
            <div className="w-glass-flat fe-card">
              <div className="top">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2" /><line x1="12" y1="4" x2="12" y2="20" /></svg></div>
                <h3 className="w-h4">Multi-agent workspace</h3>
              </div>
              <p className="w-small">Split panes to run several agents at once, pin the tabs you keep coming back to, and set the model, effort and permission mode for each session.</p>
            </div>
            <div className="w-glass-flat fe-card">
              <div className="top">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><rect x="2" y="6" width="9" height="6" rx="1.5" /><rect x="13" y="6" width="9" height="6" rx="1.5" /><line x1="2" y1="17" x2="22" y2="17" /></svg></div>
                <h3 className="w-h4">Tabs grouped by project</h3>
                <span className="w-tag" style={{ marginLeft: "auto" }}>New in 0.2</span>
              </div>
              <p className="w-small">The title strip groups tabs by project across all your projects, with a choice of tab layout and an option to show only your tabs.</p>
            </div>
            <div className="w-glass-flat fe-card">
              <div className="top">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><line x1="6" y1="3" x2="6" y2="15" /><circle cx="18" cy="6" r="3" /><circle cx="6" cy="18" r="3" /><path d="M18 9a9 9 0 0 1-9 9" /></svg></div>
                <h3 className="w-h4">Worktrees built in</h3>
              </div>
              <p className="w-small">Each parallel task gets its own Git worktree, named after the task. Setup and Teardown scripts prepare and clean up each one, and merged branches are offered for deletion.</p>
            </div>
            <div className="w-glass-flat fe-card">
              <div className="top">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="4 17 10 11 4 5" /><line x1="12" y1="19" x2="20" y2="19" /></svg></div>
                <h3 className="w-h4">Editor, terminal and scripts</h3>
              </div>
              <p className="w-small">Edit files, preview Markdown and split terminals next to your sessions. Scripts runs your test, build and dev-server commands from <span className="w-code-inline">.gespan/tasks.json</span>, package.json or a Makefile.</p>
            </div>
            <div className="w-glass-flat fe-card">
              <div className="top">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="7 10 12 15 17 10" /><line x1="12" y1="15" x2="12" y2="3" /></svg></div>
                <h3 className="w-h4">Import sessions</h3>
              </div>
              <p className="w-small">Bring past Claude Code, Codex, Cursor and Kiro sessions into the project they ran in. An imported session continues the same native session.</p>
            </div>
            <div className="w-glass-flat fe-card">
              <div className="top">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10" /><polyline points="12 6 12 12 16 14" /></svg></div>
                <h3 className="w-h4">Usage at a glance</h3>
              </div>
              <p className="w-small">See your Claude Code, Codex and Kiro plan usage in the chat footer during every turn. If a login expires, a Sign in card signs you back in and resends your message.</p>
            </div>
          </div>
        </div>
      </section>
      {/* Review */}
      <section id="review" className="w-sec" aria-labelledby="g2" style={{ paddingTop: "0" }}>
        <div className="w-glow violet" style={{ width: "900px", height: "600px", right: "-300px", top: "0" }} />
        <div className="w-wrap w-aside">
          <div className="fe-group">
            <p className="w-eyebrow">02 · Review and control</p>
            <h2 id="g2" className="w-h3">You decide what lands.</h2>
            <p className="w-small">Approve tools, read what each turn changed and keep or undo it, from one place.</p>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
            <div className="w-glass fe-card" style={{ padding: "24px" }}>
              <div className="top"><span className="w-tag wait">Approval</span><span className="w-small">Claude Code wants to run a command</span><span className="w-meta" style={{ marginLeft: "auto" }}>Bash · ~/acme-web</span></div>
              <pre className="w-code">{"gh pr create --base main --title \"Keep the original path after login\""}</pre>
              <div style={{ display: "flex", gap: "8px", flexWrap: "wrap" }}><span className="w-btn w-btn-light sm">Allow once</span><span className="w-btn w-btn-glass sm">Allow for this session</span><span className="w-btn w-btn-ghost sm">Deny</span></div>
              <p className="w-meta">Approval cards show a Bash command as real multi-line text with its folder, an edit as a small diff, and an MCP call as its server and arguments.</p>
            </div>
            <div className="w-g2">
              <div className="w-glass-flat fe-card">
                <div className="top">
                  <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="1 4 1 10 7 10" /><path d="M3.5 15a9 9 0 1 0 2.1-9.4L1 10" /></svg></div>
                  <h3 className="w-h4">Review, undo and fork</h3>
                </div>
                <p className="w-small">Checkpoints record what each turn changed, including edits made through shell commands. Keep or undo a turn's diff, or fork a new chat from any message.</p>
              </div>
              <div className="w-glass-flat fe-card">
                <div className="top">
                  <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="12" /><line x1="12" y1="16" x2="12.01" y2="16" /></svg></div>
                  <h3 className="w-h4">My tasks</h3>
                  <span className="w-tag" style={{ marginLeft: "auto" }}>New in 0.2</span>
                </div>
                <p className="w-small">Every approval, question and draft review your agents are waiting on, answered from its card. Answered and Decisions keep each request and its answer.</p>
              </div>
              <div className="w-glass-flat fe-card">
                <div className="top">
                  <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><circle cx="18" cy="18" r="3" /><circle cx="6" cy="6" r="3" /><path d="M13 6h3a2 2 0 0 1 2 2v7" /><line x1="6" y1="9" x2="6" y2="21" /></svg></div>
                  <h3 className="w-h4">Two-click PR reviews</h3>
                  <span className="w-tag" style={{ marginLeft: "auto" }}>New in 0.2</span>
                </div>
                <p className="w-small">Turn on automatic pull request reviews from a setup sheet. Review runs are read-only, and findings land as inline draft reviews on GitHub and GitLab for you to submit or keep.</p>
              </div>
              <div className="w-glass-flat fe-card">
                <div className="top">
                  <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" /><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z" /></svg></div>
                  <h3 className="w-h4">Decision log</h3>
                  <span className="w-tag" style={{ marginLeft: "auto" }}>New in 0.2</span>
                </div>
                <p className="w-small">Your preferences and project decisions are kept in one place and given to every agent, so you say them once.</p>
              </div>
            </div>
          </div>
        </div>
      </section>
      {/* Plan */}
      <section id="plan" className="w-sec" aria-labelledby="g3" style={{ paddingTop: "0" }}>
        <div className="w-wrap w-aside">
          <div className="fe-group">
            <p className="w-eyebrow">03 · Plan and automate</p>
            <h2 id="g3" className="w-h3">Work comes in. Agents pick it up.</h2>
            <p className="w-small">Tickets become cards, cards become sessions, and automations run while you sleep.</p>
          </div>
          <div className="w-g2">
            <div className="w-glass-flat fe-card">
              <div className="top">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="2" /><path d="M8 7v7M12 7v4M16 7v9" /></svg></div>
                <h3 className="w-h4">Board</h3>
              </div>
              <p className="w-small">Plan work as cards linked to tracker tickets, pull requests and worktrees. <b style={{ color: "#ebebeb", fontWeight: "600" }}>Run with…</b> starts a session with any agent, and the card moves as the work does.</p>
            </div>
            <div className="w-glass-flat fe-card">
              <div className="top">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="22 12 16 12 14 15 10 15 8 12 2 12" /><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" /></svg></div>
                <h3 className="w-h4">Inbox</h3>
              </div>
              <p className="w-small">Bring in work items from GitHub, GitLab, Linear, Jira and Azure DevOps: review requests, assigned issues and mentions, in one list.</p>
            </div>
            <div className="w-glass-flat fe-card">
              <div className="top">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" /></svg></div>
                <h3 className="w-h4">Automations</h3>
              </div>
              <p className="w-small">Run an agent on a schedule or when something happens: a pull request comment, a failed check, a review request, a merge or an assigned issue. A run can start a session or wake a Pilot.</p>
            </div>
            <div className="w-glass-flat fe-card">
              <div className="top">
                <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><line x1="18" y1="20" x2="18" y2="10" /><line x1="12" y1="20" x2="12" y2="4" /><line x1="6" y1="20" x2="6" y2="14" /></svg></div>
                <h3 className="w-h4">Runs</h3>
              </div>
              <p className="w-small">Every automation run in one place, with what passed, what failed and why. Start from examples like Find critical bugs, Generate docs or Add test coverage.</p>
            </div>
          </div>
        </div>
      </section>
      {/* Connect */}
      <section id="connect" className="w-sec" aria-labelledby="g4" style={{ paddingTop: "0" }}>
        <div className="w-wrap w-aside">
          <div className="fe-group">
            <p className="w-eyebrow">04 · Connect</p>
            <h2 id="g4" className="w-h3">Tools, browser and Gespan itself.</h2>
            <p className="w-small">Share MCP servers once, let agents browse, and let them drive the app.</p>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
            <div className="w-g2">
              <div className="w-glass-flat fe-card">
                <div className="top">
                  <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M12 22v-5" /><path d="M9 8V2" /><path d="M15 8V2" /><path d="M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8z" /></svg></div>
                  <h3 className="w-h4">MCP in one place</h3>
                </div>
                <p className="w-small">Add an MCP server once and share it with every session. Servers need your approval before they start, keep their secrets in the keychain, and can be installed from the MCP Registry.</p>
              </div>
              <div className="w-glass-flat fe-card">
                <div className="top">
                  <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10" /><line x1="2" y1="12" x2="22" y2="12" /><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" /></svg></div>
                  <h3 className="w-h4">In-app browser</h3>
                </div>
                <p className="w-small">Agents can open, read and click through web pages in a browser that Gespan runs. You watch and use the same tab in a pane beside the chat.</p>
              </div>
            </div>
            <div className="w-glass fe-card w-split" style={{ padding: "24px", gap: "24px" }}>
              <div style={{ display: "flex", flexDirection: "column", gap: "10px" }}>
                <div className="top">
                  <div className="w-tile"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M18 3a3 3 0 0 0-3 3v12a3 3 0 0 0 3 3 3 3 0 0 0 3-3 3 3 0 0 0-3-3H6a3 3 0 0 0-3 3 3 3 0 0 0 3 3 3 3 0 0 0 3-3V6a3 3 0 0 0-3-3 3 3 0 0 0-3 3 3 3 0 0 0 3 3h12a3 3 0 0 0 3-3 3 3 0 0 0-3-3z" /></svg></div>
                  <h3 className="w-h4">Agents that drive Gespan</h3>
                </div>
                <p className="w-small">Start a message with <span className="w-code-inline">/operator</span> to let an agent open sessions, split panes, create worktrees and read your notes.</p>
              </div>
              <pre className="w-code"><span className="p">{"/operator"}</span>{" start two Codex sessions:\n  one to inspect the API\n  and one to review the UI\n"}<span className="c">{"# the agent opens and arranges both for you"}</span></pre>
            </div>
          </div>
        </div>
      </section>
      {/* Beyond */}
      <section id="beyond" className="w-sec" aria-labelledby="g5" style={{ paddingTop: "0" }}>
        <div className="w-wrap w-aside">
          <div className="fe-group">
            <p className="w-eyebrow">05 · Beyond the desktop</p>
            <h2 id="g5" className="w-h3">Keep going when the laptop closes.</h2>
            <p className="w-small">These are newer and still experimental. They work today and are changing fast.</p>
          </div>
          <div className="w-g3">
            <Link className="w-glass-flat fe-card" to="/pilots">
              <div className="top">
                <h3 className="w-h4">Pilots</h3>
                <span className="w-tag" style={{ marginLeft: "auto" }}>Experimental</span>
              </div>
              <p className="w-small">Long-running agents that work across your projects, with a memory, scheduled habits and reports.</p>
              <span className="w-small" style={{ color: "#ebebeb", marginTop: "auto" }}>Meet Pilots →</span>
            </Link>
            <div className="w-glass-flat fe-card">
              <div className="top">
                <h3 className="w-h4">Remote machines</h3>
                <span className="w-tag" style={{ marginLeft: "auto" }}>Experimental</span>
              </div>
              <p className="w-small">Run sessions on a Mac, Linux or Windows host over SSH, where they keep going when your laptop closes.</p>
            </div>
            <div className="w-glass-flat fe-card">
              <div className="top">
                <h3 className="w-h4">iPhone companion</h3>
                <span className="w-tag" style={{ marginLeft: "auto" }}>Invite only</span>
              </div>
              <p className="w-small">Pair by scanning a code. Open sessions, send messages, start new ones, stop turns and read Git changes.</p>
            </div>
          </div>
        </div>
      </section>
      {/* CTA */}
      <section className="w-frame" style={{ paddingBottom: "12px" }}>
        <div className="w-hero soft">
          <div className="w-wrap w-stack-m" style={{ paddingTop: "64px", paddingBottom: "64px", display: "flex", alignItems: "center", gap: "32px" }}>
            <div style={{ flex: "1", display: "flex", flexDirection: "column", gap: "12px" }}>
              <h2 className="w-h2">Try it on a project you have open today.</h2>
              <p className="w-lead">Open a folder, pick an agent, send a message. Split the pane when you want a second one.</p>
            </div>
            <div className="w-stack-m w-full-m" style={{ display: "flex", gap: "10px" }}><Link className="w-btn w-btn-light" to="/download">Download Gespan</Link> <Link className="w-btn w-btn-glass" to="/docs">Read the quick start</Link></div>
          </div>
        </div>
      </section>
      <SiteFooter />
    </div>
  );
}
