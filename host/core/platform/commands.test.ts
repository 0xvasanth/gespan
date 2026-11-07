import { build } from "esbuild";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { convertFileSrc, invoke, listen } from "./commands";

describe("desktop commands on the host", () => {
  it("reject with the command's name instead of reaching for a Tauri backend", async () => {
    await expect(invoke("git_status", { cwd: "/repo" })).rejects.toThrow(
      "git_status is not available on the Gespan host",
    );
    await expect(listen("harness-exit", () => {})).rejects.toThrow(
      "harness-exit is not available on the Gespan host",
    );
    expect(() => convertFileSrc("/tmp/a.png")).toThrow("not available on the Gespan host");
  });
});

describe("the host bundle", () => {
  it("carries no Tauri code", async () => {
    const result = await build({
      entryPoints: ["host/cli.ts"],
      bundle: true,
      platform: "node",
      format: "esm",
      target: "node24",
      loader: { ".ps1": "text" },
      define: { "import.meta.hot": "undefined" },
      write: false,
      metafile: true,
      outfile: "gespan-host.mjs",
    });
    const inputs = Object.keys(result.metafile.inputs);
    expect(inputs.some((input) => input.endsWith("host/cli.ts"))).toBe(true);
    expect(inputs.filter((input) => /tauri/i.test(input))).toEqual([]);
    const bundle = result.outputFiles.find((file) => file.path.endsWith(".mjs"))!.text;
    expect(bundle).not.toContain("__TAURI_INTERNALS__");
  });

  it("depends on no Tauri runtime library", () => {
    const pkg = JSON.parse(readFileSync("package.json", "utf8"));
    const runtime = Object.keys({ ...pkg.dependencies, ...pkg.optionalDependencies });
    expect(runtime.filter((name) => name.startsWith("@tauri-apps/"))).toEqual([]);
  });
});
