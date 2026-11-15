#!/usr/bin/env node
// Updates the website to the latest published Gespan release.
//
//   bun run sync-release              latest published release
//   bun run sync-release -- v0.3.0    a specific published release
//   bun run sync-release -- --check   fail if src/release.json is behind
//
// It asks GitHub for the release, checks it has every desktop download, and
// writes src/release.json, which the site builds from. A draft (a release
// still in the pipeline) is refused: its downloads aren't public yet.
// Set GH_TOKEN or GITHUB_TOKEN to avoid GitHub's anonymous rate limit.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { compareVersions, releaseFromGitHub } from "./release.mjs";

const REPO = "0xvasanth/gespan";
export const RELEASE_FILE = join(dirname(fileURLToPath(import.meta.url)), "../src/release.json");

/** Fetches a release from GitHub's API: the latest published one, or a tag. */
export async function fetchRelease(tag, { fetchImpl = fetch, token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN } = {}) {
  const path = tag ? `releases/tags/${encodeURIComponent(tag.startsWith("v") ? tag : `v${tag}`)}` : "releases/latest";
  const response = await fetchImpl(`https://api.github.com/repos/${REPO}/${path}`, {
    headers: { accept: "application/vnd.github+json", "user-agent": "gespan-website-sync", ...(token ? { authorization: `Bearer ${token}` } : {}) },
  });
  if (response.status === 404) throw new Error(tag ? `no published release ${tag} (a draft isn't public yet)` : "no published release");
  if (!response.ok) throw new Error(`GitHub answered ${response.status} for ${path}`);
  return releaseFromGitHub(await response.json());
}

/** Writes the release record; returns what changed, for the summary line. */
export function writeRelease(release, file = RELEASE_FILE) {
  let previous = null;
  try {
    previous = JSON.parse(readFileSync(file, "utf8"));
  } catch {
    // First run: no file yet.
  }
  const next = `${JSON.stringify(release, null, 2)}\n`;
  const changed = !previous || JSON.stringify(previous) !== JSON.stringify(release);
  if (changed) writeFileSync(file, next);
  return { previous: previous?.version ?? null, next: release.version, changed };
}

async function main(argv) {
  const check = argv.includes("--check");
  const tag = argv.find((a) => !a.startsWith("--"));
  const release = await fetchRelease(tag);
  if (check) {
    const current = JSON.parse(readFileSync(RELEASE_FILE, "utf8"));
    if (compareVersions(current.version, release.version) < 0) {
      console.error(`The website is on ${current.version}; ${release.version} is published. Run bun run sync-release.`);
      process.exit(1);
    }
    console.log(`The website is on ${current.version}, the latest published release.`);
    return;
  }
  const result = writeRelease(release);
  console.log(result.changed ? `Website release: ${result.previous ?? "none"} → ${result.next} (${release.date}). Commit src/release.json.` : `The website is already on ${result.next}.`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(`sync-release: ${error.message}`);
    process.exit(1);
  });
}
