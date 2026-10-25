// @vitest-environment node
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

// The diagnostics contract lives in Markdown, and both apps (Native and Tauri)
// copy their formats from its examples. These tests keep the examples honest:
// each one parses, carries every field its own table marks required, and the
// documents agree with each other on the shared constants.
const root = resolve(__dirname, "../..");
/** Line endings as the parser expects them, whatever the checkout used. */
const lf = (text: string) => text.replaceAll("\r\n", "\n");
const doc = (path: string) => lf(readFileSync(join(root, path), "utf8"));

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

/** The first table under `heading`: its header cells and body rows, separator dropped. */
function table(md: string, heading: string): [string[], string[][]] {
  const rows = section(md, heading)
    .split("\n")
    .filter((l) => l.startsWith("|"))
    .map((l) => l.split("|").slice(1, -1).map((c) => c.trim()));
  const [header, , ...body] = rows;
  return [header, body];
}

/** Rows of the field table under `heading` whose Required column says yes. */
function requiredFields(md: string, heading: string): string[] {
  const [header, body] = table(md, heading);
  const req = header.findIndex((c) => c.toLowerCase() === "required");
  expect(req, `"${heading}" table has a Required column`).toBeGreaterThan(0);
  return body.filter((r) => r[req].toLowerCase() === "yes").map((r) => r[0].replaceAll("`", ""));
}

const LOG_SCHEMA = "docs/diagnostics/log-schema.md";
const DENY_LIST = ["prompt", "content", "diff", "output", "body", "env", "audio", "transcript"];

