#!/usr/bin/env node
// Decides which CI pieces a pull request can affect, from the files it changes.
// A file no rule names runs every piece: a missing rule costs time, never
// coverage. The rules follow what each piece reads across directories, and
// scripts/ci-impact.test.mjs pins the reads that are easy to miss.
import { spawnSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export const pieces = ["core", "desktop", "host", "mobile", "release", "website"];

const rules = [
  // Nothing reads these. docs/desktop/examples is compiled into Native, and
  // Native and the release scripts read CHANGELOG.md, so neither matches.
  [/^docs\/(?!desktop\/examples\/)/, []],
  [/^(CODE_OF_CONDUCT|CONTRIBUTING|SECURITY)\.md$/, []],
  // set-version rewrites its Install section, and a release test checks it.
  // The website reads its release block for the download links.
  [/^README\.md$/, ["release", "website"]],
  [/^(LICENSE|NOTICE)$/, []],
  [/^\.github\/(ISSUE_TEMPLATE\/|pull_request_template\.md$|CODEOWNERS$)/, []],
  // Core's shared_contracts test scans crates/gespan-gpui/src.
  [/^crates\/gespan-gpui\/src\//, ["core", "desktop"]],
  [/^crates\/(gespan-gpui\/(tests|assets)|gespan-first-launch|gespan-fake-agent)\//, ["desktop"]],
  [/^crates\/(board-store|gespan-mcp-config|gespan-models|gespan-redact|gespan-speech|herdr-link|keep-awake|mcp-hub|tracker-client|tracker-secrets)\//, ["core", "desktop"]],
  // Only the root workspace builds it; once gespan-core depends on it (#386/#132) it joins the rule above.
  [/^crates\/gespan-graph\//, ["core"]],
  [/^vendor\//, ["core", "desktop"]],
  // Core's shared_contracts test reads these.
  [/^host\/(package\.mjs|windows-acl\.ps1)$/, ["core", "host"]],
  [/^host\//, ["host"]],
  [/^(mobile|tests\/mobile)\//, ["mobile"]],
  [/^website\//, ["website"]],
];

export function impact(files) {
  const affected = new Set();
  if (!files.length) return new Set(pieces);
  for (const file of files) {
    const rule = rules.find(([pattern]) => pattern.test(file));
    for (const piece of rule ? rule[1] : pieces) affected.add(piece);
  }
  return affected;
}

/**
 * What to diff the checked-out commit against, or null to run every piece.
 * A pull request's merge commit has the base as its first parent. A push to
 * main is diffed against the commit main pointed at before it, so a push of
 * several commits covers all of them. A new branch or a deleted one has no
 * usable "before", and runs everything.
 */
export function diffBase(env) {
  if (env.GITHUB_EVENT_NAME === "pull_request") return "HEAD^1";
  if (env.GITHUB_EVENT_NAME === "push" && /^[0-9a-f]{40}$/.test(env.BEFORE ?? "") && !/^0+$/.test(env.BEFORE)) return env.BEFORE;
  return null;
}

function changedFiles(base) {
  if (!base) return [];
  // The checkout is shallow; a push's "before" commit may not be in it yet.
  if (base !== "HEAD^1") spawnSync("git", ["fetch", "--no-tags", "--depth=1", "origin", base], { encoding: "utf8" });
  const result = spawnSync("git", ["diff", "--no-renames", "--name-only", base, "HEAD"], { encoding: "utf8" });
  if (result.status !== 0) return [];
  return result.stdout.split("\n").filter(Boolean);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const files = changedFiles(diffBase(process.env));
  const affected = impact(files);
  const lines = pieces.map((piece) => `${piece}=${affected.has(piece)}`);
  console.log(`${files.length} changed files\n${lines.join("\n")}`);
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${lines.join("\n")}\n`);
}
