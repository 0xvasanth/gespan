import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { act, cleanup, render } from "@testing-library/react";
import { compile } from "tailwindcss";
import { afterEach, describe, expect, test, vi } from "vitest";
import { heroPose, risePose } from "../src/components/ScrollShot";
import { useReveal } from "../src/reveal";

describe("scroll poses", () => {
  test("the hero shot starts tilted back and ends upright at full size", () => {
    expect(heroPose(0)).toEqual({ rotateX: 26, scale: 0.86, y: 70, opacity: 0.55 });
    expect(heroPose(1)).toEqual({ rotateX: 0, scale: 1, y: 0, opacity: 1 });
  });

  test("each step of scrolling only ever straightens and grows the shot", () => {
    for (const pose of [heroPose, risePose]) {
      let last = pose(0);
      for (let p = 0.05; p <= 1.0001; p += 0.05) {
        const next = pose(p);
        expect(next.rotateX).toBeLessThanOrEqual(last.rotateX);
        expect(next.scale).toBeGreaterThanOrEqual(last.scale);
        expect(next.opacity).toBeGreaterThanOrEqual(last.opacity);
        last = next;
      }
    }
  });

  test("progress outside 0…1, or not a number, rests at the ends instead of overshooting", () => {
    expect(heroPose(-3)).toEqual(heroPose(0));
    expect(heroPose(7)).toEqual(heroPose(1));
    expect(risePose(Number.NaN)).toEqual(risePose(1));
  });
});

function Page() {
  useReveal("/");
  return (
    <div>
      <h2 data-reveal="">Head</h2>
      <ul data-stagger="">
        <li>One</li>
      </ul>
    </div>
  );
}

describe("useReveal", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    document.documentElement.classList.remove("reveal-ready");
  });

  function stubObserver() {
    const observed: Element[] = [];
    let callback: IntersectionObserverCallback = () => {};
    vi.stubGlobal(
      "IntersectionObserver",
      class {
        constructor(cb: IntersectionObserverCallback) {
          callback = cb;
        }
        observe(el: Element) {
          observed.push(el);
        }
        unobserve() {}
        disconnect() {}
      },
    );
    const enter = (el: Element) => act(() => callback([{ target: el, isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver));
    return { observed, enter };
  }
  const reducedMotion = (reduce: boolean) => vi.stubGlobal("matchMedia", (query: string) => ({ matches: reduce && query.includes("reduce"), media: query }));

  test("a section is shown when it scrolls into view, not before", () => {
    reducedMotion(false);
    const { observed, enter } = stubObserver();
    const { getByText } = render(<Page />);
    const head = getByText("Head");
    expect(observed).toContain(head);
    expect(head.hasAttribute("data-shown")).toBe(false);
    expect(document.documentElement.classList.contains("reveal-ready")).toBe(true);
    enter(head);
    expect(head.hasAttribute("data-shown")).toBe(true);
  });

  test("with reduced motion everything is shown at once and nothing waits on scrolling", () => {
    reducedMotion(true);
    const { observed } = stubObserver();
    const { getByText } = render(<Page />);
    expect(getByText("Head").hasAttribute("data-shown")).toBe(true);
    expect(getByText("One").parentElement!.hasAttribute("data-shown")).toBe(true);
    expect(observed).toEqual([]);
    expect(document.documentElement.classList.contains("reveal-ready")).toBe(false);
  });

  test("a browser without IntersectionObserver never hides content", () => {
    reducedMotion(false);
    vi.stubGlobal("IntersectionObserver", undefined);
    const { getByText } = render(<Page />);
    expect(getByText("Head").hasAttribute("data-shown")).toBe(true);
    expect(document.documentElement.classList.contains("reveal-ready")).toBe(false);
  });
});

describe("motion.css", () => {
  const site = join(dirname(fileURLToPath(import.meta.url)), "..");
  const css = readFileSync(join(site, "src/styles/motion.css"), "utf8");

  test("content is only hidden for a reveal when motion is welcome", () => {
    const guard = css.indexOf("@media (prefers-reduced-motion: no-preference)");
    // The one rule that hides: scoped to the running watcher, inside the guard.
    const rule = css.indexOf(".reveal-ready [data-reveal]:not([data-shown])");
    expect(guard).toBeGreaterThan(-1);
    expect(rule).toBeGreaterThan(guard);
    expect(css.slice(rule, css.indexOf("}", rule))).toContain("opacity: 0;");
    expect(css.match(/:not\(\[data-shown\]\)/g)).toHaveLength(2);
  });

  test("Tailwind can't restyle the site: no preflight reset, and everything in layers", () => {
    expect(css).toMatch(/@import "tailwindcss\/theme\.css" layer\(theme\);/);
    expect(css).toMatch(/@import "tailwindcss\/utilities\.css" layer\(utilities\);/);
    expect(css).not.toMatch(/@import "tailwindcss(\/preflight\.css)?"/);
  });

  test("no class the site defines is also a Tailwind utility", async () => {
    // Tailwind emits a utility for any class name it recognises in the source.
    // A site class with a utility's name (w-xl once was: Tailwind's width)
    // would get Tailwind's styles on top of its own.
    const files: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const path = join(dir, name);
        if (statSync(path).isDirectory()) walk(path);
        else if (path.endsWith(".css") && !path.endsWith("motion.css")) files.push(path);
      }
    };
    walk(join(site, "src"));
    const own = new Set<string>();
    for (const file of files) for (const match of readFileSync(file, "utf8").matchAll(/\.([a-zA-Z][\w-]*)/g)) own.add(match[1]);
    expect(own.size).toBeGreaterThan(100);
    const tw = await compile('@import "tailwindcss";', {
      base: site,
      loadStylesheet: async (id: string, base: string) => {
        const file = join(site, "node_modules", id === "tailwindcss" ? "tailwindcss/index.css" : id);
        return { path: file, base, content: readFileSync(file, "utf8") };
      },
    });
    const generated = [...tw.build([...own]).matchAll(/^\s*\.([\w-]+)\s*\{/gm)].map((m) => m[1]);
    expect(generated.filter((name) => own.has(name))).toEqual([]);
  });
});
