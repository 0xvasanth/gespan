// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { SearchableProjectPicker } from "./SearchableProjectPicker";

let root: Root;
let container: HTMLElement;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const stored = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => stored.set(key, value),
    removeItem: (key: string) => stored.delete(key),
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

function render(activeId?: string) {
  const onSelectProject = vi.fn();
  const onOpen = vi.fn();
  act(() =>
    root.render(
      createElement(SearchableProjectPicker, {
        cwd: "/code/app",
        recents: [{ path: "/code/app", openedAt: 1 }],
        compact: true,
        onSelectProject,
        pilots: {
          items: [{ id: "m1", name: "PilotCat", mascot: "cat", color: "#fff" }],
          activeId,
          onOpen,
        },
      }),
    ),
  );
  const trigger = container.querySelector("button")!;
  act(() => trigger.click());
  return { trigger, onSelectProject, onOpen };
}

it("lists pilots above the projects and opens one", () => {
  const { trigger, onOpen } = render();
  expect(trigger.getAttribute("aria-label")).toContain("current project app");
  const pilot = document.body.querySelector<HTMLButtonElement>(
    '[data-picker-pilot="m1"]',
  )!;
  expect(pilot.textContent).toContain("PilotCat");
  act(() => pilot.click());
  expect(onOpen).toHaveBeenCalledWith("m1");
});

it("shows the open pilot, and lets the current project lead back out of it", () => {
  const { trigger, onSelectProject } = render("m1");
  expect(trigger.getAttribute("aria-label")).toContain("current pilot PilotCat");
  const project = [...document.body.querySelectorAll("button")].find(
    (button) => button.title === "/code/app",
  )!;
  act(() => project.click());
  expect(onSelectProject).toHaveBeenCalledWith("/code/app");
});
