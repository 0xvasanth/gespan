// @vitest-environment node
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
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

const CONSENT = "docs/diagnostics/consent.md";

/** The `consent_version` a document declares as current. */
const currentConsentVersion = (md: string) =>
  Number(/current `consent_version` is \*\*(\d+)\*\*/i.exec(md)?.[1]);

describe("consent", () => {
  const md = doc(CONSENT);
  const version = currentConsentVersion(md);

  it("declares a current consent_version", () => {
    expect(Number.isInteger(version) && version >= 1).toBe(true);
  });

  it("stores a record with the five contract fields at the current version", () => {
    expect(requiredFields(md, "Consent record").sort()).toEqual(
      ["consent_version", "crash_reports", "decided_at", "include_logs", "metrics"].sort(),
    );
    expectRequired(md, "Consent record", exampleIn(md, "Consent record"));
    expect(exampleIn(md, "Consent record").consent_version).toBe(version);
  });

  it("defaults to sending nothing: ask for crashes, metrics off, logs included, undecided", () => {
    expect(exampleIn(md, "Defaults")).toEqual({
      consent_version: version,
      crash_reports: "ask",
      metrics: false,
      include_logs: true,
      decided_at: null,
    });
  });

  it("offers three prompt buttons, and dismissing the prompt is the same as Ask me each time", () => {
    const [header, rows] = table(md, "First-run prompt");
    const choice = (label: string) => {
      const row = rows.find((r) => r[0].includes(label));
      expect(row, label).toBeDefined();
      return Object.fromEntries(header.slice(1).map((h, i) => [h, row![i + 1]]));
    };
    const buttons = ["Send crash reports automatically", "Ask me each time", "No thanks"];
    expect(new Set(buttons.map((b) => choice(b)["`crash_reports`"]))).toEqual(
      new Set(["`\"always\"`", "`\"ask\"`", "`\"never\"`"]),
    );
    expect(choice("Dismissed")).toEqual(choice("Ask me each time"));
    expect(choice("No thanks")["`metrics`"]).toBe("`false`");
  });

  it("matches the consent_version recorded in the bundle example", () => {
    const report = exampleIn(doc(BUNDLE), "report.json") as { consent: Json };
    expect(report.consent.consent_version).toBe(version);
  });
});

const INTAKE = "docs/diagnostics/intake-api.md";

describe("intake API", () => {
  const md = doc(INTAKE);

  it("authenticates with a public per-build ingest key, never a secret", () => {
    const conventions = section(md, "Conventions");
    expect(conventions).toContain("`X-Gespan-Ingest-Key`");
    expect(conventions).toMatch(/not a secret/i);
  });

  it("sends the bundle zip to POST /v1/reports and answers with report_id and issue_url", () => {
    expect(section(md, "POST /v1/reports")).toMatch(/application\/zip/);
    const response = exampleIn(md, "Report response");
    expectRequired(md, "Report response", response);
    expect(Object.keys(response).sort()).toEqual(["issue_url", "report_id"]);
    expect(String(response.issue_url)).toMatch(/^https:\/\/github\.com\/0xvasanth\/gespan\/issues\/\d+$/);
  });

  it("takes a daily aggregate at POST /v1/metrics that cannot be tied to a person or a report", () => {
    const request = exampleIn(md, "Metrics request");
    expectRequired(md, "Metrics request", request);
    expect(request).not.toHaveProperty("install_id");
    expect(request).not.toHaveProperty("submission_id");
    expect(request.day).toMatch(/^\d{4}-\d\d-\d\d$/);
    for (const c of request.counters as Json[]) {
      expect(Object.keys(c).sort()).toEqual(["count", "error_kind", "name"]);
      expect(c.error_kind === null || /^[a-z_]+$/.test(String(c.error_kind))).toBe(true);
      expect(Number.isInteger(c.count)).toBe(true);
    }
  });

  it("documents the rejections both apps must handle, each with a fixed error code", () => {
    const [, rows] = table(md, "Errors");
    const statuses = rows.map((r) => r[0]);
    for (const status of ["400", "401", "413", "415", "429", "5xx"]) expect(statuses).toContain(status);
    for (const r of rows) expect(r[1]).toMatch(/^`[a-z_]+`$/);
    const error = exampleIn(md, "Errors") as { error: Json };
    expect(Object.keys(error.error).sort()).toEqual(["code", "message"]);
    expect(rows.map((r) => r[1])).toContain(`\`${error.error.code}\``);
  });
});

const PRIVACY = "docs/privacy.md";
const CONTRACT_DOCS = [LOG_SCHEMA, BUNDLE, CONSENT, INTAKE, PRIVACY];

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

  it("states the same consent_version as the consent wording", () => {
    expect(currentConsentVersion(md)).toBe(currentConsentVersion(doc(CONSENT)));
  });

  it("names every deny-listed field as never sent", () => {
    const never = section(md, "What is never sent");
    for (const field of DENY_LIST) expect(never).toContain(`\`${field}\``);
  });

  it("gives the retention limits the contract sets", () => {
    const kept = section(md, "How long it is kept");
    for (const limit of ["7 days", "50 MB", "90 days"]) expect(kept).toContain(limit);
  });

  it("links to each diagnostics contract document", () => {
    for (const path of [LOG_SCHEMA, BUNDLE, CONSENT, INTAKE]) {
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
