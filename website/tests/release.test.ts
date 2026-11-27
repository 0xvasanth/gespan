import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, test } from "vitest";
import { compareVersions, foldUnpublished, parseChangelog, publishedVersions, releaseFromGitHub, type GitHubRelease } from "../scripts/release.mjs";
import { fetchRelease, writeRelease } from "../scripts/sync-release.mjs";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const CHANGELOG = readFileSync(join(site, "../CHANGELOG.md"), "utf8");

// The asset list GitHub serves for a published release (v0.2.0's, trimmed of
// the host and updater files only where noted).
const asset = (v: string, name: string) => ({ name, browser_download_url: `https://github.com/0xvasanth/gespan/releases/download/v${v}/${name}` });
const published = (v: string, overrides: Partial<GitHubRelease> = {}): GitHubRelease => ({
  tag_name: `v${v}`,
  draft: false,
  prerelease: false,
  published_at: "2027-02-03T10:00:00Z",
  html_url: `https://github.com/0xvasanth/gespan/releases/tag/v${v}`,
  assets: [
    `gespan-${v}-1.el10.x86_64.rpm`,
    `gespan-${v}-1.el10.x86_64.rpm.sig`,
    "gespan-host-darwin-arm64.tar.gz",
    `Gespan_${v}_aarch64.app.tar.gz`,
    `Gespan_${v}_aarch64.dmg`,
    `Gespan_${v}_amd64.AppImage`,
    `Gespan_${v}_amd64.AppImage.sig`,
    `gespan_${v}_amd64.deb`,
    `Gespan_${v}_android.apk`,
    `Gespan_${v}_x64-setup.exe`,
    `Gespan_${v}_x64-setup.exe.sig`,
    `Gespan_${v}_x64.dmg`,
    "native-latest.json",
  ].map((name) => asset(v, name)),
  ...overrides,
});

describe("releaseFromGitHub", () => {
  test("picks the six desktop downloads, not signatures, updater bundles or the host", () => {
    const release = releaseFromGitHub(published("1.4.0"));
    expect(release).toMatchObject({ version: "1.4.0", tag: "v1.4.0", date: "2027-02-03", url: "https://github.com/0xvasanth/gespan/releases/tag/v1.4.0" });
    expect(Object.fromEntries(Object.entries(release.files).map(([k, url]) => [k, url.split("/").pop()]))).toEqual({
      macArm: "Gespan_1.4.0_aarch64.dmg",
      macIntel: "Gespan_1.4.0_x64.dmg",
      windows: "Gespan_1.4.0_x64-setup.exe",
      appImage: "Gespan_1.4.0_amd64.AppImage",
      deb: "gespan_1.4.0_amd64.deb",
      rpm: "gespan-1.4.0-1.el10.x86_64.rpm",
    });
  });

  test("a release still in the pipeline (a draft) is refused: its files aren't public", () => {
    expect(() => releaseFromGitHub(published("1.4.0", { draft: true }))).toThrow(/still a draft/);
    expect(() => releaseFromGitHub(published("1.4.0", { prerelease: true }))).toThrow(/pre-release/);
  });

  test("a missing download fails instead of shipping a dead button", () => {
    const release = published("1.4.0");
    release.assets = release.assets!.filter((a) => !a.name.endsWith(".rpm"));
    expect(() => releaseFromGitHub(release)).toThrow(/no rpm download/);
  });

  test("a download left over from another version is caught", () => {
    const release = published("1.4.0");
    release.assets = release.assets!.map((a) => (a.name.endsWith("_x64.dmg") ? asset("1.4.0", "Gespan_1.3.0_x64.dmg") : a));
    expect(() => releaseFromGitHub(release)).toThrow(/macIntel download Gespan_1.3.0_x64.dmg is not for 1.4.0/);
  });
});

