/**
 * Both apps read and write `sessions.blocks_json`. Rebase Native adds optional
 * fields to `agentRun`; Tauri must load them and keep them when it rewrites the
 * block.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { applyHarnessEvent } from "../../../integrations/harness/core/apply";
import type { SessionRecord } from "./sessionStore";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

const { readSession, sanitizeSessionForPersist } = await import("./sessionStore");

function record(agentRun: Record<string, unknown>): SessionRecord {
  return {
    id: "s1",
    harness: "claude",
    model: "claude-sonnet-5",
    cwd: "/repo",
    providerSessionId: "afda8336-cc48-4d2b-b8ab-d1ab1dbd059b",
    blocks: [
      { id: "b0", role: "user", text: "resume it" },
      {
        id: "b1",
        role: "tool",
        text: "Resumed · Review",
        tool: {
          callId: "toolu_send",
          title: "Resumed · Review",
          kind: "agent",
          status: "in_progress",
        },
        agentRun,
      },
    ],
  } as unknown as SessionRecord;
}

async function load(agentRun: Record<string, unknown>) {
  invoke.mockImplementation((cmd: string) =>
    Promise.resolve(cmd === "session_get" ? record(agentRun) : null),
  );
  const session = await readSession("s1");
  if (!session) throw new Error("not loaded");
  return session;
}

describe("agentRun fields shared with Rebase Native", () => {
  beforeEach(() => invoke.mockReset());

  it("keeps Native's new fields when Tauri appends a step and saves", async () => {
    // The exact shape Native's `desktop_agent` writes (tool/tasks.rs).
    const session = await load({
      name: "Resumed · Review",
      model: null,
      agentType: null,
      nativeIds: ["agent-x"],
      nativeBrief: "Check the parser",
      steps: [
        {
          id: "a",
          kind: "tool",
          text: "ls",
          detail: "",
          status: "completed",
          toolKind: "execute",
          detailOmitted: false,
          nativePresentation: { nativeName: "Bash", nativeInput: '{"command":"ls"}' },
        },
      ],
      agentId: "agent-x",
      resumeOf: "toolu_agent",
      activity: "Reading src",
      continuation: true,
    });
    const updated = applyHarnessEvent(session, {
      type: "agent.step",
      callId: "toolu_send",
      stepId: "b",
      kind: "tool",
      text: "cat README",
      status: "in_progress",
    });
    const saved = sanitizeSessionForPersist(updated).blocks[1].agentRun;
    expect(saved?.steps.map((step) => step.id)).toEqual(["a", "b"]);
    // Step-level Native fields (nativePresentation) are a known follow-up.
    expect(saved).toMatchObject({
      agentId: "agent-x",
      resumeOf: "toolu_agent",
      activity: "Reading src",
      continuation: true,
      nativeIds: ["agent-x"],
      nativeBrief: "Check the parser",
    });
  });

  it("keeps the fields when a tool update renames the row", async () => {
    const session = await load({
      name: "Resumed · Review",
      steps: [],
      agentId: "agent-x",
      resumeOf: "toolu_agent",
    });
    const updated = applyHarnessEvent(session, {
      type: "tool.updated",
      callId: "toolu_send",
      title: "Review",
      kind: "agent",
      status: "completed",
    });
    const run = sanitizeSessionForPersist(updated).blocks[1].agentRun;
    // A resumption keeps its own name, not the task's description.
    expect(run).toMatchObject({
      name: "Resumed · Review",
      agentId: "agent-x",
      resumeOf: "toolu_agent",
    });
  });

  it("loads old blocks without the new fields and writes none of them", async () => {
    const session = await load({
      name: "Review",
      steps: [{ id: "a", kind: "tool", text: "ls" }],
    });
    const saved = sanitizeSessionForPersist(session).blocks[1].agentRun;
    expect(saved?.name).toBe("Review");
    for (const key of ["agentId", "resumeOf", "activity", "continuation"]) {
      expect(saved && key in saved, key).toBe(false);
    }
    expect(JSON.stringify(saved)).not.toContain("null");
  });

  it("omits continuation when it is false, as Native does", async () => {
    const session = await load({
      name: "Review",
      steps: [],
      continuation: false,
    });
    const updated = applyHarnessEvent(session, {
      type: "agent.run",
      callId: "toolu_send",
      activity: "Reading src",
    });
    const saved = sanitizeSessionForPersist(updated).blocks[1].agentRun;
    expect(saved?.activity).toBe("Reading src");
    expect(saved && "continuation" in saved).toBe(false);
  });
});
