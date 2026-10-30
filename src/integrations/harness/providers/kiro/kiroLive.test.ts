import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Msg = Record<string, unknown>;

const sent: Msg[] = [];
const spawned: string[][] = [];
let onLine: (line: string) => void;
let onExit: (code: number | null) => void;
let onStderr: ((line: string) => void) | undefined;
let agent: (message: Msg) => void;

vi.mock("../../../../../host/core/integrations/harness/core/child", () => ({
  resolveKiroBinary: async () => ({ path: "/fake/kiro-cli" }),
  spawnChild: async (_id: string, _path: string, args: string[]) => {
    spawned.push(args);
  },
  killChild: async () => undefined,
  unwatchChild: () => undefined,
  watchChild: (
    _id: string,
    line: (value: string) => void,
    exit: (code: number | null) => void,
    stderr?: (value: string) => void,
  ) => {
    onLine = line;
    onExit = exit;
    onStderr = stderr;
  },
  writeChild: async (_id: string, raw: string) => {
    const message = JSON.parse(raw) as Msg;
    sent.push(message);
    queueMicrotask(() => agent(message));
  },
}));

const {
  sendKiroTurn,
  cancelKiroTurn,
  respondKiroApproval,
  stopKiroSession,
  bindKiroSession,
} = await import("./kiro");
import type {
  HarnessEvent,
  SendTurnInput,
} from "../../core/types";

// --- fake Kiro -------------------------------------------------------------

type Script = {
  init?: unknown;
  sessionNew?: unknown;
  /** A string is an RPC error message. */
  sessionNewError?: string;
  sessionLoad?: unknown;
  sessionLoadError?: string;
  /** Runs before `session/load` is answered, to simulate history replay. */
  beforeLoadResult?: () => void;
  prompt?: (message: Msg) => void;
};
let script: Script;

const reply = (id: unknown, result: unknown) =>
  onLine(JSON.stringify({ jsonrpc: "2.0", id, result }));
const fail = (id: unknown, message: string) =>
  onLine(JSON.stringify({ jsonrpc: "2.0", id, error: { code: -32000, message } }));
const notify = (method: string, params: unknown) =>
  onLine(JSON.stringify({ jsonrpc: "2.0", method, params }));
