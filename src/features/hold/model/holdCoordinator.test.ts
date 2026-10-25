// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from "vitest";
import type { HarnessEvent } from "../../../integrations/harness/core/types";
import { loadHold, newHold, saveHold, settle, track } from "./hold";
import { HoldCoordinator, type HoldHost, type HoldSession } from "./holdCoordinator";

type Fake = HoldHost & {
  list: HoldSession[];
  held: string[];
  resumed: [string, string | null][];
  answer: boolean;
};

function host(list: HoldSession[]): Fake {
  const fake: Fake = {
    list,
    held: [],
    resumed: [],
    answer: true,
    sessions: () => fake.list,
    holdTurn: (id) => fake.held.push(id),
    resume: (id, prompt) => {
      fake.resumed.push([id, prompt]);
      const session = fake.list.find((s) => s.id === id);
      if (session && prompt) session.busy = true;
      return !!prompt;
    },
    confirm: async () => fake.answer,
    changed: () => undefined,
  };
  return fake;
}
const session = (id: string, busy = false, pilotChat = false): HoldSession => ({
  id,
  busy,
  pilotChat,
  awaitingInput: false,
  queued: 0,
});
const delta: HarnessEvent = { type: "message.delta", text: "x" };
const tool = (callId: string, status = "in_progress"): HarnessEvent =>
  status === "in_progress"
    ? { type: "tool.started", callId, title: "Bash", status }
    : { type: "tool.updated", callId, status };

