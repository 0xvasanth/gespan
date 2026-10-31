const fs = require("node:fs");

function releaseChannel(tag) {
  const match = /^v\d+\.\d+\.\d+(-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.exec(tag || "");
  if (!match) throw new Error("Expected a release tag such as v0.9.1 or v0.9.1-beta.1");
  const prerelease = Boolean(match[1]);
  return { prerelease, feedKey: prerelease ? "beta/latest.json" : "latest.json" };
}

// GitHub's /releases/latest skips prereleases, so the stable feed can live on
// GitHub Releases. The beta feed lives on R2 when R2 is configured; otherwise
// on the fixed beta-latest release, which holds only that feed (as
// native-latest does for Gespan Native), so it never becomes releases/latest.
const GITHUB_STABLE_FEED = "https://github.com/0xvasanth/gespan/releases/latest/download/latest.json";
const GITHUB_BETA_FEED = "https://github.com/0xvasanth/gespan/releases/download/beta-latest/latest.json";

function updaterEndpoint(env) {
  const { prerelease, feedKey } = releaseChannel(env.GITHUB_REF_NAME);
  const base = (env.R2_PUBLIC_BASE_URL || "").trim().replace(/\/$/, "");
  const configured = (env.TAURI_UPDATER_ENDPOINT || "").trim();
  // Beta builds must never retain a configured production endpoint.
  let endpoint;
  if (prerelease) endpoint = base ? `${base}/${feedKey}` : GITHUB_BETA_FEED;
  else endpoint = configured || GITHUB_STABLE_FEED;
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

// R2 is optional: without it every feed is on GitHub Releases. A partial R2
// setup is a mistake for a beta, whose builds would point at the R2 feed, so
// it fails fast. One rule shared by every step.
function r2Status(env) {
  const { prerelease } = releaseChannel(env.GITHUB_REF_NAME);
  const missing = R2_REQUIRED.filter((name) => !(env[name] || "").trim());
  if (prerelease && missing.length && missing.length < R2_REQUIRED.length) {
    throw new Error(`Beta releases need all R2 secrets or none; missing: ${missing.join(", ")}`);
  }
  return { enabled: missing.length === 0, missing };
}

// Gespan Native has its own fixed feed: the `native-latest` release holds only
// latest.json (overwritten per release), so it never competes with
// /releases/latest. Packages stay on the versioned vX.Y.Z release, which
// latest.json points at.
const GITHUB_RELEASES = "https://github.com/0xvasanth/gespan/releases/download";
const GITHUB_NATIVE_FEED = `${GITHUB_RELEASES}/native-latest/latest.json`;
// Stable builds read the newest stable release's own copy of the feed. It keeps
// the name native-latest.json, because Tauri reads latest.json from this path.
const GITHUB_NATIVE_STABLE_FEED = "https://github.com/0xvasanth/gespan/releases/latest/download/native-latest.json";

function isStableTag(tag) {
  return RELEASE_TAG.test(tag || "") && !tag.includes("-");
}

function nativeUpdaterEndpoint(env) {
  const stable = isStableTag(env.GITHUB_REF_NAME);
  const configured = (stable ? env.NATIVE_STABLE_UPDATER_ENDPOINT : env.NATIVE_UPDATER_ENDPOINT) || "";
  const endpoint = configured.trim() || (stable ? GITHUB_NATIVE_STABLE_FEED : GITHUB_NATIVE_FEED);
  if (new URL(endpoint).protocol !== "https:") throw new Error("Native updater endpoints must use HTTPS");
  return endpoint;
}

// Each version is one tag and one GitHub release, vX.Y.Z: the release
// workflows run from the tag and publish into the release of the same name.
const RELEASE_TAG = /^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?$/;
function nativeReleaseTag(env) {
  const tag = env.GITHUB_REF_NAME || "";
  if (!RELEASE_TAG.test(tag)) throw new Error("Expected a release tag such as v0.0.3-beta");
  return tag;
}

function nativeAssetBase(env) {
  return `${GITHUB_RELEASES}/${nativeReleaseTag(env)}`;
}

// Where a tag's release goes. A stable release becomes GitHub's latest; a
// prerelease never does.
function nativeChannel(env) {
  const stable = isStableTag(nativeReleaseTag(env));
  return {
    prerelease: !stable,
    mirrorPrefix: stable ? "native-stable" : "native-preview",
    releaseFlags: stable ? "--prerelease=false --latest" : "--prerelease --latest=false",
  };
}

// The stable download links on the R2 root. Prereleases never move them.
function nativeFixedDownloads(env) {
  if (nativeChannel(env).prerelease) return [];
  const version = nativeReleaseTag(env).slice(1);
  return [
    { file: `Gespan_${version}_aarch64.dmg`, key: "Gespan.dmg" },
    { file: `Gespan_${version}_x64.dmg`, key: "Gespan_x64.dmg" },
  ];
}

// R2 only mirrors the native packages; it is optional and enabled when complete.
function nativeR2Status(env) {
  const missing = R2_REQUIRED.filter((name) => !(env[name] || "").trim());
  return { enabled: missing.length === 0, missing };
}

// Manual dispatch must start from a vX.Y.Z tag, never a branch. GitHub sets
// GITHUB_REF_TYPE and GITHUB_REF_NAME to the tag a "Use workflow from" tag run uses.
function requireNativeTagRef(env) {
  const name = env.GITHUB_REF_NAME || "";
  if (env.GITHUB_REF_TYPE !== "tag" || !RELEASE_TAG.test(name)) {
    throw new Error(
      `Run this workflow from a vX.Y.Z release tag (Use workflow from -> Tags), not "${name || "(none)"}" (${env.GITHUB_REF_TYPE || "unknown"}).`,
    );
  }
  return name;
}

// The signed native feed, kept on the versioned release under its own name so
// it never collides with the stable Tauri feed (latest.json) on the same release.
const NATIVE_FEED_ASSET = "native-latest.json";

// Files the versioned release must hold: every package and its .sig, plus the
// signed feed the original run produced. Recovery republishes that exact
// file; it never signs anything, so the signing key cannot be used as an oracle
// to certify assets someone replaced on the release.
function nativeExpectedFiles(records) {
  const files = [NATIVE_FEED_ASSET];
  for (const record of records) {
    files.push(record.filename);
    if (record.target) files.push(`${record.filename}.sig`);
  }
  return files;
}

// One release per version holds both apps' files, and each workflow handles
// only its own (`expected`). Whichever runs first creates the release. A
// published file is never replaced, and a workflow's files go up together, so
// an update feed never points at packages from another build:
// - no release, or a draft: upload them all
// - published without any of them (the other app published first): add them
// - published with all of them: a rerun; keep the packages, refresh feeds only
// - published with only some: an error; use a new version
//   release: null (absent) or { isDraft, assets: [{ name }] } from `gh release view`.
function releasePlan({ release, expected }) {
  if (!release) return { createRelease: true, uploadPackages: true };
  if (release.isDraft) return { createRelease: false, uploadPackages: true };
  const have = new Set((release.assets || []).map((asset) => asset.name));
  const missing = expected.filter((name) => !have.has(name));
  if (!missing.length) return { createRelease: false, uploadPackages: false };
  if (missing.length === expected.length) return { createRelease: false, uploadPackages: true };
  throw new Error(
    `The published release has only some of this build's files and is missing: ${missing.join(", ")}. Published files are never replaced; use a new version.`,
  );
}
const nativePublishPlan = releasePlan;

// Both must pass on a main commit before it can be released.
const RELEASE_CHECKS = ["CI", "Gespan Native Preview"];

// What the Release workflow does for the version on main at `sha`. One release
// per version: a published release ends it, and a draft or a tag on the same
// commit is a run to finish, so a rerun picks up where the last one failed.
//   tagSha: the commit vVERSION points at, or "" when there is no tag.
//   release: null (absent) or { isDraft } from `gh release view`.
//   runs: this commit's push runs, newest first, from `gh run list`.
function releaseDecision({ version, sha, tagSha, release, runs }) {
  const tag = `v${version}`;
  if (!RELEASE_TAG.test(tag)) throw new Error(`${version} is not a release version`);
  // A failed lookup must stop the run, not read as a tag on another commit.
  if (tagSha && !/^[0-9a-f]{40}$/.test(tagSha)) throw new Error(`Could not read the commit of ${tag}: ${tagSha}`);
  if (release && !release.isDraft) return { release: false, tag, reason: `${tag} is already published` };
  if (tagSha && tagSha !== sha) {
    return { release: false, tag, reason: `${tag} is on ${tagSha}, not ${sha}. Bump the version to release this commit.` };
  }
  for (const name of RELEASE_CHECKS) {
    const run = (runs || []).find((candidate) => candidate.workflowName === name);
    if (!run || run.conclusion !== "success") {
      return { release: false, tag, reason: `${name} has not passed on ${sha}` };
    }
  }
  return { release: true, tag, reason: `Releasing ${tag} from ${sha}` };
}

// A version's CHANGELOG section, as release notes.
function releaseNotes(changelog, version) {
  const lines = changelog.split("\n");
  const heading = new RegExp(`^## \\[${version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\]`);
  const start = lines.findIndex((line) => heading.test(line));
  if (start < 0) throw new Error(`CHANGELOG.md has no section for ${version}`);
  const end = lines.findIndex((line, ix) => ix > start && /^## \[/.test(line));
  const body = lines.slice(start + 1, end < 0 ? undefined : end).join("\n").trim();
  return `${body}\n\nFull details are in the [CHANGELOG](https://github.com/0xvasanth/gespan/blob/main/CHANGELOG.md).\n`;
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
  releaseChannel, updaterEndpoint, updaterPubkey, r2Status, GITHUB_STABLE_FEED, GITHUB_BETA_FEED,
  GITHUB_NATIVE_FEED, GITHUB_NATIVE_STABLE_FEED, nativeUpdaterEndpoint, nativeAssetBase, nativeR2Status,
  nativeChannel, nativeFixedDownloads,
  nativeReleaseTag, NATIVE_FEED_ASSET, releasePlan, releaseNotes, releaseDecision, RELEASE_CHECKS,
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
  console.log(plan.uploadPackages ? "Uploading packages to the versioned release" : "Versioned release already has these packages: keeping them, refreshing the feed only");
} else if (require.main === module && process.argv[2] === "release-plan") {
  // usage: release-plan <file listing this build's asset names>; RELEASE_JSON is empty when the release is absent.
  // Prints "<create> <upload>" for the calling step.
  const expected = fs.readFileSync(process.argv[3], "utf8").split("\n").map((n) => n.trim()).filter(Boolean);
  const raw = (process.env.RELEASE_JSON || "").trim();
  const plan = releasePlan({ release: raw ? JSON.parse(raw) : null, expected });
  console.log(`${plan.createRelease} ${plan.uploadPackages}`);
} else if (require.main === module && process.argv[2] === "release-decision") {
  // usage: release-decision; reads VERSION, SHA, TAG_SHA, RELEASE_JSON and RUNS_JSON.
  const { VERSION, SHA, TAG_SHA, RELEASE_JSON, RUNS_JSON } = process.env;
  const raw = (RELEASE_JSON || "").trim();
  const decision = releaseDecision({
    version: VERSION, sha: SHA, tagSha: (TAG_SHA || "").trim(),
    release: raw ? JSON.parse(raw) : null, runs: JSON.parse(RUNS_JSON || "[]"),
  });
  console.log(`::notice::${decision.reason}`);
  if (!process.env.GITHUB_OUTPUT) throw new Error("GITHUB_OUTPUT is required");
  fs.appendFileSync(process.env.GITHUB_OUTPUT, `release=${decision.release}\ntag=${decision.tag}\n`);
} else if (require.main === module && process.argv[2] === "release-notes") {
  // usage: release-notes <version>; prints that version's CHANGELOG section.
  process.stdout.write(releaseNotes(fs.readFileSync("CHANGELOG.md", "utf8"), process.argv[3]));
} else if (require.main === module && process.argv[2] === "native-release-tag") {
  if (!process.env.GITHUB_ENV) throw new Error("GITHUB_ENV is required");
  const tag = nativeReleaseTag(process.env);
  const { prerelease, mirrorPrefix, releaseFlags } = nativeChannel(process.env);
  fs.appendFileSync(process.env.GITHUB_ENV, `NATIVE_RELEASE_TAG=${tag}\nNATIVE_PRERELEASE=${prerelease}\nNATIVE_MIRROR_PREFIX=${mirrorPrefix}\nNATIVE_RELEASE_FLAGS=${releaseFlags}\n`);
  console.log(`Publishing into the ${tag} release (${prerelease ? "preview" : "stable"})`);
} else if (require.main === module && process.argv[2] === "native-fixed-downloads") {
  // Prints "<package> <R2 key>" per line; nothing for a prerelease.
  for (const { file, key } of nativeFixedDownloads(process.env)) console.log(`${file} ${key}`);
} else if (require.main === module && process.argv[2] === "native-verify-recovery") {
  // usage: native-verify-recovery <dir holding the downloaded native-latest.json and *.sig>
  const dir = process.argv[3];
  const sigs = {};
  for (const name of fs.readdirSync(dir)) if (name.endsWith(".sig")) sigs[name.slice(0, -4)] = fs.readFileSync(`${dir}/${name}`, "utf8");
  const expected = Object.keys(sigs);
  requireNativeTagRef(process.env);
  verifyRecoveredFeed({ latestJson: fs.readFileSync(`${dir}/${NATIVE_FEED_ASSET}`, "utf8"), sigs, tag: nativeReleaseTag(process.env), expected });
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