const askAgentRequest = (id: number, method: string, params: unknown) =>
  onLine(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
const update = (value: Record<string, unknown>, meta?: unknown) =>
  notify("session/update", {
    sessionId: "kiro-1",
    update: { ...value, ...(meta ? { _meta: meta } : {}) },
  });

function defaultAgent(message: Msg) {
  const id = message.id;
  if (id === undefined || !message.method) return;
  switch (message.method) {
    case "initialize":
      return reply(id, script.init ?? { agentCapabilities: { loadSession: false } });
    case "session/new":
      return script.sessionNewError
        ? fail(id, script.sessionNewError)
        : reply(id, script.sessionNew ?? { sessionId: "kiro-1" });
    case "session/load":
      script.beforeLoadResult?.();
      return script.sessionLoadError
        ? fail(id, script.sessionLoadError)
        : reply(id, script.sessionLoad ?? {});
    case "session/prompt":
      return script.prompt
        ? script.prompt(message)
        : reply(id, { stopReason: "end_turn" });
    default:
      return reply(id, {});
  }
}

// --- helpers ---------------------------------------------------------------

let events: HarnessEvent[] = [];
let counter = 0;
const threads: string[] = [];

function newThread() {
  const id = `kiro-thread-${++counter}`;
  threads.push(id);
  return id;
}

function input(sessionId: string, over: Partial<SendTurnInput> = {}): SendTurnInput {
  return {
    sessionId,
    cwd: "/repo",
    model: "kiro:default",
    modelSettings: {},
    runtimeMode: "supervised",
    text: "hello",
    attachments: [],
    onEvent: (event) => events.push(event),
    ...over,
  };
}

const methods = () => sent.map((message) => message.method).filter(Boolean);
const outbound = (method: string) => sent.filter((m) => m.method === method);
const eventTypes = () => events.map((event) => event.type);

async function waitFor(predicate: () => boolean, label: string) {
  for (let i = 0; i < 300; i++) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  throw new Error(`timed out waiting for ${label}; sent=${JSON.stringify(methods())}`);
}

const permission = (kind: string, options = standardOptions) => ({
  sessionId: "kiro-1",
  toolCall: { toolCallId: "call-9", title: `Run ${kind}`, kind },
  options,
});

const standardOptions = [
  { optionId: "ok-1", name: "Allow once", kind: "allow_once" },
  { optionId: "ok-always", name: "Always allow", kind: "allow_always" },
  { optionId: "no-1", name: "Reject once", kind: "reject_once" },
];

function permissionResponse() {
  return sent.find((m) => m.id === 900 && m.result !== undefined)?.result as
    | { outcome: { outcome: string; optionId?: string } }
    | undefined;
}

beforeEach(() => {
  sent.length = 0;
  spawned.length = 0;
  events = [];
  script = {};
  onStderr = undefined;
  agent = defaultAgent;
});

afterEach(async () => {
  vi.useRealTimers();
  for (const id of threads.splice(0)) await stopKiroSession(id);
});

// --- tests -----------------------------------------------------------------

describe("handshake", () => {
  it("initializes, opens a session in the project, and sends the prompt", async () => {
    const thread = newThread();
    const accepted = vi.fn();
    await sendKiroTurn(input(thread, { onAccepted: accepted, text: "explain this" }));

    expect(spawned).toEqual([["acp", "--agent-engine=v3"]]);
    expect(methods().slice(0, 3)).toEqual(["initialize", "session/new", "session/prompt"]);
    expect(outbound("initialize")[0].params).toMatchObject({
      protocolVersion: 1,
      clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false },
    });
    expect(outbound("session/new")[0].params).toEqual({ cwd: "/repo", mcpServers: [] });
    expect(outbound("session/prompt")[0].params).toEqual({
      sessionId: "kiro-1",
      prompt: [{ type: "text", text: "explain this" }],
    });
    expect(accepted).toHaveBeenCalledTimes(1);
    expect(events).toContainEqual({
      type: "session.providerBound",
      providerSessionId: "kiro-1",
    });
    expect(eventTypes()).toContain("session.started");
  });

  it("reuses the live process for a second turn in the same project", async () => {
    const thread = newThread();
    await sendKiroTurn(input(thread));
    await sendKiroTurn(input(thread, { text: "again" }));
    expect(outbound("initialize")).toHaveLength(1);
    expect(outbound("session/new")).toHaveLength(1);
    expect(outbound("session/prompt")).toHaveLength(2);
  });

  it("answers any agent request it does not implement with method-not-found", async () => {
    const thread = newThread();
    script.prompt = (message) => {
      askAgentRequest(700, "_kiro/auth/getAccessToken", {});
      setTimeout(() => reply(message.id, { stopReason: "end_turn" }), 10);
    };
    await sendKiroTurn(input(thread));
    const refusal = sent.find((m) => m.id === 700);
    expect(refusal?.error).toMatchObject({ code: -32601 });
  });
});

describe("streaming", () => {
  it("turns chunks into transcript events and closes the bubble when the prompt resolves", async () => {
    const thread = newThread();
    script.prompt = (message) => {
      update({ sessionUpdate: "agent_thought_chunk", content: { type: "text", text: "hmm" } });
      update({ sessionUpdate: "agent_message_chunk", content: { type: "text", text: "Hel" } });
      update({ sessionUpdate: "agent_message_chunk", content: { type: "text", text: "lo" } });
      reply(message.id, { stopReason: "end_turn" });
    };
    await sendKiroTurn(input(thread));
    expect(events.filter((e) => e.type === "message.delta")).toEqual([
      { type: "message.delta", text: "Hel" },
      { type: "message.delta", text: "lo" },
    ]);
    expect(events).toContainEqual({ type: "reasoning.delta", text: "hmm" });
    expect(eventTypes()).toContain("message.completed");
  });

  it("tracks a tool call from start to completion", async () => {
    const thread = newThread();
    script.prompt = (message) => {
      update({
        sessionUpdate: "tool_call",
        toolCallId: "c1",
        title: "Reading README.md",
        kind: "read",
        status: "pending",
      });
      update({ sessionUpdate: "tool_call_update", toolCallId: "c1", status: "completed" });
      reply(message.id, { stopReason: "end_turn" });
    };
    await sendKiroTurn(input(thread));
    const tool = events.filter(
      (e) => (e.type === "tool.started" || e.type === "tool.updated") && e.callId === "c1",
    );
    expect(tool.length).toBeGreaterThanOrEqual(2);
    expect(tool.at(-1)).toMatchObject({ callId: "c1", status: "completed" });
  });

  it("ignores Kiro bookkeeping updates it has no transcript for", async () => {
    const thread = newThread();
    script.prompt = (message) => {
      update({ sessionUpdate: "session_info_update" }, { kiro: { kind: "context_usage", usagePercentage: 12 } });
      update({ sessionUpdate: "current_mode_update", currentModeId: "kiro_default" });
      update({ sessionUpdate: "available_commands_update", availableCommands: [] });
      reply(message.id, { stopReason: "end_turn" });
    };
    await expect(sendKiroTurn(input(thread))).resolves.toBeUndefined();
    expect(events.some((e) => e.type === "message.delta")).toBe(false);
    expect(events.some((e) => e.type === "session.error")).toBe(false);
  });

  it("tells the user when Kiro declines a request", async () => {
    const thread = newThread();
    script.prompt = (message) => reply(message.id, { stopReason: "refusal" });
    await sendKiroTurn(input(thread));
    expect(events.find((e) => e.type === "status")).toMatchObject({
      text: expect.stringMatching(/declin/i),
    });
  });
});

