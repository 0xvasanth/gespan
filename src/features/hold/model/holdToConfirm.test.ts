import { describe, expect, it } from "vitest";
import { ARMED_MS, HINT_MS, HOLD_MS, Press } from "./holdToConfirm";

describe("Press", () => {
  it("fills linearly and confirms exactly at the hold duration", () => {
    const press = new Press();
    press.start(0);
    expect(press.progress(600)).toBeCloseTo(0.5, 3);
    expect(press.tick(1199)).toBeNull();
    expect(press.tick(HOLD_MS)).toBe("confirmed");
    expect(press.label(HOLD_MS)).toBe("done");
    expect(press.progress(HOLD_MS + 500)).toBe(1);
  });

  it("confirms only once", () => {
    const press = new Press();
    press.start(0);
    expect(press.tick(HOLD_MS)).toBe("confirmed");
    expect(press.tick(HOLD_MS + 10)).toBeNull();
    press.start(HOLD_MS + 20);
    expect(press.tick(HOLD_MS * 3)).toBeNull();
    expect(press.click(HOLD_MS * 3)).toBe(false);
    expect(press.click(HOLD_MS * 3 + 1)).toBe(false);
  });

  it("ignores repeated starts while filling", () => {
    const press = new Press();
    press.start(0);
    press.start(300);
    press.start(600);
    expect(press.progress(600)).toBeCloseTo(0.5, 3);
    expect(press.tick(1100)).toBeNull();
    expect(press.tick(HOLD_MS)).toBe("confirmed");
  });

  it("drains an early release in proportion to progress", () => {
    const press = new Press();
    press.start(0);
    expect(press.release(600)).toBe("drain");
    const mid = press.progress(725);
    expect(mid).toBeGreaterThan(0);
    expect(mid).toBeLessThan(0.25);
    expect(press.progress(850)).toBe(0);
    expect(press.animating(851)).toBe(false);
    expect(press.tick(5000)).toBeNull();
  });

  it("drains a small fill in at least 180ms", () => {
    const press = new Press();
    press.start(0);
    press.cancel(300);
    expect(press.progress(300 + 170)).toBeGreaterThan(0);
    expect(press.progress(300 + 180)).toBe(0);
  });

  it("resumes from the remaining progress when pressed again", () => {
    const press = new Press();
    press.start(0);
    press.release(900);
    const left = press.progress(950);
    expect(left).toBeGreaterThan(0.4);
    expect(left).toBeLessThan(0.75);
    press.start(950);
    expect(press.progress(950)).toBeCloseTo(left, 4);
    const remaining = Math.floor((1 - left) * HOLD_MS);
    expect(press.tick(950 + remaining - 5)).toBeNull();
    expect(press.tick(950 + remaining + 1)).toBe("confirmed");
  });

  it("shows the hint after a tap, but not after a cancel", () => {
    const press = new Press();
    press.start(0);
    expect(press.release(100)).toBe("tap");
    expect(press.label(100)).toBe("hint");
    expect(press.label(100 + HINT_MS - 1)).toBe("hint");
    expect(press.animating(100 + HINT_MS - 1)).toBe(true);
    expect(press.label(100 + HINT_MS)).toBe("rest");
    const cancelled = new Press();
    cancelled.start(0);
    cancelled.cancel(100);
    expect(cancelled.label(100)).toBe("rest");
  });

  it("confirms a release past the full fill that no tick saw", () => {
    const press = new Press();
    press.start(0);
    expect(press.release(HOLD_MS + 5)).toBe("confirmed");
    expect(press.tick(HOLD_MS + 6)).toBeNull();
    expect(press.label(HOLD_MS + 6)).toBe("done");
  });

  it("arms on a click and confirms on a second within the window", () => {
    const press = new Press();
    expect(press.click(0)).toBe(false);
    expect(press.label(0)).toBe("armed");
    expect(press.click(ARMED_MS - 1)).toBe(true);
    expect(press.label(ARMED_MS)).toBe("done");
  });

  it("lets an armed click expire", () => {
    const press = new Press();
    expect(press.click(0)).toBe(false);
    expect(press.label(ARMED_MS)).toBe("rest");
    expect(press.click(ARMED_MS)).toBe(false);
    expect(press.label(ARMED_MS)).toBe("armed");
  });

  it("shows the hint without starting a press", () => {
    const press = new Press();
    press.hint(0);
    expect(press.label(0)).toBe("hint");
    expect(press.progress(0)).toBe(0);
    expect(press.pressing()).toBe(false);
    expect(press.tick(HOLD_MS)).toBeNull();
  });
});
