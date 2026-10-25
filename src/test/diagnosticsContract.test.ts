// @vitest-environment node
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

// The diagnostics contract lives in Markdown, and both apps (Native and Tauri)
// copy their formats from its examples. These tests keep the examples honest:
// each one parses, carries every field its own table marks required, and the
// documents agree with each other on the shared constants.
const root = resolve(__dirname, "../..");
const doc = (path: string) => readFileSync(join(root, path), "utf8");

type Json = Record<string, unknown>;

/** Every fenced `json` block parsed whole, every `jsonl` block parsed per line. */
function jsonBlocks(md: string, lang: "json" | "jsonl"): Json[] {
  const fence = new RegExp("^```" + lang + "\\n([\\s\\S]*?)^```", "gm");
  return [...md.matchAll(fence)].flatMap(([, body]) =>
    lang === "json"
      ? [JSON.parse(body)]
      : body
          .split("\n")
          .filter((line) => line.trim())
          .map((line) => JSON.parse(line)),
  );
}

/** The section under `## heading` (or `### heading`), up to the next heading of that level or higher. */
function section(md: string, heading: string): string {
  const lines = md.split("\n");
  const start = lines.findIndex((l) => /^#{2,3} /.test(l) && l.replace(/^#+ /, "") === heading);
  if (start < 0) throw new Error(`no section "${heading}"`);
  const depth = lines[start].indexOf(" ");
  const end = lines.findIndex(
    (l, i) => i > start && /^#+ /.test(l) && l.indexOf(" ") <= depth,
  );
  return lines.slice(start + 1, end < 0 ? undefined : end).join("\n");
}

/** Rows of the field table under `heading` whose Required column says yes. */
function requiredFields(md: string, heading: string): string[] {
  const rows = section(md, heading)
    .split("\n")
    .filter((l) => l.startsWith("|"))
    .map((l) => l.split("|").slice(1, -1).map((c) => c.trim()));
  const [header, , ...body] = rows;
  const req = header.findIndex((c) => c.toLowerCase() === "required");
  expect(req, `"${heading}" table has a Required column`).toBeGreaterThan(0);
  return body.filter((r) => r[req].toLowerCase() === "yes").map((r) => r[0].replaceAll("`", ""));
}

const LOG_SCHEMA = "docs/diagnostics/log-schema.md";
const DENY_LIST = ["prompt", "content", "diff", "output", "body", "env", "audio", "transcript"];

describe("log schema", () => {
  const md = doc(LOG_SCHEMA);
  const lines = jsonBlocks(md, "jsonl");

  it("marks exactly the contract's fields as required", () => {
    expect(requiredFields(md, "Fields").sort()).toEqual(
      ["app", "event_id", "fields", "level", "message", "target", "ts", "version"].sort(),
    );
  });

  it("has example lines from both apps, each carrying every required field", () => {
    expect(lines.length).toBeGreaterThanOrEqual(2);
    expect(new Set(lines.map((l) => l.app))).toEqual(new Set(["native", "tauri"]));
    for (const line of lines) {
      for (const field of requiredFields(md, "Fields")) {
        expect(line, `${JSON.stringify(line)} has ${field}`).toHaveProperty(field);
      }
    }
  });

  it("uses only the documented levels, UTC timestamps and UUID event ids", () => {
    for (const line of lines) {
      expect(["error", "warn", "info", "debug", "trace"]).toContain(line.level);
      expect(line.ts).toMatch(/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/);
      expect(line.event_id).toMatch(
        /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
      );
      expect(typeof line.fields).toBe("object");
    }
  });

  it("never shows a deny-listed field in an example", () => {
    for (const line of lines) {
      const keys = Object.keys(line.fields as Json);
      expect(keys.filter((k) => DENY_LIST.includes(k))).toEqual([]);
    }
  });

  it("lists exactly the deny-listed field names the scrubber drops", () => {
    const listed = [...section(md, "Deny-listed fields").matchAll(/^- `(\w+)`/gm)].map((m) => m[1]);
    expect(listed).toEqual(DENY_LIST);
  });
});

const BUNDLE = "docs/diagnostics/bundle-format.md";

/** The first `json` example inside the section under `heading`. */
const exampleIn = (md: string, heading: string): Json => {
  const [first] = jsonBlocks(section(md, heading), "json");
  expect(first, `"${heading}" has a json example`).toBeDefined();
  return first;
};

const expectRequired = (md: string, heading: string, example: Json) => {
  const required = requiredFields(md, heading);
  expect(required.length, `"${heading}" marks some fields required`).toBeGreaterThan(0);
  for (const field of required) {
    expect(example, `"${heading}" example has ${field}`).toHaveProperty(field.split("."));
  }
};

describe("bundle format", () => {
  const md = doc(BUNDLE);

  it("lays out the five bundle entries under the shared zip name", () => {
    const layout = section(md, "Layout");
    expect(layout).toContain("gespan-diagnostics-<app>-<version>-<timestamp>.zip");
    for (const entry of ["report.json", "logs.jsonl", "crashes/*.json", "counters.json", "environment.json"]) {
      expect(layout).toContain(`\`${entry}\``);
    }
    expect(layout).toMatch(/5 MB/);
  });

  it("gives a report.json example that is version 1 and carries every required field", () => {
    const report = exampleIn(md, "report.json");
    expectRequired(md, "report.json", report);
    expect(report.bundle_version).toBe(1);
    expect(["native", "tauri"]).toContain(report.app);
  });

  it("keeps the contact email out of anything public", () => {
    const row = section(md, "report.json")
      .split("\n")
      .find((l) => l.startsWith("| `contact_email`"));
    expect(row).toMatch(/never/i);
    expect(row).toMatch(/public issue/i);
  });

  it("gives crash, counter and environment examples with their required fields", () => {
    for (const heading of ["crashes/*.json", "counters.json", "environment.json"]) {
      expectRequired(md, heading, exampleIn(md, heading));
    }
  });

  it("counts failures by fixed error codes, never free text", () => {
    const { counters } = exampleIn(md, "counters.json") as { counters: Json[] };
    expect(counters.length).toBeGreaterThan(0);
    for (const c of counters) {
      expect(c.day).toMatch(/^\d{4}-\d\d-\d\d$/);
      expect(c.error_kind === null || /^[a-z_]+$/.test(String(c.error_kind))).toBe(true);
      expect(Number.isInteger(c.count)).toBe(true);
    }
  });

  it("puts no paths, project names or repository URLs in environment.json", () => {
    const keys = (value: unknown): string[] =>
      value && typeof value === "object"
        ? Object.entries(value).flatMap(([k, v]) => [k, ...keys(v)])
        : [];
    const env = exampleIn(md, "environment.json");
    expect(keys(env).filter((k) => /path|name|url|repo/i.test(k))).toEqual([]);
  });
});
