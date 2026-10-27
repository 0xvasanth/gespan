import { describe, expect, it, vi } from "vitest";
import { holdHarnessTurn, registerHarness, type HarnessAdapter } from "./registry";

function adapter(id: string, hold?: HarnessAdapter["holdTurn"]): HarnessAdapter {
  return {
    id: id as HarnessAdapter["id"],
    live: true,
    sendTurn: vi.fn(),
    steerTurn: vi.fn(),
    cancelTurn: vi.fn(async () => undefined),
    respondApproval: vi.fn(),
    stopSession: vi.fn(async () => undefined),
    forgetSession: vi.fn(async () => undefined),
    bindSession: vi.fn(),
    ...(hold ? { holdTurn: hold } : {}),
  };
}

describe("holdHarnessTurn", () => {
  it("uses the adapter's own hold and never stops the session", async () => {
    const own = adapter("claude", vi.fn(async () => undefined));
    registerHarness(own);
    await holdHarnessTurn("claude", "s1");
    expect(own.holdTurn).toHaveBeenCalledWith("s1");
    expect(own.cancelTurn).not.toHaveBeenCalled();
    expect(own.stopSession).not.toHaveBeenCalled();
  });

  it("falls back to cancelling the turn for adapters without one", async () => {
    const plain = adapter("cursor");
    registerHarness(plain);
    await holdHarnessTurn("cursor", "s2");
    expect(plain.cancelTurn).toHaveBeenCalledWith("s2");
    expect(plain.stopSession).not.toHaveBeenCalled();
  });
});
