import { describe, expect, it } from "vitest";
import { agentValues, formatCost, formatRunTime } from "./agentFields";

describe("agent fields", () => {
  it("joins providers and models, sums run time and cost", () => {
    expect(
      agentValues([
        { provider: "Claude", model: "Sonnet", runTimeMs: 30 * 60_000, costUsd: 0.5 },
        { provider: "Claude", model: "Opus", runTimeMs: 45 * 60_000, costUsd: 1.25 },
        { provider: "Codex", model: "GPT", runTimeMs: 0 },
      ]),
    ).toEqual({
      provider: "Claude, Codex",
      model: "Sonnet, Opus, GPT",
      cost: "$1.75",
      runTime: "1h 15m",
    });
  });

  it("a card with no linked session has no agent values; an unknown cost stays empty", () => {
    expect(agentValues([])).toEqual({ provider: null, model: null, cost: null, runTime: null });
    expect(agentValues([{ provider: "Pi", model: "m" }]).cost).toBeNull();
    expect(agentValues([{ provider: "Pi", model: "m" }]).runTime).toBeNull();
  });

  it("formats times and costs", () => {
    expect(formatRunTime(20_000)).toBe("<1m");
    expect(formatRunTime(3 * 60_000)).toBe("3m");
    expect(formatRunTime(2 * 3_600_000)).toBe("2h");
    expect(formatCost(0.004)).toBe("<$0.01");
    expect(formatCost(12)).toBe("$12.00");
  });
});