describe("permissions", () => {
  const askThenFinish = (kind: string, options = standardOptions) => {
    script.prompt = (message) => {
      askAgentRequest(900, "session/request_permission", permission(kind, options));
      const finish = setInterval(() => {
        if (!permissionResponse()) return;
        clearInterval(finish);
        reply(message.id, { stopReason: "end_turn" });
      }, 5);
    };
  };

  it("asks the user in supervised mode and answers with the offered option", async () => {
    const thread = newThread();
    askThenFinish("execute");
    const turn = sendKiroTurn(input(thread));
    await waitFor(() => eventTypes().includes("approval.requested"), "approval.requested");
    expect(events.find((e) => e.type === "approval.requested")).toMatchObject({
      requestId: 900,
      kind: "execute",
    });
    respondKiroApproval(thread, 900, "allow");
    await turn;
    expect(permissionResponse()).toEqual({ outcome: { outcome: "selected", optionId: "ok-1" } });
    expect(events).toContainEqual({ type: "approval.resolved", requestId: 900, decision: "allow" });
  });

  it("rejects with the offered reject option when the user denies", async () => {
    const thread = newThread();
    askThenFinish("execute");
    const turn = sendKiroTurn(input(thread));
    await waitFor(() => eventTypes().includes("approval.requested"), "approval.requested");
    respondKiroApproval(thread, 900, "deny");
    await turn;
    expect(permissionResponse()?.outcome.optionId).toBe("no-1");
  });

  it("approves without asking at full access, preferring the always option", async () => {
    const thread = newThread();
    askThenFinish("execute");
    await sendKiroTurn(input(thread, { runtimeMode: "full-access" }));
    expect(eventTypes()).not.toContain("approval.requested");
    expect(permissionResponse()?.outcome.optionId).toBe("ok-always");
  });

  it("lets a plan turn read but never write", async () => {
    const reading = newThread();
    askThenFinish("read");
    await sendKiroTurn(input(reading, { intent: "plan", runtimeMode: "full-access" }));
    expect(permissionResponse()?.outcome.optionId).toBe("ok-1");

    sent.length = 0;
    const writing = newThread();
    askThenFinish("edit");
    await sendKiroTurn(input(writing, { intent: "plan", runtimeMode: "full-access" }));
    expect(permissionResponse()?.outcome.optionId).toBe("no-1");
  });

  it("declines with the offered reject option when nothing can be allowed", async () => {
    const thread = newThread();
    askThenFinish("execute", [{ optionId: "only-reject", name: "No", kind: "reject_once" }]);
    await sendKiroTurn(input(thread, { runtimeMode: "full-access" }));
    expect(eventTypes()).not.toContain("approval.requested");
    expect(permissionResponse()?.outcome).toEqual({ outcome: "selected", optionId: "only-reject" });
  });

  it("cancels the request instead of inventing an option when none is offered", async () => {
    const thread = newThread();
    askThenFinish("execute", []);
    await sendKiroTurn(input(thread, { runtimeMode: "full-access" }));
    expect(permissionResponse()).toEqual({ outcome: { outcome: "cancelled" } });
  });
});

