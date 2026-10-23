import { expect, it } from "vitest";
import {
  newSession,
  type Block,
  type PilotSpawnedSession,
} from "../../sessions/model/session";
import {
  launchedSessionState,
  pilotSpawnedSessions,
  recordPilotSpawnedSession,
  sanitizePilotSpawnedSessions,
} from "./pilotSpawnedSessions";

const launch: PilotSpawnedSession = {
  sessionId: "app-pilot-review",
  cwd: "/repo",
  title: "Review latest PR",
  harness: "codex",
  model: "gpt-6",
};

it("attaches accepted launches to their originating turn without mutating or duplicating them", () => {
  const pilot = newSession("codex", "/repo");
  pilot.blocks = [
    { id: "first", role: "user", text: "Review" },
    { id: "reply", role: "assistant", text: "Started" },
    { id: "second", role: "user", text: "Another task" },
  ];
  const updated = recordPilotSpawnedSession(pilot, "first", launch);
  expect(pilot.blocks[0].pilotSpawnedSessions).toBeUndefined();
  expect(updated.blocks[0].pilotSpawnedSessions).toEqual([launch]);
  expect(updated.blocks[2]).toBe(pilot.blocks[2]);
  expect(pilotSpawnedSessions(updated.blocks.slice(0, 2))).toEqual([launch]);
  expect(pilotSpawnedSessions(updated.blocks.slice(2))).toEqual([]);
  expect(recordPilotSpawnedSession(updated, "first", launch)).toBe(updated);
  expect(recordPilotSpawnedSession(pilot, "missing", launch)).toBe(pilot);
  expect(recordPilotSpawnedSession(pilot, "reply", launch)).toBe(pilot);
});

function receipt(command = "rebase app sessions.start", ok = true): Block {
  return {
    id: "call",
    role: "tool",
    text: command,
    tool: {
      status: "completed",
      detail: JSON.stringify({
        ok,
        result: { ...launch, id: launch.sessionId },
      }),
    },
  };
}

it("recovers confirmed launches from older chats, including shell wrappers, and deduplicates metadata", () => {
  expect(pilotSpawnedSessions([receipt()])).toEqual([launch]);
  expect(
    pilotSpawnedSessions([
      receipt("/bin/zsh -lc 'rebase app sessions.start'"),
    ]),
  ).toEqual([launch]);
  expect(
    pilotSpawnedSessions([
      {
        id: "user",
        role: "user",
        text: "Review",
        pilotSpawnedSessions: [launch],
      },
      receipt(),
      receipt(),
    ]),
  ).toEqual([launch]);
});

it("does not turn failed, pending, unrelated or truncated output into session links", () => {
  const call = receipt();
  expect(
    pilotSpawnedSessions([
      receipt(undefined, false),
      receipt("rebase app sessions.read"),
      receipt("echo rebase app sessions.start"),
      { ...call, role: "assistant" },
      { ...call, tool: { ...call.tool, status: "in_progress" } },
      { ...call, tool: { ...call.tool, detail: '{"ok":true,"result":' } },
    ]),
  ).toEqual([]);
});

function appReceipt(action: string, result: unknown): Block {
  return {
    id: action,
    role: "tool",
    text: action,
    tool: {
      status: "completed",
      preview: {
        kind: "shell",
        title: `/bin/zsh -lc 'rebase app ${action}'`,
      },
      detail: JSON.stringify({ ok: true, result }),
    },
  };
}

it("recovers accepted follow-ups to existing sessions without linking every listed session", () => {
  const other = {
    ...launch,
    sessionId: "existing-session",
    title: "Second article",
  };
  const listing = appReceipt("sessions.list", {
    cwd: launch.cwd,
    sessions: [launch, other].map((entry) => ({
      ...entry,
      id: entry.sessionId,
    })),
  });
  const send = appReceipt("sessions.send", {
    sessionId: other.sessionId,
    submitted: true,
    alreadySubmitted: false,
    notifyOnComplete: true,
  });
  expect(pilotSpawnedSessions([listing])).toEqual([]);
  expect(pilotSpawnedSessions([listing, send, send])).toEqual([other]);
  // The listing can be in a different folded work group from the follow-up.
  expect(pilotSpawnedSessions([send], [other])).toEqual([other]);
  expect(pilotSpawnedSessions([send])).toEqual([]);
  expect(
    pilotSpawnedSessions([
      listing,
      send,
      appReceipt("sessions.send", {
        sessionId: launch.sessionId,
        submitted: true,
        alreadySubmitted: true,
      }),
    ]),
  ).toEqual([other, launch]);
});

