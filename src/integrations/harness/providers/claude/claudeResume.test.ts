/**
 * Replays the captured resumed-background-agent stream through the Claude
 * adapter's own stdout handler (the `watchChild` line callback), the same
 * entry point every Claude stdout line takes. Mirrors Gespan Native's
 * `crates/gespan-core/src/provider/tasks/resume_tests.rs`.
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { applyHarnessEvent } from "../../core/apply";
import {
  newSession,
  type AgentRunMeta,
  type Block,
  type Session,
} from "../../../../features/sessions/model/session";
import type { HarnessEvent } from "../../core/types";

const sent: string[] = [];
let onLine: ((line: string) => void) | undefined;

vi.mock("../../../../../host/core/integrations/harness/core/child", () => ({
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

const {
  bindClaudeSession,
  restoreClaudeAgentRuns,
  sendClaudeTurn,
  steerClaudeTurn,
  stopClaudeSession,
  __claudeTestReset,
} = await import("./claude");

const FRAMES = readFileSync(
  resolve(process.cwd(), "tests/fixtures/claude/resumed-background-agent.jsonl"),
  "utf8",
)
  .split("\n")
  .filter((line) => line.trim());
const AGENT = "toolu_019x6aJjknXrg5CSy9dvntEn";
const SEND = "toolu_01LfGgNGSbW1AVPVpfLsAYkh";
const AGENT_ID = "aba491c492e906740";
const PROVIDER_SESSION = "afda8336-cc48-4d2b-b8ab-d1ab1dbd059b";
/** Lines 1..=125 are turn 1; 126..=238 are turn 2 and Claude's own wake. */
const TURN_ONE = 125;
const RESUMED_CHILDREN = [
  "toolu_01HWq7foDHczTjtHuoNvz4M7",
  "toolu_013E6GSRiGhqgcQ3vbtT7XNg",
  "toolu_01QVuG3c6ZordypvcUmhhfD6",
  "toolu_01CZhBxnaocVpUQuvCrXoa9C",
  "toolu_01LEVibA1bj4U6XFyozZ5QFk",
];
const NAME = "Run echo and sleep sequence";

/** 1-based fixture lines `from..=to`. */
function lines(from: number, to = FRAMES.length): string[] {
  return FRAMES.slice(from - 1, to);
}

function summary(toolUseId: string): string {
  for (const line of FRAMES) {
    const frame = JSON.parse(line) as Record<string, unknown>;
    if (
      frame.subtype === "task_notification" &&
      frame.tool_use_id === toolUseId
    ) {
      return frame.summary as string;
    }
  }
  throw new Error(`no notification for ${toolUseId}`);
}

function parsedSent() {
  return sent.map((line) => JSON.parse(line) as Record<string, unknown>);
}

function userMessages(): number {
  return parsedSent().filter((m) => m.type === "user").length;
}

function initializeRequests(): number {
  return parsedSent().filter(
    (m) => (m.request as Record<string, unknown> | undefined)?.subtype === "initialize",
  ).length;
}

const waitFor = async (pred: () => boolean, label: string) => {
  for (let i = 0; i < 400; i++) {
    if (pred()) return;
    await new Promise((r) => setTimeout(r, 2));
  }
  throw new Error(`timed out waiting for ${label}`);
};

/** Each fed line's events, so a test can tell which frame emitted what. */
function feed(events: HarnessEvent[], frames: string[]): HarnessEvent[][] {
  return frames.map((frame) => {
    const before = events.length;
    onLine!(frame);
    return events.slice(before);
  });
}

/**
 * Sends a user turn. A new process gets `preInit` (or a bare control
 * response) once it asks to initialize; the turn is open once its prompt is
 * written.
 */
