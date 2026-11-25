import { act, cleanup, render } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { App } from "../src/App";
import { LAPTOP_AT_REST, laptopPose } from "../src/components/Macbook";
import { NAV_COMPACT_AT } from "../src/components/SiteNav";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const release = JSON.parse(readFileSync(join(site, "src/release.json"), "utf8"));

describe("laptopPose", () => {
  test("starts with the lid leaning back on its base and ends lifted out, larger, base gone", () => {
    expect(laptopPose(0)).toEqual({ lid: 34, scale: 0.94, lift: -0, base: 1 });
    const end = laptopPose(1);
    expect(end.lid).toBe(0);
    expect(end.scale).toBeCloseTo(1.16, 5);
    expect(end.lift).toBe(-40);
    expect(end.base).toBe(0);
  });

  test("the lid is upright by halfway, before the screen starts lifting out", () => {
    expect(laptopPose(0.5).lid).toBe(0);
    expect(laptopPose(0.4).base).toBe(1);
  });

  test("scrolling only ever opens, grows and lifts it; going back reverses it", () => {
    let last = laptopPose(0);
    for (let p = 0.02; p <= 1.0001; p += 0.02) {
      const next = laptopPose(p);
      expect(next.lid).toBeLessThanOrEqual(last.lid);
      expect(next.scale).toBeGreaterThanOrEqual(last.scale);
      expect(next.lift).toBeLessThanOrEqual(last.lift);
      expect(next.base).toBeLessThanOrEqual(last.base);
      last = next;
    }
  });

  test("progress out of range rests at the ends; reduced motion is open and still", () => {
    expect(laptopPose(-2)).toEqual(laptopPose(0));
    expect(laptopPose(5)).toEqual(laptopPose(1));
    expect(LAPTOP_AT_REST).toEqual({ lid: 0, scale: 1, lift: 0, base: 1 });
  });
});

describe("Home hero", () => {
  beforeEach(() => window.history.replaceState(null, "", "/"));
  afterEach(() => {
    cleanup();
    Object.defineProperty(window, "scrollY", { configurable: true, value: 0 });
  });

  test("what's new sits inside the hero, directly above the headline", () => {
    render(<App />);
    const hero = document.querySelector('section[aria-label="Introduction"]')!;
    const pill = hero.querySelector(".w-announce")!;
    expect(pill).not.toBeNull();
    expect(pill.getAttribute("href")).toBe("/changelog");
    expect(pill.textContent).toContain(`New in ${release.version}`);
    // The pill comes right before the headline, and nothing sits outside the hero card.
    expect(pill.nextElementSibling?.tagName).toBe("H1");
    expect(document.querySelectorAll(".w-announce")).toHaveLength(1);
    expect(document.querySelector(".w-chip")).toBeNull();
  });

  test("the page has one nav, fixed above every page, that tightens once scrolled", () => {
    render(<App />);
    const bars = document.querySelectorAll(".nv-bar");
    expect(bars).toHaveLength(1);
    expect(bars[0].closest(".nv-fixed")).not.toBeNull();
    // Each page's hero keeps room for it instead of rendering its own.
    expect(document.querySelector('section[aria-label="Introduction"] .nv-space')).not.toBeNull();
    expect(bars[0].classList.contains("compact")).toBe(false);
    act(() => {
      Object.defineProperty(window, "scrollY", { configurable: true, value: NAV_COMPACT_AT + 1 });
      window.dispatchEvent(new Event("scroll"));
    });
    expect(document.querySelector(".nv-bar")!.classList.contains("compact")).toBe(true);
    act(() => {
      Object.defineProperty(window, "scrollY", { configurable: true, value: 0 });
      window.dispatchEvent(new Event("scroll"));
    });
    expect(document.querySelector(".nv-bar")!.classList.contains("compact")).toBe(false);
  });

  test("the laptop shows the real Home screen", () => {
    render(<App />);
    const laptop = document.querySelector("[data-laptop]")!;
    expect(laptop.querySelector('[data-shot="home"]')).not.toBeNull();
  });
});