describe("sync-release", () => {
  let dir = "";
  afterEach(() => rmSync(dir, { recursive: true, force: true }));

  const respond = (status: number, body: unknown) => (async () => ({ status, ok: status < 400, json: async () => body })) as unknown as typeof fetch;

  // GitHub's answers: one release for latest or a tag, the list for releases?per_page.
  const github = (list: GitHubRelease[], urls: string[] = []) =>
    (async (url: string) => {
      urls.push(url);
      const body = url.includes("releases?") ? list : published("1.4.0");
      return { status: 200, ok: true, json: async () => body };
    }) as unknown as typeof fetch;

  test("asks GitHub for the latest published release, or a tag, and the release list", async () => {
    const urls: string[] = [];
    const fetchImpl = github([published("1.4.0")], urls);
    await fetchRelease(undefined, { fetchImpl, token: "" });
    await fetchRelease("1.4.0", { fetchImpl, token: "" });
    expect(urls).toEqual([
      "https://api.github.com/repos/0xvasanth/gespan/releases/latest",
      "https://api.github.com/repos/0xvasanth/gespan/releases?per_page=100",
      "https://api.github.com/repos/0xvasanth/gespan/releases/tags/v1.4.0",
      "https://api.github.com/repos/0xvasanth/gespan/releases?per_page=100",
    ]);
  });

  test("records the published versions up to the release, skipping drafts and newer ones", async () => {
    const list = [published("1.5.0"), published("1.4.1", { draft: true }), published("1.4.0"), published("1.3.0"), published("1.2.0-beta", { prerelease: true })];
    const release = await fetchRelease("1.4.0", { fetchImpl: github(list), token: "" });
    expect(release.version).toBe("1.4.0");
    expect(release.published).toEqual(["1.4.0", "1.3.0"]);
  });

  test("a failed release list fails the sync instead of dropping the versions", async () => {
    const fetchImpl = (async (url: string) =>
      url.includes("releases?") ? { status: 502, ok: false, json: async () => ({}) } : { status: 200, ok: true, json: async () => published("1.4.0") }) as unknown as typeof fetch;
    await expect(fetchRelease(undefined, { fetchImpl, token: "" })).rejects.toThrow(/502 for releases/);
  });

  test("a tag that is only a draft reads as not published yet", async () => {
    await expect(fetchRelease("v9.9.9", { fetchImpl: respond(404, {}), token: "" })).rejects.toThrow(/no published release v9.9.9/);
  });

  test("writes the record once, and reports the version change", () => {
    dir = mkdtempSync(join(tmpdir(), "gespan-release-"));
    const file = join(dir, "release.json");
    expect(writeRelease(releaseFromGitHub(published("1.3.0")), file)).toEqual({ previous: null, next: "1.3.0", changed: true });
    expect(writeRelease(releaseFromGitHub(published("1.4.0")), file)).toEqual({ previous: "1.3.0", next: "1.4.0", changed: true });
    expect(writeRelease(releaseFromGitHub(published("1.4.0")), file)).toEqual({ previous: "1.4.0", next: "1.4.0", changed: false });
    expect(JSON.parse(readFileSync(file, "utf8")).version).toBe("1.4.0");
  });

  test("the committed release.json is a full, consistent record", () => {
    const release = JSON.parse(readFileSync(join(site, "src/release.json"), "utf8"));
    expect(release.version).toMatch(/^\d+\.\d+\.\d+$/);
    expect(release.tag).toBe(`v${release.version}`);
    expect(Object.keys(release.files).sort()).toEqual(["appImage", "deb", "macArm", "macIntel", "rpm", "windows"]);
    for (const url of Object.values(release.files) as string[]) {
      expect(url).toMatch(new RegExp(`^https://github\\.com/0xvasanth/gespan/releases/download/v${release.version.replace(/\./g, "\\.")}/`));
      expect(url).toContain(release.version);
    }
  });
});

describe("parseChangelog", () => {
  const text = `# Changelog

## [Unreleased]

- **Not out yet:** skipped

## [0.3.0] - 2027-01-20

### Added

- **In the pipeline:** not published.

## [0.2.9] - 2027-01-10

### Added

- **Board sync:** cards follow pull
  requests across two lines.
- A plain entry with \`code\`.

### Fixed

- **Crash:** fixed.

## [0.2.8] - 2027-01-02

### Changed

- **Speed:** faster.
`;

  test("skips Unreleased and keeps releases newest first", () => {
    expect(parseChangelog(text).map((r) => [r.version, r.date])).toEqual([
      ["0.3.0", "2027-01-20"],
      ["0.2.9", "2027-01-10"],
      ["0.2.8", "2027-01-02"],
    ]);
  });

  test("hides versions newer than the published release", () => {
    // The pipeline adds 0.3.0 to CHANGELOG.md before its downloads are out.
    expect(parseChangelog(text, "0.2.9").map((r) => r.version)).toEqual(["0.2.9", "0.2.8"]);
    expect(parseChangelog(text, "0.3.0").map((r) => r.version)).toEqual(["0.3.0", "0.2.9", "0.2.8"]);
  });

  test("splits a bold title from its text and joins wrapped lines", () => {
    const [, release] = parseChangelog(text);
    expect(release.sections.map((s) => s.kind)).toEqual(["Added", "Fixed"]);
    expect(release.sections[0].entries[0]).toEqual({ title: "Board sync", body: "cards follow pull requests across two lines." });
    expect(release.sections[0].entries[1]).toEqual({ title: null, body: "A plain entry with `code`." });
  });

  test("the real CHANGELOG.md has the committed release, with titles", () => {
    const version = JSON.parse(readFileSync(join(site, "src/release.json"), "utf8")).version;
    const [latest] = parseChangelog(CHANGELOG, version);
    expect(latest.version).toBe(version);
    expect(latest.sections.flatMap((s) => s.entries).every((e) => !e.body.startsWith("**"))).toBe(true);
  });

  test("publishedVersions keeps x.y.z releases, newest first", () => {
    const list = [published("0.10.0"), published("0.9.0"), published("0.11.0", { draft: true }), published("0.0.3-beta", { prerelease: true }), { ...published("0"), tag_name: "native-latest" }];
    expect(publishedVersions(list)).toEqual(["0.10.0", "0.9.0"]);
  });

  test("versions compare numerically", () => {
    expect(compareVersions("0.10.0", "0.9.9")).toBeGreaterThan(0);
    expect(compareVersions("1.0.0", "1.0.0")).toBe(0);
    expect(compareVersions("0.2.0", "0.3.0")).toBeLessThan(0);
  });
});

