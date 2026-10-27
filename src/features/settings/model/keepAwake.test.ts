// @vitest-environment happy-dom
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Session } from "../../sessions/model/session";
import {
  hasWorkingAgent,
  KEEP_AWAKE_KEY,
  loadKeepAwake,
  resetKeepAwakeSync,
  saveKeepAwake,
  subscribeKeepAwake,
  syncKeepAwake,
} from "./keepAwake";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => null),
}));

const invoked = vi.mocked(invoke);

function session(overrides: Partial<Session> = {}): Session {
  return {
    id: overrides.id ?? "s1",
    title: "Chat",
    cwd: "/repo",
    blocks: [],
    ...overrides,
  } as Session;
}

beforeEach(() => {
  localStorage.clear();
  resetKeepAwakeSync();
  invoked.mockClear();
  invoked.mockImplementation(async () => null);
});

afterEach(() => localStorage.clear());

describe("keep awake setting", () => {
  it("defaults to off and ignores unknown stored values", () => {
    expect(loadKeepAwake()).toBe("off");
    localStorage.setItem(KEEP_AWAKE_KEY, "always");
    expect(loadKeepAwake()).toBe("off");
  });

  it("persists each mode under the key the native app mirrors", () => {
    for (const mode of ["agents_running", "app_open", "off"] as const) {
      saveKeepAwake(mode);
      expect(localStorage.getItem("gespan.keepAwake")).toBe(mode);
      expect(loadKeepAwake()).toBe(mode);
    }
  });

  it("notifies subscribers for local saves and other windows' saves only", () => {
    const onChange = vi.fn();
    const unsubscribe = subscribeKeepAwake(onChange);
    saveKeepAwake("app_open");
    window.dispatchEvent(
      new StorageEvent("storage", { key: KEEP_AWAKE_KEY }),
    );
    window.dispatchEvent(
      new StorageEvent("storage", { key: "gespan.followUpBehavior" }),
    );
    expect(onChange).toHaveBeenCalledTimes(2);
    unsubscribe();
    saveKeepAwake("off");
    expect(onChange).toHaveBeenCalledTimes(2);
  });
});

describe("working agent detection", () => {
  it("counts live turns and background work, but not idle or removed chats", () => {
    expect(hasWorkingAgent([])).toBe(false);
    expect(hasWorkingAgent([session()])).toBe(false);
    expect(hasWorkingAgent([session(), session({ id: "s2", busy: true })])).toBe(
      true,
    );
    expect(hasWorkingAgent([session({ backgroundTasks: ["npm test"] })])).toBe(
      true,
    );
    expect(hasWorkingAgent([session({ backgroundTasks: [] })])).toBe(false);
    expect(
      hasWorkingAgent([session({ busy: true, worktreeRemoved: true })]),
    ).toBe(false);
  });
});

describe("syncKeepAwake", () => {
  it("reports only transitions of mode or running state", () => {
    const idle = [session()];
    const busy = [session({ busy: true })];
    syncKeepAwake(idle, "agents_running");
    syncKeepAwake(idle, "agents_running");
    syncKeepAwake(busy, "agents_running");
    syncKeepAwake(busy, "agents_running");
    syncKeepAwake(busy, "off");
    expect(invoked.mock.calls).toEqual([
      ["set_keep_awake", { mode: "agents_running", running: false }],
      ["set_keep_awake", { mode: "agents_running", running: true }],
      ["set_keep_awake", { mode: "off", running: true }],
    ]);
  });

  it("lets the Mac sleep once everything is on hold, even with background work", () => {
    const working = [session({ busy: true, backgroundTasks: ["dev server"] })];
    syncKeepAwake(working, "agents_running");
    syncKeepAwake(working, "agents_running", true);
    expect(invoked.mock.calls).toEqual([
      ["set_keep_awake", { mode: "agents_running", running: true }],
      ["set_keep_awake", { mode: "agents_running", running: false }],
    ]);
  });

  it("reads the saved mode when none is passed", () => {
    saveKeepAwake("app_open");
    syncKeepAwake([]);
    expect(invoked).toHaveBeenLastCalledWith("set_keep_awake", {
      mode: "app_open",
      running: false,
    });
  });

  it("retries the same state after a failed invoke", async () => {
    invoked.mockImplementationOnce(async () => {
      throw new Error("not ready");
    });
    syncKeepAwake([], "app_open");
    await Promise.resolve();
    await Promise.resolve();
    syncKeepAwake([], "app_open");
    expect(invoked).toHaveBeenCalledTimes(2);
  });
});
