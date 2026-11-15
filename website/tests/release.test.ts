import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";
import { parseChangelog, readRelease } from "../scripts/release.mjs";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "../..");
const repo = (path: string) => readFileSync(join(repoRoot, path), "utf8");
const README = repo("README.md");
const CHANGELOG = repo("CHANGELOG.md");

const block = (version: string, files: string[]) => `
<!-- release:start -->
The current release is
[**${version}**](https://github.com/0xvasanth/gespan/releases/tag/v${version}).

${files.map((f) => `| x | [x](https://github.com/0xvasanth/gespan/releases/download/v${version}/${f}) |`).join("\n")}
<!-- release:end -->`;
const ALL = (v: string) => [`Gespan_${v}_aarch64.dmg`, `Gespan_${v}_x64.dmg`, `Gespan_${v}_x64-setup.exe`, `Gespan_${v}_amd64.AppImage`, `gespan_${v}_amd64.deb`, `gespan-${v}-1.el10.x86_64.rpm`];

describe("readRelease", () => {
  test("the real README gives every download for its current release", () => {
    const release = readRelease(README, CHANGELOG);
    const version = README.match(/current release is\s*\[\*\*([^*]+)\*\*\]/)![1];
    expect(release.version).toBe(version);
    expect(Object.keys(release.files).sort()).toEqual(["appImage", "deb", "macArm", "macIntel", "rpm", "windows"]);
    for (const url of Object.values(release.files)) {
      expect(url).toContain(`/releases/download/v${version}/`);
      // The site links to exactly what README.md links to.
      expect(README).toContain(`(${url})`);
    }
    expect(release.date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  test("takes the release date from CHANGELOG.md", () => {
    const changelog = "## [Unreleased]\n\n## [1.4.0] - 2027-02-03\n\n### Added\n\n- **A:** b\n";
    expect(readRelease(block("1.4.0", ALL("1.4.0")), changelog).date).toBe("2027-02-03");
  });

  test("a release with no CHANGELOG entry still builds, without a date", () => {
    expect(readRelease(block("1.4.0", ALL("1.4.0")), "").date).toBeNull();
  });

  test("a missing download fails loudly instead of shipping a dead button", () => {
    const files = ALL("1.4.0").filter((f) => !f.endsWith(".rpm"));
    expect(() => readRelease(block("1.4.0", files), "")).toThrow(/no rpm download/);
  });

  test("a download left on the previous version is caught", () => {
    const files = ALL("1.4.0").map((f) => (f.endsWith(".exe") ? "Gespan_1.3.0_x64-setup.exe" : f));
    const readme = block("1.4.0", files).replace("v1.4.0/Gespan_1.3.0", "v1.3.0/Gespan_1.3.0");
    expect(() => readRelease(readme, "")).toThrow(/windows download is not for v1\.4\.0/);
  });

  test("a README without the release block is an error", () => {
    expect(() => readRelease("# Gespan", "")).toThrow(/no release block/);
  });
});

describe("parseChangelog", () => {
  const text = `# Changelog

## [Unreleased]

- **Not out yet:** skipped

## [0.3.0] - 2027-01-10

### Added

- **Board sync:** cards follow pull
  requests across two lines.
- A plain entry with \`code\`.

### Fixed

- **Crash:** fixed.

## [0.2.9] - 2027-01-02

### Changed

- **Speed:** faster.
`;

  test("skips Unreleased and keeps releases newest first", () => {
    expect(parseChangelog(text).map((r) => [r.version, r.date])).toEqual([
      ["0.3.0", "2027-01-10"],
      ["0.2.9", "2027-01-02"],
    ]);
  });

  test("splits a bold title from its text and joins wrapped lines", () => {
    const [release] = parseChangelog(text);
    expect(release.sections.map((s) => s.kind)).toEqual(["Added", "Fixed"]);
    expect(release.sections[0].entries[0]).toEqual({ title: "Board sync", body: "cards follow pull requests across two lines." });
    expect(release.sections[0].entries[1]).toEqual({ title: null, body: "A plain entry with `code`." });
  });

  test("reads the real CHANGELOG.md with titles on its newest release", () => {
    const [latest] = parseChangelog(CHANGELOG);
    const entries = latest.sections.flatMap((s) => s.entries);
    expect(entries.length).toBeGreaterThan(0);
    expect(entries.every((e) => !e.body.startsWith("**"))).toBe(true);
  });
});
