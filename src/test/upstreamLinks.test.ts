import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const root = join(__dirname, "..", "..");

// The fork lives at 0xvasanth/rebase. These point at a dead repo or the
// upstream product site and must not creep back in.
const FORBIDDEN = /hardbeat920\/rebase|usemono\.dev/i;

// The native tree must not point at the upstream MonoCode repo, its release
// downloads, or its bundle identifiers either.
const NATIVE_FORBIDDEN = /hardbeat920\/(rebase|monocode)|usemono|monocode/i;
const NATIVE_PATHS = [
  /^crates\//,
  /^docs\/native\//,
  /^\.github\/workflows\/native[^/]*\.ya?ml$/,
  /^scripts\/[^/]*native[^/]*(\/|$)/,
  /^scripts\/test-remote-ssh\.py$/,
];
const NATIVE_SKIP = [/^vendor\//, /^crates\/[^/]+\/target\//];

const EXCLUDED = [
  /(^|\/)(bun\.lockb?|package-lock\.json|Cargo\.lock|yarn\.lock|pnpm-lock\.yaml)$/,
  /(^|\/)node_modules\//,
  /^(target|dist|build|src-tauri\/target)\//,
  /^docs\/superpowers\//,
  /^\.superpowers\//,
  /^src\/test\/upstreamLinks\.test\.ts$/, // names the forbidden patterns
  /\.(png|jpe?g|gif|ico|icns|woff2?|ttf|otf|mp4|webm|zip|gz|pdf)$/i,
];

function trackedFiles(): string[] {
  return execFileSync("git", ["ls-files", "-z"], { cwd: root, maxBuffer: 64 * 1024 * 1024 })
    .toString("utf8")
    .split("\0")
    .filter((f) => f && !EXCLUDED.some((re) => re.test(f)));
}

describe("upstream links", () => {
  it("has no hardbeat920/rebase or usemono.dev references in tracked files", () => {
    const hits: string[] = [];
    for (const rel of trackedFiles()) {
      let text: string;
      try {
        text = readFileSync(join(root, rel), "utf8");
      } catch {
        continue;
      }
      text.split("\n").forEach((line, i) => {
        if (FORBIDDEN.test(line)) hits.push(`${rel}:${i + 1}`);
      });
    }
    expect(hits).toEqual([]);
  });

  it("has no upstream MonoCode repo, release URL or bundle id in the native tree", () => {
    const hits: string[] = [];
    for (const rel of trackedFiles()) {
      if (!NATIVE_PATHS.some((re) => re.test(rel)) || NATIVE_SKIP.some((re) => re.test(rel))) continue;
      let text: string;
      try {
        text = readFileSync(join(root, rel), "utf8");
      } catch {
        continue;
      }
      text.split("\n").forEach((line, i) => {
        if (NATIVE_FORBIDDEN.test(line)) hits.push(`${rel}:${i + 1}`);
      });
    }
    expect(hits).toEqual([]);
  });

  it("flags the upstream release URL and bundle ids in the native tree", () => {
    for (const bad of ["https://github.com/hardbeat920/monocode/releases/download/v1", "dev.usemono.gpui", "com.monocode.desktop", "Monocode Native"]) {
      expect(NATIVE_FORBIDDEN.test(bad), bad).toBe(true);
    }
    expect(NATIVE_FORBIDDEN.test("https://github.com/0xvasanth/rebase/releases/download/v1")).toBe(false);
  });

  it("matches the forbidden patterns but not the fork or contributor credits", () => {
    for (const bad of ["github.com/HardBeat920/rebase", "github.com/hardbeat920/rebase/issues", "https://usemono.dev", "https://dl.usemono.dev/Rebase.dmg"]) {
      expect(FORBIDDEN.test(bad), bad).toBe(true);
    }
    for (const ok of ["https://github.com/0xvasanth/rebase", "by @hardbeat920 in #12"]) {
      expect(FORBIDDEN.test(ok), ok).toBe(false);
    }
  });
});
