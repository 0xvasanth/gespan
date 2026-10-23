// @vitest-environment node
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(__dirname, "../..");

function registeredCommands(): string[] {
  const lib = readFileSync(join(root, "src-tauri/src/lib.rs"), "utf8");
  const start = lib.indexOf("generate_handler![");
  expect(start).toBeGreaterThan(-1);
  const end = lib.indexOf("])", start);
  expect(end).toBeGreaterThan(start);
  return lib
    .slice(start + "generate_handler![".length, end)
    .split(",")
    .map((entry) =>
      entry
        .replace(/\/\/.*$/gm, "")
        .replace(/#\[[^\]]*\]/g, "")
        .trim(),
    )
    .filter(Boolean)
    .map((entry) => entry.split("::").pop() as string);
}

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return sourceFiles(full);
    if (!/\.(ts|tsx)$/.test(name)) return [];
    if (/\.test\.(ts|tsx)$/.test(name)) return [];
    return [full];
  });
}

function invokedCommands(): Map<string, string> {
  const found = new Map<string, string>();
  const re = /\binvoke(?:<[^"]*?>)?\(\s*"([A-Za-z0-9_:]+)"/g;
  for (const file of sourceFiles(join(root, "src"))) {
    const text = readFileSync(file, "utf8");
    for (const match of text.matchAll(re)) {
      if (match[1].includes(":")) continue; // plugin commands
      found.set(match[1], file);
    }
  }
  return found;
}

describe("tauri invoke command names", () => {
  const registered = registeredCommands();
  const invoked = invokedCommands();

  it("finds a non-trivial set of registered and invoked commands", () => {
    expect(registered.length).toBeGreaterThan(50);
    expect(invoked.size).toBeGreaterThan(50);
  });

  it("only invokes commands registered in generate_handler!", () => {
    const known = new Set(registered);
    const unknown = [...invoked.keys()].filter((n) => !known.has(n)).sort();
    expect(unknown).toEqual([]);
  });

  it("registers no legacy mono_* commands", () => {
    expect(registered.filter((n) => /mono/.test(n))).toEqual([]);
  });
});
