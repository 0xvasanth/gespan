import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const importPath = /(?:from\s+|import\s*\(\s*|^import\s+)["'](\.[^"']+)["']/gm;

function sources(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory())
      return entry.name === "node_modules" ? [] : sources(path);
    return /\.tsx?$/.test(entry.name) ? [path] : [];
  });
}

/** What a file imports from the Tauri app's folders, as repo-relative paths. */
function desktopImports(file: string, text: string): string[] {
  return [...text.matchAll(importPath)]
    .map((match) =>
      relative(root, resolve(dirname(file), match[1])).split(sep).join("/"),
    )
    .filter((path) => /^src(-tauri)?\//.test(path));
}

describe("the mobile app and its tests without the Tauri app", () => {
  it("spots an import that climbs into src/, and leaves mobile's own src/ alone", () => {
    const test = join(root, "tests/mobile/example.test.ts");
    expect(
      desktopImports(test, 'import { a } from "../../src/features/a";'),
    ).toEqual(["src/features/a"]);
    expect(
      desktopImports(
        test,
        'import { b } from "../../mobile/src/features/b";',
      ),
    ).toEqual([]);
  });

  it("imports nothing from src/ or src-tauri/", () => {
    const files = [
      ...sources(join(root, "mobile/src")),
      ...sources(join(root, "mobile/tests")),
      ...sources(join(root, "tests/mobile")),
    ].filter((file) => !file.endsWith("independence.test.ts"));
    expect(files.length).toBeGreaterThan(50);
    expect(
      files.flatMap((file) =>
        desktopImports(file, readFileSync(file, "utf8")).map(
          (path) => `${relative(root, file)} imports ${path}`,
        ),
      ),
    ).toEqual([]);
  });
});