describe("HoldCoordinator", () => {
  afterEach(() => localStorage.clear());

  it("asks first, and a cancelled confirmation changes nothing", async () => {
    const fake = host([session("a", true)]);
    fake.answer = false;
    const hold = new HoldCoordinator(fake);
    await hold.toggle();
    expect(hold.phase()).toBeNull();
    expect(fake.held).toEqual([]);
    expect(loadHold()).toBeNull();
  });

  it("waits for a running command, interrupts after it, and reaches onHold", async () => {
    const fake = host([session("a", true), session("b")]);
    const hold = new HoldCoordinator(fake);
    hold.observe("a", tool("c1"));
    await hold.toggle();
    expect(hold.phase()).toBe("holding");
    expect(fake.held).toEqual([]);
    hold.observe("a", tool("c1", "completed"));
    expect(fake.held).toEqual(["a"]);
    fake.list[0].busy = false;
    hold.sync();
    expect(hold.phase()).toBe("onHold");
    expect(loadHold()?.sessions.map((s) => s.reason)).toEqual(["interrupted", "idle"]);
  });

  it("interrupts a streaming turn at once and ignores the toggle while holding", async () => {
    const fake = host([session("a", true)]);
    const hold = new HoldCoordinator(fake);
    hold.observe("a", delta);
    await hold.toggle();
    expect(fake.held).toEqual(["a"]);
    fake.answer = false;
    await hold.toggle();
    expect(hold.phase()).toBe("holding");
  });

  it("leaves a pending approval pending", async () => {
    const fake = host([{ ...session("a", true), awaitingInput: true }]);
    const hold = new HoldCoordinator(fake);
    await hold.toggle();
    expect(fake.held).toEqual([]);
    expect(hold.phase()).toBe("onHold");
  });

  it("resumes workers before Pilot chats, three at a time, with continuations", async () => {
    const list = ["1", "2", "3", "4"].map((id) => session(id, true));
    const fake = host([...list, session("p", true, true)]);
    // Like App: a session that is idle and no longer held drops out of the list.
    const all = fake.list;
    fake.sessions = () => all.filter((s) => s.busy || hold.isSessionHeld(s.id));
    const hold = new HoldCoordinator(fake);
    for (const s of all) hold.observe(s.id, delta);
    await hold.toggle();
    for (const s of fake.list) s.busy = false;
    hold.sync();
    expect(hold.phase()).toBe("onHold");
    await hold.toggle();
    expect(hold.phase()).toBeNull();
    expect(fake.resumed.map(([id]) => id)).toEqual(["1", "2", "3"]);
    expect(fake.resumed[0][1]).toBe("Continue from where you left off.");
    fake.list[0].busy = false;
    hold.sync();
    expect(fake.resumed.map(([id]) => id)).toEqual(["1", "2", "3", "4"]);
    fake.list[1].busy = false;
    hold.sync();
    expect(fake.resumed.map(([id]) => id)).toEqual(["1", "2", "3", "4", "p"]);
    expect(loadHold()).toBeNull();
  });

  it("is on hold at once when nothing is running", async () => {
    const hold = new HoldCoordinator(host([]));
    await hold.toggle();
    expect(hold.phase()).toBe("onHold");
  });

  it("does not interrupt a turn that has shown nothing yet", async () => {
    const fake = host([session("a", true)]);
    const hold = new HoldCoordinator(fake);
    await hold.toggle();
    expect(fake.held).toEqual([]);
    hold.observe("a", delta);
    expect(fake.held).toEqual(["a"]);
  });

  it("tracks a session that queues a message while held and releases it on Resume", async () => {
    const fake = host([]);
    const hold = new HoldCoordinator(fake);
    await hold.toggle();
    fake.list.push({ ...session("late"), queued: 1 });
    hold.noteQueued("late", false);
    expect(hold.summary()).toBe("On hold · 1 session · 1 queued");
    await hold.toggle();
    expect(fake.resumed).toEqual([["late", null]]);
  });

  it("delivers a held turn's settlement after Resume", async () => {
    const fake = host([session("idle"), session("worker", true)]);
    const hold = new HoldCoordinator(fake);
    hold.observe("worker", delta);
    await hold.toggle();
    fake.list[1].busy = false;
    hold.sync();
    const delivered: [string, string][] = [];
    hold.deferSettlement("idle", (o) => delivered.push(["idle", o ?? "original"]));
    hold.deferSettlement("worker", (o) => delivered.push(["worker", o ?? "original"]));
    await hold.toggle();
    // Nothing more runs in the idle session: its own outcome stands.
    expect(delivered).toEqual([["idle", "original"]]);
    // The worker continues; its real outcome is the continuation's.
    hold.turnEnded("worker", "completed");
    expect(delivered).toEqual([["idle", "original"], ["worker", "completed"]]);
  });

  it("idle sessions resume by releasing their queue only", async () => {
    const fake = host([session("a")]);
    const hold = new HoldCoordinator(fake);
    await hold.toggle();
    await hold.toggle();
    expect(fake.resumed).toEqual([["a", null]]);
  });

  it("restores the hold after a restart and marks unsettled sessions restarted", () => {
    let state = track(track(newHold(1), "a", false), "b", false);
    state = settle(state, "b", "idle", 2);
    saveHold(state);
    const hold = new HoldCoordinator(host([]));
    expect(hold.phase()).toBe("onHold");
    expect(hold.isHeld()).toBe(true);
    expect(loadHold()?.sessions[0].reason).toBe("restarted");
  });

  it("summarises progress and what is waiting", async () => {
    const fake = host([session("a", true), { ...session("b"), queued: 2 }]);
    const hold = new HoldCoordinator(fake);
    expect(hold.summary()).toBe("Hold all");
    hold.observe("a", tool("c1"));
    await hold.toggle();
    expect(hold.summary()).toBe("Holding 1/2…");
    expect(hold.settling()).toEqual(["a"]);
    hold.stopNow("a");
    expect(hold.summary()).toBe("On hold · 2 sessions · 2 queued");
  });

  it("a held session is not reported as finished", async () => {
    const fake = host([session("a", true)]);
    const hold = new HoldCoordinator(fake);
    hold.observe("a", delta);
    await hold.toggle();
    expect(hold.isSessionHeld("a")).toBe(true);
    expect(hold.isSessionHeld("other")).toBe(false);
  });
});