describe("cancel", () => {
  async function startBlockedTurn(thread: string) {
    let promptId: unknown;
    script.prompt = (message) => {
      promptId = message.id;
      askAgentRequest(900, "session/request_permission", permission("execute"));
    };
    const turn = sendKiroTurn(input(thread));
    await waitFor(() => eventTypes().includes("approval.requested"), "approval.requested");
    return { turn, finishPrompt: () => reply(promptId, { stopReason: "cancelled" }) };
  }

  it("answers the pending permission as cancelled and sends session/cancel", async () => {
    const thread = newThread();
    const { turn, finishPrompt } = await startBlockedTurn(thread);
    const cancelling = cancelKiroTurn(thread);
    await waitFor(() => outbound("session/cancel").length === 1, "session/cancel");
    expect(outbound("session/cancel")[0].params).toEqual({ sessionId: "kiro-1" });
    expect(permissionResponse()).toEqual({ outcome: { outcome: "cancelled" } });
    finishPrompt();
    await cancelling;
    await expect(turn).resolves.toBeUndefined();
    expect(eventTypes()).not.toContain("session.error");
  });

  it("holds the next prompt until Kiro settles the cancelled one", async () => {
    const thread = newThread();
    const { turn, finishPrompt } = await startBlockedTurn(thread);
    const cancelling = cancelKiroTurn(thread);
    await waitFor(() => outbound("session/cancel").length === 1, "session/cancel");

    script.prompt = (message) => reply(message.id, { stopReason: "end_turn" });
    const next = sendKiroTurn(input(thread, { text: "second" }));
    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(outbound("session/prompt")).toHaveLength(1);

    finishPrompt();
    await Promise.all([cancelling, turn, next]);
    expect(outbound("session/prompt")).toHaveLength(2);
    expect(JSON.stringify(outbound("session/prompt")[1].params)).toContain("second");
  });

  it("stops waiting for a cancelled prompt Kiro never answers", async () => {
    const thread = newThread();
    const { turn } = await startBlockedTurn(thread);
    vi.useFakeTimers();
    const cancelling = cancelKiroTurn(thread);
    await vi.advanceTimersByTimeAsync(8_500);
    await cancelling;
    vi.useRealTimers();
    await expect(turn).resolves.toBeUndefined();
  });

  it("does not send a prompt when the turn is cancelled while Kiro is still starting", async () => {
    const thread = newThread();
    let release: (() => void) | undefined;
    agent = (message) => {
      if (message.method === "initialize") release = () => defaultAgent(message);
      else defaultAgent(message);
    };
    const turn = sendKiroTurn(input(thread));
    await waitFor(() => release !== undefined, "initialize");
    await cancelKiroTurn(thread);
    release!();
    await turn;
    expect(outbound("session/prompt")).toHaveLength(0);
  });

  it("does not let an idle cancel swallow the next message", async () => {
    const thread = newThread();
    await cancelKiroTurn(thread);
    await sendKiroTurn(input(thread));
    expect(outbound("session/prompt")).toHaveLength(1);
  });
});

describe("resume", () => {
  const withLoad = { agentCapabilities: { loadSession: true } };

  it("loads the saved Kiro session and drops replayed history", async () => {
    const thread = newThread();
    script.init = withLoad;
    script.beforeLoadResult = () => {
      update({ sessionUpdate: "agent_message_chunk", content: { type: "text", text: "old" } });
      update(
        { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "older" } },
        { kiro: { replay: true } },
      );
    };
    script.prompt = (message) => {
      update({ sessionUpdate: "agent_message_chunk", content: { type: "text", text: "new" } });
      reply(message.id, { stopReason: "end_turn" });
    };
    bindKiroSession(thread, "kiro-old", "/repo");
    await sendKiroTurn(input(thread));

    expect(outbound("session/load")[0].params).toEqual({
      sessionId: "kiro-old",
      cwd: "/repo",
      mcpServers: [],
    });
    expect(outbound("session/new")).toHaveLength(0);
    expect(outbound("session/prompt")[0].params).toMatchObject({ sessionId: "kiro-old" });
    expect(events.filter((e) => e.type === "message.delta")).toEqual([
      { type: "message.delta", text: "new" },
    ]);
  });

  it("drops replay-marked history even when it arrives during a live turn", async () => {
    const thread = newThread();
    script.prompt = (message) => {
      update(
        { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "replayed" } },
        { kiro: { replay: true } },
      );
      update({ sessionUpdate: "agent_message_chunk", content: { type: "text", text: "live" } });
      reply(message.id, { stopReason: "end_turn" });
    };
    await sendKiroTurn(input(thread));
    expect(events.filter((e) => e.type === "message.delta")).toEqual([
      { type: "message.delta", text: "live" },
    ]);
  });

  it("starts a fresh session when Kiro does not advertise loadSession", async () => {
    const thread = newThread();
    bindKiroSession(thread, "kiro-old", "/repo");
    await sendKiroTurn(input(thread));
    expect(outbound("session/load")).toHaveLength(0);
    expect(outbound("session/new")).toHaveLength(1);
  });

  it("falls back to a fresh session when the saved one can no longer be loaded", async () => {
    const thread = newThread();
    script.init = withLoad;
    script.sessionLoadError = "session not found";
    bindKiroSession(thread, "kiro-gone", "/repo");
    await sendKiroTurn(input(thread));
    expect(outbound("session/new")).toHaveLength(1);
    expect(outbound("session/prompt")[0].params).toMatchObject({ sessionId: "kiro-1" });
  });

  it("does not load a session saved for a different project", async () => {
    const thread = newThread();
    script.init = withLoad;
    bindKiroSession(thread, "kiro-old", "/elsewhere");
    await sendKiroTurn(input(thread));
    expect(outbound("session/load")).toHaveLength(0);
  });
});

