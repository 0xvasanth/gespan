import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { HoldPress } from "./holdPress";

let confirmed: number;
let resumed: number;
let press: HoldPress;
const resume = () => {
  resumed++;
};

beforeEach(() => {
  vi.useFakeTimers();
  confirmed = 0;
  resumed = 0;
  press = new HoldPress(() => confirmed++);
});
afterEach(() => {
  press.dispose();
  vi.useRealTimers();
});

describe("HoldPress", () => {
  it("confirms once the press is held for 1.2 s, and notifies while filling", () => {
    const changed = vi.fn();
    press.subscribe(changed);
    press.start();
    vi.advanceTimersByTime(1100);
    expect(confirmed).toBe(0);
    expect(press.progress()).toBeGreaterThan(0.9);
    expect(changed.mock.calls.length).toBeGreaterThan(10);
    vi.advanceTimersByTime(150);
    expect(confirmed).toBe(1);
    vi.advanceTimersByTime(5000);
    expect(confirmed).toBe(1);
  });

  it("stops ticking once a drain finishes", () => {
    press.start();
    vi.advanceTimersByTime(300);
    press.release();
    vi.advanceTimersByTime(2500);
    expect(vi.getTimerCount()).toBe(0);
    expect(press.progress()).toBe(0);
  });

  it("holds through the shortcut, ignoring key repeats", () => {
    press.shortcutDown(null, resume);
    for (let i = 0; i < 12; i++) {
      vi.advanceTimersByTime(90);
      press.shortcutDown(null, resume);
    }
    expect(confirmed).toBe(0);
    vi.advanceTimersByTime(200);
    expect(confirmed).toBe(1);
  });

  it("drains when the shortcut is released early", () => {
    press.shortcutDown(null, resume);
    vi.advanceTimersByTime(300);
    press.shortcutUp();
    vi.advanceTimersByTime(3000);
    expect(confirmed).toBe(0);
    expect(press.keyHeld).toBe(false);
  });

  it("shows the hint after a quick shortcut tap", () => {
    press.shortcutDown(null, resume);
    vi.advanceTimersByTime(100);
    press.shortcutUp();
    expect(press.label()).toBe("hint");
  });

  it("resumes on the shortcut when on hold, and repeats never resume twice", () => {
    press.shortcutDown("onHold", resume);
    press.shortcutDown(null, resume);
    press.shortcutDown(null, resume);
    expect(resumed).toBe(1);
    expect(press.progress()).toBe(0);
    press.shortcutUp();
    press.shortcutDown(null, resume);
    vi.advanceTimersByTime(1250);
    expect(confirmed).toBe(1);
  });

  it("does nothing on the shortcut while holding", () => {
    press.shortcutDown("holding", resume);
    vi.advanceTimersByTime(2000);
    expect(confirmed + resumed).toBe(0);
  });

  it("drains without confirming when the window blurs mid-press", () => {
    press.shortcutDown(null, resume);
    vi.advanceTimersByTime(600);
    press.cancel();
    vi.advanceTimersByTime(3000);
    expect(confirmed).toBe(0);
    expect(press.keyHeld).toBe(false);
    expect(press.label()).toBe("rest");
  });

  it("arms on a click with no press behind it, then confirms on the second", () => {
    press.click();
    expect(press.label()).toBe("armed");
    expect(confirmed).toBe(0);
    vi.advanceTimersByTime(1000);
    press.click();
    expect(confirmed).toBe(1);
  });

  it("shows the hint for the palette and menus without holding", () => {
    press.hint();
    expect(press.label()).toBe("hint");
    vi.advanceTimersByTime(2000);
    expect(press.label()).toBe("rest");
    expect(confirmed).toBe(0);
  });
});
