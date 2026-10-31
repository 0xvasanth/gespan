import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { build } from "esbuild";
import { expect, it } from "vitest";

const host = resolve(import.meta.dirname);
const root = dirname(host);
const importPath =
  /(?:from\s+|import\s*\(\s*|vi\.(?:do)?mock\(\s*|^import\s+)["'](\.[^"']+)["']/gm;

function sources(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return sources(path);
    return entry.name.endsWith(".ts") ? [path] : [];
  });
}

// The host is installed by Gespan Native's SSH setup, so it has to keep
// building once the Tauri app's folders are gone.
it("imports nothing from the Tauri app's folders", () => {
  const desktop = sources(host).flatMap((file) =>
    [...readFileSync(file, "utf8").matchAll(importPath)]
      .map((match) => relative(root, resolve(dirname(file), match[1])))
      .filter((path) => /^src(-tauri)?[\\/]/.test(path))
      .map((path) => `${relative(root, file)} imports ${path}`),
  );
  expect(desktop).toEqual([]);
});

it("bundles without a single file from src/", async () => {
  const { metafile } = await build({
    entryPoints: ["host/cli.ts"],
    bundle: true,
    write: false,
    outfile: "unused.mjs",
    platform: "node",
    format: "esm",
    target: "node24",
    loader: { ".ps1": "text" },
    metafile: true,
    logLevel: "silent",
  });
  const inputs = Object.keys(metafile.inputs);
  expect(inputs).toContain("host/core/features/connections/model/protocol.ts");
  expect(inputs.filter((input) => input.startsWith("src/"))).toEqual([]);
});
