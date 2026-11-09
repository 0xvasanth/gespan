import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import { archiveFor, lockedVersion } from "./fetch-sherpa-onnx.mjs";

const script = fileURLToPath(new URL("./fetch-sherpa-onnx.mjs", import.meta.url));
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const lock = '[[package]]\nname = "sherpa-onnx-sys"\nversion = "2.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n';
const x64 = "sherpa-onnx-v2.0.0-osx-x64-static-lib.tar.bz2";
const arm64 = "sherpa-onnx-v2.0.0-osx-arm64-static-lib.tar.bz2";

// Async, so the in-process servers keep answering while the script runs.
const run = async (args, env = {}) => {
  try {
    const { stderr } = await promisify(execFile)(process.execPath, [script, ...args], {
      encoding: "utf8",
      env: { ...process.env, GH_TOKEN: "", GITHUB_TOKEN: "", ...env },
    });
    return { status: 0, stderr };
  } catch (error) {
    return { status: error.code, stderr: error.stderr };
  }
};

async function listen(t, handler) {
  const server = createServer(handler);
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  t.after(() => new Promise((done) => server.close(done)));
  return `http://127.0.0.1:${server.address().port}`;
}

// A fake GitHub API for one release. Each asset's API url redirects to a
// separate storage origin, as GitHub's does; `failures` 504s come first.
async function github(t, files, { failures = 0, digest = (body) => `sha256:${sha256(body)}` } = {}) {
  const seen = { api: [], storage: [] };
  let failed = 0;
  const storage = await listen(t, (request, response) => {
    seen.storage.push({ url: request.url, authorization: request.headers.authorization });
    const body = files[decodeURIComponent(request.url.slice(1))];
    if (body === undefined) return response.writeHead(404).end();
    if (failed < failures) {
      failed++;
      return response.writeHead(504).end("Gateway Timeout");
    }
    response.end(body);
  });
  const api = await listen(t, (request, response) => {
    seen.api.push({ url: request.url, authorization: request.headers.authorization, accept: request.headers.accept });
    if (request.url === "/repos/k2-fsa/sherpa-onnx/releases/tags/v2.0.0") {
      const assets = Object.entries(files).map(([name, body], id) => ({ name, url: `${api}/assets/${id}`, digest: digest(body) }));
      return response.setHeader("content-type", "application/json").end(JSON.stringify({ assets }));
    }
    const asset = /^\/assets\/(\d+)$/.exec(request.url);
    if (asset) {
      const name = Object.keys(files)[Number(asset[1])];
      return response.writeHead(302, { location: `${storage}/${encodeURIComponent(name)}` }).end();
    }
    response.writeHead(404).end();
  });
  return { api, seen };
}

