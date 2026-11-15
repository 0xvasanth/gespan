import logoUrl from "../assets/logo.svg";
import { GUIDES, REPO } from "../docs";
import { Link } from "../router";
import "./site-footer.css";

export function SiteFooter() {
  return (
    <footer className="w-site" style={{ borderTop: "1px solid rgba(235,235,235,.07)" }}>
      <div className="w-wrap" style={{ paddingTop: "64px", paddingBottom: "40px", display: "flex", flexDirection: "column", gap: "48px" }}>
        <div className="ft-grid">
          <div className="ft-brand" style={{ display: "flex", flexDirection: "column", gap: "14px", maxWidth: "320px" }}>
            <Link to="/" style={{ display: "flex", alignItems: "center", gap: "10px", textDecoration: "none" }}>
              <img src={logoUrl} width="30" height="30" alt="" />
              <span style={{ fontFamily: "var(--display)", fontSize: "20px", fontWeight: "600", letterSpacing: "-0.02em", color: "#ebebeb" }}>Gespan</span>
            </Link>
            <p className="w-small"><i>Gespan</i> (guh-SPAHN) is Dutch for a team of horses harnessed together to pull one wagon. You hold the reins, and a team of agents does the pulling.</p>
          </div>
          <nav className="ft-col" aria-label="Product">
            <h3>Product</h3>
            <Link to="/features">Features</Link>
            <Link to="/pilots">Pilots</Link>
            <Link to="/download">Download</Link>
            <Link to="/changelog">Changelog</Link>
          </nav>
          <nav className="ft-col" aria-label="Docs">
            <h3>Docs</h3>
            <Link to="/docs">Quick start</Link>
            <a href={GUIDES.providers}>Provider setup</a>
            <a href={GUIDES.board}>Board</a>
            <a href={GUIDES.remote}>Remote sessions</a>
          </nav>
          <nav className="ft-col" aria-label="Project">
            <h3>Project</h3>
            <a href={REPO}>GitHub</a>
            <a href={`${REPO}/releases`}>Releases</a>
            <a href={`${REPO}/issues`}>Report an issue</a>
            <a href={`${REPO}/blob/main/CONTRIBUTING.md`}>Contributing</a>
          </nav>
          <nav className="ft-col" aria-label="Trust">
            <h3>Trust</h3>
            <a href={GUIDES.privacy}>Privacy</a>
            <a href={`${REPO}/blob/main/SECURITY.md`}>Security</a>
            <a href={`${REPO}/blob/main/LICENSE`}>MIT License</a>
            <a href={`${REPO}/blob/main/CODE_OF_CONDUCT.md`}>Code of Conduct</a>
          </nav>
        </div>
        <hr className="w-rule" />
        <div className="w-stack-m" style={{ display: "flex", alignItems: "center", gap: "16px", flexWrap: "wrap" }}>
          <p className="w-meta">Gespan is open source under the MIT License. Provider names and logos are trademarks of their respective owners.</p>
          <p className="w-meta" style={{ marginLeft: "auto" }}>Built with Rust and GPUI · No telemetry</p>
        </div>
      </div>
    </footer>
  );
}
