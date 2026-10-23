const fs = require("node:fs");

function releaseChannel(tag) {
  const match = /^v\d+\.\d+\.\d+(-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.exec(tag || "");
  if (!match) throw new Error("Expected a release tag such as v0.9.1 or v0.9.1-beta.1");
  const prerelease = Boolean(match[1]);
  return { prerelease, feedKey: prerelease ? "beta/latest.json" : "latest.json" };
}

// GitHub's /releases/latest skips prereleases, so the stable feed can live on
// GitHub Releases. The beta feed (beta/latest.json) stays R2-only.
const GITHUB_STABLE_FEED = "https://github.com/0xvasanth/rebase/releases/latest/download/latest.json";

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

// Rebase Native has its own fixed feed: the `native-latest` release holds only
// latest.json (overwritten per native-v* release), so it never competes with
// the Tauri app for /releases/latest. Packages stay on the versioned native-v*
// release, which latest.json points at.
const GITHUB_RELEASES = "https://github.com/0xvasanth/rebase/releases/download";
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

module.exports = {
  releaseChannel, updaterEndpoint, updaterPubkey, r2Status, GITHUB_STABLE_FEED,
  GITHUB_NATIVE_FEED, nativeUpdaterEndpoint, nativeAssetBase, nativeR2Status,
};

if (require.main === module && process.argv[2] === "native-config") {
  if (!process.env.GITHUB_ENV) throw new Error("GITHUB_ENV is required");
  const pubkey = updaterPubkey(process.env);
  const endpoint = nativeUpdaterEndpoint(process.env);
  fs.appendFileSync(process.env.GITHUB_ENV, `REBASE_NATIVE_UPDATER_ENDPOINT=${endpoint}\nREBASE_NATIVE_UPDATER_PUBKEY=${pubkey}\n`);
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
