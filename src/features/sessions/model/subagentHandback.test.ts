import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { parseHandback } from "./subagentHandback";

const FIXTURE = resolve(process.cwd(), "tests/fixtures/claude/resumed-background-agent.jsonl");

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
            ? (block.content as Array<{ text?: string }>).map((p) => p.text ?? "").join("")
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
    expect(parsed.report.split("\n").every((l) => !l.startsWith(" "))).toBe(true);
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
});
