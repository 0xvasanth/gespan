import { describe, expect, it, vi } from "vitest";
import type { CardLinkKind, LinkedWorkEvent } from "./board";
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
