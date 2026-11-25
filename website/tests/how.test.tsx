import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { App } from "../src/App";
import { FLOW_LABELS } from "../src/components/fx";
import { STEPS, stepAt } from "../src/components/HowItWorks";

describe("stepAt", () => {
  test("each step owns an equal slice of the scroll", () => {
    expect(stepAt(0, 4)).toBe(0);
    expect(stepAt(0.249, 4)).toBe(0);
    expect(stepAt(0.25, 4)).toBe(1);
    expect(stepAt(0.5, 4)).toBe(2);
    expect(stepAt(0.75, 4)).toBe(3);
  });

  test("the last step holds at the very end instead of running off the list", () => {
    expect(stepAt(1, 4)).toBe(3);
  });

  test("out-of-range and broken progress rest on a real step", () => {
    expect(stepAt(-0.5, 4)).toBe(0);
    expect(stepAt(3, 4)).toBe(3);
    expect(stepAt(Number.NaN, 4)).toBe(0);
    expect(stepAt(0.5, 0)).toBe(0);
  });

  test("scrolling down only ever moves forward through the steps", () => {
    let last = 0;
    for (let p = 0; p <= 1; p += 0.01) {
      const next = stepAt(p, STEPS.length);
      expect(next).toBeGreaterThanOrEqual(last);
      expect(next - last).toBeLessThanOrEqual(1);
      last = next;
    }
    expect(last).toBe(STEPS.length - 1);
  });
});

describe("Home: flow and How it works", () => {
  beforeEach(() => window.history.replaceState(null, "", "/"));
  afterEach(cleanup);

  test("the flow names its beams: Inbox in, Sessions out, the board, automations and Pilots at the hub", () => {
    render(<App />);
    const flow = document.querySelector(".fx-flow")!;
    const label = (side: string) => flow.querySelector(`[data-flow-label="${side}"]`)?.textContent;
    expect(label("in")).toBe(FLOW_LABELS.in);
    expect(label("out")).toBe(FLOW_LABELS.out);
    expect(label("hub")).toBe(FLOW_LABELS.hub);
    // The beams sit between the trackers and the agents, in that order.
    const order = [...flow.querySelectorAll(".fx-from, [data-flow-label], .fx-to")].map((el) => (el as HTMLElement).dataset.flowLabel ?? el.className);
    expect(order).toEqual(["fx-col fx-from", "in", "hub", "out", "fx-col fx-to"]);
    // Screen readers get one sentence, not the drawing.
    expect(flow.getAttribute("aria-label")).toMatch(/Inbox.*sessions/);
    expect(flow.querySelector(".fx-flow-box")!.getAttribute("aria-hidden")).toBe("true");
  });

  test("How it works walks Board, Sessions, Automations and Pilots, starting on the first", () => {
    render(<App />);
    const how = document.querySelector("[data-how]")!;
    const steps = [...how.querySelectorAll(".hw-step")];
    expect(steps.map((s) => s.querySelector("h3")!.textContent)).toEqual(["Board", "Sessions", "Automations", "Pilots"]);
    expect(steps.map((s) => s.getAttribute("aria-current"))).toEqual(["step", null, null, null]);
    expect(how.querySelector(".hw-panel")!.getAttribute("data-picture")).toBe("board");
    expect(how.querySelector(".hw-panel-head .on")!.textContent).toBe("Board");
  });

  test("every step carries its own picture for narrow screens, and the pictures stay out of the reading order", () => {
    render(<App />);
    for (const step of document.querySelectorAll(".hw-step")) {
      const inline = step.querySelector(".hw-inline")!;
      expect(inline).not.toBeNull();
      expect(inline.getAttribute("aria-hidden")).toBe("true");
    }
    expect(document.querySelector(".hw-sticky")!.getAttribute("aria-hidden")).toBe("true");
    // The old four-card grid is gone.
    expect(document.querySelector(".lp-step")).toBeNull();
  });
});
