import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, test } from "vitest";
import { copyRoutes } from "../scripts/copy-routes.mjs";
import routes from "../src/routes.json";

let dist = "";
afterEach(() => rmSync(dist, { recursive: true, force: true }));

test("every page gets its own index.html, and unknown paths get 404.html", () => {
  dist = mkdtempSync(join(tmpdir(), "gespan-site-"));
  writeFileSync(join(dist, "index.html"), "<!doctype html><title>Gespan</title>");
  copyRoutes(dist);
  for (const { path } of routes) {
    const file = path === "/" ? join(dist, "index.html") : join(dist, path.slice(1), "index.html");
    expect(readFileSync(file, "utf8"), path).toContain("<title>Gespan</title>");
  }
  expect(readFileSync(join(dist, "404.html"), "utf8")).toContain("<title>Gespan</title>");
});
