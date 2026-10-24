/**
 * Claude's ~30 s `tool_progress` heartbeats, replayed through the adapter's
 * own stdout handler. Mirrors Rebase Native's
 * `crates/rebase-core/src/provider/tasks/heartbeat_tests.rs`.
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { applyHarnessEvent } from "../../core/apply";
import { newSession, type Session } from "../../../../features/sessions/model/session";
import { sanitizeSessionForPersist } from "../../../../features/sessions/data/sessionStore";
import type { HarnessEvent } from "../../core/types";
import { formatElapsed, toolElapsed } from "../../core/toolElapsed";
import { heartbeatElapsed, isHeartbeatId, isHeartbeatProgress } from "./claudeHeartbeat";
import { parseToolProgress } from "./claudeProtocol";

const sent: string[] = [];
let onLine: ((line: string) => void) | undefined;

vi.mock("../../core/child", () => ({
  resolveClaudeBinary: async () => ({ path: "/fake/claude" }),
  spawnChild: async () => undefined,
  killChild: async () => undefined,
  unwatchChild: () => undefined,
  watchChild: (_id: string, line: (l: string) => void) => {
    onLine = line;
  },
  writeChild: vi.fn(async (_id: string, line: string) => {
    sent.push(line);
  }),
}));

const { sendClaudeTurn, cancelClaudeTurn, __claudeTestReset } = await import("./claude");

function fixture(name: string): string[] {
  return readFileSync(
    resolve(process.cwd(), `tests/fixtures/claude/${name}.jsonl`),
    "utf8",
  )
    .split("\n")
    .filter((line) => line.trim());
}

const waitFor = async (pred: () => boolean, label: string) => {
  for (let i = 0; i < 400; i++) {
    if (pred()) return;
    await new Promise((r) => setTimeout(r, 2));
  }
  throw new Error(`timed out waiting for ${label}`);
};

const isHeartbeatFrame = (line: string) =>
  line.includes('"type":"tool_progress"') && line.includes("-heartbeat-");

/**
 * Plays a fixture as one turn. `after(index, events)` runs after each frame
 * (0-based, over the frames after init) so a test can look mid-run.
 */
async function replay(
  name: string,
  after?: (frame: string, events: HarnessEvent[]) => void,
): Promise<{ events: HarnessEvent[]; session: Session }> {
  const frames = fixture(name);
  const events: HarnessEvent[] = [];
  const turn = sendClaudeTurn({
    sessionId: `heartbeat-${name}`,
    cwd: "/repo",
    model: "claude:claude-sonnet-5",
    modelSettings: {},
    runtimeMode: "supervised",
    text: "go",
    attachments: [],
    onEvent: (event) => events.push(event),
  });
  await waitFor(() => sent.length > 0, "initialize");
  for (const frame of frames.slice(0, 18)) onLine!(frame);
  await waitFor(
    () => sent.some((line) => line.includes('"type":"user"')),
    "prompt",
  );
  for (const frame of frames.slice(18)) {
    onLine!(frame);
    after?.(frame, events);
  }
  await turn;
  return {
    events,
    session: events.reduce(applyHarnessEvent, newSession("claude", "/repo")),
  };
}

beforeEach(() => {
  sent.length = 0;
  onLine = undefined;
  __claudeTestReset();
});
afterEach(() => __claudeTestReset());

