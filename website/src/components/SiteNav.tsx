import { useState } from "react";
import logoUrl from "../assets/logo.svg";
import { Link, type Page } from "../router";
import { GitHubIcon } from "./icons";
import "./site-nav.css";

const LINKS: { page: Page; to: string; label: string }[] = [
  { page: "features", to: "/features", label: "Features" },
  { page: "pilots", to: "/pilots", label: "Pilots" },
  { page: "docs", to: "/docs", label: "Docs" },
  { page: "changelog", to: "/changelog", label: "Changelog" },
  { page: "download", to: "/download", label: "Download" },
];

/** The glass nav pill. `onHero` drops its own ground so the hero mesh shows through. */
export function SiteNav({ active, onHero = false }: { active: Page; onHero?: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="w-site" style={{ background: onHero ? "transparent" : "#111111", overflow: "visible", padding: "18px 24px" }}>
      <header className="w-glass nv-bar">
        <Link to="/" className="nv-brand" aria-label="Gespan home">
          <img src={logoUrl} width="28" height="28" alt="" />
          <span>Gespan</span>
        </Link>
        <nav aria-label="Site" className="w-hide-m nv-links">
          {LINKS.map((link) => (
            <Link key={link.page} className={link.page === active ? "nv-link on" : "nv-link"} to={link.to} aria-current={link.page === active ? "page" : undefined}>
              {link.label}
            </Link>
          ))}
        </nav>
        <div className="nv-actions">
          <a className="w-btn w-btn-glass sm w-hide-m" href="https://github.com/0xvasanth/gespan" style={{ borderRadius: "999px" }}>
            <GitHubIcon size={15} />
            Star on GitHub
          </a>
          <Link className="w-btn w-btn-light sm" to="/download" style={{ borderRadius: "999px" }}>Download</Link>
          <button type="button" className="nv-menu w-only-m" aria-label={open ? "Close menu" : "Open menu"} aria-expanded={open} aria-controls="nv-sheet" onClick={() => setOpen(!open)}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
              {open ? (
                <>
                  <line x1="6" y1="6" x2="18" y2="18" />
                  <line x1="18" y1="6" x2="6" y2="18" />
                </>
              ) : (
                <>
                  <line x1="4" y1="8" x2="20" y2="8" />
                  <line x1="4" y1="16" x2="20" y2="16" />
                </>
              )}
            </svg>
          </button>
        </div>
      </header>
      {open && (
        <nav id="nv-sheet" aria-label="Site" className="w-glass nv-sheet w-only-m">
          {LINKS.map((link) => (
            <Link key={link.page} className={link.page === active ? "nv-sheet-link on" : "nv-sheet-link"} to={link.to} aria-current={link.page === active ? "page" : undefined} onClick={() => setOpen(false)}>
              {link.label}
            </Link>
          ))}
          <a className="nv-sheet-link" href="https://github.com/0xvasanth/gespan">GitHub</a>
        </nav>
      )}
    </div>
  );
}
