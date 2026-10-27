#!/usr/bin/env node
// Builds the rpm. RPM versions cannot contain "-", so a prerelease such as
// 0.0.1-beta.2 is packaged the Fedora way, as Version 0.0.1 with Release
// 0.beta.2, which sorts before the final release's 0.0.1-1. Tauri takes the
// rpm version from the app version, so a prerelease rpm reports the base
// version; rpm installs update through the package manager, not the app.
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export function rpmVersion(version) {
  const match = /^(\d+\.\d+\.\d+)(?:-([0-9A-Za-z.-]+))?$/.exec(version);
  if (!match) throw new Error(`Not a release version: ${version}`);
  const [, base, prerelease] = match;
  return prerelease
    ? { version: base, release: `0.${prerelease.replaceAll("-", "_")}` }
    : { version, release: "1" };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  const { version } = JSON.parse(
    readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"),
  );
  const rpm = rpmVersion(version);
  const args = ["tauri", "build", "--bundles", "rpm"];
  if (rpm.version !== version) {
    args.push(
      "--config",
      JSON.stringify({
        version: rpm.version,
        bundle: { linux: { rpm: { release: rpm.release } } },
      }),
    );
  }
  const result = spawnSync("bunx", args, { cwd: root, stdio: "inherit" });
  process.exit(result.status ?? 1);
}
