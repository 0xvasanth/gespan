import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
// The first Cargo run on a fresh Windows runner can exceed one minute.
const cargoTimeout = process.platform === "win32" ? 180_000 : 90_000;

function cargoMetadata(cwd, ...args) {
  const result = spawnSync(
    "cargo",
    ["metadata", "--locked", "--format-version", "1", ...args],
    {
      cwd,
      encoding: "utf8",
      maxBuffer: 256 * 2 ** 20,
      timeout: cargoTimeout,
    },
  );
  assert.equal(result.status, 0, result.stderr);
  return JSON.parse(result.stdout);
}

// Names of the crates whose manifest says `version.workspace = true`.
async function workspaceVersionedCrates() {
  const names = new Set();
  for (const parent of ["crates", "vendor"]) {
    for (const entry of await readdir(join(root, parent), { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      let manifest;
      try {
        manifest = await readFile(join(root, parent, entry.name, "Cargo.toml"), "utf8");
      } catch {
        continue;
      }
      const name = manifest.match(/^name = "([^"]+)"/m)?.[1];
      if (
        name &&
        /^version(\.workspace\s*=\s*true|\s*=\s*\{\s*workspace\s*=\s*true\s*\})/m.test(manifest)
      )
        names.add(name);
    }
  }
  return names;
}

// Packages a lockfile records without a `source`: the local path crates.
function localPackages(lock) {
  return lock
    .split(/^\[\[package\]\]$/m)
    .slice(1)
    .filter((block) => !/^source = /m.test(block))
    .map((block) => block.match(/^name = "([^"]+)"/m)?.[1])
    .filter(Boolean);
}

test("crates taking the workspace version are found from their manifests", async () => {
  const versioned = await workspaceVersionedCrates();
  for (const name of ["gespan-browser", "gespan-core", "gespan-gpui", "gespan-models"])
    assert.ok(versioned.has(name), name);
  assert.ok(!versioned.has("mcp-hub"));
});

test("the root workspace holds the mobile link and no Tauri crates", async () => {
  const { packages } = cargoMetadata(root, "--no-deps");
  assert.ok(packages.some(({ name }) => name === "gespan-mobile-link"));
  const lock = await readFile(join(root, "Cargo.lock"), "utf8");
  assert.doesNotMatch(lock, /^name = "(gespan|tauri)"$/m);
});

for (const [label, eol] of [
  ["LF", "\n"],
  ["CRLF", "\r\n"],
])
  test(`set-version reaches both workspaces (${label})`, async (t) => {
    const checkout = await mkdtemp(join(tmpdir(), "gespan-set-version-"));
    t.after(() => rm(checkout, { recursive: true, force: true }));
    const files = [
      "package.json",
      "Cargo.toml",
      "Cargo.lock",
      "crates/gespan-gpui/Cargo.toml",
      "crates/gespan-gpui/Cargo.lock",
      "scripts/bump-version.mjs",
      "scripts/package.mjs",
      "scripts/fetch-herdr.mjs",
      "scripts/readme-release.mjs",
      "README.md",
    ];
    for (const file of files) {
      await mkdir(dirname(join(checkout, file)), { recursive: true });
      const text = await readFile(join(root, file), "utf8");
      await writeFile(
        join(checkout, file),
        text.replaceAll("\r\n", "\n").replaceAll("\n", eol),
      );
    }
    const result = spawnSync(
      process.execPath,
      [join(checkout, "scripts/bump-version.mjs"), "9.8.7-beta.6"],
      { encoding: "utf8" },
    );
    assert.equal(result.status, 0, result.stderr);
    // Windows checkouts have CRLF line endings.
    const read = async (file) =>
      (await readFile(join(checkout, file), "utf8")).replaceAll("\r\n", "\n");
    const locked = (name) =>
      new RegExp(`^name = "${name}"\\nversion = "9\\.8\\.7-beta\\.6"$`, "m");
    assert.match(await read("Cargo.toml"), /^version = "9\.8\.7-beta\.6"$/m);
    for (const name of ["gespan-browser", "gespan-models", "gespan-speech"])
      assert.match(await read("Cargo.lock"), locked(name));
    for (const name of [
      "gespan-browser",
      "gespan-core",
      "gespan-first-launch",
      "gespan-gpui",
      "gespan-models",
      "gespan-speech",
    ])
      assert.match(await read("crates/gespan-gpui/Cargo.lock"), locked(name));
    // Not just the crates named above: every local crate that takes the
    // workspace version must move in every lockfile that records it, or the
    // next `--locked` build fails.
    const versioned = await workspaceVersionedCrates();
    for (const lockfile of ["Cargo.lock", "crates/gespan-gpui/Cargo.lock"]) {
      const lock = await read(lockfile);
      const due = localPackages(lock).filter((name) => versioned.has(name));
      assert.ok(due.length > 0, `${lockfile} records no workspace-versioned crate`);
      for (const name of due)
        assert.match(lock, locked(name), `${name} in ${lockfile} kept its old version`);
    }
    // A prerelease keeps the README on the last stable release.
    const readme = (await readFile(join(root, "README.md"), "utf8")).replaceAll(
      "\r\n",
      "\n",
    );
    assert.equal(await read("README.md"), readme);

    const stable = spawnSync(
      process.execPath,
      [join(checkout, "scripts/bump-version.mjs"), "9.8.7"],
      { encoding: "utf8" },
    );
    assert.equal(stable.status, 0, stable.stderr);
    const bumped = await readFile(join(checkout, "README.md"), "utf8");
    assert.match(bumped, /releases\/download\/v9\.8\.7\/Gespan_9\.8\.7_aarch64\.dmg/);
    assert.equal(bumped.includes("\r\n"), eol === "\r\n");
    assert.match(await read("package.json"), /"version": "9\.8\.7"/);
  });
