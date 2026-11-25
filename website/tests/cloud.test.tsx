import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { App } from "../src/App";
import { HOSTING, whereThingsLive } from "../src/components/CloudMap";
import { CLOUD_INTEREST, CLOUD_STEPS } from "../src/pages/CloudPage";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");

// Gespan Cloud isn't out yet. Every place the site shows it must say so, so the
// design is never read as something a visitor can use today.
describe("Gespan Cloud (coming soon)", () => {
  afterEach(cleanup);

  describe("the /cloud page", () => {
    beforeEach(() => window.history.replaceState(null, "", "/cloud"));

    test("opens from its own address with its own title, marked current in the nav as Cloud", () => {
      render(<App />);
      expect(document.title).toBe("Gespan Cloud concept (coming soon) · Gespan");
      expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("A concept: your whole team's agents, in one place.");
      const current = [...document.querySelectorAll('[aria-current="page"]')].map((a) => a.textContent);
      expect(current).toEqual(["Cloud"]);
    });

    test("every section that shows Cloud says Coming soon and Concept", () => {
      render(<App />);
      const sections = [...document.querySelectorAll("main section")].filter((s) => !s.closest("footer") && s.querySelector("h1, h2"));
      // The hero, how it would work, the screens, what it covers and the closing call to action.
      expect(sections.length).toBe(5);
      for (const section of sections) {
        const heading = section.querySelector("h1, h2")?.textContent;
        expect(within(section as HTMLElement).queryAllByText("Coming soon").length, `${heading} has no Coming soon tag`).toBeGreaterThan(0);
        expect(within(section as HTMLElement).queryAllByText("Concept").length, `${heading} has no Concept tag`).toBeGreaterThan(0);
      }
    });

    test("the copy says it is a concept, offers hosted and self-hosted, and never promises it", () => {
      render(<App />);
      const text = document.querySelector("main")!.textContent!;
      expect(text).toContain("Work on it hasn't started");
      expect(text).toContain("hosted, or run it on their own servers");
      for (const old of [/web hub/i, /\bthe hub\b/i]) expect(old.test(text), `${old}`).toBe(false);
      for (const promise of [/\bwill\b/i, /building toward/i, /\bnext step\b/i, /sign up now/i]) expect(promise.test(text), `${promise}`).toBe(false);
    });

    test("the only ways forward are registering interest, the desktop app and release updates", () => {
      render(<App />);
      const hrefs = [...document.querySelectorAll("main a")].map((a) => a.getAttribute("href"));
      expect(CLOUD_INTEREST).toBe("https://github.com/0xvasanth/gespan/issues/554");
      expect(screen.getAllByRole("link", { name: "Register interest" }).map((a) => a.getAttribute("href"))).toEqual([CLOUD_INTEREST, CLOUD_INTEREST]);
      expect(hrefs).toContain("/download");
      expect(hrefs).toContain("https://github.com/0xvasanth/gespan/releases");
      expect(hrefs.some((h) => h?.includes("example.com"))).toBe(false);
      expect(screen.queryByRole("button", { name: /sign in|sign up/i })).toBeNull();
    });

    test("the screens are the design canvas's own, at its 1440×900 frame, each described for screen readers", () => {
      render(<App />);
      const hero = document.querySelector("[data-laptop] img.gc-screen") as HTMLImageElement;
      expect(hero.getAttribute("src")).toMatch(/home/);
      // The hero image loads straight away; the scroll story's load as they near the screen.
      expect(hero.getAttribute("loading")).toBe("eager");
      const story = [...document.querySelectorAll(".hw-inline img.gc-screen")] as HTMLImageElement[];
      expect(story.map((img) => img.getAttribute("src")!.match(/(for-you|decisions|board|devboxes|admin)/)?.[1])).toEqual(["for-you", "decisions", "board", "devboxes", "admin"]);
      for (const img of [hero, ...story]) {
        expect([img.width, img.height]).toEqual([1440, 900]);
        expect(img.alt).toMatch(/^The concept's /);
      }
      expect(story.every((img) => img.getAttribute("loading") === "lazy")).toBe(true);
    });

    test("the hosting switch flips the map and where things live between hosted and self-hosted", () => {
      render(<App />);
      const hosted = screen.getByRole("button", { name: "Hosted" });
      const self = screen.getByRole("button", { name: "Self-hosted" });
      const where = () => document.querySelector('[data-cloud-label="where"]')!.textContent;
      expect(hosted.getAttribute("aria-pressed")).toBe("true");
      expect(where()).toBe("Run by Gespan");
      act(() => {
        fireEvent.click(self);
      });
      expect(self.getAttribute("aria-pressed")).toBe("true");
      expect(hosted.getAttribute("aria-pressed")).toBe("false");
      expect(where()).toBe("On your own servers");
      expect(document.querySelector(".cm-detail")!.textContent).toBe(HOSTING.self.detail);
      expect(document.querySelector(".fx-flow")!.getAttribute("aria-label")).toContain("on your own servers");
    });

    test("the screens play as a scroll story, one tab per screen", () => {
      render(<App />);
      const story = document.querySelector("[data-how]")!;
      expect([...story.querySelectorAll(".hw-panel-head span")].map((s) => s.textContent)).toEqual(["For you", "Decisions", "Board", "Devboxes", "Admin"]);
      expect(story.querySelectorAll(".hw-step")).toHaveLength(CLOUD_STEPS.length);
      expect(story.querySelector(".hw-panel")!.getAttribute("data-picture")).toBe("for-you");
    });

    test("the bento grid fills every row of three: wide + narrow, then narrow + wide", () => {
      render(<App />);
      const spans = [...document.querySelectorAll(".gc-bento > *")].map((c) => (c.classList.contains("gc-wide") ? 2 : 1));
      expect(spans).toEqual([2, 1, 1, 2]);
    });
  });

  test("the old /hub address opens Cloud and swaps itself for /cloud", () => {
    window.history.replaceState(null, "", "/hub");
    render(<App />);
    expect(window.location.pathname).toBe("/cloud");
    expect(document.title).toBe("Gespan Cloud concept (coming soon) · Gespan");
    expect(readFileSync(join(site, "public/_redirects"), "utf8")).toMatch(/^\/hub \/cloud 301$/m);
  });

  test("Home has a Coming soon teaser that links to /cloud", () => {
    window.history.replaceState(null, "", "/");
    render(<App />);
    const teaser = screen.getByRole("heading", { name: "A concept: your whole team's agents, in one place." }).closest("section") as HTMLElement;
    expect(within(teaser).getByText("Gespan Cloud")).toBeTruthy();
    expect(within(teaser).getByText("Coming soon")).toBeTruthy();
    expect(within(teaser).getByRole("link", { name: "See the concept" }).getAttribute("href")).toBe("/cloud");
  });

  test("the footer names it Gespan Cloud", () => {
    window.history.replaceState(null, "", "/");
    render(<App />);
    const footer = document.querySelector("footer")!;
    expect(within(footer).getByRole("link", { name: "Gespan Cloud (concept)" }).getAttribute("href")).toBe("/cloud");
    expect(footer.textContent).not.toMatch(/web hub/i);
  });
});

describe("where things live", () => {
  test("code and agent logins stay on Devboxes whichever way Cloud is run", () => {
    for (const hosting of ["hosted", "self"] as const) {
      const rows = Object.fromEntries(whereThingsLive(hosting));
      expect(rows["Your code"]).toMatch(/Devboxes/);
      expect(rows["Agent logins"]).toMatch(/Devbox/);
    }
    expect(Object.fromEntries(whereThingsLive("hosted"))["Tasks, decisions and run status"]).toBe("In Gespan Cloud, run by Gespan.");
    expect(Object.fromEntries(whereThingsLive("self"))["Tasks, decisions and run status"]).toBe("In Gespan Cloud, on your own servers.");
  });
});

// Each stylesheet owns its class prefix (fx-, gc-, cl-…). Two files styling
// the same prefixed class leak into each other's pages, as gc- (then cl-) once
// did with the Changelog.
test("no prefixed class is styled by two stylesheets", () => {
  const files = ["src/pages", "src/components", "src/styles"].flatMap((dir) => readdirSync(join(site, dir)).filter((f) => f.endsWith(".css")).map((f) => join(dir, f)));
  const owners = new Map<string, Set<string>>();
  for (const file of files) {
    const css = readFileSync(join(site, file), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
    for (const [, selector] of css.matchAll(/([^{}]+)\{/g)) {
      if (selector.trim().startsWith("@")) continue;
      for (const [, cls] of selector.matchAll(/\.([a-z]{2,3}-[\w-]+)/g)) owners.set(cls, (owners.get(cls) ?? new Set()).add(file));
    }
  }
  const shared = [...owners].filter(([cls, by]) => by.size > 1 && !cls.startsWith("w-")).map(([cls, by]) => `${cls}: ${[...by].join(", ")}`);
  expect(shared).toEqual([]);
});
