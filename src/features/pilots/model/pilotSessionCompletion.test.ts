import { describe, expect, it } from "vitest";
import { submitWithSettlement } from "../../../app/model/managedSubmission";
import type { ControlOutcome } from "../../orchestration/model/orchestration";
import {
  canDispatchQueuedHead,
  canSteerQueuedHead,
  dequeueQueuedMessage,
  queuedMessageForSubmit,
} from "../../sessions/model/messageQueue";
import {
  newSession,
  type QueuedMessage,
  type Session,
} from "../../sessions/model/session";
import {
  dismissQueuedPilotSessionCompletion,
  enqueuePilotSessionCompletion,
  pilotSessionCompletionMessage,
  pilotSessionCompletionResult,
  PilotSessionCompletionBatches,
} from "./pilotSessionCompletion";

const options = {
  requestId: "app-pilot-request",
  sessionId: "worker",
  project: "/code/project",
  prompt: "Fix the API and run tests",
  outcome: {
    status: "completed",
    text: "The API is fixed and tests pass.",
  } as ControlOutcome,
};

function groupedMessage() {
  let message!: QueuedMessage;
  const batches = new PilotSessionCompletionBatches((_, ready) => {
    message = ready;
  });
  for (const [sessionId, status] of [
    ["stopped", "cancelled"],
    ["failed", "failed"],
    ["completed", "completed"],
  ] as const) {
    batches.watch(
      { pilotId: "pilot", turn: 1 },
      sessionId,
    )(
      pilotSessionCompletionResult({
        ...options,
        sessionId,
        requestId: sessionId,
        outcome: { status, text: `Result for ${sessionId}` },
      }),
    );
  }
  batches.closeInactive(() => false);
  return message;
}

