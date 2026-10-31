#!/usr/bin/env node
// Decides which CI pieces a pull request can affect, from the files it changes.
// A file no rule names runs every piece: a missing rule costs time, never
// coverage. The rules follow what each piece reads across directories, and
// scripts/ci-impact.test.mjs pins the reads that are easy to miss.
import { spawnSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export const pieces = ["core", "desktop", "host", "mobile", "release"];

const rules = [
  // Nothing reads these. docs/native/examples is compiled into Native, and
  // Native and the release scripts read CHANGELOG.md, so neither matches.
  [/^docs\/(?!native\/examples\/)/, []],
  [/^(CODE_OF_CONDUCT|CONTRIBUTING|README|SECURITY)\.md$/, []],
  [/^(LICENSE|NOTICE)$/, []],
  [/^\.github\/(ISSUE_TEMPLATE\/|pull_request_template\.md$|CODEOWNERS$)/, []],
  // bump-version rewrites these three, and Core's Node tests run it.
  [/^src-tauri\/(Cargo\.toml|Cargo\.lock|tauri\.conf\.json)$/, ["core"]],
  [/^src-tauri\//, []],
  // Core's shared_contracts test scans crates/gespan-gpui/src.
  [/^crates\/gespan-gpui\/src\//, ["core", "desktop"]],
  [/^crates\/(gespan-gpui\/(tests|assets)|gespan-first-launch)\//, ["desktop"]],
  [/^crates\/(board-store|gespan-mcp-config|gespan-models|gespan-redact|gespan-speech|keep-awake|tracker-client|tracker-secrets)\//, ["core", "desktop"]],
  [/^vendor\//, ["core", "desktop"]],
  // Core's shared_contracts test reads these.
  [/^host\/(package\.mjs|windows-acl\.ps1)$/, ["core", "host", "mobile"]],
  // mobile imports src/, and src/ re-exports host/core.
  [/^host\//, ["host", "mobile"]],
  [/^src\//, ["host", "mobile"]],
  [/^(mobile|tests\/mobile)\//, ["mobile"]],
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

function changedFiles() {
  // The pull request's merge commit: its first parent is the base.
  const result = spawnSync("git", ["diff", "--no-renames", "--name-only", "HEAD^1", "HEAD"], { encoding: "utf8" });
  if (result.status !== 0) return [];
  return result.stdout.split("\n").filter(Boolean);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const pullRequest = process.env.GITHUB_EVENT_NAME === "pull_request";
  const files = pullRequest ? changedFiles() : [];
  const affected = impact(files);
  const lines = pieces.map((piece) => `${piece}=${affected.has(piece)}`);
  console.log(`${files.length} changed files\n${lines.join("\n")}`);
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${lines.join("\n")}\n`);
}