describe("foldUnpublished", () => {
  const rel = (version: string, sections: [string, string[]][]) => ({
    version,
    date: `2027-01-${version.split(".")[2].padStart(2, "0")}`,
    sections: sections.map(([kind, titles]) => ({ kind, entries: titles.map((title) => ({ title, body: "." })) })),
  });
  // 0.3.1 and 0.3.3 were never published: 0.3.2 and 0.3.4 shipped them.
  const releases = [
    rel("0.3.4", [["Fixed", ["Menu"]]]),
    rel("0.3.3", [["Added", ["Hub"]], ["Removed", ["Operator"]]]),
    rel("0.3.2", [["Fixed", ["Rail"]]]),
    rel("0.3.1", [["Added", ["Sessions"]], ["Fixed", ["Releases"]]]),
    rel("0.3.0", [["Added", ["Pilot"]]]),
  ];

  test("shows an unpublished version inside the release that shipped it", () => {
    const folded = foldUnpublished(releases, ["0.3.4", "0.3.2", "0.3.0"]);
    expect(folded.map((r) => [r.version, r.includes ?? []])).toEqual([
      ["0.3.4", ["0.3.3"]],
      ["0.3.2", ["0.3.1"]],
      ["0.3.0", []],
    ]);
    // The release keeps its own date, and its sections go Added, Changed, Fixed, Removed.
    expect(folded[0].date).toBe("2027-01-04");
    expect(folded[0].sections.map((s) => [s.kind, s.entries.map((e) => e.title)])).toEqual([
      ["Added", ["Hub"]],
      ["Fixed", ["Menu"]],
      ["Removed", ["Operator"]],
    ]);
    // Within a kind, the shipping release's own entries come first.
    expect(folded[1].sections.map((s) => [s.kind, s.entries.map((e) => e.title)])).toEqual([
      ["Added", ["Sessions"]],
      ["Fixed", ["Rail", "Releases"]],
    ]);
  });

  test("folds several unpublished versions in a row, newest first", () => {
    const folded = foldUnpublished(releases, ["0.3.4", "0.3.0"]);
    expect(folded.map((r) => [r.version, r.includes ?? []])).toEqual([
      ["0.3.4", ["0.3.3", "0.3.2", "0.3.1"]],
      ["0.3.0", []],
    ]);
    expect(folded[0].sections.find((s) => s.kind === "Fixed")?.entries.map((e) => e.title)).toEqual(["Menu", "Rail", "Releases"]);
  });

  test("leaves the changelog alone without a published list, and before the oldest release", () => {
    expect(foldUnpublished(releases)).toBe(releases);
    expect(foldUnpublished(releases, [])).toBe(releases);
    // Only 0.3.2 is known: older versions predate the list and stay as they are.
    expect(foldUnpublished(releases, ["0.3.2"]).map((r) => r.version)).toEqual(["0.3.4", "0.3.3", "0.3.2", "0.3.1", "0.3.0"]);
  });

  test("the real changelog shows the committed release with what it shipped", () => {
    const release = JSON.parse(readFileSync(join(site, "src/release.json"), "utf8"));
    const folded = foldUnpublished(parseChangelog(CHANGELOG, release.version), release.published);
    expect(folded[0].version).toBe(release.version);
    for (const entry of folded) for (const v of entry.includes ?? []) expect(release.published).not.toContain(v);
    // Every version shown newer than the oldest release has a release of its own to link to.
    const oldest = release.published[release.published.length - 1];
    for (const entry of folded) if (/^\d+\.\d+\.\d+$/.test(entry.version) && compareVersions(entry.version, oldest) > 0) expect(release.published).toContain(entry.version);
  });
});
