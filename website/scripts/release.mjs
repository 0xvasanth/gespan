// The site's release data. scripts/sync-release.mjs turns GitHub's latest
// published release into src/release.json, and vite.config.ts reads that file
// and CHANGELOG.md at build time. README.md is not a source: the release
// pipeline bumps it before the release is published, while its downloads
// still 404.

/** The desktop downloads the site links to, matched by asset name. */
const FILES = {
  macArm: /^Gespan_[\d.]+_aarch64\.dmg$/,
  macIntel: /^Gespan_[\d.]+_x64\.dmg$/,
  windows: /^Gespan_[\d.]+_x64-setup\.exe$/,
  appImage: /^Gespan_[\d.]+_amd64\.AppImage$/,
  deb: /^gespan_[\d.]+_amd64\.deb$/,
  rpm: /^gespan-[\d.]+-\d+\.[\w.]+\.rpm$/,
};

/**
 * The site's release record from a GitHub release (the REST API's release
 * object). Throws for a draft or pre-release, and when any download is
 * missing or isn't for this version, so the site never links a dead file.
 */
export function releaseFromGitHub(release) {
  if (!release || typeof release.tag_name !== "string") throw new Error("not a GitHub release");
  if (release.draft) throw new Error(`${release.tag_name} is still a draft; it has no public downloads yet`);
  if (release.prerelease) throw new Error(`${release.tag_name} is a pre-release`);
  const version = release.tag_name.replace(/^v/, "");
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`unexpected tag ${release.tag_name}`);
  const assets = release.assets ?? [];
  const files = {};
  for (const [key, pattern] of Object.entries(FILES)) {
    const asset = assets.find((a) => pattern.test(a.name));
    if (!asset) throw new Error(`${release.tag_name} has no ${key} download`);
    if (!asset.name.includes(version)) throw new Error(`${key} download ${asset.name} is not for ${version}`);
    files[key] = asset.browser_download_url;
  }
  return {
    version,
    tag: release.tag_name,
    date: (release.published_at ?? "").slice(0, 10) || null,
    url: release.html_url,
    files,
  };
}

/** Compares two x.y.z versions: negative, zero or positive. */
export function compareVersions(a, b) {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pa[i] ?? 0) - (pb[i] ?? 0);
  return 0;
}

/**
 * Releases in CHANGELOG.md, newest first: version, date and each section's
 * entries. "Unreleased" is skipped, and so is any version newer than
 * `published` when it's given, since its downloads aren't out yet.
 */
export function parseChangelog(text, published) {
  const releases = [];
  let release = null;
  let section = null;
  let entry = null;
  for (const line of text.split(/\r?\n/)) {
    const heading = line.match(/^## \[([^\]]+)\](?: - (\d{4}-\d{2}-\d{2}))?/);
    if (heading) {
      release = heading[1] === "Unreleased" ? null : { version: heading[1], date: heading[2] ?? null, sections: [] };
      if (release) releases.push(release);
      section = entry = null;
      continue;
    }
    if (!release) continue;
    const kind = line.match(/^### (.+)/);
    if (kind) {
      section = { kind: kind[1].trim(), entries: [] };
      release.sections.push(section);
      entry = null;
      continue;
    }
    if (!section) continue;
    const bullet = line.match(/^- (.*)/);
    if (bullet) {
      entry = { text: bullet[1].trim() };
      section.entries.push(entry);
    } else if (entry && /^\s+\S/.test(line)) {
      entry.text += ` ${line.trim()}`;
    } else if (!line.trim()) {
      entry = null;
    }
  }
  for (const r of releases)
    for (const s of r.sections)
      for (const e of s.entries) {
        const titled = e.text.match(/^\*\*(.+?):?\*\*:?\s*(.*)$/);
        e.title = titled ? titled[1].replace(/:$/, "") : null;
        e.body = titled ? titled[2] : e.text;
        delete e.text;
      }
  if (!published) return releases;
  return releases.filter((r) => !/^\d+\.\d+\.\d+$/.test(r.version) || compareVersions(r.version, published) <= 0);
}

/** The published x.y.z versions in a list of GitHub releases, newest first. Drafts, pre-releases and other tags are left out. */
export function publishedVersions(releases) {
  return releases
    .filter((r) => r && !r.draft && !r.prerelease && typeof r.tag_name === "string")
    .map((r) => r.tag_name.replace(/^v/, ""))
    .filter((v) => /^\d+\.\d+\.\d+$/.test(v))
    .sort((a, b) => compareVersions(b, a));
}

/** Section order when releases are folded together; other kinds follow in the order they appear. */
const KIND_ORDER = ["Added", "Changed", "Fixed", "Removed"];

/**
 * Folds each CHANGELOG version that never got a GitHub release of its own
 * into the next published version, which shipped it (0.3.1 went out inside
 * 0.3.2). The published version keeps its own number and date, lists what it
 * took in under `includes`, and merges the sections by kind, its own entries
 * first. With no `published` list, or for versions older than the oldest
 * published one, the changelog is returned as it is.
 */
export function foldUnpublished(releases, published) {
  if (!published?.length) return releases;
  const shipped = new Set(published);
  const oldest = published.reduce((a, b) => (compareVersions(a, b) <= 0 ? a : b));
  const out = [];
  let pending = [];
  // Oldest first, so each unpublished version waits for the release that shipped it.
  for (const release of [...releases].reverse()) {
    const numbered = /^\d+\.\d+\.\d+$/.test(release.version);
    if (numbered && !shipped.has(release.version) && compareVersions(release.version, oldest) > 0) {
      pending.push(release);
      continue;
    }
    if (!pending.length) {
      out.push(release);
      continue;
    }
    const parts = [release, ...pending.reverse()];
    const kinds = [...new Set(parts.flatMap((p) => p.sections.map((s) => s.kind)))].sort((a, b) => {
      const ia = KIND_ORDER.indexOf(a);
      const ib = KIND_ORDER.indexOf(b);
      return (ia < 0 ? KIND_ORDER.length : ia) - (ib < 0 ? KIND_ORDER.length : ib);
    });
    out.push({
      ...release,
      includes: pending.map((p) => p.version),
      sections: kinds.map((kind) => ({ kind, entries: parts.flatMap((p) => p.sections.filter((s) => s.kind === kind).flatMap((s) => s.entries)) })),
    });
    pending = [];
  }
  // Unpublished versions with no newer release yet stay as they are.
  return [...out, ...pending].reverse();
}
