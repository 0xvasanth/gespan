import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";
import { foldUnpublished, parseChangelog, type Release } from "./scripts/release.mjs";

// The release the site offers is src/release.json, written by
// `bun run sync-release` from GitHub's latest published release. The
// Changelog page is CHANGELOG.md, up to that release: a version still in the
// release pipeline isn't shown until it's published, and a version that never
// got a release of its own is shown inside the release that shipped it.
const release: Release = JSON.parse(readFileSync(fileURLToPath(new URL("./src/release.json", import.meta.url)), "utf8"));
const changelog = readFileSync(fileURLToPath(new URL("../CHANGELOG.md", import.meta.url)), "utf8");
const releases = foldUnpublished(parseChangelog(changelog, release.version), release.published).slice(0, 6);

export default defineConfig({
  // Vite 8 bundles with Rolldown and transforms with Oxc; `compiler` turns on
  // React Compiler through oxc-transform-react.
  plugins: [react({ compiler: true }), tailwindcss()],
  define: {
    __GESPAN_RELEASE__: JSON.stringify(release),
    __GESPAN_CHANGELOG__: JSON.stringify(releases),
  },
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  build: { target: "es2022" },
  test: {
    environment: "jsdom",
    include: ["tests/**/*.test.{ts,tsx}"],
    setupFiles: ["tests/setup.ts"],
  },
});
