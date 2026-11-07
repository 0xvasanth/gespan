// Download the pinned herdr release asset and check its SHA-256.
//   bun run scripts/fetch-herdr.mjs --asset <asset|host> --out <file>
// `--release <file>` reads a different release list (the tests serve a local file).
import { createHash } from "node:crypto";
import { chmod, mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const pinned = fileURLToPath(new URL("./herdr-release.json", import.meta.url));

// The release asset for a Node `process.platform` / `process.arch` pair.
export function assetForHost(platform = process.platform, arch = process.arch) {
  if (platform === "win32") throw new Error("no herdr asset for windows");
  const os = { darwin: "macos", linux: "linux" }[platform];
  const cpu = { arm64: "aarch64", x64: "x86_64" }[arch];
  if (!os || !cpu) throw new Error(`no herdr asset for ${platform}/${arch}`);
  return `${os}-${cpu}`;
}

export async function fetchHerdr({ asset, out, release = pinned }) {
  const name = asset === "host" ? assetForHost() : asset;
  const { assets } = JSON.parse(await readFile(release, "utf8"));
  const entry = assets[name];
  if (!entry)
    throw new Error(`no herdr asset named ${name}; choose one of ${Object.keys(assets).join(", ")}`);
  const response = await fetch(entry.url);
  if (!response.ok) throw new Error(`downloading ${entry.url} failed: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  const actual = createHash("sha256").update(bytes).digest("hex");
  if (actual !== entry.sha256.toLowerCase())
    throw new Error(`checksum mismatch for ${name}: expected ${entry.sha256}, got ${actual}`);
  const target = resolve(out);
  await mkdir(dirname(target), { recursive: true });
  // Write beside the target and rename, so a failed run never leaves a
  // half-written binary where a later step would pick it up.
  const partial = `${target}.partial`;
  try {
    await writeFile(partial, bytes);
    await chmod(partial, 0o755);
    await rename(partial, target);
  } catch (error) {
    await rm(partial, { force: true });
    throw error;
  }
  return target;
}

function option(args, flag) {
  const at = args.indexOf(flag);
  if (at === -1) return undefined;
  const value = args[at + 1];
  if (value === undefined || value.startsWith("--")) throw new Error(`${flag} needs a value`);
  return value;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const args = process.argv.slice(2);
    const asset = option(args, "--asset");
    const out = option(args, "--out");
    if (!asset || !out)
      throw new Error("usage: fetch-herdr.mjs --asset <asset|host> --out <file> [--release <file>]");
    const release = option(args, "--release");
    console.log(await fetchHerdr({ asset, out, ...(release && { release }) }));
  } catch (error) {
    console.error(`fetch-herdr: ${error.message}`);
    process.exit(1);
  }
}
