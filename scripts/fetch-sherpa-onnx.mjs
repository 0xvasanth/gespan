// Download the sherpa-onnx static libraries a build links, for SHERPA_ONNX_ARCHIVE_DIR.
//   node scripts/fetch-sherpa-onnx.mjs --out <dir> --target <rust target> [--target ...]
// sherpa-onnx-sys fetches its archive from the release's public download link
// once, with no retry, and that link can fail for one file for hours while
// the GitHub API still serves it. This goes through the API instead, retries,
// and checks each archive against the release's SHA-256 digest.
// `--lock <file>`, `--api <origin>`, `--attempts <n>` and `--retry-delay <ms>`
// are for the tests.
import { createHash } from "node:crypto";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const lockfile = fileURLToPath(new URL("../crates/gespan-gpui/Cargo.lock", import.meta.url));
const repository = "k2-fsa/sherpa-onnx";

// The static archive sherpa-onnx-sys links for a Rust target, as its build.rs names it.
export function archiveFor(target, version) {
  const name = {
    "x86_64-unknown-linux-gnu": "linux-x64-static-lib",
    "aarch64-unknown-linux-gnu": "linux-aarch64-static-lib",
    "x86_64-apple-darwin": "osx-x64-static-lib",
    "aarch64-apple-darwin": "osx-arm64-static-lib",
    "x86_64-pc-windows-msvc": "win-x64-static-MT-Release-lib",
    "aarch64-pc-windows-msvc": "win-arm64-static-MT-Release-lib",
  }[target];
  if (!name) throw new Error(`no sherpa-onnx static archive for ${target}`);
  return `sherpa-onnx-v${version}-${name}.tar.bz2`;
}

// The sherpa-onnx-sys version a Cargo.lock pins.
export function lockedVersion(lock) {
  const match = /\[\[package\]\]\nname = "sherpa-onnx-sys"\nversion = "([^"]+)"/.exec(lock.replace(/\r\n/g, "\n"));
  if (!match) throw new Error("the lockfile has no sherpa-onnx-sys package");
  return match[1];
}

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

class Transient extends Error {}

async function request(url, { token, accept, timeoutMs }) {
  const headers = { accept, "user-agent": "gespan-release" };
  if (token) headers.authorization = `Bearer ${token}`;
  let response;
  try {
    // The signal covers the body too, so a stalled transfer is cut as well.
    response = await fetch(url, { headers, signal: AbortSignal.timeout(timeoutMs) });
    if (response.ok) return accept === "application/json" ? await response.json() : Buffer.from(await response.arrayBuffer());
  } catch (error) {
    throw new Transient(error.name === "TimeoutError" ? `${url} timed out after ${timeoutMs} ms` : `${url}: ${error.message}`);
  }
  const message = `${url} failed: HTTP ${response.status}`;
  throw response.status >= 500 || response.status === 429 ? new Transient(message) : new Error(message);
}

// Runs `work` again after a network error, a timeout or a 5xx/429, up to `attempts` times.
async function retrying(attempts, delayMs, work) {
  for (let attempt = 1; ; attempt++) {
    try {
      return await work();
    } catch (error) {
      if (!(error instanceof Transient) || attempt >= attempts) throw error;
      console.error(`fetch-sherpa-onnx: ${error.message}; retrying (${attempt} of ${attempts - 1})`);
      await new Promise((done) => setTimeout(done, delayMs * attempt));
    }
  }
}

export async function fetchSherpaOnnx({
  targets,
  out,
  lock = lockfile,
  api = "https://api.github.com",
  token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN,
  attempts = 5,
  retryDelayMs = 10_000,
  timeoutMs = 300_000,
}) {
  const version = lockedVersion(await readFile(lock, "utf8"));
  const names = [...new Set(targets.map((target) => archiveFor(target, version)))];
  const directory = resolve(out);
  const options = { token, timeoutMs };
  const release = await retrying(attempts, retryDelayMs, () =>
    request(`${api}/repos/${repository}/releases/tags/v${version}`, { ...options, accept: "application/json" }),
  );
  await mkdir(directory, { recursive: true });
  for (const name of names) {
    const asset = release.assets?.find((candidate) => candidate.name === name);
    if (!asset) throw new Error(`sherpa-onnx v${version} has no asset named ${name}`);
    const expected = /^sha256:([0-9a-f]{64})$/i.exec(asset.digest ?? "")?.[1]?.toLowerCase();
    if (!expected) throw new Error(`sherpa-onnx v${version} lists no SHA-256 digest for ${name}`);
    const target = join(directory, name);
    const existing = await readFile(target).catch(() => null);
    if (existing && sha256(existing) === expected) continue;
    // The asset's API url, not its download link: the API redirects to storage
    // and drops the token on the way, as fetch does for another origin.
    const bytes = await retrying(attempts, retryDelayMs, async () => {
      const body = await request(asset.url, { ...options, accept: "application/octet-stream" });
      const actual = sha256(body);
      if (actual !== expected) throw new Transient(`checksum mismatch for ${name}: expected ${expected}, got ${actual}`);
      return body;
    });
    // Write beside the target and rename, so a failed run never leaves a
    // half-written archive where the build would pick it up.
    const partial = `${target}.partial`;
    try {
      await writeFile(partial, bytes);
      await rename(partial, target);
    } catch (error) {
      await rm(partial, { force: true });
      throw error;
    }
  }
  return directory;
}

function options(args, flag) {
  const values = [];
  for (let at = args.indexOf(flag); at !== -1; at = args.indexOf(flag, at + 2)) {
    const value = args[at + 1];
    if (value === undefined || value.startsWith("--")) throw new Error(`${flag} needs a value`);
    values.push(value);
  }
  return values;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const args = process.argv.slice(2);
    const targets = options(args, "--target");
    const [out] = options(args, "--out");
    if (!out || targets.length === 0)
      throw new Error("usage: fetch-sherpa-onnx.mjs --out <dir> --target <rust target> [--target ...]");
    const [lock] = options(args, "--lock");
    const [api] = options(args, "--api");
    const [attempts] = options(args, "--attempts");
    const [delay] = options(args, "--retry-delay");
    console.log(
      await fetchSherpaOnnx({
        targets,
        out,
        ...(lock && { lock }),
        ...(api && { api }),
        ...(attempts && { attempts: Number(attempts) }),
        ...(delay && { retryDelayMs: Number(delay) }),
      }),
    );
  } catch (error) {
    console.error(`fetch-sherpa-onnx: ${error.message}`);
    process.exit(1);
  }
}
