// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConfirmHold } from "./ConfirmHold";
import { HoldControl } from "./HoldControl";

let container: HTMLDivElement;
let root: Root | null = null;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div");
  document.body.append(container);
});

afterEach(() => {
  if (root) {
    act(() => root!.unmount());
    root = null;
  }
  container.remove();
  vi.unstubAllGlobals();
});

const render = (element: ReturnType<typeof createElement>) => {
  root = createRoot(container);
  act(() => root!.render(element));
};

describe("HoldControl", () => {
  it("offers Hold all when nothing is held", () => {
    const onToggle = vi.fn();
    render(
      createElement(HoldControl, {
        phase: null,
        summary: "Hold all",
        settling: [],
        onToggle,
        onStopNow: vi.fn(),
      }),
    );
    const button = container.querySelector("button")!;
    expect(button.textContent).toContain("Hold all");
    expect(button.getAttribute("aria-label")).toContain("Hold everything");
    act(() => button.click());
    expect(onToggle).toHaveBeenCalledOnce();
  });

  it("shows progress while holding, lists what is finishing, and never offers Resume", () => {
    const onStopNow = vi.fn();
    const onToggle = vi.fn();
    render(
      createElement(HoldControl, {
        phase: "holding",
        summary: "Holding 1/2…",
        settling: [{ id: "a", title: "Fix flaky reminders", tool: "cargo test" }],
        onToggle,
        onStopNow,
      }),
    );
    expect(container.textContent).toContain("Holding 1/2…");
    expect(container.textContent).not.toContain("Resume");
    act(() => container.querySelector("button")!.click());
    expect(onToggle).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Fix flaky reminders");
    expect(container.textContent).toContain("cargo test");
    const stop = [...container.querySelectorAll("button")].find(
      (b) => b.textContent === "Stop now",
    )!;
    act(() => stop.click());
    expect(onStopNow).toHaveBeenCalledWith("a");
  });

  it("counts what is held and offers Resume once on hold", () => {
    const onToggle = vi.fn();
    render(
      createElement(HoldControl, {
        phase: "onHold",
        summary: "On hold · 2 sessions · 1 queued",
        settling: [],
        onToggle,
        onStopNow: vi.fn(),
      }),
    );
    expect(container.textContent).toContain("On hold · 2 sessions · 1 queued");
    const resume = [...container.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("Resume"),
    )!;
    act(() => resume.click());
    expect(onToggle).toHaveBeenCalledOnce();
  });
});

describe("ConfirmHold", () => {
  it("explains what Hold does and offers Hold or Cancel", () => {
    const onClose = vi.fn();
    render(createElement(ConfirmHold, { running: 2, onClose }));
    const text = document.body.textContent ?? "";
    expect(text).toContain("Hold everything?");
    expect(text).toContain(
      "2 running sessions will stop once their current command finishes",
    );
    expect(text).toContain("You can't resume until every session has stopped.");
    const buttons = [...document.body.querySelectorAll("button")];
    act(() => buttons.find((b) => b.textContent === "Cancel")!.click());
    expect(onClose).toHaveBeenCalledWith(false);
    act(() => buttons.find((b) => b.textContent === "Hold")!.click());
    expect(onClose).toHaveBeenCalledWith(true);
  });
});
