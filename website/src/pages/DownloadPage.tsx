import { SiteNav } from "../components/SiteNav";
import { SiteFooter } from "../components/SiteFooter";
import logoUrl from "../assets/logo.svg";
import { GUIDES } from "../docs";
import { formatDate, release } from "../release";
import { AppleIcon } from "../components/icons";
import { recommendDownload } from "../platform";
import { Beam } from "../components/fx";
import "./download.css";

const EXT: Record<string, string> = { mac: ".dmg", windows: ".exe", linux: ".AppImage" };

/** The download for the visitor's own OS; on a phone, a note that Gespan is a desktop app. */
function Recommended() {
  const pick = recommendDownload(navigator.userAgent, release.files, navigator.maxTouchPoints);
  if (pick.os === "phone" || pick.os === "unknown")
    return (
      <p className="w-glass w-small" style={{ marginTop: "12px", padding: "14px 20px", borderRadius: "18px", maxWidth: "520px", color: "#d6d6d6" }}>
        {pick.os === "phone" ? "Gespan is a desktop app for macOS, Windows and Linux. Open this page on your computer, or pick a download below." : "Pick the download for your computer below."}
      </p>
    );
  return (
    <div className="w-glass w-stack-m w-full-m" style={{ marginTop: "12px", padding: "10px 10px 10px 20px", borderRadius: "18px", display: "flex", alignItems: "center", gap: "16px", textAlign: "left", position: "relative" }}>
      <Beam duration={6} size={120} />
      <div style={{ display: "flex", flexDirection: "column" }}>
        <span style={{ fontSize: "14px", fontWeight: "600" }}>{pick.label}</span>
        <span className="w-meta">{pick.detail}</span>
        {pick.os === "mac" && <a className="w-link w-meta" href={pick.alternate.href} style={{ marginTop: "2px" }}>{pick.alternate.label}</a>}
      </div>
      <a className="w-btn w-btn-light" href={pick.href}>
        {pick.os === "mac" && <AppleIcon />}
        Download {EXT[pick.os]}
      </a>
    </div>
  );
}

