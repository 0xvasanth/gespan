#!/usr/bin/env node
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { updateReadme } from "./readme-release.mjs";

const version = process.argv[2];
// X.Y.Z with an optional prerelease such as -beta or -beta.2.
if (
  !version ||
  !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(version)
) {
  console.error("usage: bun run set-version 0.1.1 (or 0.0.1-beta)");
  process.exit(1);
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
// Built before anything is written, so a README without its release block
// fails the bump without leaving the other files half updated.
const readme = join(root, "README.md");
const nextReadme = updateReadme(readFileSync(readme, "utf8"), version);

function replaceFirst(path, pattern, replacement) {
  const text = readFileSync(path, "utf8");
  const next = text.replace(pattern, replacement);
  if (next === text) {
    console.error(`failed to update ${path}`);
    process.exit(1);
  }
  writeFileSync(path, next);
}

replaceFirst(
  join(root, "package.json"),
  /("version": ")[^"]+(")/,
  `$1${version}$2`,
);
replaceFirst(
  join(root, "Cargo.toml"),
  /^(version = ")[^"]+(")/m,
  `$1${version}$2`,
);
replaceFirst(
  join(root, "crates/gespan-gpui/Cargo.toml"),
  /(\[workspace\.package\]\r?\nversion = ")[^"]+(")/,
  `$1${version}$2`,
);
// Every package that takes the workspace version, in both lockfiles, so
// `--locked` builds keep working.
const lockfiles = {
  "Cargo.lock": ["gespan-browser", "gespan-models", "gespan-speech"],
  "crates/gespan-gpui/Cargo.lock": [
    "gespan-browser",
    "gespan-core",
    "gespan-first-launch",
    "gespan-gpui",
    "gespan-models",
    "gespan-speech",
  ],
};
for (const [lockfile, names] of Object.entries(lockfiles)) {
  for (const name of names) {
    replaceFirst(
      join(root, lockfile),
      new RegExp(`(name = "${name}"\\r?\\nversion = ")[^"]+(")`),
      `$1${version}$2`,
    );
  }
}

writeFileSync(readme, nextReadme);

console.log(`version ${version}`);
