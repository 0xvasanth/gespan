// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearAllToolElapsed,
  setToolElapsed,
} from "../../../integrations/harness/core/toolElapsed";
import { sanitizeSessionForPersist } from "../data/sessionStore";
import type { Block } from "../model/session";
import { newSession } from "../model/session";
import {
  withoutLegacyHeartbeats,
  workSummaryLine,
} from "../model/transcriptActivity";
import { AgentTranscript } from "./AgentTranscript";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
      unobserve() {}
    },
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  clearAllToolElapsed();
  vi.unstubAllGlobals();
});

const beat = (parent: string, n: number) => ({
  id: `${parent}-heartbeat-${n}`,
  kind: "tool" as const,
  text: "Agent",
  status: "in_progress",
});
const real = { id: "toolu_real", kind: "tool" as const, text: "Read a.ts", status: "completed" };

function background(id: string, steps: Block["agentRun"] extends infer R ? NonNullable<R>["steps"] : never): Block {
  return {
    id: `b-${id}`,
    role: "tool",
    text: "Background agent",
    tool: { callId: id, kind: "agent", status: "completed" },
    agentRun: { name: "Background agent", steps },
  };
}

const bash = (id: string, status = "completed"): Block => ({
  id: `bash-${id}`,
  role: "tool",
  text: "Run echo hi",
  tool: { callId: id, kind: "execute", status },
});

const render = (blocks: Block[], busy = true) =>
  act(() => root.render(createElement(AgentTranscript, { blocks, busy })));

describe("legacy heartbeat hiding", () => {
  it("drops a background run made only of heartbeats", () => {
    const ghost = background("background:P-1", [beat("P", 0), beat("P", 1)]);
    expect(withoutLegacyHeartbeats([bash("a"), ghost])).toEqual([bash("a")]);
  });

  it("keeps a mixed background run with only its real steps", () => {
    const mixed = background("background:P-1", [beat("P", 0), real, beat("P", 1)]);
    const [out] = withoutLegacyHeartbeats([mixed]);
    expect(out.agentRun?.steps).toEqual([real]);
    expect(mixed.agentRun?.steps).toHaveLength(3); // never mutated
  });

  it("strips heartbeat steps from a real sub-agent run but keeps the run", () => {
    const agent: Block = {
      id: "agent",
      role: "tool",
      text: "Review",
      tool: { callId: "toolu_agent", kind: "agent", status: "in_progress" },
      agentRun: { name: "Review", steps: [real, beat("toolu_agent", 0)] },
    };
    const [out] = withoutLegacyHeartbeats([agent]);
    expect(out.agentRun?.steps.map((s) => s.id)).toEqual(["toolu_real"]);
  });

  it("keeps a non-background run that has only heartbeat steps, and an empty background run", () => {
    const agent: Block = {
      ...background("toolu_agent", [beat("toolu_agent", 0)]),
      id: "agent",
    };
    expect(withoutLegacyHeartbeats([agent])).toHaveLength(1);
    const empty = background("background:P-1", []);
    expect(withoutLegacyHeartbeats([empty])).toEqual([empty]);
  });

  it("returns the same array and blocks when nothing is a heartbeat", () => {
    const blocks = [bash("a"), background("background:P-1", [real])];
    expect(withoutLegacyHeartbeats(blocks)).toBe(blocks);
  });

  it("keeps Ran N commands and subagent counts free of ghosts", () => {
    const blocks = [
      bash("a"),
      bash("b"),
      background("background:P-1", [beat("P", 0)]),
    ];
    expect(workSummaryLine(blocks)).toBe("Ran 2 commands · Ran a subagent");
    expect(workSummaryLine(withoutLegacyHeartbeats(blocks))).toBe(
      "Ran 2 commands",
    );
  });

  it("hides the ghost row and the phantom step when rendered, without touching the saved blocks", () => {
    const blocks: Block[] = [
      { id: "user", role: "user", text: "Go" },
      background("background:P-1", [beat("P", 0)]),
      {
        id: "agent",
        role: "tool",
        text: "Review code",
        tool: { callId: "toolu_agent", kind: "agent", status: "in_progress" },
        agentRun: { name: "Review code", steps: [real, beat("toolu_agent", 0)] },
      },
    ];
    const before = JSON.stringify(blocks);
    render(blocks);
    const text = container.textContent ?? "";
    expect(text).not.toContain("Background agent");
    expect(text).toContain("Review code");
    expect(text).toContain("1 step"); // not 2
    expect(text).not.toContain("2 steps");
    expect(JSON.stringify(blocks)).toBe(before);
  });
});

describe("live elapsed", () => {
  const running = (id: string): Block[] => [
    { id: "user", role: "user", text: "Go" },
    bash(id, "in_progress"),
  ];

  it("shows nothing before the first heartbeat", () => {
    render(running("toolu_bash"));
    expect(container.querySelector('[data-testid="tool-elapsed"]')).toBeNull();
  });

  it("appears and updates as heartbeats arrive, without new blocks", () => {
    render(running("toolu_bash"));
    act(() => setToolElapsed("toolu_bash", 30));
    const label = () =>
      container.querySelector('[data-testid="tool-elapsed"]')?.textContent;
    expect(label()).toBe("running · 30s");
    act(() => setToolElapsed("toolu_bash", 150));
    expect(label()).toBe("running · 2m 30s");
  });

  it("is not shown on a settled call or for another call", () => {
    act(() => setToolElapsed("toolu_bash", 60));
    render([
      { id: "user", role: "user", text: "Go" },
      bash("toolu_bash", "completed"),
      bash("toolu_other", "in_progress"),
    ]);
    expect(container.querySelector('[data-testid="tool-elapsed"]')).toBeNull();
  });

  it("shows on a running sub-agent row too", () => {
    act(() => setToolElapsed("toolu_agent", 90));
    render([
      { id: "user", role: "user", text: "Go" },
      {
        id: "agent",
        role: "tool",
        text: "Review code",
        tool: { callId: "toolu_agent", kind: "agent", status: "in_progress" },
        agentRun: { name: "Review code", steps: [] },
      },
    ]);
    expect(
      container.querySelector('[data-testid="tool-elapsed"]')?.textContent,
    ).toBe("running · 1m 30s");
  });

  it("is never part of the persisted session", () => {
    setToolElapsed("toolu_bash", 30);
    const session = { ...newSession("claude", "/repo"), blocks: running("toolu_bash") };
    expect(JSON.stringify(sanitizeSessionForPersist(session))).not.toMatch(/elapsed|running ·/);
  });
});
