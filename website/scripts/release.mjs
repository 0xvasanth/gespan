// Reads the published release and the changelog from the repository, so the
// site's download links and changelog never drift from README.md and
// CHANGELOG.md. vite.config.ts runs this at build time.

/** Download names in the README's release table, matched by file suffix. */
const FILES = {
  macArm: /_aarch64\.dmg$/,
  macIntel: /_x64\.dmg$/,
  windows: /_x64-setup\.exe$/,
  appImage: /\.AppImage$/,
  deb: /\.deb$/,
  rpm: /\.rpm$/,
};

/**
 * The current release from README.md's release block, which set-version
 * rewrites on every release. Throws when the block or a download is missing,
 * so a broken README fails the site build instead of shipping dead links.
 */
export function readRelease(readme, changelog) {
  const block = readme.match(/<!-- release:start -->([\s\S]*?)<!-- release:end -->/)?.[1];
  if (!block) throw new Error("README.md has no release block");
  const version = block.match(/current release is\s*\[\*\*([^*]+)\*\*\]/)?.[1];
  if (!version) throw new Error("README.md release block names no version");
  const urls = [...block.matchAll(/\((https:\/\/github\.com\/[^)\s]+\/releases\/download\/[^)\s]+)\)/g)].map((m) => m[1]);
  const files = {};
  for (const [key, pattern] of Object.entries(FILES)) {
    const url = urls.find((u) => pattern.test(u));
    if (!url) throw new Error(`README.md release block has no ${key} download`);
    if (!url.includes(`/v${version}/`)) throw new Error(`${key} download is not for v${version}: ${url}`);
    files[key] = url;
  }
  const entry = parseChangelog(changelog).find((release) => release.version === version);
  return { version, date: entry?.date ?? null, files };
}

/**
 * Releases in CHANGELOG.md, newest first: version, date and each section's
 * entries. "Unreleased" is skipped. An entry keeps its bold title apart from
 * the rest of its text; continuation lines are joined.
 */
export function parseChangelog(text) {
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
  return releases;
}
