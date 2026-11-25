import { SiteFooter } from "../components/SiteFooter";
import { NavSpace } from "../components/SiteNav";
import { Link } from "../router";

export function NotFoundPage() {
  return (
    <div className="w-site">
      <div className="w-frame">
        <section className="w-hero soft" aria-labelledby="nf-h">
          <NavSpace />
          <div className="w-wrap" style={{ paddingTop: "72px", paddingBottom: "96px", display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", gap: "20px" }}>
            <p className="w-eyebrow">404</p>
            <h1 id="nf-h" className="w-h1">This page isn't here.</h1>
            <p className="w-lead">The link may be old, or the page may have moved.</p>
            <Link className="w-btn w-btn-light" to="/">Back to Gespan</Link>
          </div>
        </section>
      </div>
      <SiteFooter />
    </div>
  );
}
