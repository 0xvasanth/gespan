import { describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { onLinkedWork } from "./autoMove";
import { boardWriteBack, type CardLinkKind, type LinkedWorkEvent } from "./board";
import {
  createLinkedTurns,
  settledTurnEvent,
  turnPilotId,
} from "./linkedTurns";

type Call = [CardLinkKind, string, LinkedWorkEvent];

function setup(
  notify: (...call: Call) => Promise<void> = () => Promise.resolve(),
) {
  const calls: Call[] = [];
  const report = vi.fn();
  const turns = createLinkedTurns((kind, id, event) => {
    calls.push([kind, id, event]);
    return notify(kind, id, event);
  }, report);
  return { turns, calls, report };
}

const flush = async () => {
  for (let i = 0; i < 3; i += 1) await Promise.resolve();
};

describe("settledTurnEvent", () => {
  it("maps a turn's outcome to the board event it reports", () => {
    expect(settledTurnEvent("completed")).toBe("completed");
    expect(settledTurnEvent("failed")).toBe("failed");
  });

  it("reports nothing for a turn the user stopped", () => {
    expect(settledTurnEvent("cancelled")).toBeNull();
  });
});

describe("createLinkedTurns", () => {
  it("reports a started turn once, for the session only when it has no Pilot", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1);
    expect(calls).toEqual([["session", "s1", "started"]]);
  });

  it("also reports a Pilot's turn for the Pilot", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1, "p1");
    expect(calls).toEqual([
      ["session", "s1", "started"],
      ["pilot", "p1", "started"],
    ]);
  });

  it("reports a successful settle as completed, for the Pilot seen at start", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1, "p1");
    calls.length = 0;
    turns.settled("s1", 1, "completed");
    expect(calls).toEqual([
      ["session", "s1", "completed"],
      ["pilot", "p1", "completed"],
    ]);
  });

  it("reports an error settle as failed", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1);
    calls.length = 0;
    turns.settled("s1", 1, "failed");
    expect(calls).toEqual([["session", "s1", "failed"]]);
  });

  it("reports nothing when a turn is cancelled, and stays closed afterward", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1, "p1");
    calls.length = 0;
    turns.settled("s1", 1, "cancelled");
    // A late settle for the same turn must not count it as done.
    turns.settled("s1", 1, "completed");
    expect(calls).toEqual([]);
  });

  it("never repeats an event when the same turn starts or settles again", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1);
    turns.started("s1", 1);
    turns.settled("s1", 1, "completed");
    turns.settled("s1", 1, "completed");
    turns.settled("s1", 1, "failed");
    expect(calls).toEqual([
      ["session", "s1", "started"],
      ["session", "s1", "completed"],
    ]);
  });

  it("ignores a settle for a turn that never started", () => {
    const { turns, calls } = setup();
    turns.settled("s1", 1, "failed");
    expect(calls).toEqual([]);
  });

  it("reports every new turn of the same session", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1);
    turns.settled("s1", 1, "completed");
    turns.started("s1", 2);
    turns.settled("s1", 2, "failed");
    expect(calls).toEqual([
      ["session", "s1", "started"],
      ["session", "s1", "completed"],
      ["session", "s1", "started"],
      ["session", "s1", "failed"],
    ]);
  });

  it("keeps turns of different sessions, and superseded turns, apart", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1);
    turns.started("s2", 1, "p2");
    // s1's first turn was stopped and a second one started before it settled.
    turns.started("s1", 2);
    turns.settled("s1", 1, "cancelled");
    turns.settled("s2", 1, "completed");
    turns.settled("s1", 2, "completed");
    expect(calls).toEqual([
      ["session", "s1", "started"],
      ["session", "s2", "started"],
      ["pilot", "p2", "started"],
      ["session", "s1", "started"],
      ["session", "s2", "completed"],
      ["pilot", "p2", "completed"],
      ["session", "s1", "completed"],
    ]);
  });

  it("reports a board failure without throwing, and still sends the rest", async () => {
    const { turns, calls, report } = setup((kind) =>
      kind === "session"
        ? Promise.reject(new Error("database is locked"))
        : Promise.resolve(),
    );
    expect(() => turns.started("s1", 1, "p1")).not.toThrow();
    expect(calls).toEqual([
      ["session", "s1", "started"],
      ["pilot", "p1", "started"],
    ]);
    await flush();
    expect(report).toHaveBeenCalledTimes(1);
    expect(report).toHaveBeenCalledWith(new Error("database is locked"));
  });

  it("reports a notifier that throws synchronously without throwing", () => {
    const { turns, report } = setup(() => {
      throw new Error("boom");
    });
    expect(() => turns.started("s1", 1)).not.toThrow();
    expect(() => turns.settled("s1", 1, "completed")).not.toThrow();
    expect(report).toHaveBeenCalledTimes(2);
  });
});

