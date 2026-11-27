import { useEffect, useRef, useState } from "react";
import { SiteFooter } from "../components/SiteFooter";
import { NavSpace } from "../components/SiteNav";
import { GUIDES, REPO } from "../docs";
import { QUICK_START, Terminal } from "../components/Terminal";
import "./docs.css";

/** The guides in README.md's Documentation table. */
export const DOC_GUIDES = [
  { title: "Board", href: GUIDES.board, summary: "Cards, columns and tracker tickets, and starting sessions from them." },
  { title: "Project scripts", href: GUIDES.actions, summary: "Test, build and dev-server commands for the footer, the palette and agents." },
  { title: "Agent access", href: GUIDES.agentAccess, summary: "Sessions that see each other, start sessions and message each other." },
  { title: "Remote sessions", href: GUIDES.remote, summary: "Run agents on another machine over SSH.", experimental: true },
  { title: "Jira inbox", href: GUIDES.jira, summary: "Connect Jira Cloud to the Inbox." },
  { title: "Linux installation", href: GUIDES.install, summary: "System dependencies and the .deb, .rpm and AppImage packages." },
  { title: "Privacy", href: GUIDES.privacy, summary: "What is recorded locally and what never leaves your machine." },
  { title: "Building from source", href: GUIDES.building, summary: "Platform prerequisites and packaging." },
  { title: "Desktop guide", href: GUIDES.desktop, summary: "The Rust and GPUI app: building, checks and current limitations." },
];

/** The search shortcut's keys as this visitor's keyboard labels them: ⌘ on Apple devices, Ctrl elsewhere. */
export function shortcutKeys(userAgent: string): [string, string] {
  return /Mac|iPhone|iPad|iPod/.test(userAgent) ? ["⌘", "K"] : ["Ctrl", "K"];
}

/** Guides whose title or summary has every word of the query, in any order and case. */
export function filterGuides(query: string) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return DOC_GUIDES.filter((guide) => {
    const text = `${guide.title} ${guide.summary}`.toLowerCase();
    return words.every((word) => text.includes(word));
  });
}

