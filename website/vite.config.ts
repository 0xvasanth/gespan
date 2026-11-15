import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";
import { parseChangelog, readRelease } from "./scripts/release.mjs";

// The site's release data comes from the repository's own README.md and
// CHANGELOG.md, read once at build time.
const repo = (path: string) => readFileSync(fileURLToPath(new URL(`../${path}`, import.meta.url)), "utf8");
const readme = repo("README.md");
const changelog = repo("CHANGELOG.md");
const release = readRelease(readme, changelog);
const releases = parseChangelog(changelog).slice(0, 6);

export default defineConfig({
  // Vite 8 bundles with Rolldown and transforms with Oxc; `compiler` turns on
  // React Compiler through oxc-transform-react.
  plugins: [react({ compiler: true })],
  define: {
    __GESPAN_RELEASE__: JSON.stringify(release),
    __GESPAN_CHANGELOG__: JSON.stringify(releases),
  },
  build: { target: "es2022" },
  test: {
    environment: "jsdom",
    include: ["tests/**/*.test.{ts,tsx}"],
  },
});
