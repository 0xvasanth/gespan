import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import { assetForHost } from "./fetch-herdr.mjs";

const script = fileURLToPath(new URL("./fetch-herdr.mjs", import.meta.url));
// Async, so the in-process file server keeps answering while the script runs.
const run = async (args) => {
  try {
    const { stderr } = await promisify(execFile)(process.execPath, [script, ...args], { encoding: "utf8" });
    return { status: 0, stderr };
  } catch (error) {
    return { status: error.code, stderr: error.stderr };
  }
};
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

// Serve one file from an ephemeral local port; the tests never reach the network.
async function serve(t, body) {
  const server = createServer((request, response) => {
    if (request.url === "/herdr-fake") response.end(body);
    else response.writeHead(404).end();
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => new Promise((resolve) => server.close(resolve)));
  return `http://127.0.0.1:${server.address().port}`;
}

async function fetchWith(t, { body, sha, asset = "fake", out }) {
  const directory = await mkdtemp(join(tmpdir(), "fetch-herdr-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const origin = await serve(t, body);
  const release = join(directory, "release.json");
  await writeFile(
    release,
    JSON.stringify({
      version: "0.9.5",
      protocol: 22,
      assets: { [asset]: { url: `${origin}/herdr-fake`, sha256: sha } },
    }),
  );
  const target = join(directory, out ?? "herdr");
  const ran = await run(["--release", release, "--asset", asset, "--out", target]);
  return { ran, target };
}

test("a download that matches the pinned SHA-256 is written executable", async (t) => {
  const body = Buffer.from("#!/bin/sh\necho herdr\n");
  const { ran, target } = await fetchWith(t, { body, sha: sha256(body) });
  assert.equal(ran.status, 0, ran.stderr);
  assert.deepEqual(await readFile(target), body);
  if (process.platform !== "win32") assert.equal((await stat(target)).mode & 0o777, 0o755);
});

test("a wrong SHA-256 fails with a checksum mismatch and leaves no file behind", async (t) => {
  const { ran, target } = await fetchWith(t, { body: Buffer.from("tampered"), sha: sha256("original") });
  assert.notEqual(ran.status, 0);
  assert.match(ran.stderr, /checksum mismatch/);
  await assert.rejects(() => stat(target), { code: "ENOENT" });
});

test("a digest in upper case still matches", async (t) => {
  const body = Buffer.from("herdr");
  const { ran } = await fetchWith(t, { body, sha: sha256(body).toUpperCase() });
  assert.equal(ran.status, 0, ran.stderr);
});

test("an HTTP error is reported, not written out", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "fetch-herdr-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const origin = await serve(t, "x");
  const release = join(directory, "release.json");
  await writeFile(
    release,
    JSON.stringify({ assets: { fake: { url: `${origin}/missing`, sha256: sha256("x") } } }),
  );
  const target = join(directory, "herdr");
  const ran = await run(["--release", release, "--asset", "fake", "--out", target]);
  assert.notEqual(ran.status, 0);
  assert.match(ran.stderr, /404/);
  await assert.rejects(() => stat(target), { code: "ENOENT" });
});

test("an unknown asset names the choices", async () => {
  const ran = await run(["--asset", "nope", "--out", join(tmpdir(), "never")]);
  assert.notEqual(ran.status, 0);
  assert.match(ran.stderr, /no herdr asset named nope/);
  assert.match(ran.stderr, /macos-aarch64/);
});

test("the pinned release lists all four assets with SHA-256 digests", async () => {
  const release = JSON.parse(await readFile(new URL("./herdr-release.json", import.meta.url), "utf8"));
  assert.equal(release.version, "0.9.5");
  assert.equal(release.protocol, 22);
  assert.deepEqual(Object.keys(release.assets).sort(), [
    "linux-aarch64",
    "linux-x86_64",
    "macos-aarch64",
    "macos-x86_64",
  ]);
  for (const [name, { url, sha256: digest }] of Object.entries(release.assets)) {
    assert.equal(url, `https://github.com/herdrdev/herdr/releases/download/v0.9.5/herdr-${name}`);
    assert.match(digest, /^[0-9a-f]{64}$/);
  }
});

test("--asset host maps this machine to a release asset", () => {
  assert.equal(assetForHost("darwin", "arm64"), "macos-aarch64");
  assert.equal(assetForHost("darwin", "x64"), "macos-x86_64");
  assert.equal(assetForHost("linux", "x64"), "linux-x86_64");
  assert.equal(assetForHost("linux", "arm64"), "linux-aarch64");
});

test("hosts herdr does not ship for are refused by name", () => {
  assert.throws(() => assetForHost("win32", "x64"), /no herdr asset for windows/);
  assert.throws(() => assetForHost("linux", "ia32"), /no herdr asset for linux\/ia32/);
  assert.throws(() => assetForHost("freebsd", "x64"), /no herdr asset for freebsd\/x64/);
});

test("a server that never answers is cut off by the timeout", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "fetch-herdr-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const sockets = new Set();
  const server = createServer(() => {});
  server.on("connection", (socket) => sockets.add(socket));
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => {
    for (const socket of sockets) socket.destroy();
    server.close();
  });
  const release = join(directory, "release.json");
  await writeFile(
    release,
    JSON.stringify({ assets: { fake: { url: `http://127.0.0.1:${server.address().port}/herdr`, sha256: sha256("x") } } }),
  );
  const target = join(directory, "herdr");
  const started = Date.now();
  const ran = await run(["--release", release, "--asset", "fake", "--out", target, "--timeout", "300"]);
  assert.notEqual(ran.status, 0);
  assert.match(ran.stderr, /timed out after 300 ms/);
  assert.ok(Date.now() - started < 10_000);
  await assert.rejects(() => stat(target), { code: "ENOENT" });
});

test("--reuse keeps a matching file without downloading and replaces a stale one", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "fetch-herdr-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const body = Buffer.from("the real herdr");
  let requests = 0;
  const server = createServer((request, response) => {
    requests += 1;
    response.end(body);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => new Promise((resolve) => server.close(resolve)));
  const release = join(directory, "release.json");
  await writeFile(
    release,
    JSON.stringify({ assets: { fake: { url: `http://127.0.0.1:${server.address().port}/herdr`, sha256: sha256(body) } } }),
  );
  const target = join(directory, "herdr");
  const args = ["--release", release, "--asset", "fake", "--out", target, "--reuse"];

  await writeFile(target, "stale or tampered");
  let ran = await run(args);
  assert.equal(ran.status, 0, ran.stderr);
  assert.equal(requests, 1, "a file that does not match is downloaded again");
  assert.deepEqual(await readFile(target), body);

  ran = await run(args);
  assert.equal(ran.status, 0, ran.stderr);
  assert.equal(requests, 1, "a matching file is not downloaded again");

  // Without --reuse the download always happens.
  ran = await run(args.slice(0, -1));
  assert.equal(ran.status, 0, ran.stderr);
  assert.equal(requests, 2);
});