export function DocsPage() {
  const [query, setQuery] = useState("");
  const search = useRef<HTMLInputElement>(null);
  const shown = filterGuides(query);
  useEffect(() => {
    const focus = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        search.current?.focus();
      }
    };
    window.addEventListener("keydown", focus);
    return () => window.removeEventListener("keydown", focus);
  }, []);
  return (
    <div className="w-site">
      <div className="w-frame">
        <section className="w-hero soft" aria-label="Docs">
          <NavSpace />
          <div className="w-wrap" style={{ paddingTop: "48px", paddingBottom: "64px", display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", gap: "20px" }}>
            <p className="w-eyebrow">Docs</p>
            <h1 className="w-h1">Learn Gespan.</h1>
            <p className="w-lead" style={{ maxWidth: "560px" }}>Set up your agents, run your first session and get the most out of the board, automations and Pilots.</p>
            <label className="dc-search" style={{ marginTop: "8px" }}>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true"><circle cx="11" cy="11" r="8" /><line x1="21" y1="21" x2="16.65" y2="16.65" /></svg>
              <input ref={search} type="search" placeholder="Search the guides" aria-label="Search the guides" aria-keyshortcuts="Meta+K Control+K" aria-controls="gd-list" value={query} onChange={(event) => setQuery(event.target.value)} />
              {/* The shortcut hint steps aside once there's a query beside it. */}
              {!query && (
                <span className="dc-keys w-hide-m" aria-hidden="true">
                  {shortcutKeys(navigator.userAgent).map((key) => <kbd key={key}>{key}</kbd>)}
                </span>
              )}
            </label>
          </div>
        </section>
      </div>
      <section className="w-sec-tight" aria-label="Start here">
        <div className="w-wrap w-split rev" style={{ alignItems: "start" }}>
          <div className="w-glass" style={{ padding: "28px", display: "flex", flexDirection: "column", gap: "20px" }}>
            <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
              <p className="w-eyebrow">Quick start</p>
              <span className="w-meta" style={{ marginLeft: "auto" }}>Set up, then 5 steps</span>
            </div>
            <Terminal title="Before you open Gespan" lines={QUICK_START} />
            <h3 className="dc-then">Then, in Gespan</h3>
            <ol className="dc-qs">
              <li>
                <span className="n">1</span>
                <div>
                  <h3 className="w-h4">Open a project folder</h3>
                  <p className="w-small">Gespan groups your sessions by project.</p>
                </div>
              </li>
              <li>
                <span className="n">2</span>
                <div>
                  <h3 className="w-h4">Pick an agent and a model</h3>
                  <p className="w-small">Any installed agent appears in the picker.</p>
                </div>
              </li>
              <li>
                <span className="n">3</span>
                <div>
                  <h3 className="w-h4">Send a message</h3>
                  <p className="w-small">The session opens as a tab, and replies stream in.</p>
                </div>
              </li>
              <li>
                <span className="n">4</span>
                <div>
                  <h3 className="w-h4">Go parallel</h3>
                  <p className="w-small">Split the pane, start a second agent, and put it in its own worktree so the two never clash.</p>
                </div>
              </li>
              <li>
                <span className="n">5</span>
                <div>
                  <h3 className="w-h4">Review</h3>
                  <p className="w-small">Look through the diff, then keep or undo each turn's changes.</p>
                </div>
              </li>
            </ol>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
            <a className="w-glass-flat dc-guide" href={GUIDES.providers} style={{ borderColor: "rgba(232,197,71,.24)" }}>
              <div className="hd"><span className="w-tag wait">Start here</span><span className="arr">→</span></div>
              <h3 className="w-h4">Provider setup</h3>
              <p className="w-small">Install and log in to each supported agent. One is enough to get started.</p>
            </a>
            <div className="w-glass-flat dc-guide">
              <div className="hd">
                <p className="w-eyebrow">Tip</p>
              </div>
              <p className="w-small">Try <span className="w-code-inline">start two sessions in new worktrees: one to inspect the API and one to review the UI</span>. The agent starts both under this chat, and each reports back when it’s done.</p>
            </div>
          </div>
        </div>
      </section>
      <section className="w-sec-tight" aria-labelledby="gd-h" style={{ paddingTop: "0" }}>
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "24px" }}>
          <h2 id="gd-h" className="w-h3">Guides</h2>
          <div id="gd-list" data-stagger="" className="w-g3">
            {shown.map((guide) => (
              <a key={guide.href} className="w-glass-flat dc-guide" href={guide.href}>
                <div className="hd">
                  <h3 className="w-h4">{guide.title}</h3>
                  {guide.experimental && <span className="w-tag">Experimental</span>}
                  <span className="arr" aria-hidden="true">→</span>
                </div>
                <p className="w-small">{guide.summary}</p>
              </a>
            ))}
          </div>
          {shown.length === 0 && (
            <output className="w-small" style={{ display: "block" }}>
              No guide matches “{query}”. Try another word, or <a className="w-link" href={`${REPO}/tree/main/docs`}>browse every doc on GitHub</a>.
            </output>
          )}
        </div>
      </section>
      <section className="w-sec-tight" aria-label="Get help" style={{ paddingTop: "0" }}>
        <div className="w-wrap">
          <div className="w-hero soft w-stack-m" style={{ padding: "32px 36px", display: "flex", alignItems: "center", gap: "24px" }}>
            <div style={{ flex: "1", display: "flex", flexDirection: "column", gap: "8px" }}>
              <h2 className="w-h3">Something broke?</h2>
              <p className="w-small">Gespan is still early. Open an issue with what you saw, and the logs from Settings → Diagnostics if you can.</p>
            </div>
            <div className="w-stack-m w-full-m" style={{ display: "flex", gap: "10px" }}><a className="w-btn w-btn-light" href="https://github.com/0xvasanth/gespan/issues">Open an issue</a> <a className="w-btn w-btn-glass" href="https://github.com/0xvasanth/gespan/blob/main/CONTRIBUTING.md">Contribute</a></div>
          </div>
        </div>
      </section>
      <SiteFooter />
    </div>
  );
}
