import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { impact, pieces } from "./ci-impact.mjs";

const root = new URL("../", import.meta.url);
const read = (path) => readFile(new URL(path, root), "utf8");
const affected = (...files) => [...impact(files)].sort();
const all = [...pieces].sort();

test("a file no rule names runs every piece", () => {
  for (const file of ["brand-new.toml", "package.json", "bun.lock", "Cargo.lock", "scripts/bump-version.mjs", ".github/workflows/ci.yml","tests/fixtures/shared/ticket-work-item.json"])
    assert.deepEqual(affected(file), all, file);
});

test("CI keeps the job names main requires", async () => {
  // Protect main requires these by name, so a renamed job blocks every PR.
  const ci = await read(".github/workflows/ci.yml");
  for (const name of ["name: Core (${{ matrix.os }})", "name: Gespan Native (${{ matrix.name }})", "name: Mobile checks", "name: Release channel isolation"])
    assert.ok(ci.includes(name), name);
  assert.match(ci, /^  host:\n    needs: impact\n/m);
  assert.equal(ci.match(/os: \[ubuntu-latest, macos-latest, windows-latest\]/g).length, 2);
  for (const name of ["macOS", "Linux", "Windows"]) assert.match(ci, new RegExp(`- name: ${name}\\n`));
});

test("no changed files, or an unreadable diff, runs every piece", () => {
  assert.deepEqual(affected(), all);
});

test("the crates other pieces read run every piece", () => {
  // host/ tests read crates/gespan-core/src/remote, and the mobile protocol
  // follows crates/mobile-link.
  for (const file of ["crates/gespan-core/src/remote/remote_bootstrap.sh", "crates/mobile-link/src/lib.rs", "crates/gespan-gpui/Cargo.toml", "crates/gespan-gpui/Cargo.lock", "crates/gespan-gpui/packaging/icon.icns", "crates/gespan-gpui/.config/nextest.toml"])
    assert.deepEqual(affected(file), all, file);
});

test("files only people read run nothing", () => {
  assert.deepEqual(affected("CONTRIBUTING.md", "docs/desktop/verification.md", "LICENSE", ".github/CODEOWNERS"), []);
});

test("the README runs the release checks that keep its downloads current", async () => {
  assert.deepEqual(affected("README.md"), ["release"]);
  assert.match(await read("scripts/readme-release.test.mjs"), /README\.md/);
});

test("files Native compiles in are not documentation", async () => {
  assert.deepEqual(affected("docs/desktop/examples/visual-scene.js"), all);
  assert.deepEqual(affected("CHANGELOG.md"), all);
  assert.match(await read("crates/gespan-gpui/src/fixtures.rs"), /docs\/desktop\/examples\/visual-scene\.js/);
  assert.match(await read("crates/gespan-gpui/src/notifications/app_update.rs"), /CHANGELOG\.md/);
});

test("a change to the GUI crate runs Core, because a Core test scans its source", async () => {
  assert.deepEqual(affected("crates/gespan-gpui/src/shell.rs"), ["core", "desktop"]);
  assert.match(await read("crates/gespan-core/tests/shared_contracts.rs"), /\["gespan-core", "gespan-gpui"\]/);
});

test("GUI fixtures, assets and the first-launch tests stay with the GUI job", () => {
  assert.deepEqual(affected("crates/gespan-gpui/tests/fixtures/large.tsx", "crates/gespan-gpui/assets/gespan.png", "crates/gespan-first-launch/tests/first_launch.rs"), ["desktop"]);
});

test("shared crates and vendored GPUI run both Native jobs", () => {
  assert.deepEqual(affected("crates/board-store/src/ops.rs", "vendor/gpui-pre/src/lib.rs"), ["core", "desktop"]);
});

test("every host file a Core contract test reads runs Core, the rest of the host does not", async () => {
  const source = await read("crates/gespan-core/tests/shared_contracts.rs");
  const hostFiles = [...source.matchAll(/"(host\/[^"\s]+)"/g)].map((match) => match[1]);
  assert.ok(hostFiles.includes("host/package.mjs"));
  assert.ok(hostFiles.includes("host/windows-acl.ps1"));
  for (const file of hostFiles) assert.deepEqual(affected(file), ["core", "host"], file);
  assert.deepEqual(affected("host/core/index.ts"), ["host"]);
});

test("mobile changes stay with mobile", () => {
  assert.deepEqual(affected("mobile/src/app.tsx", "tests/mobile/protocol.test.ts"), ["mobile"]);
});

test("one unknown file among known ones still runs every piece", () => {
  assert.deepEqual(affected("README.md", "mobile/src/app.tsx", "unknown.txt"), all);
});

test("pieces union across files", () => {
  assert.deepEqual(affected("mobile/src/app.tsx", "crates/gespan-gpui/tests/fixtures/large.tsx"), ["desktop", "mobile"]);
});
