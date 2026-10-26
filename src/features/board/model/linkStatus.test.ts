import { describe, expect, it } from "vitest";
import {
  checksState,
  latestEventByCard,
  parsePrUrl,
  pilotState,
  prState,
  runningPairs,
  sessionState,
  settledSince,
} from "./linkStatus";

describe("link status", () => {
  it("a session is running, then waiting, then failed, then completed", () => {
    expect(sessionState({ running: true, waiting: true, failed: true })).toBe("running");
    expect(sessionState({ running: false, waiting: true, failed: true })).toBe("waiting");
    expect(sessionState({ running: false, waiting: false, failed: true })).toBe("failed");
    expect(sessionState({ running: false, waiting: false, failed: false })).toBe("completed");
  });

  it("a Pilot is running or idle", () => {
    expect(pilotState(true)).toBe("running");
    expect(pilotState(false)).toBe("idle");
  });

  it("reads gh PR states in any case and anything else as unknown", () => {
    expect(prState("OPEN")).toBe("open");
    expect(prState("Merged")).toBe("merged");
    expect(prState("closed")).toBe("closed");
    expect(prState("DRAFT")).toBe("unknown");
    expect(prState(null)).toBe("unknown");
  });

  it("checks: failing beats pending beats passing; skipped and cancelled don't count", () => {
    expect(checksState([{ state: "pass" }, { state: "fail" }, { state: "pending" }])).toBe("failing");
    expect(checksState([{ state: "pass" }, { state: "pending" }])).toBe("pending");
    expect(checksState([{ state: "pass" }, { state: "skipping" }])).toBe("passing");
    expect(checksState([{ state: "skipping" }, { state: "cancel" }])).toBeUndefined();
    expect(checksState([])).toBeUndefined();
  });

  it("parses only github.com pull request links", () => {
    expect(parsePrUrl("https://github.com/acme/api/pull/42")).toEqual({ repo: "acme/api", number: 42 });
    expect(parsePrUrl("https://github.com/acme/api/pull/42/files?x=1")).toBeNull();
    expect(parsePrUrl("https://github.com/acme/api/pull/42?diff=split")).toEqual({ repo: "acme/api", number: 42 });
    expect(parsePrUrl("https://evil.example/acme/api/pull/42")).toBeNull();
    expect(parsePrUrl("https://github.com/acme/api/issues/42")).toBeNull();
    expect(parsePrUrl("https://github.com/acme/api/pull/0")).toBeNull();
  });

  it("running pairs skip flow runs and stopped work, sorted", () => {
    expect(
      runningPairs([
        { kind: "session", id: "b", status: "running" },
        { kind: "session", id: "a", status: "running" },
        { kind: "pilot", id: "p", status: "completed" },
        { kind: "session", id: "w", status: "waiting" },
        { kind: "flow-run", id: "f", status: "running" },
      ]),
    ).toEqual([
      ["session", "a"],
      ["session", "b"],
      ["session", "w"],
    ]);
  });

  it("a turn settled when something that ran no longer does", () => {
    expect(settledSince([["session", "a"]], [])).toBe(true);
    expect(settledSince([["session", "a"]], [["session", "a"], ["session", "b"]])).toBe(false);
    expect(settledSince([], [["session", "a"]])).toBe(false);
  });

  it("the latest linked-work event per card comes from newest-first rows", () => {
    expect(
      latestEventByCard([
        { cardId: "c1", kind: "failed" },
        { cardId: "c1", kind: "started" },
        { cardId: "c2", kind: "moved" },
        { cardId: "c2", kind: "completed" },
        { cardId: null, kind: "failed" },
      ]),
    ).toEqual({ c1: "failed", c2: "completed" });
  });
});
