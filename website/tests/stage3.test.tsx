import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { App } from "../src/App";
import { FEATURES, glowAt } from "../src/components/FeatureBento";
import { LAMP } from "../src/components/Lamp";

const card = { left: 100, top: 100, width: 200, height: 100 };

describe("glowAt", () => {
  test("turns the glow to face the pointer: 0° above, 90° right, 180° below, 270° left", () => {
    expect(glowAt(card, 200, 0).angle).toBeCloseTo(0);
    expect(glowAt(card, 400, 150).angle).toBeCloseTo(90);
    expect(glowAt(card, 200, 300).angle).toBeCloseTo(180);
    expect(glowAt(card, 0, 150).angle).toBeCloseTo(270);
    // Always a turn in 0–360, never negative.
    expect(glowAt(card, 50, 50).angle).toBeGreaterThanOrEqual(0);
    expect(glowAt(card, 50, 50).angle).toBeLessThan(360);
  });

  test("lights the card while the pointer is on it or within reach, and not beyond", () => {
    expect(glowAt(card, 200, 150).active).toBe(true);
    expect(glowAt(card, 300 + 64, 150).active).toBe(true);
    expect(glowAt(card, 300 + 65, 150).active).toBe(false);
    expect(glowAt(card, 200, 100 - 65).active).toBe(false);
    expect(glowAt(card, 300 + 20, 150, 10).active).toBe(false);
  });
});

describe("the features bento", () => {
  test("fills every row of four: wide + narrow + narrow, then narrow + narrow + wide", () => {
    const spans = FEATURES.map((f) => (f.wide ? 2 : 1));
    expect(spans).toEqual([2, 1, 1, 1, 1, 2]);
    // Every wide card has a picture, and only wide cards do.
    for (const f of FEATURES) expect(Boolean(f.picture)).toBe(Boolean(f.wide));
  });

  describe("on Home", () => {
    beforeEach(() => window.history.replaceState(null, "", "/"));
    afterEach(cleanup);

    test("shows the six features as a list, glow off until a pointer comes near", () => {
      render(<App />);
      const section = document.querySelector('section[aria-labelledby="feat-h"]')!;
      const cards = [...section.querySelectorAll("ul.bt-grid > li.bt-card")];
      expect(cards.map((c) => c.querySelector("h3")!.textContent)).toEqual(FEATURES.map((f) => f.title));
      expect(cards.map((c) => c.classList.contains("wide"))).toEqual([true, false, false, false, false, true]);
      expect(cards.every((c) => (c as HTMLElement).dataset.glow === "off")).toBe(true);
    });

    test("the closing call to action sits under a lamp, with nothing left of the particles", () => {
      render(<App />);
      const cta = document.querySelector('section[aria-labelledby="cta-h"]')!;
      const lamp = cta.querySelector("[data-lamp]")!;
      expect(lamp.querySelector(".lm-light")!.getAttribute("aria-hidden")).toBe("true");
      expect(lamp.querySelector(".lm-content #cta-h")!.textContent).toBe("Hold the reins.");
      expect(cta.querySelector("canvas")).toBeNull();
      expect([...cta.querySelectorAll(".lm-content a")].map((a) => a.textContent)).toEqual(["macOS", "Windows", "Linux", "All releases"]);
    });
  });
});

test("the lamp only ever turns on: wider and brighter than it starts", () => {
  for (const part of ["cone", "line"] as const) {
    expect(LAMP.on[part].scaleX).toBeGreaterThan(LAMP.off[part].scaleX);
    expect(LAMP.on[part].opacity).toBeGreaterThan(LAMP.off[part].opacity);
    expect(LAMP.on[part]).toEqual({ scaleX: 1, opacity: 1 });
  }
});

test("the glow follows a real pointer move once a frame is drawn", async () => {
  window.history.replaceState(null, "", "/");
  render(<App />);
  const cards = [...document.querySelectorAll<HTMLElement>(".bt-card")];
  // jsdom lays nothing out: every card sits at 0,0 with no size, so a pointer at
  // 10,10 is within reach of all of them and below-right of their centres.
  await act(async () => {
    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 10, clientY: 10 }));
    await new Promise((r) => requestAnimationFrame(() => r(null)));
  });
  expect(cards.every((c) => c.dataset.glow === "on")).toBe(true);
  expect(parseFloat(cards[0].style.getPropertyValue("--glow-angle"))).toBeCloseTo(135);
  cleanup();
});