describe("turnPilotId", () => {
  it("is the Pilot whose own chat the session is", () => {
    expect(turnPilotId({ own: "p1" })).toBe("p1");
  });

  it("is the Pilot a habit run works for", () => {
    expect(turnPilotId({ habitRun: "p2" })).toBe("p2");
  });

  it("is the Pilot that spawned or prompted the session", () => {
    expect(turnPilotId({ spawnedBy: "p3" })).toBe("p3");
  });

  it("prefers the session's own Pilot, then its habit's, then its spawner", () => {
    expect(turnPilotId({ own: "p1", habitRun: "p2", spawnedBy: "p3" })).toBe(
      "p1",
    );
    expect(turnPilotId({ habitRun: "p2", spawnedBy: "p3" })).toBe("p2");
  });

  it("is nobody for a session no Pilot is behind", () => {
    expect(turnPilotId({})).toBeUndefined();
  });
});

describe("Pilot-driven turns on the board", () => {
  it.each([
    ["completed", "completed"],
    ["failed", "failed"],
  ] as const)(
    "reports a spawned session's %s turn to the session and its Pilot",
    (status, event) => {
      const { turns, calls } = setup();
      turns.started("spawned", 1, turnPilotId({ spawnedBy: "p3" }));
      turns.settled("spawned", 1, status);
      expect(calls).toEqual([
        ["session", "spawned", "started"],
        ["pilot", "p3", "started"],
        ["session", "spawned", event],
        ["pilot", "p3", event],
      ]);
    },
  );

  it("reports a habit run's turn to its Pilot", () => {
    const { turns, calls } = setup();
    turns.started("habit-run", 1, turnPilotId({ habitRun: "p2" }));
    turns.settled("habit-run", 1, "failed");
    expect(calls).toEqual([
      ["session", "habit-run", "started"],
      ["pilot", "p2", "started"],
      ["session", "habit-run", "failed"],
      ["pilot", "p2", "failed"],
    ]);
  });

  it("reports a plain session's turn to the session only", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1, turnPilotId({}));
    turns.settled("s1", 1, "completed");
    expect(calls).toEqual([
      ["session", "s1", "started"],
      ["session", "s1", "completed"],
    ]);
  });
});

describe("linked turns through onLinkedWork", () => {
  it("sends each turn event to the store engine as board_linked_event", async () => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
    const report = vi.fn();
    const turns = createLinkedTurns(onLinkedWork, report);
    turns.started("s1", 1, "p1");
    turns.settled("s1", 1, "failed");
    await flush();
    expect(invoke.mock.calls).toEqual([
      ["board_linked_event", { kind: "session", id: "s1", event: "started" }],
      ["board_linked_event", { kind: "pilot", id: "p1", event: "started" }],
      ["board_linked_event", { kind: "session", id: "s1", event: "failed" }],
      ["board_linked_event", { kind: "pilot", id: "p1", event: "failed" }],
    ]);
    expect(report).not.toHaveBeenCalled();
  });

  it("reports a store failure without throwing into the session", async () => {
    invoke.mockReset();
    invoke.mockRejectedValue("database is locked");
    const report = vi.fn();
    const turns = createLinkedTurns(onLinkedWork, report);
    expect(() => turns.started("s1", 1)).not.toThrow();
    await flush();
    expect(report).toHaveBeenCalledWith("database is locked");
  });
});

