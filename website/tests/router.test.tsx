import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { App } from "../src/App";
import { matchRoute } from "../src/router";

describe("matchRoute", () => {
  test.each([
    ["/", "home"],
    ["/features", "features"],
    ["/features/", "features"],
    ["/pilots/index.html", "pilots"],
    ["/cloud", "cloud"],
    ["/hub", "cloud"],
    ["/index.html", "home"],
    ["/download", "download"],
  ])("%s is the %s page", (path, page) => {
    expect(matchRoute(path)?.page).toBe(page);
  });

  test.each(["/feature", "/FEATURES", "/docs/board", "/download.html"])("%s matches no page", (path) => {
    expect(matchRoute(path)).toBeNull();
  });
});

describe("navigation", () => {
  beforeEach(() => window.history.replaceState(null, "", "/"));
  afterEach(cleanup);

  const navLink = (name: string) => screen.getAllByRole("navigation", { name: "Site" })[0].querySelector(`a[href="/${name.toLowerCase()}"]`) as HTMLAnchorElement;

  test("a nav link opens its page without a reload and marks itself current", () => {
    render(<App />);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toContain("Hold the reins.");
    act(() => {
      fireEvent.click(navLink("Pilots"));
    });
    expect(window.location.pathname).toBe("/pilots");
    expect(document.title).toBe("Pilots · Gespan");
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("A Pilot keeps the work moving.");
    const current = [...document.querySelectorAll('[aria-current="page"]')].map((a) => a.textContent);
    expect(current).toEqual(["Pilots"]);
  });

  test("Back returns to the previous page", () => {
    render(<App />);
    act(() => {
      fireEvent.click(navLink("Docs"));
    });
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Learn Gespan.");
    act(() => {
      window.history.back();
    });
    // jsdom delivers popstate asynchronously.
    return new Promise<void>((resolve) =>
      setTimeout(() => {
        expect(window.location.pathname).toBe("/");
        expect(screen.getByRole("heading", { level: 1 }).textContent).toContain("Hold the reins.");
        resolve();
      }, 20),
    );
  });

  test("a modified click is left to the browser (new tab)", () => {
    render(<App />);
    const link = navLink("Features");
    const event = new MouseEvent("click", { bubbles: true, cancelable: true, metaKey: true });
    link.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
    expect(window.location.pathname).toBe("/");
  });

  test("an unknown path shows the not-found page with a way home", () => {
    window.history.replaceState(null, "", "/nope");
    render(<App />);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("This page isn't here.");
    expect(document.title).toBe("Page not found · Gespan");
    expect(screen.getByRole("link", { name: "Back to Gespan" }).getAttribute("href")).toBe("/");
  });

  test("the phone menu opens, lists every page and closes after a pick", () => {
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Open menu" }));
    const sheet = document.getElementById("nv-sheet")!;
    expect([...sheet.querySelectorAll("a")].map((a) => a.textContent)).toEqual(["Features", "Pilots", "Cloud", "Docs", "Changelog", "Download", "GitHub"]);
    act(() => {
      fireEvent.click(sheet.querySelector('a[href="/changelog"]')!);
    });
    expect(window.location.pathname).toBe("/changelog");
    expect(document.getElementById("nv-sheet")).toBeNull();
  });
});