describe("reading the contract docs", () => {
  it("parses a Windows (CRLF) checkout exactly like an LF one", () => {
    const md = doc(LOG_SCHEMA);
    const crlf = lf(md.replaceAll("\n", "\r\n"));
    expect(requiredFields(crlf, "Fields")).toEqual(requiredFields(md, "Fields"));
    expect(jsonBlocks(crlf, "jsonl")).toEqual(jsonBlocks(md, "jsonl"));
    expect(section(crlf, "Deny-listed fields")).toBe(section(md, "Deny-listed fields"));
  });
});

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
      expect(line.fields).toBeTypeOf("object");
      expect(line.fields).not.toBeNull();
    }
  });

  it("never shows a deny-listed name in an example, at the top level or in any dotted segment", () => {
    for (const line of lines) {
      const keys = [...Object.keys(line), ...Object.keys(line.fields as Json)];
      const denied = keys.filter((k) => k.split(".").some((part) => DENY_LIST.includes(part)));
      expect(denied).toEqual([]);
    }
  });

  it("tells the scrubber to match deny-listed names in any dotted segment", () => {
    expect(section(md, "Deny-listed fields")).toMatch(/any\s+dotted\s+segment/i);
  });

  it("gives Native dev builds their own log folder on every OS", () => {
    const files = section(md, "Files");
    expect(files).toContain("`~/Library/Logs/dev.gespan.native.dev/`");
    expect(files).toContain("`$XDG_STATE_HOME/gespan-native-dev/logs/`");
    expect(files).toContain("`%LOCALAPPDATA%\\dev.gespan.native.dev\\logs\\`");
  });

  it("prunes while running, not only at startup, so a long session stays inside the limits", () => {
    const files = section(md, "Files");
    expect(files).toMatch(/at\s+startup/);
    expect(files).toMatch(/UTC\s+midnight/);
    expect(files).toMatch(/would\s+take\s+the\s+total\s+over\s+50\s+MB/);
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

  it("says which entries each kind of report carries, and a crash send carries no logs", () => {
    const [header, rows] = table(md, "Contents by kind");
    const entries = header.slice(1);
    expect(entries).toEqual(
      ["report.json", "logs.jsonl", "crashes/*.json", "counters.json", "environment.json"].map(
        (e) => `\`${e}\``,
      ),
    );
    const kinds = rows.map((r) => r[0]);
    expect(kinds).toEqual(["`user`", "`crash`"]);
    const crash = Object.fromEntries(entries.map((e, i) => [e, rows[1][i + 1]]));
    expect(crash["`report.json`"]).toMatch(/^always/i);
    expect(crash["`crashes/*.json`"]).toMatch(/^always/i);
    for (const e of ["`logs.jsonl`", "`counters.json`", "`environment.json`"]) {
      expect(crash[e], e).toMatch(/^never/i);
    }
  });

  it("names crash files so two panics in the same second cannot overwrite each other", () => {
    expect(section(md, "crashes/*.json")).toContain("crashes/<timestamp>-<random>.json");
  });

  it("titles a crash send from the panic location only, never the panic message", () => {
    const row = section(md, "report.json")
      .split("\n")
      .find((l) => l.startsWith("| `title`"));
    expect(row).toContain("`Crash in <file>:<line>`");
    expect(row).toMatch(/never\s+the\s+panic\s+message/i);
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

const PRIVACY = "docs/privacy.md";
const CONTRACT_DOCS = [LOG_SCHEMA, BUNDLE, PRIVACY];

/** GitHub's anchor for a heading: lowercase, punctuation dropped, spaces to dashes. */
const slug = (heading: string) =>
  heading
    .toLowerCase()
    .replace(/[^\p{L}\p{N} _-]/gu, "")
    .replaceAll(" ", "-");

describe("privacy notice", () => {
  const md = doc(PRIVACY);

  it("answers what is collected, where it goes, how long it is kept and how to delete it", () => {
    for (const heading of [
      "What is collected",
      "Where it goes",
      "How long it is kept",
      "How to have it deleted",
    ]) {
      expect(section(md, heading).trim(), heading).not.toBe("");
    }
  });

  it("promises that Gespan never sends anything by itself", () => {
    expect(md).toMatch(/\*\*Gespan never sends diagnostics by itself\.\*\*/);
    expect(md).not.toMatch(/consent_version|install ID|intake|opt in to something/i);
  });

  it("names both ways a user sends a report on, and says which one is public", () => {
    const where = section(md, "Where it goes");
    expect(where).toContain('"Save zip and email"');
    expect(where).toContain('"Save zip and open GitHub"');
    expect(where).toMatch(/\*\*public\*\*/);
  });

  it("gives the same report address as the app", () => {
    const address = /REPORT_EMAIL: &str = "([^"]+)"/.exec(
      doc("crates/gespan-core/src/diagnostics/fallback.rs"),
    )?.[1];
    expect(address).toBeTruthy();
    expect(section(md, "Where it goes")).toContain(`\`${address}\``);
  });

  it("says what every report carries even without attached diagnostics", () => {
    const report = section(md, "In a report you save");
    for (const item of ["locale", "app and OS versions", "IDs of the\nlog lines"]) {
      expect(report).toContain(item);
    }
  });

  it("prunes crash records even when they are never reported", () => {
    const [, rows] = table(md, "How long it is kept");
    expect(rows.find((r) => r[0] === "Crash records" && r[1] === "Your machine")?.[2]).toMatch(/7 days/);
  });

  it("names every deny-listed field as never collected", () => {
    const never = section(md, "What is never collected");
    for (const field of DENY_LIST) expect(never).toContain(`\`${field}\``);
  });

  it("gives the retention limits the contract sets", () => {
    const kept = section(md, "How long it is kept");
    for (const limit of ["7 days", "50 MB", "30 days"]) expect(kept).toContain(limit);
  });

  it("links to each diagnostics contract document", () => {
    for (const path of [LOG_SCHEMA, BUNDLE]) {
      expect(md).toContain(`](${path.replace("docs/", "")}`);
    }
  });
});

describe("contract documents link to each other correctly", () => {
  it.each(CONTRACT_DOCS)("every relative link and anchor in %s resolves", (from) => {
    const links = [...doc(from).matchAll(/\]\(([^)\s]+)\)/g)]
      .map((m) => m[1])
      .filter((href) => !/^[a-z]+:/.test(href));
    expect(links.length).toBeGreaterThan(0);
    for (const href of links) {
      const [file, anchor] = href.split("#");
      const target = file ? join(dirname(from), file) : from;
      expect(existsSync(join(root, target)), `${from} → ${href}`).toBe(true);
      if (anchor) {
        const anchors = [...doc(target).matchAll(/^#+ (.+)$/gm)].map((m) => slug(m[1]));
        expect(anchors, `${from} → ${href}`).toContain(anchor);
      }
    }
  });
});
