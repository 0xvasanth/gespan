// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { HoldPress } from "../model/holdPress";
import { HoldControl } from "./HoldControl";
import { HoldToConfirm } from "./HoldToConfirm";

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
  it("offers Hold all as a hold-to-confirm button when nothing is held", () => {
    const press = new HoldPress(vi.fn());
    render(
      createElement(HoldControl, {
        phase: null,
        summary: "Hold all",
        settling: [],
        press,
        onResume: vi.fn(),
        onStopNow: vi.fn(),
      }),
    );
    const button = container.querySelector("button")!;
    expect(button.textContent).toContain("Hold all");
    expect(button.getAttribute("aria-label")).toContain("press and hold");
    press.dispose();
  });

  it("shows progress while holding, lists what is finishing, and never offers Resume", () => {
    const onStopNow = vi.fn();
    const onToggle = vi.fn();
    render(
      createElement(HoldControl, {
        phase: "holding",
        summary: "Holding 1/2…",
        settling: [{ id: "a", title: "Fix flaky reminders", tool: "cargo test" }],
        press: new HoldPress(vi.fn()),
        onResume: onToggle,
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
        press: new HoldPress(vi.fn()),
        onResume: onToggle,
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

describe("HoldToConfirm", () => {
  let press: HoldPress;
  let onConfirm: ReturnType<typeof vi.fn>;
  const button = () => container.querySelector("button")!;
  const fill = () => container.querySelector<HTMLElement>("[data-hold-fill]")!;
  const fire = (type: string, init: Record<string, unknown> = {}) =>
    act(() => {
      button().dispatchEvent(
        new (type.startsWith("pointer") ? PointerEvent : KeyboardEvent)(type, {
          bubbles: true,
          cancelable: true,
          button: 0,
          ...init,
        }),
      );
    });
  const advance = (ms: number) => act(() => vi.advanceTimersByTime(ms));

  beforeEach(() => {
    vi.useFakeTimers();
    onConfirm = vi.fn();
    press = new HoldPress(onConfirm);
    render(createElement(HoldToConfirm, { press }));
  });
  afterEach(() => {
    press.dispose();
    vi.useRealTimers();
  });

  it("confirms after the pointer is held for 1.2 s, filling left to right", () => {
    fire("pointerdown");
    advance(600);
    expect(fill().style.clipPath).toMatch(/inset\(0(px)? 5\d(\.\d+)?% 0(px)? 0(px)?\)/);
    expect(onConfirm).not.toHaveBeenCalled();
    advance(650);
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it("drains on an early pointer release and never confirms", () => {
    fire("pointerdown");
    advance(500);
    fire("pointerup");
    advance(3000);
    expect(onConfirm).not.toHaveBeenCalled();
    expect(fill().style.clipPath).toMatch(/100%/);
  });

  it("drains when the pointer leaves mid-press", () => {
    fire("pointerdown");
    advance(500);
    fire("pointerout", { relatedTarget: document.body });
    advance(3000);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("shows Hold to confirm after a tap, keeping the button's width", () => {
    const width = button().className.match(/\bw-\S+/)?.[0];
    expect(width).toBeTruthy();
    fire("pointerdown");
    advance(100);
    fire("pointerup");
    expect(button().textContent).toContain("Hold to confirm");
    expect(button().className).toContain(width!);
    advance(2000);
    expect(button().textContent).toContain("Hold all");
  });

  it("confirms when Space is held, and not on a Space tap", () => {
    fire("keydown", { key: " " });
    advance(100);
    fire("keyup", { key: " " });
    advance(3000);
    expect(onConfirm).not.toHaveBeenCalled();
    fire("keydown", { key: " " });
    fire("keydown", { key: " ", repeat: true });
    advance(1250);
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it("never confirms an Enter tap on the first press", () => {
    fire("keydown", { key: "Enter" });
    fire("keyup", { key: "Enter" });
    advance(3000);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("arms on a click with no pointer behind it, and confirms on the second", () => {
    act(() => button().dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 0 })));
    expect(button().textContent).toContain("Press again to confirm");
    expect(onConfirm).not.toHaveBeenCalled();
    act(() => button().dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 0 })));
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it("ignores the click that ends a pointer press", () => {
    fire("pointerdown");
    advance(100);
    fire("pointerup");
    act(() => button().dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 })));
    expect(button().textContent).not.toContain("Press again");
  });
});
