import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { App } from "../src/App";
import { shortcutKeys } from "../src/pages/DocsPage";

describe("the Docs search shortcut", () => {
  test("names the keys the visitor's keyboard has: ⌘ on Apple devices, Ctrl everywhere else", () => {
    expect(shortcutKeys("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15")).toEqual(["⌘", "K"]);
    expect(shortcutKeys("Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X)")).toEqual(["⌘", "K"]);
    expect(shortcutKeys("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/131.0")).toEqual(["Ctrl", "K"]);
    expect(shortcutKeys("Mozilla/5.0 (X11; Linux x86_64) Firefox/133.0")).toEqual(["Ctrl", "K"]);
  });

  describe("on /docs", () => {
    beforeEach(() => window.history.replaceState(null, "", "/docs"));
    afterEach(cleanup);

    test("shows the hint as two keys while the field is empty, and steps aside once you type", () => {
      render(<App />);
      const input = screen.getByRole("searchbox", { name: "Search the guides" });
      expect(input.getAttribute("aria-keyshortcuts")).toBe("Meta+K Control+K");
      const keys = () => [...document.querySelectorAll(".dc-keys kbd")].map((k) => k.textContent);
      expect(keys()).toEqual(shortcutKeys(navigator.userAgent));
      act(() => {
        fireEvent.change(input, { target: { value: "jira" } });
      });
      expect(document.querySelector(".dc-keys")).toBeNull();
      act(() => {
        fireEvent.change(input, { target: { value: "" } });
      });
      expect(keys()).toHaveLength(2);
    });

    test("Ctrl+K or ⌘K puts the cursor in the search field", () => {
      render(<App />);
      const input = screen.getByRole("searchbox", { name: "Search the guides" });
      for (const modifier of [{ metaKey: true }, { ctrlKey: true }]) {
        (document.activeElement as HTMLElement | null)?.blur();
        act(() => {
          fireEvent.keyDown(window, { key: "k", ...modifier });
        });
        expect(document.activeElement).toBe(input);
      }
    });
  });
});
