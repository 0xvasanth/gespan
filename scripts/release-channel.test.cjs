const assert = require("node:assert/strict");
const { test } = require("node:test");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { releaseChannel, updaterEndpoint, updaterPubkey, r2Status, GITHUB_STABLE_FEED, GITHUB_BETA_FEED, GITHUB_NATIVE_FEED, nativeUpdaterEndpoint, nativeAssetBase, nativeR2Status, requireNativeTagRef, nativeExpectedFiles, nativePublishPlan, verifyRecoveredFeed } = require("./release-channel.cjs");

test("stable tags, including hyphens in build metadata, use the stable feed", () => {
  for (const tag of ["v0.9.1", "v0.9.1+build-beta"]) {
    assert.deepEqual(releaseChannel(tag), { prerelease: false, feedKey: "latest.json" });
  }
});

test("prerelease tags use only the beta feed", () => {
  for (const tag of ["v0.9.1-beta", "v0.9.1-beta.1+build"]) {
    assert.deepEqual(releaseChannel(tag), { prerelease: true, feedKey: "beta/latest.json" });
  }
});

test("non-release refs are rejected", () => {
  for (const tag of ["main", "vnext", "", "v0.9.1/beta"]) {
    assert.throws(() => releaseChannel(tag), /Expected a release tag/);
  }
});

test("stable builds preserve the configured endpoint", () => {
  assert.equal(updaterEndpoint({
    GITHUB_REF_NAME: "v0.9.1",
    TAURI_UPDATER_ENDPOINT: "https://updates.example.com/stable.json",
    R2_PUBLIC_BASE_URL: "https://cdn.example.com",
  }), "https://updates.example.com/stable.json");
});

test("stable builds default to the GitHub Releases feed, even when R2 is configured", () => {
  assert.equal(GITHUB_STABLE_FEED, "https://github.com/0xvasanth/gespan/releases/latest/download/latest.json");
  assert.equal(updaterEndpoint({ GITHUB_REF_NAME: "v0.9.1" }), GITHUB_STABLE_FEED);
  assert.equal(updaterEndpoint({ GITHUB_REF_NAME: "v0.9.1", TAURI_UPDATER_ENDPOINT: "  " }), GITHUB_STABLE_FEED);
  assert.equal(updaterEndpoint({
    GITHUB_REF_NAME: "v0.9.1",
    R2_PUBLIC_BASE_URL: "https://cdn.example.com/",
  }), GITHUB_STABLE_FEED);
});

test("a non-HTTPS stable override is rejected", () => {
  assert.throws(() => updaterEndpoint({
    GITHUB_REF_NAME: "v0.9.1",
    TAURI_UPDATER_ENDPOINT: "http://updates.example.com/latest.json",
  }), /must use HTTPS/);
});

test("the pubkey is required and the error names the secret and how to generate it", () => {
  assert.equal(updaterPubkey({ TAURI_UPDATER_PUBKEY: " abc \n" }), "abc");
  for (const env of [{}, { TAURI_UPDATER_PUBKEY: "" }, { TAURI_UPDATER_PUBKEY: "  \n" }]) {
    assert.throws(() => updaterPubkey(env), /TAURI_UPDATER_PUBKEY.*bunx tauri signer generate/s);
  }
});

test("beta builds override a configured production endpoint", () => {
  assert.equal(updaterEndpoint({
    GITHUB_REF_NAME: "v0.9.1-beta.1",
    TAURI_UPDATER_ENDPOINT: "https://updates.example.com/stable.json",
    R2_PUBLIC_BASE_URL: "https://cdn.example.com/",
  }), "https://cdn.example.com/beta/latest.json");
});

test("beta builds without R2 use the GitHub beta feed, never the production endpoint", () => {
  assert.equal(GITHUB_BETA_FEED, "https://github.com/0xvasanth/gespan/releases/download/beta-latest/latest.json");
  assert.equal(updaterEndpoint({
    GITHUB_REF_NAME: "v0.9.1-beta.1",
    TAURI_UPDATER_ENDPOINT: "https://updates.example.com/stable.json",
  }), GITHUB_BETA_FEED);
  // Kept apart from the stable and native feeds.
  assert.notEqual(GITHUB_BETA_FEED, GITHUB_STABLE_FEED);
  assert.notEqual(GITHUB_BETA_FEED, GITHUB_NATIVE_FEED);
});