async function openTurn(
  sessionId: string,
  events: HarnessEvent[],
  preInit?: string[],
): Promise<{ turn: Promise<void> }> {
  const users = userMessages();
  const inits = initializeRequests();
  const turn = sendClaudeTurn({
    sessionId,
    cwd: "/repo",
    model: "claude:claude-sonnet-5",
    modelSettings: {},
    runtimeMode: "supervised",
    text: "go",
    attachments: [],
    onEvent: (event) => events.push(event),
  });
  await waitFor(
    () => initializeRequests() > inits || userMessages() > users,
    "initialize or prompt",
  );
  if (initializeRequests() > inits) {
    if (preInit) feed(events, preInit);
    else
      onLine!(
        JSON.stringify({
          type: "control_response",
          response: { subtype: "success", request_id: "gespan_2" },
        }),
      );
  }
  await waitFor(() => userMessages() > users, "user prompt");
  return { turn };
}

/** Turn 1 (lines 1..=125) in a fresh process for `sessionId`. */
async function replayTurnOne(sessionId: string, events: HarnessEvent[]) {
  const { turn } = await openTurn(sessionId, events, lines(1, 18));
  feed(events, lines(19, TURN_ONE));
  await turn;
}

function sessionFrom(events: HarnessEvent[], blocks: Block[] = []): Session {
  return events.reduce(applyHarnessEvent, {
    ...newSession("claude", "/repo"),
    blocks,
  });
}

function block(session: Session, callId: string): Block {
  const found = session.blocks.find((b) => b.tool?.callId === callId);
  if (!found) throw new Error(`no block for ${callId}`);
  return found;
}

function run(session: Session, callId: string): AgentRunMeta {
  const found = block(session, callId).agentRun;
  if (!found) throw new Error(`${callId} is not an agent run`);
  return found;
}

function toolSteps(agent: AgentRunMeta) {
  return agent.steps.filter((step) => step.kind === "tool");
}

function markers(events: HarnessEvent[]): string[] {
  return events.flatMap((event) =>
    event.type === "interjection" && event.customType === "task-notification"
      ? [event.text]
      : [],
  );
}

function backgroundDetails(events: HarnessEvent[]) {
  return events.flatMap((event) =>
    event.type === "background.updated" ? [event.details ?? []] : [],
  );
}

function assertResumedRun(session: Session) {
  const resumed = run(session, SEND);
  expect(resumed.resumeOf).toBe(AGENT);
  expect(resumed.agentId).toBe(AGENT_ID);
  expect(resumed.name).toBe(`Resumed · ${NAME}`);
  const steps = toolSteps(resumed);
  expect(steps.map((step) => step.id)).toEqual(RESUMED_CHILDREN);
  expect(steps.every((step) => step.status === "completed")).toBe(true);
  const finalText = (
    JSON.parse(FRAMES[192]) as {
      message: { content: { text: string }[] };
    }
  ).message.content[0].text;
  expect(
    resumed.steps.some(
      (step) => step.kind === "message" && step.text === finalText.trim(),
    ),
  ).toBe(true);
  expect(block(session, SEND).tool?.status).toBe("completed");
  expect(block(session, SEND).tool?.detail).toBe(summary(SEND).trim());
  expect(resumed.activity).toBe("Running Print resumed-3");
}

/** Turn 1 replayed, saved and loaded back as a restart would find it. */
async function persistedTurnOne(): Promise<Block[]> {
  const events: HarnessEvent[] = [];
  await replayTurnOne("seed", events);
  await stopClaudeSession("seed");
  __claudeTestReset();
  const { sanitizeSessionForPersist } = await import(
    "../../../../features/sessions/data/sessionStore"
  );
  const saved = sanitizeSessionForPersist(sessionFrom(events));
  return JSON.parse(JSON.stringify(saved.blocks)) as Block[];
}

/** A fresh process for `sessionId` resuming the fixture conversation. */
function rebind(sessionId: string, blocks: Block[]) {
  bindClaudeSession(sessionId, PROVIDER_SESSION, "/repo");
  restoreClaudeAgentRuns(sessionId, blocks);
}

