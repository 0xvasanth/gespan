import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const root = join(__dirname, "..", "..");

const SCAN_DIRS = ["src", "src-tauri/src", "mobile/src", "mobile/tests", "tests"];
const SKIP_DIRS = new Set(["node_modules", "target", "dist", "build", ".expo", "Pods"]);
const TEXT_EXT = /\.(tsx?|jsx?|css|html|rs|json|md|toml|swift)$/;
const LOCKFILES = /(package-lock\.json|Cargo\.lock|yarn\.lock|pnpm-lock\.yaml)$/;

// Typography tokens that legitimately contain "mono"; stripped before matching.
const TYPOGRAPHY =
  /font-mono|monospace|monokai|monochrome|SF Mono|SFMono-Regular|JetBrains ?Mono|Liberation Mono|Nerd Font Mono|Symbols Nerd Font Mono|Menlo|mono(?:repo|tonic|lith|graph|poly|type)/gi;

// Narrow, per-file line allowlist. Each entry is one specific deliberate use.
const ALLOWED: { file: string; line: RegExp; why: string }[] = [
  { file: "src/test/noMonoReferences.test.ts", line: /./, why: "this guard names the pattern it forbids" },
  { file: "mobile/src/ui/theme.ts", line: /./, why: "the unrelated monochrome-style \"mono\" theme (brief)" },
  { file: "src-tauri/src/menu.rs", line: /usemono\.dev/, why: "external product URL" },
  { file: "src-tauri/src/pilot_transcript.rs", line: /no_mono_tables|"mono_"|stale mono table/, why: "asserts legacy tables are gone" },
  { file: "src/test/invokeCommands.test.ts", line: /legacy mono_\*|\/mono\//, why: "asserts no legacy mono commands" },
  { file: "src/features/sessions/model/operatorCommand.test.ts", line: /\/mono\b/, why: "retired /mono is plain text" },
  { file: "src/features/pilots/model/pilotFiles.test.ts", line: /Monos\?/, why: "asserts no Mono wording" },
];

const THEME_LINE = /theme.*["']mono["']/;
const REVERSE_DAMAGE =
  /(SF|JetBrains|Liberation|Nerd Font|Symbols Nerd Font)\s?Pilot|Pilotspace|font-pilot|pilot(repo|tonic|lith|graph|poly|type)/i;

function walk(dir: string, out: string[] = []): string[] {
  let entries: string[];
  try {
    entries = readdirSync(dir);
  } catch {
    return out;
  }
  for (const name of entries) {
    if (SKIP_DIRS.has(name)) continue;
    const full = join(dir, name);
    if (statSync(full).isDirectory()) walk(full, out);
    else if (TEXT_EXT.test(name) && !LOCKFILES.test(name)) out.push(full);
  }
  return out;
}

function scannedFiles(): string[] {
  const files = SCAN_DIRS.flatMap((d) => walk(join(root, d)));
  for (const name of readdirSync(root)) {
    if (name.endsWith(".html") || name === "vite.config.ts") files.push(join(root, name));
  }
  return files;
}

function read(rel: string): string {
  return readFileSync(join(root, rel), "utf8");
}

function scan(pattern: (line: string) => boolean, honorAllowlist: boolean): string[] {
  const hits: string[] = [];
  for (const file of scannedFiles()) {
    const rel = relative(root, file);
    if (!honorAllowlist && rel === "src/test/noMonoReferences.test.ts") continue; // holds the forbidden pattern
    const rules = honorAllowlist ? ALLOWED.filter((a) => a.file === rel) : [];
    readFileSync(file, "utf8")
      .split("\n")
      .forEach((line, i) => {
        if (rules.some((r) => r.line.test(line))) return;
        if (honorAllowlist && THEME_LINE.test(line)) return;
        if (pattern(line)) hits.push(`${rel}:${i + 1}`);
      });
  }
  return hits;
}

describe("Monos -> Pilots rename", () => {
  it("leaves no stray mono references", () => {
    const offenders = scan((l) => /mono/i.test(l.replace(TYPOGRAPHY, "")), true);
    expect(offenders).toEqual([]);
  });

  it("did not damage font names or typography tokens", () => {
    expect(scan((l) => REVERSE_DAMAGE.test(l), false)).toEqual([]);
  });

  it("kept monospace typography", () => {
    expect(read("src/styles/index.css")).toMatch(/font-mono|monospace/);
  });

  it("scans the mobile tree", () => {
    const rels = scannedFiles().map((f) => relative(root, f));
    expect(rels.some((f) => f.startsWith("mobile/src/"))).toBe(true);
    expect(rels.some((f) => f.includes("node_modules"))).toBe(false);
  });
});