test("an R2 beta endpoint must use HTTPS", () => {
  assert.throws(() => updaterEndpoint({
    GITHUB_REF_NAME: "v0.9.1-beta.1",
    R2_PUBLIC_BASE_URL: "http://cdn.example.com",
  }), /must use HTTPS/);
});

test("the CLI exports the same channel used for feed publication", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "gespan-release-channel-"));
  try {
    const envFile = path.join(dir, "github-env");
    execFileSync(process.execPath, [path.join(__dirname, "release-channel.cjs")], {
      env: { ...process.env, GITHUB_REF_NAME: "v0.9.1-beta.1", GITHUB_ENV: envFile },
    });
    assert.equal(fs.readFileSync(envFile, "utf8"), "RELEASE_PRERELEASE=true\nUPDATER_FEED_KEY=beta/latest.json\n");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

const R2_NAMES = ["R2_PUBLIC_BASE_URL", "R2_ACCOUNT_ID", "R2_BUCKET_NAME", "R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY"];
const fullR2 = {
  R2_PUBLIC_BASE_URL: "https://cdn.example.com",
  R2_ACCOUNT_ID: "acct",
  R2_BUCKET_NAME: "bucket",
  R2_ACCESS_KEY_ID: "key",
  R2_SECRET_ACCESS_KEY: "secret",
};

test("stable releases treat R2 as optional, enabled only when fully configured", () => {
  assert.deepEqual(r2Status({ GITHUB_REF_NAME: "v0.9.1", ...fullR2 }), { enabled: true, missing: [] });
  const partial = { GITHUB_REF_NAME: "v0.9.1", R2_PUBLIC_BASE_URL: "https://cdn.example.com", R2_BUCKET_NAME: " " };
  const status = r2Status(partial);
  assert.equal(status.enabled, false);
  assert.deepEqual(status.missing, ["R2_ACCOUNT_ID", "R2_BUCKET_NAME", "R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY"]);
  assert.equal(r2Status({ GITHUB_REF_NAME: "v0.9.1" }).enabled, false);
});

test("beta releases use R2 fully or not at all, naming every missing secret", () => {
  assert.equal(r2Status({ GITHUB_REF_NAME: "v0.9.1-beta.1", ...fullR2 }).enabled, true);
  const { R2_BUCKET_NAME, R2_SECRET_ACCESS_KEY, ...rest } = fullR2;
  assert.throws(
    () => r2Status({ GITHUB_REF_NAME: "v0.9.1-beta.1", ...rest }),
    /Beta releases need all R2 secrets or none; missing: R2_BUCKET_NAME, R2_SECRET_ACCESS_KEY/,
  );
  assert.deepEqual(r2Status({ GITHUB_REF_NAME: "v0.9.1-beta.1" }), { enabled: false, missing: R2_NAMES });
});

test("the r2 CLI exports R2_ENABLED and fails for an incomplete beta", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "gespan-r2-"));
  try {
    const envFile = path.join(dir, "github-env");
    const run = (env) => execFileSync(process.execPath, [path.join(__dirname, "release-channel.cjs"), "r2"], {
      env: { PATH: process.env.PATH, GITHUB_ENV: envFile, ...env },
      stdio: "pipe",
    });
    run({ GITHUB_REF_NAME: "v0.9.1" });
    assert.equal(fs.readFileSync(envFile, "utf8"), "R2_ENABLED=false\n");
    run({ GITHUB_REF_NAME: "v0.9.1-beta.1" });
    assert.equal(fs.readFileSync(envFile, "utf8"), "R2_ENABLED=false\nR2_ENABLED=false\n");
    assert.throws(() => run({ GITHUB_REF_NAME: "v0.9.1-beta.1", R2_ACCOUNT_ID: "acct" }), /Command failed/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("the native feed is the fixed native-latest release, separate from Tauri's releases/latest", () => {
  assert.equal(GITHUB_NATIVE_FEED, "https://github.com/0xvasanth/gespan/releases/download/native-latest/latest.json");
  assert.notEqual(GITHUB_NATIVE_FEED, GITHUB_STABLE_FEED);
  // R2 and the Tauri override never leak into the native endpoint.
  assert.equal(nativeUpdaterEndpoint({}), GITHUB_NATIVE_FEED);
  assert.equal(
    nativeUpdaterEndpoint({
      GITHUB_REF_NAME: "native-v0.16.0-preview.2",
      R2_PUBLIC_BASE_URL: "https://cdn.example.com",
      TAURI_UPDATER_ENDPOINT: "https://updates.example.com/stable.json",
      NATIVE_UPDATER_ENDPOINT: "  ",
    }),
    GITHUB_NATIVE_FEED,
  );
});

test("the native endpoint can be overridden, but only with HTTPS", () => {
  assert.equal(
    nativeUpdaterEndpoint({ NATIVE_UPDATER_ENDPOINT: "https://mirror.example.com/native.json" }),
    "https://mirror.example.com/native.json",
  );
  assert.throws(() => nativeUpdaterEndpoint({ NATIVE_UPDATER_ENDPOINT: "http://mirror.example.com/native.json" }), /HTTPS/);
});

test("native packages are referenced from the versioned native-v release, not the feed release", () => {
  assert.equal(
    nativeAssetBase({ GITHUB_REF_NAME: "native-v0.16.0-preview.2" }),
    "https://github.com/0xvasanth/gespan/releases/download/native-v0.16.0-preview.2",
  );
  for (const ref of ["v0.9.1", "native-latest", "native-v", "", undefined]) {
    assert.throws(() => nativeAssetBase({ GITHUB_REF_NAME: ref }), /native-v/);
  }
});

test("native R2 mirroring is optional and only enabled when every secret is present", () => {
  assert.deepEqual(nativeR2Status({}), { enabled: false, missing: R2_NAMES });
  assert.deepEqual(nativeR2Status(fullR2), { enabled: true, missing: [] });
  const { R2_BUCKET_NAME, ...rest } = fullR2;
  assert.deepEqual(nativeR2Status(rest), { enabled: false, missing: ["R2_BUCKET_NAME"] });
  assert.equal(nativeR2Status({ ...fullR2, R2_ACCOUNT_ID: "  " }).enabled, false);
});

test("native builds still require the updater pubkey", () => {
  assert.throws(() => updaterPubkey({}), /TAURI_UPDATER_PUBKEY/);
});

test("the native-config CLI exports the endpoint and fails without a pubkey", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "native-config-"));
  const envFile = path.join(dir, "env");
  fs.writeFileSync(envFile, "");
  const run = (env) => execFileSync(process.execPath, [path.join(__dirname, "release-channel.cjs"), "native-config"], {
    env: { PATH: process.env.PATH, GITHUB_ENV: envFile, ...env }, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
  });
  assert.throws(() => run({}), (error) => /TAURI_UPDATER_PUBKEY/.test(error.stderr));
  run({ TAURI_UPDATER_PUBKEY: "pub" });
  const written = fs.readFileSync(envFile, "utf8");
  assert.match(written, new RegExp(`GESPAN_NATIVE_UPDATER_ENDPOINT=${GITHUB_NATIVE_FEED}\n`));
  assert.match(written, /GESPAN_NATIVE_UPDATER_PUBKEY=pub\n/);
});

