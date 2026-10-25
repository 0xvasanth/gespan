import { describe, expect, it } from "vitest";
import type { HarnessEvent } from "../../../integrations/harness/core/types";
import { HoldGate } from "./holdGate";

const started = (callId: string, background = false): HarnessEvent => ({
  type: "tool.started",
  callId,
  title: "Bash",
  status: "in_progress",
  background,
});
const finished = (callId: string): HarnessEvent => ({
  type: "tool.updated",
  callId,
  status: "completed",
});
const background = (toolId: string): HarnessEvent => ({
  type: "background.updated",
  tasks: ["dev server"],
  details: [{ id: "b", description: "dev server", toolId, steps: 0 }],
});

describe("HoldGate", () => {
  it("does nothing until Hold is requested", () => {
    const gate = new HoldGate();
    gate.observe({ type: "message.delta", text: "x" });
    expect(gate.decide(false)).toBeNull();
  });

  it("interrupts a streaming turn at once", () => {
    const gate = new HoldGate();
    gate.request();
    expect(gate.decide(false)).toEqual({ kind: "interrupt" });
    expect(gate.decide(false)).toBeNull();
  });

  it("waits for a running command, then interrupts", () => {
    const gate = new HoldGate();
    gate.observe(started("c1"));
    gate.request();
    expect(gate.decide(false)).toBeNull();
    gate.observe(finished("c1"));
    expect(gate.decide(false)).toEqual({ kind: "interrupt" });
  });

  it("settles once on pending input without interrupting", () => {
    const gate = new HoldGate();
    gate.request();
    expect(gate.decide(true)).toEqual({ kind: "settle", point: "awaitingInput" });
    expect(gate.decide(true)).toBeNull();
  });

  it("settles on background work and interrupts if the agent wakes", () => {
    const gate = new HoldGate();
    gate.observe(started("t0"));
    gate.observe(background("t0"));
    gate.request();
    expect(gate.decide(false)).toEqual({ kind: "settle", point: "background" });
    expect(gate.decide(false)).toBeNull();
    gate.observe({ type: "message.delta", text: "Server is up" });
    expect(gate.decide(false)).toEqual({ kind: "interrupt" });
  });

  it("does not count a background tool as a running command", () => {
    const gate = new HoldGate();
    gate.observe(background("t0"));
    gate.observe(started("t0", true));
    gate.request();
    expect(gate.decide(false)).toEqual({ kind: "settle", point: "background" });
  });
});