describe("heartbeat detection", () => {
  const frame = (rec: Record<string, unknown>) =>
    parseToolProgress({ type: "tool_progress", ...rec })!;

  it("flags the heartbeat marker, or the <parent>-heartbeat-<n> id of its own parent", () => {
    expect(isHeartbeatProgress(frame({ tool_use_id: "x", heartbeat: true }))).toBe(true);
    expect(
      isHeartbeatProgress(
        frame({ tool_use_id: "P-heartbeat-3", parent_tool_use_id: "P" }),
      ),
    ).toBe(true);
  });

  it("does not treat real child progress as a heartbeat", () => {
    expect(
      isHeartbeatProgress(
        frame({ tool_use_id: "toolu_1", parent_tool_use_id: "P", tool_name: "Bash" }),
      ),
    ).toBe(false);
    // The id looks like a heartbeat but names someone else's parent.
    expect(
      isHeartbeatProgress(
        frame({ tool_use_id: "Q-heartbeat-1", parent_tool_use_id: "P" }),
      ),
    ).toBe(false);
    expect(isHeartbeatProgress(frame({ tool_use_id: "Q-heartbeat-1" }))).toBe(false);
    expect(
      isHeartbeatProgress(
        frame({ tool_use_id: "P-heartbeat-x", parent_tool_use_id: "P" }),
      ),
    ).toBe(false);
  });

  it("matches saved ids by the numeric suffix only", () => {
    expect(isHeartbeatId("toolu_1-heartbeat-12")).toBe(true);
    expect(isHeartbeatId("toolu_1-heartbeat-")).toBe(false);
    expect(isHeartbeatId("toolu_1-heartbeat-2x")).toBe(false);
    expect(isHeartbeatId("-heartbeat-1")).toBe(false);
    expect(isHeartbeatId("toolu_1")).toBe(false);
  });

  it("reads elapsed from the frame, or 30*(n+1) when it is missing", () => {
    expect(
      heartbeatElapsed(
        frame({ tool_use_id: "P-heartbeat-1", parent_tool_use_id: "P", elapsed_time_seconds: 61.9 }),
      ),
    ).toEqual({ callId: "P", seconds: 61 });
    expect(
      heartbeatElapsed(frame({ tool_use_id: "P-heartbeat-2", parent_tool_use_id: "P" })),
    ).toEqual({ callId: "P", seconds: 90 });
    expect(heartbeatElapsed(frame({ tool_use_id: "x", heartbeat: true }))).toBeUndefined();
  });

  it("formats Ns under a minute and Mm Ss after", () => {
    expect(formatElapsed(0)).toBe("0s");
    expect(formatElapsed(59)).toBe("59s");
    expect(formatElapsed(60)).toBe("1m 0s");
    expect(formatElapsed(150)).toBe("2m 30s");
  });
});

describe("Agent heartbeats", () => {
  it("add no steps and open no background run", async () => {
    const { session } = await replay("heartbeat-agent");
    const ids = session.blocks.flatMap((b) => [
      b.tool?.callId ?? "",
      ...(b.agentRun?.steps.map((s) => s.id) ?? []),
    ]);
    expect(ids.some((id) => id.includes("-heartbeat-"))).toBe(false);
    expect(session.blocks.some((b) => b.tool?.callId?.startsWith("background:"))).toBe(false);
    const agent = session.blocks.find((b) => b.tool?.callId === "toolu_019fPfTyVvoUGhPAWW4zfBqW");
    expect(agent?.agentRun?.steps.length ?? 0).toBeGreaterThan(0);
    // The real work stays: the agent's own child calls are still steps.
    expect(agent?.agentRun?.steps.every((s) => !isHeartbeatId(s.id))).toBe(true);
  });

  it("emit no agent.step event for a heartbeat", async () => {
    const { events } = await replay("heartbeat-agent");
    const stepIds = events.flatMap((e) => (e.type === "agent.step" ? [e.stepId] : []));
    expect(stepIds.length).toBeGreaterThan(0);
    expect(stepIds.some((id) => id.includes("-heartbeat-"))).toBe(false);
  });

  it("update the running Agent call's live elapsed, then clear it on completion", async () => {
    const seen: Array<number | undefined> = [];
    await replay("heartbeat-agent", (frame) => {
      if (isHeartbeatFrame(frame)) seen.push(toolElapsed("toolu_019fPfTyVvoUGhPAWW4zfBqW"));
    });
    expect(seen).toEqual([30, 60, 90, 120]);
    expect(toolElapsed("toolu_019fPfTyVvoUGhPAWW4zfBqW")).toBeUndefined();
  });
});

