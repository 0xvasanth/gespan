import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
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
    for (const name of ["gespan-models", "gespan-speech"])
      assert.match(await read("Cargo.lock"), locked(name));
    for (const name of [
      "gespan-core",
      "gespan-first-launch",
      "gespan-gpui",
      "gespan-models",
      "gespan-speech",
    ])
      assert.match(await read("crates/gespan-gpui/Cargo.lock"), locked(name));
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
