import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, sep } from "node:path";
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

test("the root workspace and its lockfile leave out the Tauri app", async () => {
  const { packages } = cargoMetadata(root, "--no-deps");
  assert.ok(packages.some(({ name }) => name === "gespan-mobile-link"));
  for (const { name, manifest_path } of packages)
    assert.ok(
      !manifest_path.includes(`${sep}src-tauri${sep}`),
      `${name} is still in the root workspace`,
    );
  const lock = await readFile(join(root, "Cargo.lock"), "utf8");
  assert.doesNotMatch(lock, /^name = "(gespan|tauri)"$/m);
});

test("the mobile link resolves from a checkout without src-tauri", async (t) => {
  const checkout = await mkdtemp(join(tmpdir(), "gespan-no-tauri-"));
  t.after(() => rm(checkout, { recursive: true, force: true }));
  await cp(join(root, "Cargo.toml"), join(checkout, "Cargo.toml"));
  await cp(join(root, "Cargo.lock"), join(checkout, "Cargo.lock"));
  await cp(join(root, "crates"), join(checkout, "crates"), {
    recursive: true,
    filter: (path) => path.split(sep).at(-1) !== "target",
  });
  const { packages, resolve } = cargoMetadata(checkout);
  const link = packages.find(({ name }) => name === "gespan-mobile-link");
  assert.ok(link);
  assert.ok(resolve.nodes.some(({ id }) => id === link.id));
});

for (const [label, eol] of [
  ["LF", "\n"],
  ["CRLF", "\r\n"],
])
  test(`set-version reaches both workspaces and the Tauri app's own lockfile (${label})`, async (t) => {
    const checkout = await mkdtemp(join(tmpdir(), "gespan-set-version-"));
    t.after(() => rm(checkout, { recursive: true, force: true }));
    const files = [
      "package.json",
      "Cargo.toml",
      "Cargo.lock",
      "src-tauri/Cargo.toml",
      "src-tauri/Cargo.lock",
      "src-tauri/tauri.conf.json",
      "crates/gespan-gpui/Cargo.toml",
      "crates/gespan-gpui/Cargo.lock",
      "scripts/bump-version.mjs",
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
    assert.match(await read("src-tauri/Cargo.toml"), /^version = "9\.8\.7-beta\.6"$/m);
    assert.match(await read("Cargo.toml"), /^version = "9\.8\.7-beta\.6"$/m);
    for (const name of ["gespan", "gespan-models", "gespan-speech"])
      assert.match(await read("src-tauri/Cargo.lock"), locked(name));
    for (const name of ["gespan-models", "gespan-speech"])
      assert.match(await read("Cargo.lock"), locked(name));
  });
