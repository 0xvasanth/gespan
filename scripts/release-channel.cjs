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

module.exports = { releaseChannel, updaterEndpoint, updaterPubkey, GITHUB_STABLE_FEED };

if (require.main === module) {
  const { prerelease, feedKey } = releaseChannel(process.env.GITHUB_REF_NAME);
  if (!process.env.GITHUB_ENV) throw new Error("GITHUB_ENV is required");
  fs.appendFileSync(process.env.GITHUB_ENV, `RELEASE_PRERELEASE=${prerelease}\nUPDATER_FEED_KEY=${feedKey}\n`);
  console.log(`Release channel: ${prerelease ? "beta" : "stable"} (${feedKey})`);
}
