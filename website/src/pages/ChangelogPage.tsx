import { Inline } from "../components/Inline";
import { ScrollShot } from "../components/ScrollShot";
import { SiteFooter } from "../components/SiteFooter";
import { NavSpace } from "../components/SiteNav";
import { REPO } from "../docs";
import { changelog, formatDate, highlights, release } from "../release";
import type { ChangelogRelease } from "../../scripts/release.mjs";
import "./changelog.css";

/** How many entries a release shows before "and N more". */
const SHOWN = 6;
const KIND_CLASS: Record<string, string> = { Added: "add", Changed: "chg", Fixed: "fix" };

const capitalize = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);

function Release({ entry, latest }: { entry: ChangelogRelease; latest: boolean }) {
  const items = entry.sections.flatMap((section) => section.entries.map((e) => ({ kind: section.kind, ...e })));
  const hidden = items.length - SHOWN;
  const id = `r${entry.version.replace(/\W/g, "")}`;
  return (
    <article className="cl-rel" aria-labelledby={id}>
      <div className="cl-side">
        {latest && <span className="w-tag ok" style={{ alignSelf: "flex-start" }}>Latest</span>}
        <h2 id={id} className="cl-ver">{entry.version}</h2>
        <p className="w-meta">{formatDate(entry.date)}</p>
        {entry.includes?.length ? <p className="w-meta">Also ships {entry.includes.join(", ")}, which had no release of its own.</p> : null}
      </div>
      <div className="cl-body">
        {latest && (
          <>
            <h3 className="w-h2" style={{ fontSize: "36px" }}>{highlights(entry)}.</h3>
            <ScrollShot name="tasks" className="cl-shot" />
          </>
        )}
        <ul className="cl-list">
          {items.slice(0, SHOWN).map((item, i) => (
            <li key={i}>
              <span className={`cl-kind ${KIND_CLASS[item.kind] ?? ""}`}>{item.kind}</span>
              <span>
                {item.title && <b>{item.title}. </b>}
                <Inline text={capitalize(item.body)} />
              </span>
            </li>
          ))}
        </ul>
        <a className="w-link w-small" href={`${REPO}/releases/tag/v${entry.version}`} style={{ color: "#ebebeb", alignSelf: "flex-start" }}>
          {hidden > 0 ? `Full release notes, ${hidden} more` : "Full release notes"}
        </a>
      </div>
    </article>
  );
}

export function ChangelogPage() {
  return (
    <div className="w-site">
      <div className="w-frame">
        <section className="w-hero soft" aria-label="Changelog">
          <NavSpace />
          <div className="w-wrap w-stack-m" style={{ paddingTop: "48px", paddingBottom: "56px", display: "flex", alignItems: "flex-end", gap: "32px" }}>
            <div data-reveal="" className="w-head" style={{ flex: "1" }}>
              <p className="w-eyebrow">Changelog</p>
              <h1 className="w-h1">What's new in Gespan.</h1>
              <p className="w-lead">Every release, newest first. Gespan ships often while it is early, so the list moves fast.</p>
            </div>
            <div className="w-stack-m w-full-m" style={{ display: "flex", gap: "10px" }}>
              <a className="w-btn w-btn-glass" href={`${REPO}/releases`}>Watch releases on GitHub</a>
            </div>
          </div>
        </section>
      </div>
      <section className="w-sec-tight" aria-label="Releases">
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column" }}>
          {changelog.map((entry) => (
            <Release key={entry.version} entry={entry} latest={entry.version === release.version} />
          ))}
          <div style={{ display: "flex", justifyContent: "center", paddingTop: "24px" }}>
            <a className="w-btn w-btn-glass" href={`${REPO}/blob/main/CHANGELOG.md`}>Older releases</a>
          </div>
        </div>
      </section>
      <SiteFooter />
    </div>
  );
}
