// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from "vitest";
import {
  afterRestart,
  continuation,
  loadHold,
  newHold,
  resumePlan,
  ResumeQueue,
  saveHold,
  settle,
  track,
} from "./hold";

const state = (ids: [string, boolean][]) =>
  ids.reduce((s, [id, pilot]) => track(s, id, pilot), newHold(1000));

describe("hold state", () => {
  afterEach(() => localStorage.clear());

  it("reaches onHold only when every session has settled", () => {
    let s = state([["a", false], ["b", false]]);
    s = settle(s, "a", "interrupted", 2000, "cargo test");
    expect(s.phase).toBe("holding");
    s = settle(s, "b", "idle", 3000);
    expect(s.phase).toBe("onHold");
    expect(s.sessions[0]).toMatchObject({ heldAt: 2000, lastTool: "cargo test" });
  });

  it("tracks a session once", () => {
    const s = settle(state([["a", false], ["a", false]]), "a", "idle", 2);
    expect(s.sessions).toHaveLength(1);
    expect(s.phase).toBe("onHold");
  });

  it("turns unsettled and live waits into restarted after a restart", () => {
    let s = state([["a", false], ["b", false], ["c", false], ["d", false], ["e", false]]);
    s = settle(s, "b", "awaitingInput", 2);
    s = settle(s, "c", "background", 2, undefined, ["bun run dev"]);
    s = settle(s, "d", "interrupted", 2);
    s = settle(s, "e", "idle", 2);
    const restored = afterRestart(s);
    expect(restored.sessions.map((h) => h.reason)).toEqual([
      "restarted",
      "restarted",
      "restarted",
      "interrupted",
      "idle",
    ]);
    expect(restored.phase).toBe("onHold");
  });

  it("continues only interrupted or restarted turns", () => {
    expect(continuation("interrupted")).toBe("Continue from where you left off.");
    expect(continuation("restarted")).toBe("Continue from where you left off.");
    expect(continuation("idle")).toBeNull();
    expect(continuation("awaitingInput")).toBeNull();
    expect(continuation("background")).toBeNull();
  });

  it("resumes Pilot chats last and skips unknown sessions", () => {
    const s = state([["p1", true], ["a", false], ["b", false], ["p2", true]]);
    expect(resumePlan(s, () => true)).toEqual([["a", "b"], ["p1", "p2"]]);
    expect(resumePlan(s, (id) => id === "b")).toEqual([["b"]]);
    expect(resumePlan(s, () => false)).toEqual([]);
  });

  it("starts at most the limit and begins the next stage once the last worker started", () => {
    const queue = new ResumeQueue([["1", "2", "3", "4", "5"], ["9"]]);
    expect(queue.next(3)).toEqual(["1", "2", "3"]);
    expect(queue.next(3)).toEqual([]);
    queue.finished("2");
    expect(queue.next(3)).toEqual(["4"]);
    queue.finished("1");
    queue.finished("3");
    expect(queue.next(3)).toEqual(["5", "9"]);
    expect(queue.done()).toBe(false);
    for (const id of ["4", "5", "9"]) queue.finished(id);
    expect(queue.done()).toBe(true);
  });

  it("saves, loads and clears the hold", () => {
    expect(loadHold()).toBeNull();
    const s = settle(state([["a", false]]), "a", "idle", 2);
    saveHold(s);
    expect(loadHold()).toEqual(s);
    saveHold(null);
    expect(loadHold()).toBeNull();
  });

  it("treats unreadable storage as no hold", () => {
    localStorage.setItem("gespan:hold", "{not json");
    expect(loadHold()).toBeNull();
  });
});
