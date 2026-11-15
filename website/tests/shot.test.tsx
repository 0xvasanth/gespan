import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { SHOT_HEIGHT, SHOT_WIDTH, Shot, shotScale } from "../src/components/Shot";

describe("shotScale", () => {
  test("fits the whole 1440px screen into the frame", () => {
    expect(shotScale(1104)).toBeCloseTo(1104 / 1440, 6);
    // A 390px phone has a 358px column: the full screen at about a quarter size.
    expect(shotScale(358) * SHOT_WIDTH).toBeCloseTo(358, 6);
    expect(shotScale(358) * SHOT_HEIGHT).toBeCloseTo(223.75, 2);
  });

  test("never scales up past the real size", () => {
    expect(shotScale(2560)).toBe(1);
  });

  test("an unmeasured frame draws nothing rather than a wrong size", () => {
    expect(shotScale(0)).toBe(0);
    expect(shotScale(Number.NaN)).toBe(0);
  });
});

describe("Shot", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  function stubWidth(width: number) {
    const observers: ResizeObserverCallback[] = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(cb: ResizeObserverCallback) {
          observers.push(cb);
        }
        observe() {}
        disconnect() {}
      },
    );
    let current = width;
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(() => current);
    return (next: number) => {
      current = next;
      for (const cb of observers) cb([], {} as ResizeObserver);
    };
  }

  const scaleOf = (el: Element) => Number(/scale\(([\d.]+)\)/.exec((el.querySelector(".w-shot-in") as HTMLElement).style.transform)?.[1]);

  test("scales to the frame on a phone and rescales when the frame resizes", async () => {
    const resize = stubWidth(358);
    const { container } = render(<Shot name="home" />);
    const frame = container.querySelector(".w-shot")!;
    expect(scaleOf(frame)).toBeCloseTo(358 / 1440, 6);
    resize(1104);
    await waitFor(() => expect(scaleOf(frame)).toBeCloseTo(1104 / 1440, 6));
  });

  test("draws the real app screen in a shadow root, named for screen readers", async () => {
    stubWidth(600);
    const { container } = render(<Shot name="tasks" />);
    const frame = container.querySelector(".w-shot")!;
    expect(frame.getAttribute("role")).toBe("img");
    expect(frame.getAttribute("aria-label")).toMatch(/My tasks/);
    const screen = frame.querySelector(".w-shot-in")!;
    expect(screen.hasAttribute("inert")).toBe(true);
    await waitFor(() => expect(screen.shadowRoot?.textContent).toContain("Claude Code wants to run a command"));
    // The screen's own styles stay inside the shadow root.
    expect(document.head.innerHTML).not.toContain(".desk");
  });
});
