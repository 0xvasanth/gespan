// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  isTauri: () => false,
  convertFileSrc: (path: string) => path,
}));
import { ResourcePressureNotice } from "./ResourcePressureNotice";
import { newSession } from "../../sessions/model/session";

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  invoke.mockReset();
});

it("only lists processes when opened and stops a tool only after its button is clicked", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const session = {
    ...newSession("claude", "/repo"),
    id: "owned",
    title: "Build",
  };
  const tool = {
    pid: 42,
    identity: "42:123:456",
    sessionId: "owned",
    cwd: "/repo/tree",
    command: "cargo build",
    cpuPercent: 150,
    memoryBytes: 1024,
    agent: false,
  };
  const other = { ...tool, pid: 43, identity: "43:124:456", command: "rustc" };
  const agent = {
    ...tool,
    pid: 41,
    identity: "41:122:456",
    command: "claude",
    agent: true,
  };
  let rows = [tool, other, agent];
  invoke.mockImplementation(async (command: string) => {
    if (command === "resource_usage")
      return { cpuPercent: 95, memoryPressure: 1 };
    if (command === "resource_processes") return rows;
    if (command === "resource_stop_tool") return;
    throw new Error(command);
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onStopAgent = vi.fn();
  const onStopSession = vi.fn();
  try {
    await act(async () =>
      root.render(
        createElement(ResourcePressureNotice, {
          sessions: { current: [session] },
          onStopAgent,
          onStopSession,
        }),
      ),
    );
    await act(async () => vi.advanceTimersByTimeAsync(10_000));
    expect(container.textContent).toContain("Resource pressure");
    expect(
      invoke.mock.calls.some(([command]) => command !== "resource_usage"),
    ).toBe(false);
    await act(async () => container.querySelector("button")!.click());
    expect(document.body.textContent).toContain("cargo build");
    const stop = [...document.querySelectorAll("button")].find(
      (button) =>
        button.getAttribute("aria-label") ===
        "Stop build/tool for Build (PID 42)",
    )!;
    rows = [{ ...other, cpuPercent: 180 }, { ...tool, cpuPercent: 10 }, agent];
    await act(async () => vi.advanceTimersByTimeAsync(3_000));
    const controls = [...document.querySelectorAll("button")].filter(
      (button) => button.textContent === "Stop build/tool",
    );
    expect(controls[0]).toBe(stop);
    expect(controls[1].getAttribute("aria-label")).toContain("PID 43");
    expect(document.body.textContent).toContain("10% CPU");
    await act(async () => stop.click());
    expect(invoke).toHaveBeenCalledWith("resource_stop_tool", {
      pid: 42,
      identity: "42:123:456",
    });
    expect(onStopAgent).not.toHaveBeenCalled();
    expect(onStopSession).not.toHaveBeenCalled();
    const stopAgent = document.querySelector<HTMLButtonElement>(
      '[aria-label="Stop agent for Build (PID 41)"]',
    )!;
    await act(async () => stopAgent.click());
    expect(onStopAgent).toHaveBeenCalledExactlyOnceWith("owned");
    expect(onStopSession).not.toHaveBeenCalled();
    const stopSession = document.querySelector<HTMLButtonElement>(
      '[aria-label="Stop session Build"]',
    )!;
    await act(async () => stopSession.click());
    expect(onStopSession).toHaveBeenCalledExactlyOnceWith("owned");
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