async function fetchInto(t, api, targets, { env, attempts = 3, directory } = {}) {
  const root = await mkdtemp(join(tmpdir(), "fetch-sherpa-onnx-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const lockPath = join(root, "Cargo.lock");
  await writeFile(lockPath, lock);
  const out = directory ?? join(root, "archives");
  const flags = targets.flatMap((target) => ["--target", target]);
  const ran = await run(
    ["--out", out, ...flags, "--lock", lockPath, "--api", api, "--attempts", String(attempts), "--retry-delay", "0"],
    env,
  );
  return { ran, out };
}

test("each release target maps to the archive sherpa-onnx-sys links statically", () => {
  assert.equal(archiveFor("aarch64-apple-darwin", "2.0.0"), arm64);
  assert.equal(archiveFor("x86_64-apple-darwin", "2.0.0"), x64);
  assert.equal(archiveFor("x86_64-unknown-linux-gnu", "2.0.0"), "sherpa-onnx-v2.0.0-linux-x64-static-lib.tar.bz2");
  assert.equal(archiveFor("x86_64-pc-windows-msvc", "2.0.0"), "sherpa-onnx-v2.0.0-win-x64-static-MT-Release-lib.tar.bz2");
  assert.throws(() => archiveFor("x86_64-unknown-freebsd", "2.0.0"), /no sherpa-onnx static archive for x86_64-unknown-freebsd/);
});

test("the version comes from the native lockfile, and a lockfile without it is refused", async () => {
  const native = await readFile(fileURLToPath(new URL("../crates/gespan-gpui/Cargo.lock", import.meta.url)), "utf8");
  assert.match(lockedVersion(native), /^\d+\.\d+\.\d+$/);
  assert.equal(lockedVersion(lock.replace(/\n/g, "\r\n")), "2.0.0");
  assert.throws(() => lockedVersion('[[package]]\nname = "sherpa-onnx"\nversion = "2.0.0"\n'), /no sherpa-onnx-sys package/);
});

test("a host and a cross target each get their archive, through the API with the token", async (t) => {
  const files = { [arm64]: Buffer.from("arm64 libs"), [x64]: Buffer.from("x64 libs") };
  const { api, seen } = await github(t, files);
  const { ran, out } = await fetchInto(t, api, ["aarch64-apple-darwin", "x86_64-apple-darwin", "aarch64-apple-darwin"], {
    env: { GH_TOKEN: "secret-token" },
  });
  assert.equal(ran.status, 0, ran.stderr);
  assert.deepEqual((await readdir(out)).sort(), [arm64, x64]);
  assert.deepEqual(await readFile(join(out, x64)), files[x64]);
  assert.deepEqual(await readFile(join(out, arm64)), files[arm64]);
  assert.ok(seen.api.every((call) => call.authorization === "Bearer secret-token"));
  assert.ok(seen.api.some((call) => call.url.startsWith("/assets/") && call.accept === "application/octet-stream"));
  // The redirect leaves for another origin, so the token must not follow it.
  assert.equal(seen.storage.length, 2);
  assert.ok(seen.storage.every((call) => call.authorization === undefined));
});

test("504s from storage are retried until the archive comes through", async (t) => {
  const { api, seen } = await github(t, { [x64]: Buffer.from("x64 libs") }, { failures: 2 });
  const { ran, out } = await fetchInto(t, api, ["x86_64-apple-darwin"], { attempts: 3 });
  assert.equal(ran.status, 0, ran.stderr);
  assert.match(ran.stderr, /HTTP 504; retrying \(1 of 2\)/);
  assert.equal(seen.storage.length, 3);
  assert.deepEqual(await readFile(join(out, x64)), Buffer.from("x64 libs"));
});

test("when every attempt fails it says so and leaves no archive behind", async (t) => {
  const { api, seen } = await github(t, { [x64]: Buffer.from("x64 libs") }, { failures: 10 });
  const { ran, out } = await fetchInto(t, api, ["x86_64-apple-darwin"], { attempts: 3 });
  assert.notEqual(ran.status, 0);
  assert.match(ran.stderr, /failed: HTTP 504\n$/);
  assert.equal(seen.storage.length, 3);
  await assert.rejects(() => stat(join(out, x64)), { code: "ENOENT" });
});

test("an archive that does not match the release digest is refused", async (t) => {
  const { api } = await github(t, { [x64]: Buffer.from("tampered") }, { digest: () => `sha256:${sha256("original")}` });
  const { ran, out } = await fetchInto(t, api, ["x86_64-apple-darwin"], { attempts: 2 });
  assert.notEqual(ran.status, 0);
  assert.match(ran.stderr, /checksum mismatch for sherpa-onnx-v2\.0\.0-osx-x64-static-lib\.tar\.bz2/);
  await assert.rejects(() => stat(join(out, x64)), { code: "ENOENT" });
});

test("a release without a digest for the archive is refused rather than trusted", async (t) => {
  const { api } = await github(t, { [x64]: Buffer.from("x64 libs") }, { digest: () => null });
  const { ran } = await fetchInto(t, api, ["x86_64-apple-darwin"]);
  assert.notEqual(ran.status, 0);
  assert.match(ran.stderr, /lists no SHA-256 digest for sherpa-onnx-v2\.0\.0-osx-x64-static-lib\.tar\.bz2/);
});

test("a release missing the archive fails without retrying", async (t) => {
  const { api, seen } = await github(t, { [arm64]: Buffer.from("arm64 libs") });
  const { ran } = await fetchInto(t, api, ["x86_64-apple-darwin"]);
  assert.notEqual(ran.status, 0);
  assert.match(ran.stderr, /has no asset named sherpa-onnx-v2\.0\.0-osx-x64-static-lib\.tar\.bz2/);
  assert.equal(seen.storage.length, 0);
});

test("an archive already there with the right digest is kept without downloading", async (t) => {
  const body = Buffer.from("x64 libs");
  const { api, seen } = await github(t, { [x64]: body });
  const directory = await mkdtemp(join(tmpdir(), "fetch-sherpa-onnx-out-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await writeFile(join(directory, x64), body);
  const { ran } = await fetchInto(t, api, ["x86_64-apple-darwin"], { directory });
  assert.equal(ran.status, 0, ran.stderr);
  assert.equal(seen.storage.length, 0);
});