describe("model selection", () => {
  const configSession = {
    sessionId: "kiro-1",
    configOptions: [
      {
        id: "model",
        category: "model",
        currentValue: "auto",
        options: [
          { value: "auto", name: "Auto" },
          { value: "opus", name: "Opus" },
        ],
      },
    ],
  };

  it("sets a V3 model through the config option", async () => {
    const thread = newThread();
    script.sessionNew = configSession;
    await sendKiroTurn(input(thread, { model: "kiro:opus" }));
    expect(outbound("session/set_config_option")[0].params).toEqual({
      sessionId: "kiro-1",
      configId: "model",
      value: "opus",
    });
    expect(methods().indexOf("session/set_config_option")).toBeLessThan(
      methods().indexOf("session/prompt"),
    );
  });

  it("only changes the model when it differs from the current one", async () => {
    const thread = newThread();
    script.sessionNew = configSession;
    await sendKiroTurn(input(thread, { model: "kiro:auto" }));
    await sendKiroTurn(input(thread, { model: "kiro:opus" }));
    await sendKiroTurn(input(thread, { model: "kiro:opus" }));
    expect(outbound("session/set_config_option")).toHaveLength(1);
  });

  it("uses session/set_model for a legacy models state", async () => {
    const thread = newThread();
    script.sessionNew = {
      sessionId: "kiro-1",
      models: { currentModelId: "auto", availableModels: [{ modelId: "auto" }, { modelId: "opus" }] },
    };
    await sendKiroTurn(input(thread, { model: "kiro:opus" }));
    expect(outbound("session/set_model")[0].params).toEqual({
      sessionId: "kiro-1",
      modelId: "opus",
    });
  });

  it("leaves Kiro's configured model alone for the default entry or no model control", async () => {
    const thread = newThread();
    script.sessionNew = configSession;
    await sendKiroTurn(input(thread, { model: "kiro:default" }));
    const bare = newThread();
    script.sessionNew = { sessionId: "kiro-1" };
    await sendKiroTurn(input(bare, { model: "kiro:opus" }));
    expect(outbound("session/set_config_option")).toHaveLength(0);
    expect(outbound("session/set_model")).toHaveLength(0);
  });
});

describe("failures", () => {
  it("tells a signed-out user how to log in", async () => {
    const thread = newThread();
    script.sessionNewError = "Not logged in";
    await expect(sendKiroTurn(input(thread))).rejects.toThrow(/kiro-cli login/);
  });

  it("surfaces a sign-in failure printed on stderr", async () => {
    const thread = newThread();
    script.prompt = (message) => {
      onStderr?.("Error: authentication expired");
      reply(message.id, { stopReason: "end_turn" });
    };
    await sendKiroTurn(input(thread));
    expect(events.find((e) => e.type === "session.error")).toMatchObject({
      message: expect.stringContaining("kiro-cli login"),
    });
  });

  it("does not report ordinary stderr chatter as an error", async () => {
    const thread = newThread();
    script.prompt = (message) => {
      onStderr?.("[info] loaded 3 steering files");
      reply(message.id, { stopReason: "end_turn" });
    };
    await sendKiroTurn(input(thread));
    expect(eventTypes()).not.toContain("session.error");
  });

  it("reports the session ended when Kiro exits after startup", async () => {
    const thread = newThread();
    await sendKiroTurn(input(thread));
    onExit(1);
    expect(events).toContainEqual({ type: "session.ended", code: 1 });
    // The next turn starts a fresh process instead of writing to a dead one.
    await sendKiroTurn(input(thread, { text: "after" }));
    expect(outbound("initialize")).toHaveLength(2);
  });

  it("rejects an image Kiro cannot take when there is no file to point at", async () => {
    const thread = newThread();
    await expect(
      sendKiroTurn(
        input(thread, {
          attachments: [
            { id: "i", name: "paste.png", mimeType: "image/png", kind: "image", size: 4, data: "AAAA" },
          ],
        }),
      ),
    ).rejects.toThrow(/paste\.png/);
    expect(outbound("session/prompt")).toHaveLength(0);
  });
});
