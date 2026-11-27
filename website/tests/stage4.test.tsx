import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, test, vi } from "vitest";
import { App } from "../src/App";
import { lineDelay, QUICK_START } from "../src/components/Terminal";
import { litReleases } from "../src/pages/ChangelogPage";
import { changelog } from "../src/release";

const repo = join(dirname(fileURLToPath(import.meta.url)), "../..");
const docs = ["docs/providers.md", "docs/install.md"].map((f) => readFileSync(join(repo, f), "utf8")).join("\n");

afterEach(cleanup);

describe("litReleases", () => {
  const dots = [0, 0.3, 0.6, 0.9];
  test("lights every release the beam has reached, in order", () => {
    expect(litReleases(0, dots)).toBe(1);
    expect(litReleases(0.29, dots)).toBe(1);
    expect(litReleases(0.3, dots)).toBe(2);
    expect(litReleases(0.75, dots)).toBe(3);
    expect(litReleases(1, dots)).toBe(4);
  });

  test("never lights more as the beam goes back, and rests on the ends out of range", () => {
    let last = Infinity;
    for (let p = 1; p >= 0; p -= 0.05) {
      const lit = litReleases(p, dots);
      expect(lit).toBeLessThanOrEqual(last);
      last = lit;
    }
    expect(litReleases(-1, dots)).toBe(1);
    expect(litReleases(9, dots)).toBe(4);
    expect(litReleases(Number.NaN, dots)).toBe(1);
    expect(litReleases(0.5, [])).toBe(0);
  });
});

describe("the Changelog timeline", () => {
  test("puts one dot on the rail per release, the newest lit from the start", () => {
    window.history.replaceState(null, "", "/changelog");
    render(<App />);
    const timeline = document.querySelector("[data-timeline]")!;
    const releases = [...timeline.querySelectorAll("article.cl-rel")];
    expect(releases).toHaveLength(changelog.length);
    expect(releases.map((r) => r.querySelector(".cl-dot")!.classList.contains("lit"))).toEqual(changelog.map((_, i) => i === 0));
    expect(timeline.querySelector(".cl-track")!.getAttribute("aria-hidden")).toBe("true");
    // The rail is decoration: release headings stay the outline screen readers use.
    expect(releases.map((r) => r.querySelector("h2")!.textContent)).toEqual(changelog.map((r) => r.version));
  });
});

describe("the Docs quick start terminal", () => {
  test("only runs commands the docs give", () => {
    const cmds = QUICK_START.filter((l) => l.kind === "cmd").map((l) => l.text);
    expect(cmds.length).toBeGreaterThan(0);
    for (const cmd of cmds) expect(docs, `${cmd} is not in the docs`).toContain(cmd);
    for (const alt of QUICK_START.flatMap((l) => (l.kind === "cmd" && l.alt?.startsWith("or ") ? l.alt.slice(3).split(", ") : []))) expect(docs, `${alt} is not in the docs`).toContain(alt);
  });

  test("brings its lines in one by one, or all at once for reduced motion", () => {
    for (let i = 1; i < QUICK_START.length; i += 1) expect(lineDelay(i, false)).toBeGreaterThan(lineDelay(i - 1, false));
    expect(QUICK_START.map((_, i) => lineDelay(i, true))).toEqual(QUICK_START.map(() => 0));
  });

  test("sits at the top of the quick start, with a copy button per command that copies exactly it", async () => {
    window.history.replaceState(null, "", "/docs");
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    render(<App />);
    const terminal = document.querySelector("[data-terminal]")!;
    expect(terminal.closest("section")!.getAttribute("aria-label")).toBe("Start here");
    const copy = screen.getByRole("button", { name: "Copy claude auth login" });
    expect(terminal.querySelectorAll(".tm-copy")).toHaveLength(QUICK_START.filter((l) => l.kind === "cmd").length);
    await act(async () => {
      fireEvent.click(copy);
    });
    expect(writeText).toHaveBeenCalledWith("claude auth login");
    expect(screen.getByRole("button", { name: "Copied claude auth login" }).textContent).toBe("Copied");
    // The in-app steps follow the terminal.
    expect(screen.getByRole("heading", { name: "Then, in Gespan" })).toBeTruthy();
  });
});
