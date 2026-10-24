import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { handbackMeta, parseHandback } from "./subagentHandback";

const FIXTURE = resolve(
  process.cwd(),
  "tests/fixtures/claude/resumed-background-agent.jsonl",
);

/** The text of the Agent tool_result that carries the hand-back (found by content). */
function fixtureHandbackText(): string {
  for (const line of readFileSync(FIXTURE, "utf8").split("\n")) {
    if (!line.trim()) continue;
    const frame = JSON.parse(line) as { message?: { content?: unknown } };
    const blocks = frame.message?.content;
    if (!Array.isArray(blocks)) continue;
    for (const block of blocks as Array<{ type?: string; content?: unknown }>) {
      if (block.type !== "tool_result") continue;
      const text =
        typeof block.content === "string"
          ? block.content
          : Array.isArray(block.content)
            ? (block.content as Array<{ text?: string }>)
                .map((p) => p.text ?? "")
                .join("")
            : "";
      if (text.includes("[Subagent hand-back]")) return text;
    }
  }
  throw new Error("no hand-back tool_result in fixture");
}

describe("parseHandback", () => {
  it("parses the fixture hand-back", () => {
    const parsed = parseHandback(fixtureHandbackText());
    expect(
      parsed.report.startsWith(
        "Here are the outputs of the two echo commands:\n\n1. `echo step-one`",
      ),
    ).toBe(true);
    expect(parsed.report).not.toContain("[Subagent hand-back]");
    expect(parsed.report).not.toContain("The report follows:");
    expect(parsed.report).not.toContain("agentId:");
    expect(parsed.report).not.toContain("<usage>");
    expect(parsed.report.endsWith("run `echo step-two` now.")).toBe(true);
    expect(parsed.report.split("\n").every((l) => !l.startsWith(" "))).toBe(
      true,
    );
    expect(parsed.agentId).toBe("aba491c492e906740");
    expect(parsed.subagentTokens).toBe(269110);
    expect(parsed.toolUses).toBe(2);
    expect(parsed.durationMs).toBe(9517);
  });

  it("returns plain text unchanged", () => {
    const text = "Just a normal tool result.\n  with indentation\n";
    expect(parseHandback(text)).toEqual({ report: text });
  });

  it("ignores a mid-text agentId line and uses the real trailer", () => {
    const text =
      "Findings:\nagentId: fake\nmore text\nagentId: real1 (use SendMessage with to: 'real1') subagent_tokens: 10 tool_uses: 3 duration_ms: 400";
    expect(parseHandback(text)).toEqual({
      report: "Findings:\nagentId: fake\nmore text",
      agentId: "real1",
      subagentTokens: 10,
      toolUses: 3,
      durationMs: 400,
    });
  });

  it("extracts a trailer without the preamble", () => {
    const text =
      "Done.\nagentId: abc123 (use SendMessage with to: 'abc123', summary: '<5-10 word recap>' to continue this agent)\n<usage>subagent_tokens: 5\ntool_uses: 1\nduration_ms: 20</usage>";
    expect(parseHandback(text)).toEqual({
      report: "Done.",
      agentId: "abc123",
      subagentTokens: 5,
      toolUses: 1,
      durationMs: 20,
    });
  });

  it("returns an empty report for an empty string", () => {
    expect(parseHandback("")).toEqual({ report: "" });
  });

  it("keeps nested indentation and fenced code after dedent", () => {
    const text =
      "[Subagent hand-back] x The report follows:\n  Plan:\n  - one\n    - nested\n  \n    ```rust\n    let a = 1;\n    ```\nagentId: z9 (use SendMessage) subagent_tokens: 1 tool_uses: 2 duration_ms: 3";
    const parsed = parseHandback(text);
    expect(parsed.report).toBe(
      "Plan:\n- one\n  - nested\n\n  ```rust\n  let a = 1;\n  ```",
    );
    expect(parsed.agentId).toBe("z9");
  });

  it("parses CRLF input", () => {
    const text =
      "[Subagent hand-back] x The report follows:\r\n  Line one\r\n  Line two\r\nagentId: c1 (use SendMessage)\r\n<usage>subagent_tokens: 7\r\ntool_uses: 8\r\nduration_ms: 9</usage>\r\n";
    expect(parseHandback(text)).toEqual({
      report: "Line one\nLine two",
      agentId: "c1",
      subagentTokens: 7,
      toolUses: 8,
      durationMs: 9,
    });
  });

  it("parses a trailer without the usage wrapper", () => {
    const parsed = parseHandback(
      "Body\nagentId: w1 (hint)\nsubagent_tokens: 11\ntool_uses: 12\nduration_ms: 13\n",
    );
    expect(parsed).toEqual({
      report: "Body",
      agentId: "w1",
      subagentTokens: 11,
      toolUses: 12,
      durationMs: 13,
    });
  });

  it("treats a trailer with no usage numbers as plain text", () => {
    const text = "Body\nagentId: only1 (use SendMessage with to: 'only1')";
    expect(parseHandback(text)).toEqual({ report: text });
  });

  describe("performance", () => {
    const trailer =
      "agentId: real (hint)\n<usage>subagent_tokens: 1\ntool_uses: 2\nduration_ms: 3</usage>";

    it("stays fast with 50k line-start agentId lines in the report", () => {
      const report = "agentId: fake\n".repeat(50_000);
      const start = performance.now();
      const parsed = parseHandback(report + trailer);
      expect(performance.now() - start).toBeLessThan(200);
      expect(parsed.agentId).toBe("real");
      expect(parsed.report).toBe(report.trimEnd());
    });

    it("stays fast with a trailer followed by 200KB of whitespace and a stray char", () => {
      const text = `${trailer}${" ".repeat(200_000)}x`;
      const start = performance.now();
      const parsed = parseHandback(text);
      expect(performance.now() - start).toBeLessThan(200);
      expect(parsed).toEqual({ report: text });
    });
  });
});

describe("handbackMeta", () => {
  it("joins the fields with the Gespan Native wording", () => {
    expect(
      handbackMeta({
        report: "",
        toolUses: 5,
        subagentTokens: 1234567,
        durationMs: 4_200,
      }),
    ).toBe("5 tool uses · 1,234,567 tokens · 4.2s");
  });
  it("uses the singular and omits missing fields", () => {
    expect(handbackMeta({ report: "", toolUses: 1 })).toBe("1 tool use");
    expect(
      handbackMeta({ report: "", subagentTokens: 999, durationMs: 0 }),
    ).toBe("999 tokens · 0.0s");
    expect(handbackMeta({ report: "x" })).toBeUndefined();
  });
  it("rounds durations like Native: one decimal under ten seconds, none after", () => {
    const at = (durationMs: number) => handbackMeta({ report: "", durationMs });
    expect(at(9_949)).toBe("9.9s");
    expect(at(9_960)).toBe("10s");
    expect(at(10_499)).toBe("10s");
    expect(at(125_000)).toBe("125s");
  });
});
