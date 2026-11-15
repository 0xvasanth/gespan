#!/usr/bin/env node
// After `vite build`, gives every page its own index.html (dist/features/index.html
// and so on) and a 404.html, so any static host serves /features directly and
// unknown paths get the site's own not-found page.
import { copyFileSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const routes = JSON.parse(readFileSync(join(here, "../src/routes.json"), "utf8"));

/** Copies dist/index.html to each route's folder and to 404.html; returns the files written. */
export function copyRoutes(dist) {
  const index = join(dist, "index.html");
  const written = [];
  for (const { path } of routes) {
    if (path === "/") continue;
    const dir = join(dist, ...path.split("/").filter(Boolean));
    mkdirSync(dir, { recursive: true });
    copyFileSync(index, join(dir, "index.html"));
    written.push(join(dir, "index.html"));
  }
  copyFileSync(index, join(dist, "404.html"));
  written.push(join(dist, "404.html"));
  return written;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const files = copyRoutes(process.argv[2] ?? "dist");
  console.log(`copy-routes: wrote ${files.length} pages`);
}