function lateChild(parent: string, id: string): string {
  return JSON.stringify({
    type: "assistant",
    parent_tool_use_id: parent,
    session_id: PROVIDER_SESSION,
    message: {
      id,
      content: [
        { type: "tool_use", id, name: "Bash", input: { command: "pwd" } },
      ],
    },
  });
}

function stepOwner(events: HarnessEvent[], stepId: string): string {
  const step = events.find(
    (event) => event.type === "agent.step" && event.stepId === stepId,
  );
  if (!step || step.type !== "agent.step")
    throw new Error(`child ${stepId} was not attached`);
  return step.callId;
}

const INIT = JSON.stringify({
  type: "system",
  subtype: "init",
  session_id: PROVIDER_SESSION,
});

beforeEach(() => {
  sent.length = 0;
  __claudeTestReset();
});

afterEach(async () => {
  for (const id of ["s1", "s2", "seed"]) await stopClaudeSession(id);
  __claudeTestReset();
});

describe("Claude resumed background agents", () => {
  it("keeps the turn-1 foreground agent's child steps", async () => {
    const events: HarnessEvent[] = [];
    await replayTurnOne("s1", events);
    const session = sessionFrom(events);
    const agent = run(session, AGENT);
    expect(toolSteps(agent).map((step) => step.id)).toEqual([
      "toolu_017gf48arc1zKr5QDXoyqy5C",
      "toolu_01Ry2tgDyg3yBxyKAGJA1qhf",
    ]);
    expect(toolSteps(agent).every((s) => s.status === "completed")).toBe(true);
    // The hand-back trailer and the task events both name the agent.
    expect(agent.agentId).toBe(AGENT_ID);
    expect(agent.name).toBe(NAME);
    expect(block(session, AGENT).tool?.status).toBe("completed");
  });

  it("turns the SendMessage block into the resumed run with every resumed step", async () => {
    const events: HarnessEvent[] = [];
    await replayTurnOne("s1", events);
    const { turn } = await openTurn("s1", events);
    feed(events, lines(TURN_ONE + 1));
    await turn;
    const session = sessionFrom(events);
    assertResumedRun(session);
    // History stays append-only: the original Agent block gains nothing.
    expect(toolSteps(run(session, AGENT))).toHaveLength(2);
    expect(block(session, AGENT).tool?.detail).toContain("step-one");
  });

  it("takes status, summary and activity from task events, never the name", async () => {
    const events: HarnessEvent[] = [];
    await replayTurnOne("s1", events);
    const { turn } = await openTurn("s1", events);
    // Through the last task_progress (line 191): still running, name intact.
    feed(events, lines(TURN_ONE + 1, 191));
    let session = sessionFrom(events);
    let resumed = run(session, SEND);
    expect(block(session, SEND).tool?.status).toBe("in_progress");
    expect(resumed.name).toBe(`Resumed · ${NAME}`);
    expect(resumed.activity).toBe("Running Print resumed-3");
    // The launch receipt is not a report.
    expect(block(session, SEND).tool?.detail ?? "").not.toContain("resumedAgentId");
    // Through the task_notification (line 195).
    feed(events, lines(192, 195));
    session = sessionFrom(events);
    resumed = run(session, SEND);
    expect(block(session, SEND).tool?.status).toBe("completed");
    expect(block(session, SEND).tool?.detail).toBe(summary(SEND).trim());
    expect(resumed.name).toBe(`Resumed · ${NAME}`);
    feed(events, lines(196));
    await turn;
  });

  it("points the background card at the resumed block and counts its steps", async () => {
    const events: HarnessEvent[] = [];
    await replayTurnOne("s1", events);
    const { turn } = await openTurn("s1", events);
    feed(events, lines(TURN_ONE + 1, 192));
    const cards = backgroundDetails(events);
    const task = cards.at(-1)?.find((row) => row.id === AGENT_ID);
    expect(task).toMatchObject({
      toolId: SEND,
      latest: "Running Print resumed-3",
      steps: 5,
    });
    feed(events, lines(193, 195));
    expect(
      backgroundDetails(events).at(-1)?.some((row) => row.id === AGENT_ID),
    ).toBeFalsy();
    feed(events, lines(196));
    await turn;
  });

  it("seeds from persisted blocks after a restart and routes turn 2", async () => {
    const blocks = await persistedTurnOne();
    rebind("s2", blocks);
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    feed(events, lines(TURN_ONE + 1, 192));
    const task = backgroundDetails(events)
      .at(-1)
      ?.find((row) => row.id === AGENT_ID);
    expect(task).toMatchObject({ toolId: SEND, steps: 5 });
    feed(events, lines(193));
    await turn;
    // Turn 1's block is not re-emitted.
    expect(
      events.some(
        (event) => "callId" in event && event.callId === AGENT,
      ),
    ).toBe(false);
    assertResumedRun(sessionFrom(events, blocks));
  });

  it("gives a second resume its own steps", async () => {
    const replace = (frame: string) => {
      let text = frame.split(SEND).join("toolu_S2");
      RESUMED_CHILDREN.forEach((child, index) => {
        text = text.split(child).join(`S2_child${index + 1}`);
      });
      return text;
    };
    const events: HarnessEvent[] = [];
    await replayTurnOne("s1", events);
    const { turn } = await openTurn("s1", events);
    feed(events, lines(TURN_ONE + 1, 195));
    feed(events, lines(158, 195).map(replace));
    feed(events, lines(196));
    await turn;
    const session = sessionFrom(events);
    const first = run(session, SEND);
    const second = run(session, "toolu_S2");
    expect(toolSteps(first)).toHaveLength(5);
    expect(toolSteps(second)).toHaveLength(5);
    expect(second.resumeOf).toBe(AGENT);
    expect(toolSteps(second)[0].id).toBe("S2_child1");
    expect(toolSteps(first).some((s) => s.id.includes("S2_child"))).toBe(false);
    expect(block(session, "toolu_S2").tool?.status).toBe("completed");
  });

  it("keeps a child of an unknown parent as a visible Background agent step", async () => {
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s1", events);
    feed(events, [lateChild("toolu_unknown", "child")]);
    const owner = stepOwner(events, "child");
    expect(owner.startsWith("background:toolu_unknown:")).toBe(true);
    let session = sessionFrom(events);
    expect(run(session, owner).name).toBe("Background agent");
    // Its result lands on the same step rather than being dropped.
    feed(events, [
      JSON.stringify({
        type: "user",
        parent_tool_use_id: "toolu_unknown",
        message: {
          content: [
            { type: "tool_result", tool_use_id: "child", content: "listing" },
          ],
        },
      }),
    ]);
    session = sessionFrom(events);
    const fallback = run(session, owner);
    expect(fallback.steps).toHaveLength(1);
    expect(fallback.steps[0]).toMatchObject({ id: "child", status: "completed" });
    expect(block(session, owner).tool?.status).toBe("completed");
    feed(events, [JSON.stringify({ type: "result", subtype: "success" })]);
    await turn;
  });

  it("gives a known but finished dispatch one continuation block per turn", async () => {
    rebind("s2", await persistedTurnOne());
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    feed(events, [lateChild(AGENT, "late-1"), lateChild(AGENT, "late-2")]);
    const owner = stepOwner(events, "late-1");
    expect(stepOwner(events, "late-2")).toBe(owner);
    expect(owner.startsWith(`continued:${AGENT}:`)).toBe(true);
    const session = sessionFrom(events);
    const continued = run(session, owner);
    expect(continued.name).toBe(`Continued · ${NAME}`);
    expect(continued.steps).toHaveLength(2);
    expect(continued.resumeOf).toBe(AGENT);
    expect(continued.continuation).toBe(true);
    // Nothing reports when stray work ends: the block must not spin forever.
    expect(block(session, owner).tool?.status).toBe("completed");
    expect(continued.steps[0].status).toBe("in_progress");
    feed(events, [JSON.stringify({ type: "result", subtype: "success" })]);
    await turn;
  });

  it("never repeats a synthetic block id across turns and processes", async () => {
    const blocks = await persistedTurnOne();
    rebind("s2", blocks);
    const events: HarnessEvent[] = [];
    let { turn } = await openTurn("s2", events);
    feed(events, [INIT, lateChild(AGENT, "late-1"), lateChild("toolu_unknown", "lost-1")]);
    feed(events, [INIT, lateChild(AGENT, "late-2"), lateChild("toolu_unknown", "lost-2")]);
    feed(events, [JSON.stringify({ type: "result", subtype: "success" })]);
    await turn;
    // A new process starts its own turn count again.
    await stopClaudeSession("s2");
    ({ turn } = await openTurn("s2", events));
    feed(events, [INIT, lateChild(AGENT, "late-3"), lateChild("toolu_unknown", "lost-3")]);
    const continued = ["late-1", "late-2", "late-3"].map((id) =>
      stepOwner(events, id),
    );
    const lost = ["lost-1", "lost-2", "lost-3"].map((id) =>
      stepOwner(events, id),
    );
    for (const id of continued) expect(id.startsWith("continued:")).toBe(true);
    for (const id of lost) expect(id.startsWith("background:")).toBe(true);
    expect(new Set([...continued, ...lost]).size).toBe(6);
    // Within one turn the same parent keeps one block.
    feed(events, [lateChild(AGENT, "late-4")]);
    expect(stepOwner(events, "late-4")).toBe(continued[2]);
    feed(events, [JSON.stringify({ type: "result", subtype: "success" })]);
    await turn;
  });

  it("marks Claude's self-started turn with the notification summary, once", async () => {
    const events: HarnessEvent[] = [];
    await replayTurnOne("s1", events);
    const { turn } = await openTurn("s1", events);
    const perLine = feed(events, lines(TURN_ONE + 1));
    await turn;
    // The marker leads the follow-up: it comes with the wake turn's init.
    expect(markers(perLine[197 - TURN_ONE - 1])).toEqual([summary(SEND)]);
    expect(markers(events)).toHaveLength(1);
    // Stored exactly as Gespan Native stores it: an interjection row.
    const row = sessionFrom(events).blocks.find(
      (b) => b.interjection?.customType === "task-notification",
    );
    expect(row).toMatchObject({
      role: "system",
      text: summary(SEND),
      interjection: { customType: "task-notification" },
    });
  });

  it("does not mark a user follow-up as a wake, but still marks Claude's own", async () => {
    rebind("s2", await persistedTurnOne());
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    feed(events, lines(TURN_ONE + 1, 195));
    await steerClaudeTurn({
      sessionId: "s2",
      cwd: "/repo",
      model: "claude:claude-sonnet-5",
      text: "Thanks, what next?",
    });
    // The turn that answers the user starts, streams and finishes.
    const answer = feed(events, lines(196, 237));
    expect(markers(answer.flat())).toEqual([]);
    const userResult = feed(events, [
      JSON.stringify({
        type: "result",
        subtype: "success",
        result: "Here is what is next.",
        session_id: PROVIDER_SESSION,
      }),
    ]);
    expect(markers(userResult.flat())).toEqual([]);
    await turn;
    // Claude's own wake is still marked, by its result's origin.
    const wake = feed(events, lines(238));
    expect(markers(wake.flat())).toEqual([summary(SEND)]);
  });

  it("marks Claude's own wake after a restart when it starts", async () => {
    rebind("s2", await persistedTurnOne());
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    const perLine = feed(events, lines(TURN_ONE + 1));
    await turn;
    expect(markers(perLine[197 - TURN_ONE - 1])).toEqual([summary(SEND)]);
    expect(markers(events)).toHaveLength(1);
  });

  it("seeds from a Native-written block that names its agent by nativeIds only", async () => {
    const blocks: Block[] = [
      {
        id: "native-block",
        role: "tool",
        text: NAME,
        tool: { callId: AGENT, kind: "agent", title: NAME, status: "completed" },
        agentRun: { name: NAME, steps: [], nativeIds: [AGENT_ID] } as AgentRunMeta,
      },
    ];
    rebind("s2", blocks);
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    feed(events, lines(TURN_ONE + 1));
    await turn;
    assertResumedRun(sessionFrom(events, blocks));
  });
});