test("the native-r2 CLI exports R2_ENABLED=false when secrets are absent", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "native-r2-"));
  const envFile = path.join(dir, "env");
  fs.writeFileSync(envFile, "");
  execFileSync(process.execPath, [path.join(__dirname, "release-channel.cjs"), "native-r2"], {
    env: { PATH: process.env.PATH, GITHUB_ENV: envFile }, encoding: "utf8",
  });
  assert.equal(fs.readFileSync(envFile, "utf8"), "R2_ENABLED=false\n");
});

test("native releases only run from a native-v* tag", () => {
  assert.equal(requireNativeTagRef({ GITHUB_REF_TYPE: "tag", GITHUB_REF_NAME: "native-v0.16.0-preview.2" }), "native-v0.16.0-preview.2");
  for (const env of [
    { GITHUB_REF_TYPE: "branch", GITHUB_REF_NAME: "main" },
    { GITHUB_REF_TYPE: "branch", GITHUB_REF_NAME: "native-v0.16.0-preview.2" },
    { GITHUB_REF_TYPE: "tag", GITHUB_REF_NAME: "v0.9.1" },
    { GITHUB_REF_TYPE: "tag", GITHUB_REF_NAME: "native-v" },
    {},
  ]) assert.throws(() => requireNativeTagRef(env), /native-v\* tag/);
});

test("expected native files are each package plus its signature", () => {
  assert.deepEqual(
    nativeExpectedFiles([{ filename: "a.dmg", target: "darwin-aarch64-app" }, { filename: "b.deb" }]),
    ["latest.json", "a.dmg", "a.dmg.sig", "b.deb"],
  );
});

