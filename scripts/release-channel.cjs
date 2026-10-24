const fs = require("node:fs");

function releaseChannel(tag) {
  const match = /^v\d+\.\d+\.\d+(-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.exec(tag || "");
  if (!match) throw new Error("Expected a release tag such as v0.9.1 or v0.9.1-beta.1");
  const prerelease = Boolean(match[1]);
  return { prerelease, feedKey: prerelease ? "beta/latest.json" : "latest.json" };
}

// GitHub's /releases/latest skips prereleases, so the stable feed can live on
// GitHub Releases. The beta feed (beta/latest.json) stays R2-only.
const GITHUB_STABLE_FEED = "https://github.com/0xvasanth/gespan/releases/latest/download/latest.json";

function updaterEndpoint(env) {
  const { prerelease, feedKey } = releaseChannel(env.GITHUB_REF_NAME);
  const base = (env.R2_PUBLIC_BASE_URL || "").trim().replace(/\/$/, "");
  const configured = (env.TAURI_UPDATER_ENDPOINT || "").trim();
  // Beta builds must never retain a configured production endpoint.
  let endpoint;
  if (prerelease) endpoint = base ? `${base}/${feedKey}` : "";
  else endpoint = configured || GITHUB_STABLE_FEED;
  if (!endpoint) throw new Error("R2_PUBLIC_BASE_URL is required for beta updates; stable builds default to GitHub Releases");
  if (new URL(endpoint).protocol !== "https:") throw new Error("Release updater endpoints must use HTTPS");
  return endpoint;
}

function updaterPubkey(env) {
  const pubkey = (env.TAURI_UPDATER_PUBKEY || "").trim();
  if (!pubkey) {
    throw new Error(
      "The TAURI_UPDATER_PUBKEY repository secret is missing. Generate a key pair with `bunx tauri signer generate`, " +
        "store the public key as TAURI_UPDATER_PUBKEY and the private key as TAURI_SIGNING_PRIVATE_KEY (plus TAURI_SIGNING_PRIVATE_KEY_PASSWORD).",
    );
  }
  return pubkey;
}

const R2_REQUIRED = ["R2_PUBLIC_BASE_URL", "R2_ACCOUNT_ID", "R2_BUCKET_NAME", "R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY"];

// R2 is optional for stable releases (GitHub Releases is the default feed) but
// required for beta, whose feed is R2-only. One rule shared by every step.
function r2Status(env) {
  const { prerelease } = releaseChannel(env.GITHUB_REF_NAME);
  const missing = R2_REQUIRED.filter((name) => !(env[name] || "").trim());
  if (prerelease && missing.length) {
    throw new Error(`Beta releases require all R2 secrets; missing: ${missing.join(", ")}`);
  }
  return { enabled: missing.length === 0, missing };
}

// Gespan Native has its own fixed feed: the `native-latest` release holds only
// latest.json (overwritten per native-v* release), so it never competes with
// the Tauri app for /releases/latest. Packages stay on the versioned native-v*
// release, which latest.json points at.
const GITHUB_RELEASES = "https://github.com/0xvasanth/gespan/releases/download";
const GITHUB_NATIVE_FEED = `${GITHUB_RELEASES}/native-latest/latest.json`;

function nativeUpdaterEndpoint(env) {
  const endpoint = (env.NATIVE_UPDATER_ENDPOINT || "").trim() || GITHUB_NATIVE_FEED;
  if (new URL(endpoint).protocol !== "https:") throw new Error("Native updater endpoints must use HTTPS");
  return endpoint;
}

function nativeAssetBase(env) {
  const tag = env.GITHUB_REF_NAME || "";
  if (!/^native-v.+/.test(tag)) throw new Error("Expected a native release tag such as native-v0.16.0-preview.2");
  return `${GITHUB_RELEASES}/${tag}`;
}

// R2 only mirrors the native packages; it is optional and enabled when complete.
function nativeR2Status(env) {
  const missing = R2_REQUIRED.filter((name) => !(env[name] || "").trim());
  return { enabled: missing.length === 0, missing };
}

// Manual dispatch must start from a native-v* tag, never a branch. GitHub sets
// GITHUB_REF_TYPE and GITHUB_REF_NAME to the tag a "Use workflow from" tag run uses.
function requireNativeTagRef(env) {
  const name = env.GITHUB_REF_NAME || "";
  if (env.GITHUB_REF_TYPE !== "tag" || !/^native-v.+/.test(name)) {
    throw new Error(
      `Run this workflow from a native-v* tag (Use workflow from -> Tags), not "${name || "(none)"}" (${env.GITHUB_REF_TYPE || "unknown"}).`,
    );
  }
  return name;
}

// Files the versioned release must hold: every package and its .sig, plus the
// signed latest.json the original run produced. Recovery republishes that exact
// file; it never signs anything, so the signing key cannot be used as an oracle
// to certify assets someone replaced on the release.
function nativeExpectedFiles(records) {
  const files = ["latest.json"];
  for (const record of records) {
    files.push(record.filename);
    if (record.target) files.push(`${record.filename}.sig`);
  }
  return files;
}

// A published native-v* release is immutable, but the native-latest feed can
// be rewritten. So a rerun after a failure between the two steps may repair the
// feed without touching the published packages.
//   release: null (absent) or { isDraft, assets: [{ name }] } from `gh release view`.
function nativePublishPlan({ release, expected }) {
  if (!release) return { createRelease: true, uploadPackages: true };
  if (release.isDraft) return { createRelease: false, uploadPackages: true };
  const have = new Set((release.assets || []).map((asset) => asset.name));
  const missing = expected.filter((name) => !have.has(name));
  if (missing.length) {
    throw new Error(
      `The published release is immutable but is missing assets: ${missing.join(", ")}. Use a new preview tag.`,
    );
  }
  return { createRelease: false, uploadPackages: false };
}

// Checks a latest.json taken from a published versioned release against that
// release's own .sig assets, so a recovery republishes it only when it is
// consistent. The client still verifies every package against these signatures
// and the manifest against the updater public key.
//   sigs: { "<package filename>": "<contents of its .sig asset>" }
function verifyRecoveredFeed({ latestJson, sigs, tag, expected }) {
  let manifest;
  try {
    const feed = JSON.parse(latestJson);
    if (!feed.signature || !feed.payload) throw new Error("no payload or signature");
    manifest = JSON.parse(Buffer.from(feed.payload, "base64").toString("utf8"));
  } catch (error) {
    throw new Error(`The published latest.json is unreadable: ${error.message}`);
  }
  const base = `${GITHUB_RELEASES}/${tag}/`;
  const targets = Object.entries(manifest.platforms || {});
  if (!targets.length) throw new Error("The published latest.json lists no packages");
  for (const [target, platform] of targets) {
    if (typeof platform.url !== "string" || !platform.url.startsWith(base)) {
      throw new Error(`${target} does not point at the ${tag} release`);
    }
    const filename = decodeURIComponent(platform.url.slice(base.length));
    if (!expected.includes(filename)) throw new Error(`${target} lists ${filename}, which is not a published package`);
    const sig = (sigs[filename] || "").trim();
    if (!sig) throw new Error(`Missing signature asset ${filename}.sig`);
    if (sig !== (platform.signature || "").trim()) {
      throw new Error(`The signature for ${target} does not match ${filename}.sig`);
    }
  }
  return manifest;
}

module.exports = {
  verifyRecoveredFeed, requireNativeTagRef, nativeExpectedFiles, nativePublishPlan,
  releaseChannel, updaterEndpoint, updaterPubkey, r2Status, GITHUB_STABLE_FEED,
  GITHUB_NATIVE_FEED, nativeUpdaterEndpoint, nativeAssetBase, nativeR2Status,
};

if (require.main === module && process.argv[2] === "native-ref") {
  console.log(`Native release ref: ${requireNativeTagRef(process.env)}`);
} else if (require.main === module && process.argv[2] === "native-publish-plan") {
  // usage: native-publish-plan <native-dist> <packages-out>; NATIVE_RELEASE_JSON is empty when the release is absent.
  const dir = process.argv[3];
  const records = fs.readdirSync(dir).filter((n) => /^artifacts-.*\.json$/.test(n))
    .flatMap((n) => JSON.parse(fs.readFileSync(`${dir}/${n}`, "utf8")));
  const expected = nativeExpectedFiles(records);
  const raw = (process.env.NATIVE_RELEASE_JSON || "").trim();
  const plan = nativePublishPlan({ release: raw ? JSON.parse(raw) : null, expected });
  if (!process.env.GITHUB_ENV) throw new Error("GITHUB_ENV is required");
  fs.writeFileSync(process.argv[4], expected.filter((n) => !n.endsWith(".sig")).join("\n") + "\n");
  fs.appendFileSync(process.env.GITHUB_ENV, `NATIVE_CREATE_RELEASE=${plan.createRelease}\nNATIVE_UPLOAD_PACKAGES=${plan.uploadPackages}\n`);
  console.log(plan.uploadPackages ? "Uploading packages to the versioned release" : "Versioned release already published: keeping its packages, refreshing the feed only");
} else if (require.main === module && process.argv[2] === "native-verify-recovery") {
  // usage: native-verify-recovery <dir holding the downloaded latest.json and *.sig>
  const dir = process.argv[3];
  const sigs = {};
  for (const name of fs.readdirSync(dir)) if (name.endsWith(".sig")) sigs[name.slice(0, -4)] = fs.readFileSync(`${dir}/${name}`, "utf8");
  const expected = Object.keys(sigs);
  verifyRecoveredFeed({ latestJson: fs.readFileSync(`${dir}/latest.json`, "utf8"), sigs, tag: requireNativeTagRef(process.env), expected });
  console.log("Recovered feed matches the published signatures");
} else if (require.main === module && process.argv[2] === "native-config") {
  if (!process.env.GITHUB_ENV) throw new Error("GITHUB_ENV is required");
  const pubkey = updaterPubkey(process.env);
  const endpoint = nativeUpdaterEndpoint(process.env);
  fs.appendFileSync(process.env.GITHUB_ENV, `GESPAN_NATIVE_UPDATER_ENDPOINT=${endpoint}\nGESPAN_NATIVE_UPDATER_PUBKEY=${pubkey}\n`);
  console.log(`Native updater endpoint: ${endpoint}`);
} else if (require.main === module && process.argv[2] === "native-r2") {
  const { enabled, missing } = nativeR2Status(process.env);
  if (!process.env.GITHUB_ENV) throw new Error("GITHUB_ENV is required");
  fs.appendFileSync(process.env.GITHUB_ENV, `R2_ENABLED=${enabled}\n`);
  console.log(enabled ? "R2 mirror enabled" : `R2 mirror disabled (missing: ${missing.join(", ")})`);
} else if (require.main === module && process.argv[2] === "r2") {
  const { enabled, missing } = r2Status(process.env);
  if (!process.env.GITHUB_ENV) throw new Error("GITHUB_ENV is required");
  fs.appendFileSync(process.env.GITHUB_ENV, `R2_ENABLED=${enabled}\n`);
  console.log(enabled ? "R2 enabled" : `R2 disabled (missing: ${missing.join(", ")})`);
} else if (require.main === module) {
  const { prerelease, feedKey } = releaseChannel(process.env.GITHUB_REF_NAME);
  if (!process.env.GITHUB_ENV) throw new Error("GITHUB_ENV is required");
  fs.appendFileSync(process.env.GITHUB_ENV, `RELEASE_PRERELEASE=${prerelease}\nUPDATER_FEED_KEY=${feedKey}\n`);
  console.log(`Release channel: ${prerelease ? "beta" : "stable"} (${feedKey})`);
}
