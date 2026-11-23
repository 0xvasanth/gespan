import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { App } from "../src/App";

// The Web Hub isn't out yet. Every place the site shows it must say so, so the
// design is never read as something a visitor can use today.
describe("Gespan Web Hub (coming soon)", () => {
  afterEach(cleanup);

  describe("the /hub page", () => {
    beforeEach(() => window.history.replaceState(null, "", "/hub"));

    test("opens from its own address with its own title, marked current in the nav", () => {
      render(<App />);
      expect(document.title).toBe("Web Hub concept (coming soon) · Gespan");
      expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("A concept: your whole team's agents, in one place.");
      const current = [...document.querySelectorAll('[aria-current="page"]')].map((a) => a.textContent);
      expect(current).toEqual(["Web Hub"]);
    });

    test("every section that shows the hub says Coming soon", () => {
      render(<App />);
      const sections = [...document.querySelectorAll("main section")].filter((s) => !s.closest("footer") && s.querySelector("h1, h2"));
      // The hero, the five feature sections and the closing call to action.
      expect(sections.length).toBe(7);
      for (const section of sections) {
        const heading = section.querySelector("h1, h2")?.textContent;
        expect(within(section as HTMLElement).queryAllByText("Coming soon").length, `${heading} has no Coming soon tag`).toBeGreaterThan(0);
        expect(within(section as HTMLElement).queryAllByText("Concept").length, `${heading} has no Concept tag`).toBeGreaterThan(0);
      }
    });

    test("the copy says it is a concept and never promises it", () => {
      render(<App />);
      const text = document.querySelector("main")!.textContent!;
      expect(text).toContain("Work on it hasn't started");
      for (const promise of [/\bwill\b/i, /building toward/i, /when the Web Hub is ready/i, /\bnext step\b/i]) expect(promise.test(text), `${promise}`).toBe(false);
    });

    test("offers no way to sign in or download a hub, only the desktop app and release updates", () => {
      render(<App />);
      const hrefs = [...document.querySelectorAll("main a")].map((a) => a.getAttribute("href"));
      expect(hrefs).not.toContain("https://hub.example.com");
      expect(hrefs).toContain("/download");
      expect(hrefs).toContain("https://github.com/0xvasanth/gespan/releases");
      expect(screen.queryByRole("button", { name: /sign in/i })).toBeNull();
    });
  });

  test("Home has a Coming soon teaser that links to /hub", () => {
    window.history.replaceState(null, "", "/");
    render(<App />);
    const teaser = screen.getByRole("heading", { name: "A concept: your whole team's agents, in one place." }).closest("section") as HTMLElement;
    expect(within(teaser).getByText("Coming soon")).toBeTruthy();
    expect(within(teaser).getByText("Concept")).toBeTruthy();
    expect(within(teaser).getByRole("link", { name: "See the concept" }).getAttribute("href")).toBe("/hub");
  });
});