describe("Claude resumed background agents: parity details", () => {
  it("stores each child result's output on its step", async () => {
    const events: HarnessEvent[] = [];
    await replayTurnOne("s1", events);
    const { turn } = await openTurn("s1", events);
    feed(events, lines(TURN_ONE + 1));
    await turn;
    const steps = toolSteps(run(sessionFrom(events), SEND));
    expect(steps[0].detail).toBe("resumed-1");
    // Output is stored whatever the outcome; failure is still the status.
    expect(steps[0].status).toBe("completed");
  });

  it("merges a later seed into a registry the app already holds", async () => {
    const blocks = await persistedTurnOne();
    // An earlier bind left an empty registry for this thread.
    rebind("s2", []);
    restoreClaudeAgentRuns("s2", blocks);
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    feed(events, lines(TURN_ONE + 1));
    await turn;
    assertResumedRun(sessionFrom(events, blocks));
  });

  it("seeds through the real bindHarnessSession hook", async () => {
    const blocks = await persistedTurnOne();
    const { ensureClaudeRegistered } = await import("./claudeAdapter");
    const { bindHarnessSession } = await import("../../core/registry");
    ensureClaudeRegistered();
    bindHarnessSession("claude", "s2", PROVIDER_SESSION, "/repo", undefined, blocks);
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    feed(events, lines(TURN_ONE + 1));
    await turn;
    assertResumedRun(sessionFrom(events, blocks));
  });

  it("treats a running resumption as a live parent", async () => {
    const events: HarnessEvent[] = [];
    await replayTurnOne("s1", events);
    const { turn } = await openTurn("s1", events);
    feed(events, lines(TURN_ONE + 1, 175));
    feed(events, [lateChild(SEND, "under-send")]);
    expect(stepOwner(events, "under-send")).toBe(SEND);
    feed(events, lines(176));
    await turn;
  });

  it("ends the resumption on any terminal status, as Native does", async () => {
    rebind("s2", await persistedTurnOne());
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    feed(events, lines(TURN_ONE + 1, 175));
    feed(events, [
      JSON.stringify({
        type: "system",
        subtype: "task_updated",
        task_id: AGENT_ID,
        patch: { status: "cancelled" },
      }),
      lateChild(AGENT, "after-cancel"),
    ]);
    expect(stepOwner(events, "after-cancel").startsWith(`continued:${AGENT}:`)).toBe(true);
    feed(events, [
      JSON.stringify({
        type: "system",
        subtype: "task_notification",
        task_id: AGENT_ID,
        tool_use_id: SEND,
        status: "cancelled",
      }),
      // Claude takes the thread back up, then yields with nothing running.
      INIT,
      JSON.stringify({ type: "result", subtype: "success" }),
    ]);
    await turn;
  });

  it("falls back to the task description for the wake marker", async () => {
    const events: HarnessEvent[] = [];
    await replayTurnOne("s1", events);
    const { turn } = await openTurn("s1", events);
    const notice = JSON.parse(FRAMES[194]) as Record<string, unknown>;
    delete notice.summary;
    notice.description = NAME;
    const perLine = feed(events, [
      ...lines(TURN_ONE + 1, 194),
      JSON.stringify(notice),
      ...lines(196),
    ]);
    await turn;
    expect(markers(perLine.flat())).toEqual([NAME]);
  });

  it("marks each of two close wakes with its own summary, in order", async () => {
    rebind("s2", await persistedTurnOne());
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    // The first agent's notification is line 195; a second background task
    // finishes right behind it, before Claude wakes for either.
    feed(events, lines(TURN_ONE + 1, 195));
    feed(events, [
      JSON.stringify({
        type: "system",
        subtype: "task_started",
        task_id: "bg-2",
        description: "Second job",
        is_backgrounded: true,
        task_type: "local_bash",
        session_id: PROVIDER_SESSION,
      }),
      JSON.stringify({
        type: "system",
        subtype: "task_notification",
        task_id: "bg-2",
        status: "completed",
        summary: "Second job finished",
        session_id: PROVIDER_SESSION,
      }),
    ]);
    const wakeResult = JSON.stringify({
      type: "result",
      subtype: "success",
      origin: { kind: "task-notification" },
      session_id: PROVIDER_SESSION,
    });
    const first = feed(events, [INIT, wakeResult]);
    expect(markers(first[0])).toEqual([summary(SEND)]);
    // The first wake's result must not take the second summary early.
    expect(markers(first[1])).toEqual([]);
    const second = feed(events, [INIT, wakeResult]);
    expect(markers(second[0])).toEqual(["Second job finished"]);
    expect(markers(second[1])).toEqual([]);
    await turn;
    expect(markers(events)).toEqual([summary(SEND), "Second job finished"]);
  });

  it("marks two wakes whose starts went unseen from their results, in order", async () => {
    rebind("s2", await persistedTurnOne());
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    feed(events, lines(TURN_ONE + 1, 195));
    feed(events, [
      JSON.stringify({
        type: "system",
        subtype: "task_started",
        task_id: "bg-2",
        description: "Second job",
        is_backgrounded: true,
        task_type: "local_bash",
        session_id: PROVIDER_SESSION,
      }),
      JSON.stringify({
        type: "system",
        subtype: "task_notification",
        task_id: "bg-2",
        status: "completed",
        summary: "Second job finished",
        session_id: PROVIDER_SESSION,
      }),
    ]);
    const wakeResult = JSON.stringify({
      type: "result",
      subtype: "success",
      origin: { kind: "task-notification" },
      session_id: PROVIDER_SESSION,
    });
    const results = feed(events, [wakeResult, wakeResult]);
    expect(markers(results[0])).toEqual([summary(SEND)]);
    expect(markers(results[1])).toEqual(["Second job finished"]);
    // No wake started, so the turn waits out its grace; stopping settles it.
    await stopClaudeSession("s2");
    await turn.catch(() => undefined);
  });

  /** The first agent's notification (line 195) plus a second job's. */
  function secondNotification(): string[] {
    return [
      JSON.stringify({
        type: "system",
        subtype: "task_started",
        task_id: "bg-2",
        description: "Second job",
        is_backgrounded: true,
        task_type: "local_bash",
        session_id: PROVIDER_SESSION,
      }),
      JSON.stringify({
        type: "system",
        subtype: "task_notification",
        task_id: "bg-2",
        status: "completed",
        summary: "Second job finished",
        session_id: PROVIDER_SESSION,
      }),
    ];
  }
  const WAKE_RESULT = JSON.stringify({
    type: "result",
    subtype: "success",
    origin: { kind: "task-notification" },
    session_id: PROVIDER_SESSION,
  });

  it("opens the window for the next wake at the first wake's result, however long it ran", async () => {
    rebind("s2", await persistedTurnOne());
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    feed(events, lines(TURN_ONE + 1, 195));
    feed(events, secondNotification());
    vi.useFakeTimers();
    try {
      const start = feed(events, [INIT]);
      expect(markers(start[0])).toEqual([summary(SEND)]);
      // The wake runs far longer than the 15 s grace.
      vi.advanceTimersByTime(60_000);
      const result = feed(events, [WAKE_RESULT]);
      expect(markers(result[0])).toEqual([]);
      // Counted from the result: still waiting after 10 s, so the turn is open.
      vi.advanceTimersByTime(10_000);
      let settled = false;
      void turn.then(() => {
        settled = true;
      });
      await Promise.resolve();
      expect(settled).toBe(false);
      const second = feed(events, [INIT, WAKE_RESULT]);
      expect(markers(second[0])).toEqual(["Second job finished"]);
      expect(markers(second[1])).toEqual([]);
      await turn;
      expect(markers(events)).toEqual([summary(SEND), "Second job finished"]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not wait for another wake, or mark the reply, when the user steers during a wake", async () => {
    rebind("s2", await persistedTurnOne());
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    feed(events, lines(TURN_ONE + 1, 195));
    feed(events, secondNotification());
    const start = feed(events, [INIT]);
    expect(markers(start[0])).toEqual([summary(SEND)]);
    await steerClaudeTurn({
      sessionId: "s2",
      cwd: "/repo",
      model: "claude:claude-sonnet-5",
      text: "Also check the logs",
    });
    const rest = feed(events, [WAKE_RESULT, INIT]);
    await turn;
    // The reply to the user carries no wake marker.
    expect(markers(rest.flat())).toEqual([]);
    expect(markers(events)).toEqual([summary(SEND)]);
  });

  it("does not carry a summary left by one turn into a later turn", async () => {
    rebind("s2", await persistedTurnOne());
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    feed(events, lines(TURN_ONE + 1, 195));
    // The user's follow-up wins the race and leaves the summary unused.
    await steerClaudeTurn({
      sessionId: "s2",
      cwd: "/repo",
      model: "claude:claude-sonnet-5",
      text: "Thanks, what next?",
    });
    feed(events, [
      ...lines(196, 237),
      JSON.stringify({ type: "result", subtype: "success", session_id: PROVIDER_SESSION }),
    ]);
    await turn;
    // A later turn whose own notification set nothing.
    const later = await openTurn("s2", events);
    const out = feed(events, [
      INIT,
      JSON.stringify({
        type: "result",
        subtype: "success",
        origin: { kind: "task-notification" },
        session_id: PROVIDER_SESSION,
      }),
    ]);
    await later.turn;
    expect(markers(out.flat())).toEqual([]);
  });

  it("opens no synthetic block for a frame it cannot attach", async () => {
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s1", events);
    feed(events, [
      JSON.stringify({
        type: "tool_progress",
        tool_use_id: "child",
        parent_tool_use_id: "toolu_unknown",
      }),
    ]);
    expect(
      events.some((event) => event.type === "tool.started"),
    ).toBe(false);
    feed(events, [JSON.stringify({ type: "result", subtype: "success" })]);
    await turn;
  });

  it("stamps the wake marker row with sentAt, as Native does", async () => {
    const events: HarnessEvent[] = [];
    await replayTurnOne("s1", events);
    const { turn } = await openTurn("s1", events);
    feed(events, lines(TURN_ONE + 1));
    await turn;
    const row = sessionFrom(events).blocks.find(
      (b) => b.interjection?.customType === "task-notification",
    );
    expect(typeof row?.sentAt).toBe("number");
  });

  it("marks the user's turn before its write resolves", async () => {
    rebind("s2", await persistedTurnOne());
    const events: HarnessEvent[] = [];
    const { turn } = await openTurn("s2", events);
    feed(events, lines(TURN_ONE + 1, 195));
    // Claude starts the answering turn before the write settles.
    const steer = steerClaudeTurn({
      sessionId: "s2",
      cwd: "/repo",
      model: "claude:claude-sonnet-5",
      text: "Thanks, what next?",
    });
    const answer = feed(events, lines(196, 237));
    await steer;
    expect(markers(answer.flat())).toEqual([]);
    feed(events, [
      JSON.stringify({ type: "result", subtype: "success", session_id: PROVIDER_SESSION }),
    ]);
    await turn;
  });
});