describe("Pilot session completion notifications", () => {
  it("accepts immediately, then queues completion behind chat without steering a busy Pilot", async () => {
    let pilot: Session = {
      ...newSession("codex", "/code/project"),
      busy: true,
      turnReady: true,
      queuedMessages: [
        { id: "chat", text: "Also explain the design", attachments: [] },
      ],
    };
    let settle!: (outcome: ControlOutcome) => void;
    await expect(
      submitWithSettlement({
        submit: (done) => {
          settle = done;
          return true;
        },
        onSettled: (outcome) => {
          pilot = enqueuePilotSessionCompletion(
            pilot,
            pilotSessionCompletionMessage({ ...options, outcome }),
          );
        },
        rejectionMessage: "Could not start",
      }),
    ).resolves.toBe(true);
    expect(pilot.queuedMessages).toHaveLength(1);
    settle(options.outcome);
    settle(options.outcome);
    expect(pilot.queuedMessages?.map((message) => message.id)).toEqual([
      "chat",
      "pilot-completion-app-pilot-request",
    ]);
    const next = dequeueQueuedMessage(pilot, "chat");
    expect(canSteerQueuedHead(next)).toBe(false);
    expect(
      queuedMessageForSubmit(next, "pilot-completion-app-pilot-request", "steer"),
    ).toBeUndefined();
    expect(canDispatchQueuedHead(next)).toBe(false);
    expect(canDispatchQueuedHead({ ...next, busy: false })).toBe(true);
  });

  it("uses the final answer from the watched turn even after a later turn begins", () => {
    const session = newSession("codex", options.project);
    session.title = "API fix";
    session.blocks = [
      {
        id: "u",
        role: "user",
        text: options.prompt,
        appRequestId: options.requestId,
      },
      { id: "progress", role: "assistant", text: "I am checking" },
      {
        id: "tool",
        role: "tool",
        text: "private tool output",
        tool: { kind: "shell" },
      },
      {
        id: "reply",
        role: "assistant",
        text: "Fixed the endpoint; 12 tests pass.",
      },
      { id: "later", role: "user", text: "New task" },
      { id: "later-reply", role: "assistant", text: "Unrelated answer" },
    ];
    const message = pilotSessionCompletionMessage({ ...options, session });
    expect(message.text).toContain("Fixed the endpoint; 12 tests pass.");
    expect(message.text).toContain(options.project);
    expect(message.text).toContain(options.prompt);
    expect(message.text).not.toContain("Unrelated answer");
    expect(message.text).not.toContain("private tool output");
    expect(message.pilotSessionCompletion).toEqual({
      sessionId: "worker",
      title: "API fix",
      status: "completed",
    });
  });

  it.each(["failed", "cancelled"] as const)(
    "reports %s with available output and an error",
    (status) => {
      const message = pilotSessionCompletionMessage({
        ...options,
        outcome: { status, text: "Partial work", error: "Agent stopped" },
      });
      expect(message.pilotSessionCompletion?.status).toBe(status);
      expect(message.text).toContain("Partial work");
      expect(message.text).toContain("Agent stopped");
    },
  );

  it("deduplicates both pending and consumed receipts and preserves a paused queue", () => {
    const pilot = newSession("codex", options.project);
    const message = pilotSessionCompletionMessage(options);
    const next = enqueuePilotSessionCompletion(
      { ...pilot, queueStatus: "paused" },
      message,
    );
    expect(next.queueStatus).toBe("paused");
    expect(enqueuePilotSessionCompletion(next, message)).toBe(next);
    const delivered = {
      ...pilot,
      blocks: [
        {
          id: "notice",
          role: "user" as const,
          text: message.text,
          appRequestId: message.id,
          internal: true,
        },
      ],
    };
    expect(enqueuePilotSessionCompletion(delivered, message)).toBe(delivered);
  });

  it("dismisses a queued session report while keeping chat, other reports and the consumed transcript", () => {
    const target = pilotSessionCompletionMessage(options);
    const other = pilotSessionCompletionMessage({
      ...options,
      requestId: "other-request",
      sessionId: "other",
    });
    const chat = { id: "chat", text: "Explain the change", attachments: [] };
    const pilot: Session = {
      ...newSession("codex", options.project),
      queueStatus: "paused",
      queuedMessages: [chat, target, other],
      blocks: [
        {
          id: "old-notice",
          role: "user",
          text: target.text,
          appRequestId: target.id,
          internal: true,
        },
      ],
    };
    const next = dismissQueuedPilotSessionCompletion(pilot, "worker");
    expect(next.queuedMessages).toEqual([chat, other]);
    expect(next.queueStatus).toBe("paused");
    expect(next.blocks).toBe(pilot.blocks);
    expect(dismissQueuedPilotSessionCompletion(next, "worker")).toBe(next);
  });

  it("removes reports from a saved batch and updates its identity, count and aggregate status", () => {
    // A save/load keeps the JSON text and metadata; it has no in-memory watches.
    const message = JSON.parse(
      JSON.stringify(groupedMessage()),
    ) as QueuedMessage;
    const pilot: Session = {
      ...newSession("codex", options.project),
      queuedMessages: [message],
    };
    const next = dismissQueuedPilotSessionCompletion(pilot, "failed");
    const remaining = next.queuedMessages![0];
    expect(remaining.id).toBe(message.id);
    expect(remaining.pilotSessionCompletion).toEqual({
      sessionId: "stopped",
      title: "2 session results",
      status: "cancelled",
      sessionCount: 2,
    });
    expect(
      JSON.parse(remaining.text.split("\n\n")[1]).sessions.map(
        (entry: { sessionId: string }) => entry.sessionId,
      ),
    ).toEqual(["stopped", "completed"]);

    const last = dismissQueuedPilotSessionCompletion(next, "stopped");
    expect(last.queuedMessages![0].id).toBe(message.id);
    expect(last.queuedMessages![0].pilotSessionCompletion).toEqual({
      sessionId: "completed",
      title: "Agent session",
      status: "completed",
    });
    expect(
      JSON.parse(last.queuedMessages![0].text.split("\n\n")[1]).sessionId,
    ).toBe("completed");
    expect(
      dismissQueuedPilotSessionCompletion(last, "completed").queuedMessages,
    ).toEqual([]);
  });

  it("leaves unrelated or unrecognized saved batches untouched", () => {
    const message = groupedMessage();
    const pilot: Session = {
      ...newSession("codex", options.project),
      queuedMessages: [message],
    };
    expect(dismissQueuedPilotSessionCompletion(pilot, "unrelated")).toBe(pilot);
    const malformed: Session = {
      ...pilot,
      queuedMessages: [{ ...message, text: "Incomplete saved report" }],
    };
    expect(dismissQueuedPilotSessionCompletion(malformed, "stopped")).toBe(
      malformed,
    );
  });

  it("bounds long results and points the Pilot to the source conversation", () => {
    const message = pilotSessionCompletionMessage({
      ...options,
      outcome: { status: "completed", text: "x".repeat(15_000) },
    });
    expect(message.text).toContain('"truncated":true');
    expect(message.text).toContain("app sessions.read");
    expect(message.text.length).toBeLessThan(13_000);
  });
});
