#!/usr/bin/env node
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

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
  join(root, "src-tauri/Cargo.toml"),
  /^(version = ")[^"]+(")/m,
  `$1${version}$2`,
);
replaceFirst(
  join(root, "src-tauri/tauri.conf.json"),
  /("version": ")[^"]+(")/,
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
  "Cargo.lock": ["gespan-models", "gespan-speech"],
  "src-tauri/Cargo.lock": ["gespan", "gespan-models", "gespan-speech"],
  "crates/gespan-gpui/Cargo.lock": [
    "gespan-core",
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

console.log(`version ${version}`);
