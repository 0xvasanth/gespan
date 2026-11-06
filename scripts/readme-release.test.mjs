import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { END, START, releaseBlock, updateReadme } from "./readme-release.mjs";

const root = new URL("../", import.meta.url);
const read = (file) =>
  readFileSync(new URL(file, root), "utf8").replaceAll("\r\n", "\n");

test("the README names the version in package.json", () => {
  const { version } = JSON.parse(read("package.json"));
  const readme = read("README.md");
  if (version.includes("-")) {
    assert.ok(readme.includes(START), "README.md lost its release block");
    return;
  }
  assert.ok(
    readme.includes(releaseBlock(version)),
    `README.md's Install section is not on ${version}: run bun run set-version ${version}`,
  );
});

test("links match the files a stable release publishes", () => {
  // The names v0.0.7 shipped with on GitHub.
  const links = [...releaseBlock("0.0.7").matchAll(/\]\(([^)]+)\)/g)].map(
    ([, url]) => url,
  );
  const base = "https://github.com/0xvasanth/gespan/releases";
  assert.deepEqual(links, [
    `${base}/tag/v0.0.7`,
    `${base}/download/v0.0.7/Gespan_0.0.7_aarch64.dmg`,
    `${base}/download/v0.0.7/Gespan_0.0.7_x64.dmg`,
    `${base}/download/v0.0.7/Gespan_0.0.7_x64-setup.exe`,
    `${base}/download/v0.0.7/Gespan_0.0.7_amd64.AppImage`,
    `${base}/download/v0.0.7/gespan_0.0.7_amd64.deb`,
    `${base}/download/v0.0.7/gespan-0.0.7-1.el10.x86_64.rpm`,
  ]);
});

const page = `# Gespan\n\n## Install\n\n${releaseBlock("0.0.7")}\n\nGespan on Windows.\n`;

test("a stable bump rewrites only the release block", () => {
  const next = updateReadme(page, "0.1.0");
  assert.equal(
    next,
    `# Gespan\n\n## Install\n\n${releaseBlock("0.1.0")}\n\nGespan on Windows.\n`,
  );
  assert.doesNotMatch(next, /0\.0\.7/);
  assert.equal(updateReadme(next, "0.1.0"), next);
});

test("a prerelease leaves the README on the last stable release", () => {
  for (const version of ["0.1.0-beta", "0.1.0-beta.2", "0.1.0-preview.1"])
    assert.equal(updateReadme(page, version), page);
});

test("a CRLF README keeps its line endings", () => {
  const crlf = page.replaceAll("\n", "\r\n");
  const next = updateReadme(crlf, "0.1.0");
  assert.match(next, /v0\.1\.0/);
  assert.doesNotMatch(next.replaceAll("\r\n", ""), /\n/);
});

test("a README without the block fails instead of silently skipping", () => {
  assert.throws(
    () => updateReadme("# Gespan\n\nThe current release is 0.0.7.\n", "0.1.0"),
    /release:start/,
  );
  assert.throws(() => updateReadme(`${END}\n${START}\n`, "0.1.0"), /release:start/);
});