it("requires a successful submission and trustworthy session details to recover a follow-up", () => {
  const send = appReceipt("sessions.send", {
    sessionId: launch.sessionId,
    submitted: true,
  });
  const listing = appReceipt("sessions.list", {
    cwd: launch.cwd,
    sessions: [{ ...launch, id: launch.sessionId }],
  });
  const malformedListing = {
    ...listing,
    tool: { ...listing.tool, detail: '{"ok":true,"result":' },
  };
  expect(
    pilotSpawnedSessions([
      listing,
      appReceipt("sessions.send", {
        sessionId: launch.sessionId,
        submitted: false,
      }),
      appReceipt("sessions.read", {
        sessionId: launch.sessionId,
        submitted: true,
      }),
      appReceipt("sessions.draft", {
        sessionId: launch.sessionId,
        submitted: true,
      }),
      appReceipt("sessions.send", { sessionId: "unlisted", submitted: true }),
      { ...send, role: "assistant" },
      { ...send, tool: { ...send.tool, status: "in_progress" } },
      {
        ...send,
        tool: {
          ...send.tool,
          detail: JSON.stringify({
            ok: false,
            result: { sessionId: launch.sessionId, submitted: true },
          }),
        },
      },
    ]),
  ).toEqual([]);
  expect(pilotSpawnedSessions([malformedListing, send])).toEqual([]);
  expect(
    pilotSpawnedSessions([
      { ...listing, tool: { ...listing.tool, status: "failed" } },
      send,
    ]),
  ).toEqual([]);
});

it("keeps a reused session in both Pilots without moving or duplicating its original card", () => {
  const first = {
    ...newSession("codex", "/repo"),
    blocks: [{ id: "first", role: "user" as const, text: "Start" }],
  };
  const second = {
    ...newSession("codex", "/repo"),
    blocks: [{ id: "second", role: "user" as const, text: "Continue" }],
  };
  const original = recordPilotSpawnedSession(first, "first", launch);
  const followUp = recordPilotSpawnedSession(second, "second", launch);
  expect(pilotSpawnedSessions(original.blocks)).toEqual([launch]);
  expect(pilotSpawnedSessions(followUp.blocks)).toEqual([launch]);
  expect(second.blocks[0]).not.toHaveProperty("pilotSpawnedSessions");
  expect(recordPilotSpawnedSession(followUp, "second", launch)).toBe(followUp);
});

it("sanitizes saved references and strips extra fields", () => {
  expect(
    sanitizePilotSpawnedSessions([
      { ...launch, title: "  Review  ", extra: "discard" },
      launch,
      { ...launch, sessionId: "../bad" },
      { ...launch, sessionId: "other", harness: "unknown" },
      { ...launch, sessionId: "empty", cwd: "" },
      null,
    ]),
  ).toEqual([{ ...launch, title: "Review" }]);
  expect(sanitizePilotSpawnedSessions({})).toEqual([]);
});

it("reads a launched session's state for its card", () => {
  const session = newSession("codex", "/repo");
  const ran = (blocks: Block[], extra = {}) =>
    launchedSessionState({ ...session, blocks, ...extra });
  const prompt: Block = { id: "u", role: "user", text: "Fix it" };
  expect(ran([prompt], { busy: true })).toBe("working");
  // Waiting on the user outranks working, so the card asks for attention.
  expect(
    ran(
      [
        prompt,
        { id: "a", role: "tool", text: "rm", approval: { requestId: 1 } },
      ],
      { busy: true },
    ),
  ).toBe("input");
  expect(ran([{ ...prompt, draft: true }])).toBe("draft");
  expect(
    ran([prompt, { id: "e", role: "system", text: "Boom", notice: "error" }]),
  ).toBe("failed");
  expect(
    ran([
      prompt,
      { id: "i", role: "system", text: "Stopped", notice: "interrupt" },
    ]),
  ).toBe("stopped");
  expect(ran([prompt, { id: "r", role: "assistant", text: "Done." }])).toBe(
    "done",
  );
});