describe("Bash heartbeats", () => {
  const BASH = "toolu_014zyZxk9DWdV8hmrxt56vP3";

  it("never open a Background agent run", async () => {
    const { session } = await replay("heartbeat-bash");
    expect(session.blocks.some((b) => b.tool?.callId?.startsWith("background:"))).toBe(false);
    expect(session.blocks.some((b) => /Background agent/.test(b.tool?.title ?? ""))).toBe(false);
    expect(session.blocks.some((b) => b.tool?.callId?.includes("-heartbeat-"))).toBe(false);
  });

  it("show the running Bash call's elapsed live and drop it when the call ends", async () => {
    const seen: Array<number | undefined> = [];
    await replay("heartbeat-bash", (frame) => {
      if (isHeartbeatFrame(frame)) seen.push(toolElapsed(BASH));
    });
    expect(seen).toEqual([30, 60]);
    expect(toolElapsed(BASH)).toBeUndefined();
  });

  it("keep elapsed out of the saved session", async () => {
    const { session } = await replay("heartbeat-bash", (frame) => {
      if (isHeartbeatFrame(frame)) expect(toolElapsed(BASH)).toBeDefined();
    });
    expect(JSON.stringify(sanitizeSessionForPersist(session))).not.toMatch(/elapsed|heartbeat/i);
  });
});

it("ignores a heartbeat for a call this turn does not know", async () => {
  await replay("heartbeat-bash", (frame) => {
    if (isHeartbeatFrame(frame)) expect(toolElapsed("toolu_unknown")).toBeUndefined();
  });
  onLine?.(
    JSON.stringify({
      type: "tool_progress",
      tool_use_id: "toolu_unknown-heartbeat-0",
      parent_tool_use_id: "toolu_unknown",
      tool_name: "Bash",
      heartbeat: true,
      elapsed_time_seconds: 30,
    }),
  );
  expect(toolElapsed("toolu_unknown")).toBeUndefined();
});

describe("elapsed entries for calls that stop without a result", () => {
  const BASH = "toolu_014zyZxk9DWdV8hmrxt56vP3";

  /** Starts a Bash turn and feeds frames up to and including the first heartbeat. */
  async function runningBash(): Promise<{ turn: Promise<void>; frames: string[] }> {
    const frames = fixture("heartbeat-bash");
    const turn = sendClaudeTurn({
      sessionId: "heartbeat-stop",
      cwd: "/repo",
      model: "claude:claude-sonnet-5",
      modelSettings: {},
      runtimeMode: "supervised",
      text: "go",
      attachments: [],
      onEvent: () => undefined,
    });
    await waitFor(() => sent.length > 0, "initialize");
    for (const frame of frames.slice(0, 18)) onLine!(frame);
    await waitFor(() => sent.some((line) => line.includes('"type":"user"')), "prompt");
    const firstBeat = frames.findIndex(isHeartbeatFrame);
    for (const frame of frames.slice(18, firstBeat + 1)) onLine!(frame);
    expect(toolElapsed(BASH)).toBe(30);
    return { turn, frames };
  }

  it("drops the entry when the turn is cancelled with the call still running", async () => {
    const { turn } = await runningBash();
    await cancelClaudeTurn("heartbeat-stop");
    expect(toolElapsed(BASH)).toBeUndefined();
    await turn;
  });

  it("does not bring the entry back for a heartbeat after the turn stopped", async () => {
    const { turn, frames } = await runningBash();
    await cancelClaudeTurn("heartbeat-stop");
    await turn;
    onLine!(frames.filter(isHeartbeatFrame)[1]);
    expect(toolElapsed(BASH)).toBeUndefined();
  });

  it("does not create an entry for a heartbeat that arrives after a Bash tool_result", async () => {
    const frames = fixture("heartbeat-bash");
    const late = frames.filter(isHeartbeatFrame)[1];
    const seen: Array<number | undefined> = [];
    await replay("heartbeat-bash", (frame) => {
      if (frame.includes('"type":"tool_result"')) {
        onLine!(late);
        seen.push(toolElapsed(BASH));
      }
    });
    expect(seen).toEqual([undefined]);
    expect(toolElapsed(BASH)).toBeUndefined();
  });
});