describe("write-back on a settled turn", () => {
  function withWriteBack(
    write: (key: string, reply: string) => Promise<unknown> = () =>
      Promise.resolve([]),
  ) {
    const calls: Call[] = [];
    const writes: [string, string][] = [];
    const report = vi.fn();
    const turns = createLinkedTurns(
      (kind, id, event) => {
        calls.push([kind, id, event]);
        return Promise.resolve();
      },
      report,
      (key, reply) => {
        writes.push([key, reply]);
        return write(key, reply);
      },
    );
    return { turns, calls, writes, report };
  }

  it("hands a completed turn's final reply to write-back once, keyed by the session", () => {
    const { turns, writes } = withWriteBack();
    turns.started("s1", 1, "p1");
    turns.settled("s1", 1, "completed", "Fixed the login bug.");
    turns.settled("s1", 1, "completed", "Fixed the login bug.");
    // One write for the session; a Pilot behind the turn is not a card author.
    expect(writes).toEqual([["s1", "Fixed the login bug."]]);
  });

  it("passes the reply as is: the store trims, the app does not", () => {
    const { turns, writes } = withWriteBack();
    const long = `  ${"x".repeat(5_000)}\n\n`;
    turns.started("s1", 1);
    turns.settled("s1", 1, "completed", long);
    expect(writes).toEqual([["s1", long]]);
  });

  it("keeps a remote session key as the store knows it", () => {
    const { turns, writes, calls } = withWriteBack();
    turns.started("remote:env-1:abc", 1);
    turns.settled("remote:env-1:abc", 1, "completed", "Done");
    expect(writes).toEqual([["remote:env-1:abc", "Done"]]);
    expect(calls.at(-1)).toEqual(["session", "remote:env-1:abc", "completed"]);
  });

  it.each(["failed", "cancelled"] as const)(
    "writes nothing for a %s turn",
    (status) => {
      const { turns, writes } = withWriteBack();
      turns.started("s1", 1);
      turns.settled("s1", 1, status, "Partial reply before it broke");
      expect(writes).toEqual([]);
    },
  );

  it.each([undefined, "", "   \n"])(
    "writes nothing for an empty reply (%j)",
    (reply) => {
      const { turns, writes, calls } = withWriteBack();
      turns.started("s1", 1);
      turns.settled("s1", 1, "completed", reply);
      expect(writes).toEqual([]);
      // The card still hears the turn completed.
      expect(calls.at(-1)).toEqual(["session", "s1", "completed"]);
    },
  );

  it("writes nothing for a turn that never started or settled twice", () => {
    const { turns, writes } = withWriteBack();
    turns.settled("s1", 7, "completed", "Stray");
    expect(writes).toEqual([]);
  });

  it("reports a write-back failure without throwing into the session", async () => {
    const { turns, report } = withWriteBack(() =>
      Promise.reject("database is locked"),
    );
    turns.started("s1", 1);
    expect(() => turns.settled("s1", 1, "completed", "Done")).not.toThrow();
    await flush();
    expect(report).toHaveBeenCalledWith("database is locked");
  });

  it("calls the store's board_write_back with the session key and reply", async () => {
    invoke.mockReset();
    invoke.mockResolvedValue([]);
    const report = vi.fn();
    const turns = createLinkedTurns(onLinkedWork, report, boardWriteBack);
    turns.started("s1", 1);
    turns.settled("s1", 1, "completed", "All green");
    await flush();
    expect(invoke).toHaveBeenCalledWith("board_write_back", {
      sessionKey: "s1",
      summary: "All green",
    });
    expect(report).not.toHaveBeenCalled();
  });
});

describe("needs input", () => {
  it("reports needs_input when a running session starts waiting, for it and its Pilot", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1, "p1");
    calls.length = 0;
    turns.needsInput(["s1"]);
    expect(calls).toEqual([
      ["session", "s1", "needs_input"],
      ["pilot", "p1", "needs_input"],
    ]);
  });

  it("reports once while the session keeps waiting, and again on a new wait", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1);
    calls.length = 0;
    turns.needsInput(["s1"]);
    turns.needsInput(["s1"]);
    turns.needsInput([]);
    turns.needsInput(["s1"]);
    expect(calls).toEqual([
      ["session", "s1", "needs_input"],
      ["session", "s1", "needs_input"],
    ]);
  });

  it("ignores a session with no open turn (a restored or finished one)", () => {
    const { turns, calls } = setup();
    turns.needsInput(["restored"]);
    turns.started("s1", 1);
    turns.settled("s1", 1, "completed");
    calls.length = 0;
    turns.needsInput(["s1"]);
    expect(calls).toEqual([]);
  });

  it("reports only the sessions that newly wait", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1);
    turns.started("s2", 1);
    turns.needsInput(["s1"]);
    calls.length = 0;
    turns.needsInput(["s1", "s2"]);
    expect(calls).toEqual([["session", "s2", "needs_input"]]);
  });

  it("forgets a wait once its turn settles, so the next turn's wait reports", () => {
    const { turns, calls } = setup();
    turns.started("s1", 1);
    turns.needsInput(["s1"]);
    turns.settled("s1", 1, "completed");
    turns.started("s1", 2);
    calls.length = 0;
    // Still in the waiting set when the new turn hits its own question.
    turns.needsInput(["s1"]);
    expect(calls).toEqual([["session", "s1", "needs_input"]]);
  });
});