export function DownloadPage() {
  return (
    <div className="w-site">
      <div className="w-frame">
        <section className="w-hero soft" aria-label="Download">
          <SiteNav active="download" onHero />
          <div className="w-wrap" style={{ paddingTop: "56px", paddingBottom: "64px", display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", gap: "20px" }}>
            <img src={logoUrl} width="72" height="72" alt="" />
            <h1 className="w-h1">Download Gespan.</h1>
            <p className="w-lead">Version {release.version} · Released {formatDate(release.date)} · Free and open source</p>
            <Recommended />
          </div>
        </section>
      </div>
      {/* Before you start */}
      <section className="w-sec-tight" aria-label="Before you start" style={{ paddingBottom: "0" }}>
        <div className="w-wrap">
          <div className="w-glass-flat w-stack-m" style={{ padding: "18px 22px", display: "flex", alignItems: "center", gap: "16px", borderColor: "rgba(232,197,71,.24)", background: "rgba(232,197,71,.05)" }}>
            <span className="w-tag wait">Before you start</span>
            <p className="w-small" style={{ color: "#d6d6d6" }}>Install and sign in to at least one supported agent, such as Claude Code, Codex or Cursor CLI. Gespan checks for each CLI at startup and points you to the install steps for any that are missing.</p>
            <a className="w-btn w-btn-glass sm" href={GUIDES.providers} style={{ marginLeft: "auto" }}>Provider setup</a>
          </div>
        </div>
      </section>
      {/* Platforms */}
      <section className="w-sec-tight" aria-labelledby="pl-h">
        <div className="w-wrap" style={{ display: "flex", flexDirection: "column", gap: "24px" }}>
          <h2 id="pl-h" className="w-h3">Every platform</h2>
          <div data-stagger="" className="w-g3">
            <div className="w-glass dl-card">
              <div className="hd">
                <div className="w-tile" style={{ color: "#ebebeb" }}><svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M16.4 12.6c0-2.3 1.9-3.4 2-3.5-1.1-1.6-2.8-1.8-3.4-1.8-1.4-.1-2.8.9-3.5.9-.7 0-1.8-.9-3-.8-1.5 0-3 .9-3.8 2.3-1.6 2.8-.4 7 1.2 9.3.8 1.1 1.7 2.4 2.9 2.3 1.2 0 1.6-.7 3-.7s1.8.7 3 .7c1.3 0 2.1-1.1 2.8-2.3.9-1.3 1.3-2.6 1.3-2.6s-2.5-1-2.5-3.8zM14.1 5.8c.6-.8 1.1-1.9 1-3-.9 0-2.1.6-2.7 1.4-.6.7-1.1 1.8-1 2.9 1 .1 2.1-.5 2.7-1.3z" /></svg></div>
                <h3 className="w-h4">macOS</h3>
                <span className="w-tag ok" style={{ marginLeft: "auto" }}>Notarized</span>
              </div>
              <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}><a className="dl-file" href={release.files.macArm}>Apple Silicon<span className="ext">.dmg</span></a> <a className="dl-file" href={release.files.macIntel}>Intel<span className="ext">.dmg</span></a></div>
              <ol className="dl-steps">
                <li>Open the DMG.</li>
                <li>Drag Gespan to Applications.</li>
              </ol>
            </div>
            <div className="w-glass dl-card">
              <div className="hd">
                <div className="w-tile" style={{ color: "#ebebeb" }}><svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><rect x="3" y="3" width="8.5" height="8.5" /><rect x="12.5" y="3" width="8.5" height="8.5" /><rect x="3" y="12.5" width="8.5" height="8.5" /><rect x="12.5" y="12.5" width="8.5" height="8.5" /></svg></div>
                <h3 className="w-h4">Windows</h3>
                <span className="w-tag mute" style={{ marginLeft: "auto" }}>Less tested</span>
              </div>
              <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}><a className="dl-file" href={release.files.windows}>x64 installer<span className="ext">.exe</span></a></div>
              <ol className="dl-steps">
                <li>Run the installer.</li>
                <li>It isn't code-signed yet. In the SmartScreen prompt, choose <b style={{ color: "#ebebeb" }}>More info</b>, then <b style={{ color: "#ebebeb" }}>Run anyway</b>.</li>
              </ol>
            </div>
            <div className="w-glass dl-card">
              <div className="hd">
                <div className="w-tile" style={{ color: "#ebebeb" }}><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="4 17 10 11 4 5" /><line x1="12" y1="19" x2="20" y2="19" /></svg></div>
                <h3 className="w-h4">Linux</h3>
                <span className="w-tag mute" style={{ marginLeft: "auto" }}>Less tested</span>
              </div>
              <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}><a className="dl-file" href={release.files.appImage}>x64 AppImage<span className="ext">.AppImage</span></a> <a className="dl-file" href={release.files.deb}>Debian, Ubuntu<span className="ext">.deb</span></a> <a className="dl-file" href={release.files.rpm}>Fedora, RHEL<span className="ext">.rpm</span></a></div>
              <ol className="dl-steps">
                <li>Make the AppImage executable with <span className="w-code-inline">chmod +x</span> and run it.</li>
                <li>See <a className="w-link" href={GUIDES.install}>Linux installation</a> for dependencies.</li>
              </ol>
            </div>
          </div>
          <p className="w-meta">Gespan on Windows and Linux is less tested than on macOS. Every release is listed on the <a className="w-link" href="https://github.com/0xvasanth/gespan/releases">Releases page</a>.</p>
        </div>
      </section>
      {/* Mobile + source */}
      <section className="w-sec-tight" aria-label="More ways to run Gespan" style={{ paddingTop: "0" }}>
        <div className="w-wrap w-g2">
          <div className="w-hero soft dl-card" style={{ padding: "32px" }}>
            <div className="hd">
              <p className="w-eyebrow">Mobile companion</p>
              <span className="w-tag" style={{ marginLeft: "auto" }}>Invite only</span>
            </div>
            <h2 className="w-h3">Gespan for iPhone</h2>
            <p className="w-small">Pair with your desktop by scanning a code in <b style={{ color: "#ebebeb" }}>Settings → Mobile</b>, then choose the projects and Pilots to include. Codes expire after 15 minutes and work once. Your desktop must stay awake and running.</p>
            <div className="w-stack-m" style={{ display: "flex", gap: "10px", marginTop: "auto" }}><a className="w-btn w-btn-glass" href="https://github.com/0xvasanth/gespan/issues">Request an invite</a> <a className="w-btn w-btn-ghost" href={GUIDES.mobile}>How pairing works</a></div>
          </div>
          <div className="w-glass-flat dl-card" style={{ padding: "32px" }}>
            <p className="w-eyebrow">Build from source</p>
            <h2 className="w-h3">Rust, GPUI and Bun</h2>
            <p className="w-small">You'll need Node.js 22+, Bun, a stable Rust toolchain and, on macOS, Xcode.</p>
            <pre className="w-code">{"git clone https://github.com/0xvasanth/gespan.git\ncd gespan\nbun run dev     "}<span className="c">{"# separate development database"}</span>{"\nbun run build   "}<span className="c">{"# optimized app on your Gespan data"}</span></pre>
            <a className="w-link w-small" href={GUIDES.building} style={{ color: "#ebebeb" }}>Build setup for each platform</a>
          </div>
        </div>
      </section>
      <SiteFooter />
    </div>
  );
}