test("a missing or draft versioned release uploads packages", () => {
  const expected = ["latest.json", "a.dmg", "a.dmg.sig"];
  assert.deepEqual(nativePublishPlan({ release: null, expected }), { createRelease: true, uploadPackages: true });
  assert.deepEqual(nativePublishPlan({ release: { isDraft: true, assets: [{ name: "a.dmg" }] }, expected }), { createRelease: false, uploadPackages: true });
});

test("a published versioned release keeps its packages and only the feed is refreshed", () => {
  const expected = ["latest.json", "a.dmg", "a.dmg.sig"];
  const release = { isDraft: false, assets: [{ name: "latest.json" }, { name: "a.dmg" }, { name: "a.dmg.sig" }, { name: "extra.txt" }] };
  assert.deepEqual(nativePublishPlan({ release, expected }), { createRelease: false, uploadPackages: false });
});

test("a published release with missing assets is an error naming them", () => {
  const release = { isDraft: false, assets: [{ name: "a.dmg" }] };
  assert.throws(() => nativePublishPlan({ release, expected: ["a.dmg", "a.dmg.sig"] }), /missing assets: a\.dmg\.sig/);
  assert.throws(() => nativePublishPlan({ release: { isDraft: false, assets: [{ name: "a.dmg" }] }, expected: ["latest.json", "a.dmg"] }), /missing assets: latest\.json/);
  assert.throws(() => nativePublishPlan({ release: { isDraft: false }, expected: ["a.dmg"] }), /missing assets: a\.dmg/);
});

const TAG = "native-v0.16.0-preview.2";
const feedFor = (platforms) => JSON.stringify({
  payload: Buffer.from(JSON.stringify({ platforms })).toString("base64"),
  signature: "manifest-signature",
});
const url = (name, tag = TAG) => `https://github.com/0xvasanth/gespan/releases/download/${tag}/${encodeURIComponent(name)}`;

test("recovery reuses the original signatures and signs nothing", () => {
  const latestJson = feedFor({ "darwin-aarch64-app": { url: url("Gespan Native.app.tar.gz"), signature: "sig-original" } });
  const before = latestJson;
  const manifest = verifyRecoveredFeed({
    latestJson,
    sigs: { "Gespan Native.app.tar.gz": "sig-original\n" },
    tag: TAG,
    expected: ["Gespan Native.app.tar.gz"],
  });
  assert.equal(manifest.platforms["darwin-aarch64-app"].signature, "sig-original");
  assert.equal(latestJson, before);
});

test("the workflow exposes the signing key only to the normal-path signing step", () => {
  const workflow = fs.readFileSync(path.join(__dirname, "..", ".github", "workflows", "native-release.yml"), "utf8");
  const publish = workflow.slice(workflow.indexOf("\n  publish:"));
  const holders = publish.split("\n      - ").filter((step) => step.includes("TAURI_SIGNING_PRIVATE_KEY"));
  assert.equal(holders.length, 1);
  assert.match(holders[0], /^name: Sign all update packages/);
  assert.match(holders[0], /if: env\.NATIVE_UPLOAD_PACKAGES == 'true'/);
  assert.doesNotMatch(publish.split("\n    steps:")[0], /TAURI_SIGNING_PRIVATE_KEY/);
});

test("recovery fails when a .sig asset is missing or differs from the feed", () => {
  const latestJson = feedFor({ t: { url: url("a.dmg"), signature: "sig-original" } });
  const args = { latestJson, tag: TAG, expected: ["a.dmg"] };
  assert.throws(() => verifyRecoveredFeed({ ...args, sigs: {} }), /Missing signature asset a\.dmg\.sig/);
  assert.throws(() => verifyRecoveredFeed({ ...args, sigs: { "a.dmg": "swapped" } }), /does not match a\.dmg\.sig/);
});

test("recovery rejects a feed that points elsewhere, at an unknown file, or is unreadable", () => {
  const sigs = { "a.dmg": "s" };
  assert.throws(() => verifyRecoveredFeed({ latestJson: feedFor({ t: { url: url("a.dmg", "native-v9.9.9"), signature: "s" } }), sigs, tag: TAG, expected: ["a.dmg"] }), /does not point at/);
  assert.throws(() => verifyRecoveredFeed({ latestJson: feedFor({ t: { url: url("evil.dmg"), signature: "s" } }), sigs, tag: TAG, expected: ["a.dmg"] }), /not a published package/);
  assert.throws(() => verifyRecoveredFeed({ latestJson: "not json", sigs, tag: TAG, expected: [] }), /unreadable/);
  assert.throws(() => verifyRecoveredFeed({ latestJson: feedFor({}), sigs, tag: TAG, expected: [] }), /lists no packages/);
});
